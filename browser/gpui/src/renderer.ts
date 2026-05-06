// @ts-nocheck
/**
 * renderer.ts — Neutron React Renderer
 *
 * A custom React reconciler that targets the Neutron scene buffer instead of
 * the DOM. React components author UI using Neutron primitives; the reconciler
 * commits mutations directly into the SharedArrayBuffer scene graph. The Rust
 * GPU thread consumes that buffer. No DOM. No layout engine. No compositor.
 * No IPC on the render path.
 *
 * This is the same model as React Native (which targets native views) and
 * React Three Fiber (which targets Three.js). Neutron targets wgpu.
 *
 * Host types
 * ──────────
 * In DOM React, "host" types are div, span, canvas etc.
 * In Neutron, host types are the Neutron primitives:
 *   <n-rect>, <n-text>, <n-border>, <n-shadow>, <n-clip>
 *
 * These are registered as custom JSX elements. React components compose them
 * exactly like DOM elements. The reconciler's createInstance / commitUpdate
 * calls write the appropriate SceneBuffer primitive instead of creating DOM nodes.
 *
 * Two-renderer architecture
 * ─────────────────────────
 * Neutron runs alongside the existing DOM renderer, not replacing it.
 * The DOM renderer still owns the WebView layer (sidebar, modals, status bar).
 * Neutron owns only the surfaces registered with registerSurface():
 *   - The editor canvas (the main canvas that replaces Monaco's paint layer)
 *   - The terminal canvas
 *   - Any future GPU-native surface
 *
 * Components inside a <NeutronRoot> render via this renderer.
 * Components outside render normally via ReactDOM. They can coexist.
 *
 * Reconciler contract
 * ───────────────────
 * React calls these in order every commit:
 *   prepareForCommit() → createInstance() × N → appendChildToContainer()
 *   → commitMount() → finalizeInitialChildren() → commitUpdate()
 *
 * Our implementations write to the scene buffer. At finalizeInitialChildren /
 * commitUpdate, we call scene.commit() to publish the frame.
 */

import ReactReconciler from 'react-reconciler'
import { DefaultEventPriority } from 'react-reconciler/constants'
import { getScene, SceneBuffer } from './scene'
import { stringToGlyphIds } from './glyphTable'

// ── Host element types ─────────────────────────────────────────────────────────

type NeutronType =
  | 'n-rect'
  | 'n-text'
  | 'n-border'
  | 'n-shadow'
  | 'n-clip'

interface BaseProps {
  x?: number
  y?: number
  width?: number
  height?: number
  color?: number       // 0xRRGGBBAA
  radius?: number
}

interface RectProps    extends BaseProps {}
interface BorderProps  extends BaseProps { borderWidth?: number }
interface TextProps    extends BaseProps { text: string; fontSize?: number }
interface ShadowProps  extends BaseProps { blur?: number; spread?: number; offsetX?: number; offsetY?: number }
interface ClipProps    extends BaseProps { children?: any }

type NeutronProps = RectProps | BorderProps | TextProps | ShadowProps | ClipProps

// ── Host instance ──────────────────────────────────────────────────────────────

interface NeutronInstance {
  type:     NeutronType
  props:    NeutronProps
  children: NeutronInstance[]
}

// ── Scene writer ───────────────────────────────────────────────────────────────

function writeInstance(scene: SceneBuffer, inst: NeutronInstance): void {
  const p = inst.props
  const x = p.x ?? 0
  const y = p.y ?? 0
  const w = p.width  ?? 0
  const h = p.height ?? 0
  const c = p.color  ?? 0xffffffff

  switch (inst.type) {
    case 'n-rect':
      scene.rect(x, y, w, h, c, (p as RectProps).radius ?? 0)
      break

    case 'n-border': {
      const bp = p as BorderProps
      scene.border(x, y, w, h, c, bp.borderWidth ?? 1, bp.radius ?? 0)
      break
    }

    case 'n-text': {
      const tp = p as TextProps
      const ids = stringToGlyphIds(tp.text ?? '')
      // Split long runs into 20-glyph chunks (scene primitive max)
      let offset = 0
      let glyphX = x
      const fontSize = tp.fontSize ?? 14
      const glyphW   = fontSize * 0.6  // monospace approximation; Rust uses exact metrics
      while (offset < ids.length) {
        const chunk = ids.slice(offset, offset + 20)
        scene.textRun(glyphX, y, c, fontSize, chunk)
        glyphX  += chunk.length * glyphW
        offset  += 20
      }
      break
    }

    case 'n-shadow': {
      const sp = p as ShadowProps
      scene.shadow(x, y, w, h, c, sp.blur ?? 8, sp.spread ?? 0, sp.offsetX ?? 0, sp.offsetY ?? 0)
      break
    }

    case 'n-clip':
      scene.clipPush(x, y, w, h)
      for (const child of inst.children) writeInstance(scene, child)
      scene.clipPop()
      return  // children already written above, skip default child walk
  }

  // Write children (except n-clip which handled them above)
  for (const child of inst.children) {
    writeInstance(scene, child)
  }
}

