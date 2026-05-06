/**
 * scene.ts — Neutron Scene Graph
 *
 * The core primitive of the Neutron engine. A flat, typed binary buffer
 * that React writes render primitives into, and the Rust GPU thread reads
 * directly without serialization.
 *
 * How it works
 * ────────────
 * React's reconciler commits mutations to this buffer in a tight loop.
 * The buffer is a SharedArrayBuffer — the same physical memory pages
 * mapped into both the JS heap and the Rust process via Tauri's
 * custom IPC shim. No JSON. No copying. No bridge overhead.
 *
 * On every commit cycle:
 *   1. React calls scene.begin() — resets the write cursor
 *   2. Components call scene.rect(), scene.text(), scene.glyph() etc.
 *      Each writes a fixed-size binary primitive into the buffer
 *   3. React calls scene.commit() — atomically flips a generation counter
 *   4. The Rust GPU thread sees the counter change (via Atomics.wait on
 *      its side), reads the primitives, and issues one wgpu draw call
 *
 * Primitive layout (all little-endian):
 *   Each primitive is 64 bytes. Tag byte at offset 0 identifies the type.
 *   This fixed size means the GPU thread can index directly: ptr + i*64.
 *   No length-prefixed strings, no variable-width fields.
 *
 * Text primitives use a glyph ID (u32) looked up in a shared glyph table
 * that is built once at startup and never changes for the ASCII+common set.
 *
 * This is not a copy of GPUI. GPUI is Rust-first. Neutron is the first
 * engine that makes React the authoring layer with zero runtime cost.
 */

// ── Primitive tags ─────────────────────────────────────────────────────────────

export const TAG_RECT        = 1  // filled rectangle
export const TAG_BORDER      = 2  // rectangle outline
export const TAG_TEXT_RUN    = 3  // one line of glyphs with a single color
export const TAG_SHADOW      = 4  // box shadow (GPU-composited)
export const TAG_IMAGE       = 5  // atlas-backed image quad
export const TAG_CLIP_PUSH   = 6  // push scissor rect
export const TAG_CLIP_POP    = 7  // pop scissor rect
export const TAG_TRANSFORM   = 8  // push transform matrix
export const TAG_TRANSFORM_POP = 9

// ── Buffer sizing ──────────────────────────────────────────────────────────────

const PRIMITIVE_BYTES   = 64          // each primitive is exactly 64 bytes
const MAX_PRIMITIVES    = 16_384      // 16K primitives = 1MB buffer — enough for any frame
const HEADER_BYTES      = 16          // gen counter (u32) + primitive count (u32) + reserved
const BUFFER_BYTES      = HEADER_BYTES + MAX_PRIMITIVES * PRIMITIVE_BYTES

// Header offsets (byte offsets into SharedArrayBuffer)
const OFF_GEN           = 0   // u32: generation counter, flipped on every commit
const OFF_COUNT         = 4   // u32: number of valid primitives in this frame
const OFF_FLAGS         = 8   // u32: reserved flags
const OFF_RESERVED      = 12  // u32: padding

// ── SceneBuffer ───────────────────────────────────────────────────────────────

/**
 * The shared memory buffer that forms the contract between React and the GPU.
 * One instance per window, created at startup, lives for the process lifetime.
 */
export class SceneBuffer {
  /** The raw shared memory — same pages the Rust thread reads */
  readonly sab:    SharedArrayBuffer
  /** Typed views into the buffer for efficient writes */
  private u8:     Uint8Array
  private u32:    Uint32Array
  private f32:    Float32Array
  /** Write cursor — byte offset of next primitive slot */
  private cursor: number = HEADER_BYTES
  /** Generation counter — incremented on each commit */
  private gen:    number = 0

  constructor() {
    this.sab  = new SharedArrayBuffer(BUFFER_BYTES)
    this.u8   = new Uint8Array(this.sab)
    this.u32  = new Uint32Array(this.sab)
    this.f32  = new Float32Array(this.sab)
  }

  /** Reset write cursor. Call at the start of every React render commit. */
  begin(): void {
    this.cursor = HEADER_BYTES
  }

  /**
   * Commit the current frame — atomically publishes it to the GPU thread.
   * The GPU thread spins on the generation counter via Atomics.load;
   * when it sees a new value it reads the primitives and draws.
   */
  commit(): void {
    const count = (this.cursor - HEADER_BYTES) / PRIMITIVE_BYTES
    // Write count first, then generation (GPU thread checks gen last)
    Atomics.store(this.u32, OFF_COUNT >> 2, count)
    this.gen = (this.gen + 1) & 0x7fffffff
    Atomics.store(this.u32, OFF_GEN >> 2, this.gen)
  }

  /** Current number of primitives written this frame */
  get primitiveCount(): number {
    return (this.cursor - HEADER_BYTES) / PRIMITIVE_BYTES
  }

  /** True if buffer is full — callers should flush early */
  get full(): boolean {
    return this.cursor + PRIMITIVE_BYTES > BUFFER_BYTES
  }

  // ── Primitive writers ──────────────────────────────────────────────────────
  // Each writes exactly 64 bytes at the current cursor and advances it.
  // Layout is documented per-primitive below.

