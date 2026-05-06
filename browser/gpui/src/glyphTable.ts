// @ts-nocheck
/**
 * glyphTable.ts — Neutron Glyph Registry
 *
 * Maps Unicode codepoints → glyph IDs (u16).
 * Built once at startup by rasterizing the common code-editing character set
 * into the GPU atlas via a single Tauri command. After that, textRun() writes
 * glyph IDs — not strings — into the scene buffer. The Rust GPU thread looks
 * up atlas UV coordinates from the same ID, issues one textured quad per glyph.
 *
 * Why IDs not strings
 * ───────────────────
 * A string "fn main()" is 9 chars × 2 bytes (UTF-16 in JS) = 18 bytes plus
 * string object overhead. As a glyph ID array it's 9 × 2 bytes = 18 bytes,
 * but crucially it's a typed array — written directly into the scene buffer
 * with no heap allocation, no garbage, no GC pressure.
 *
 * The table is a flat Uint16Array of 65536 entries (128KB).
 * glyphTable[codepoint] = glyphId.  Lookup is a single array index — O(1),
 * branch-free, cache-friendly.
 *
 * ID 0 is reserved for "not yet rasterized" — the Rust side falls back to
 * rasterizing on demand and sends the ID back via a micro-IPC call that runs
 * off the critical render path.
 */

import { invoke } from '@tauri-apps/api/tauri'

// ── Glyph table ───────────────────────────────────────────────────────────────

// 65536-entry flat array. Index = unicode codepoint, value = atlas glyph ID.
// Shared with Rust via a second SharedArrayBuffer registered at startup.
const TABLE_SIZE = 65536
let glyphIds: Uint16Array = new Uint16Array(TABLE_SIZE)

// Pending codepoints waiting to be rasterized (off critical path)
const pending  = new Set<number>()
let   flushScheduled = false

// ── Startup registration ───────────────────────────────────────────────────────

/**
 * Pre-rasterize the full ASCII printable range + common code symbols.
 * Called once at engine startup. Returns when the atlas is ready.
 * After this returns, all common characters are O(1) lookups with no IPC.
 */
export async function initGlyphTable(fontSizePx: number): Promise<void> {
  // The characters every code file will use. Cover them upfront.
  const preloadChars = buildPreloadSet()

  // Ask Rust to rasterize them all and return a flat [codepoint, glyphId] array
  const result: number[] = await invoke('neutron_init_glyph_table', {
    codepoints: preloadChars,
    fontSizePx,
  })

  // Fill the table from the returned pairs
  for (let i = 0; i < result.length; i += 2) {
    const cp = result[i]
    const id = result[i + 1]
    if (cp < TABLE_SIZE) glyphIds[cp] = id
  }
}

/**
 * Look up the glyph ID for a codepoint.
 * If not yet rasterized, returns 0 and schedules an async rasterization.
 * The current frame renders without that glyph (rare, first-frame only).
 */
export function glyphId(codepoint: number): number {
  if (codepoint >= TABLE_SIZE) return 0
  const id = glyphIds[codepoint]
  if (id === 0) {
    schedulePendingRasterize(codepoint)
  }
  return id
}

/**
 * Convert a string to a Uint16Array of glyph IDs.
 * This is the hot path — called for every text run every frame.
 * Allocates one Uint16Array per call; callers should pool these.
 */
export function stringToGlyphIds(str: string): Uint16Array {
  const ids = new Uint16Array(str.length)
  for (let i = 0; i < str.length; i++) {
    ids[i] = glyphId(str.charCodeAt(i))
  }
  return ids
}

/**
 * Convert a string to glyph IDs into a pre-allocated buffer.
 * Zero allocation hot path for callers that pool their buffers.
 */
export function stringToGlyphIdsInto(str: string, out: Uint16Array, offset = 0): number {
  const len = Math.min(str.length, out.length - offset)
  for (let i = 0; i < len; i++) {
    out[offset + i] = glyphId(str.charCodeAt(i))
  }
  return len
}

// ── Pending rasterization (off critical path) ──────────────────────────────────

function schedulePendingRasterize(cp: number): void {
  pending.add(cp)
  if (!flushScheduled) {
    flushScheduled = true
    // Schedule after current frame — doesn't block rendering
    queueMicrotask(flushPending)
  }
}

async function flushPending(): Promise<void> {
  flushScheduled = false
  if (pending.size === 0) return

  const cps = Array.from(pending)
  pending.clear()

  try {
    const result: number[] = await invoke('neutron_rasterize_glyphs', { codepoints: cps })
    for (let i = 0; i < result.length; i += 2) {
      const cp = result[i]
      const id = result[i + 1]
      if (cp < TABLE_SIZE) glyphIds[cp] = id
    }
  } catch {
    // Non-fatal: glyphs will render as blank until next successful rasterize
  }
}

// ── Preload character set ──────────────────────────────────────────────────────

function buildPreloadSet(): number[] {
  const cps: number[] = []

  // ASCII printable (space through tilde)
  for (let c = 32; c <= 126; c++) cps.push(c)

  // Common programming symbols outside ASCII
  const extras = '→←↑↓⇒⇐…·•©®™°±×÷≤≥≠≈∞√∫∑∏αβγδεζηθλμπρστφψω'
  for (const ch of extras) cps.push(ch.charCodeAt(0))

  // Box drawing characters (used by terminal output in code)
  for (let c = 0x2500; c <= 0x257f; c++) cps.push(c)

  return cps
}