// ── Reconciler host config ─────────────────────────────────────────────────────

const hostConfig: ReactReconciler.HostConfig<
  NeutronType,        // Type
  NeutronProps,       // Props
  SceneBuffer,        // Container
  NeutronInstance,    // Instance
  never,              // TextInstance (no text nodes — use n-text)
  never,              // SuspenseInstance
  never,              // HydratableInstance
  NeutronInstance,    // PublicInstance
  {},                 // HostContext
  boolean,            // UpdatePayload (true = needs repaint)
  never,              // ChildSet
  number,             // TimeoutHandle
  number              // NoTimeout
> = {

  // ── Instance creation ──────────────────────────────────────────────────────

  createInstance(type, props): NeutronInstance {
    return { type, props, children: [] }
  },

  createTextInstance(): never {
    throw new Error('Neutron: use <n-text text="…"> instead of raw text nodes')
  },

  appendInitialChild(parent: NeutronInstance, child: NeutronInstance) {
    parent.children.push(child)
  },

  appendChild(parent: NeutronInstance, child: NeutronInstance) {
    parent.children.push(child)
  },

  appendChildToContainer(container: SceneBuffer, child: NeutronInstance) {
    // Root-level child — write it into the scene
    writeInstance(container, child)
  },

  insertBefore(parent: NeutronInstance, child: NeutronInstance, before: NeutronInstance) {
    const idx = parent.children.indexOf(before)
    if (idx >= 0) parent.children.splice(idx, 0, child)
    else parent.children.push(child)
  },

  removeChild(parent: NeutronInstance, child: NeutronInstance) {
    const idx = parent.children.indexOf(child)
    if (idx >= 0) parent.children.splice(idx, 1)
  },

  removeChildFromContainer(container: SceneBuffer, child: NeutronInstance) {
    // Scene buffer is rewritten each frame — removal is implicit on next begin()
  },

  // ── Updates ────────────────────────────────────────────────────────────────

  prepareUpdate(_inst, _type, oldProps, newProps): boolean {
    // Return true if any prop changed — drives commitUpdate
    return oldProps !== newProps
  },

  commitUpdate(inst: NeutronInstance, _payload, _type, _old, newProps) {
    inst.props = newProps
  },

  // ── Commit phase ───────────────────────────────────────────────────────────

  prepareForCommit(container: SceneBuffer) {
    container.begin()
    return null
  },

  resetAfterCommit(container: SceneBuffer) {
    // All primitives written during this commit cycle — publish the frame
    container.commit()
  },

  finalizeInitialChildren() { return false },

  // ── Container ──────────────────────────────────────────────────────────────

  getRootHostContext()    { return {} },
  getChildHostContext()   { return {} },
  getPublicInstance(inst) { return inst },

  // ── Text (disabled — use n-text) ───────────────────────────────────────────

  shouldSetTextContent() { return false },

  // ── Time / scheduling ──────────────────────────────────────────────────────

  scheduleTimeout:  setTimeout,
  cancelTimeout:    clearTimeout,
  noTimeout:        -1,
  isPrimaryRenderer: false,   // coexists with ReactDOM

  getCurrentEventPriority() { return DefaultEventPriority },
  getInstanceFromNode()     { return null },
  beforeActiveInstanceBlur()  {},
  afterActiveInstanceBlur()   {},
  prepareScopeUpdate()        {},
  getInstanceFromScope()      { return null },
  detachDeletedInstance()     {},

  // ── Mutations ──────────────────────────────────────────────────────────────

  supportsMutation:      true,
  supportsPersistence:   false,
  supportsHydration:     false,
}

// ── Reconciler instance ────────────────────────────────────────────────────────

export const NeutronReconciler = ReactReconciler(hostConfig)

// ── NeutronRoot ────────────────────────────────────────────────────────────────

/**
 * Mount a React tree into a Neutron surface (the scene buffer).
 * Call once per surface at startup.
 *
 * ```tsx
 * const root = createNeutronRoot(getScene())
 * root.render(<EditorCanvas ... />)
 * ```
 */
export function createNeutronRoot(container: SceneBuffer) {
  const root = NeutronReconciler.createContainer(
    container,
    0,           // ConcurrentMode tag
    null,
    false,
    null,
    '',
    () => {},
    null,
  )

  return {
    render(element: React.ReactNode) {
      NeutronReconciler.updateContainer(element, root, null, () => {})
    },
    unmount() {
      NeutronReconciler.updateContainer(null, root, null, () => {})
    },
  }
}