  /**
   * Filled rectangle.
   * Bytes: [0] tag, [1-3] pad, [4-7] x f32, [8-11] y f32,
   *        [12-15] w f32, [16-19] h f32,
   *        [20-23] color u32 (0xRRGGBBAA),
   *        [24-27] corner_radius f32, [28-63] reserved
   */
  rect(x: number, y: number, w: number, h: number, color: number, radius = 0): void {
    if (this.full) return
    const base = this.cursor
    this.u8[base]     = TAG_RECT
    this.writeF32(base + 4,  x)
    this.writeF32(base + 8,  y)
    this.writeF32(base + 12, w)
    this.writeF32(base + 16, h)
    this.writeU32(base + 20, color)
    this.writeF32(base + 24, radius)
    this.cursor += PRIMITIVE_BYTES
  }

  /**
   * Rectangle border (outline only).
   * Same layout as rect plus [28-31] border_width f32.
   */
  border(x: number, y: number, w: number, h: number, color: number, width: number, radius = 0): void {
    if (this.full) return
    const base = this.cursor
    this.u8[base]     = TAG_BORDER
    this.writeF32(base + 4,  x)
    this.writeF32(base + 8,  y)
    this.writeF32(base + 12, w)
    this.writeF32(base + 16, h)
    this.writeU32(base + 20, color)
    this.writeF32(base + 24, radius)
    this.writeF32(base + 28, width)
    this.cursor += PRIMITIVE_BYTES
  }

  /**
   * A single styled text run — one line, one color, one font size.
   * Text content is written as a compact glyph ID array (u16 per glyph).
   * Max 20 glyphs per primitive; longer runs are split by the caller.
   *
   * Bytes: [0] tag, [1] glyph_count u8, [2-3] pad,
   *        [4-7] x f32, [8-11] y f32,
   *        [12-15] color u32, [16-17] font_size u16 (tenths of px),
   *        [18-19] pad,
   *        [20-59] glyph_ids u16[20] (packed glyph atlas IDs)
   */
  textRun(
    x: number, y: number,
    color: number, fontSize: number,
    glyphIds: Uint16Array,
  ): void {
    if (this.full) return
    const count = Math.min(glyphIds.length, 20)
    const base  = this.cursor
    this.u8[base]     = TAG_TEXT_RUN
    this.u8[base + 1] = count
    this.writeF32(base + 4,  x)
    this.writeF32(base + 8,  y)
    this.writeU32(base + 12, color)
    this.writeU16(base + 16, Math.round(fontSize * 10))
    // Write glyph IDs packed from offset 20
    for (let i = 0; i < count; i++) {
      this.writeU16(base + 20 + i * 2, glyphIds[i])
    }
    this.cursor += PRIMITIVE_BYTES
  }

  /**
   * Box shadow.
   * [0] tag, [4-7] x, [8-11] y, [12-15] w, [16-19] h,
   * [20-23] color, [24-27] blur_radius, [28-31] spread, [32-35] offset_x, [36-39] offset_y
   */
  shadow(
    x: number, y: number, w: number, h: number,
    color: number, blur: number, spread = 0, offX = 0, offY = 0,
  ): void {
    if (this.full) return
    const base = this.cursor
    this.u8[base]     = TAG_SHADOW
    this.writeF32(base + 4,  x)
    this.writeF32(base + 8,  y)
    this.writeF32(base + 12, w)
    this.writeF32(base + 16, h)
    this.writeU32(base + 20, color)
    this.writeF32(base + 24, blur)
    this.writeF32(base + 28, spread)
    this.writeF32(base + 32, offX)
    this.writeF32(base + 36, offY)
    this.cursor += PRIMITIVE_BYTES
  }

  /** Push a scissor clip rectangle. All subsequent primitives are clipped. */
  clipPush(x: number, y: number, w: number, h: number): void {
    if (this.full) return
    const base = this.cursor
    this.u8[base] = TAG_CLIP_PUSH
    this.writeF32(base + 4,  x)
    this.writeF32(base + 8,  y)
    this.writeF32(base + 12, w)
    this.writeF32(base + 16, h)
    this.cursor += PRIMITIVE_BYTES
  }

  /** Pop the most recent scissor clip. */
  clipPop(): void {
    if (this.full) return
    this.u8[this.cursor] = TAG_CLIP_POP
    this.cursor += PRIMITIVE_BYTES
  }

  // ── Private write helpers ──────────────────────────────────────────────────

  private writeF32(byteOffset: number, value: number): void {
    this.f32[byteOffset >> 2] = value
  }

  private writeU32(byteOffset: number, value: number): void {
    this.u32[byteOffset >> 2] = value >>> 0
  }

  private writeU16(byteOffset: number, value: number): void {
    this.u8[byteOffset]     = value & 0xff
    this.u8[byteOffset + 1] = (value >> 8) & 0xff
  }
}

// ── Singleton ─────────────────────────────────────────────────────────────────

let _scene: SceneBuffer | null = null

/** Get or create the process-global scene buffer. */
export function getScene(): SceneBuffer {
  if (!_scene) _scene = new SceneBuffer()
  return _scene
}
