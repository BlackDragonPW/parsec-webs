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
import ReactReconciler from 'react-reconciler';
import { DefaultEventPriority } from 'react-reconciler/constants';
import { stringToGlyphIds } from './glyphTable';
// ── Scene writer ───────────────────────────────────────────────────────────────
function writeInstance(scene, inst) {
    const p = inst.props;
    const x = p.x ?? 0;
    const y = p.y ?? 0;
    const w = p.width ?? 0;
    const h = p.height ?? 0;
    const c = p.color ?? 0xffffffff;
    switch (inst.type) {
        case 'n-rect':
            scene.rect(x, y, w, h, c, p.radius ?? 0);
            break;
        case 'n-border': {
            const bp = p;
            scene.border(x, y, w, h, c, bp.borderWidth ?? 1, bp.radius ?? 0);
            break;
        }
        case 'n-text': {
            const tp = p;
            const ids = stringToGlyphIds(tp.text ?? '');
            // Split long runs into 20-glyph chunks (scene primitive max)
            let offset = 0;
            let glyphX = x;
            const fontSize = tp.fontSize ?? 14;
            const glyphW = fontSize * 0.6; // monospace approximation; Rust uses exact metrics
            while (offset < ids.length) {
                const chunk = ids.slice(offset, offset + 20);
                scene.textRun(glyphX, y, c, fontSize, chunk);
                glyphX += chunk.length * glyphW;
                offset += 20;
            }
            break;
        }
        case 'n-shadow': {
            const sp = p;
            scene.shadow(x, y, w, h, c, sp.blur ?? 8, sp.spread ?? 0, sp.offsetX ?? 0, sp.offsetY ?? 0);
            break;
        }
        case 'n-clip':
            scene.clipPush(x, y, w, h);
            for (const child of inst.children)
                writeInstance(scene, child);
            scene.clipPop();
            return; // children already written above, skip default child walk
    }
    // Write children (except n-clip which handled them above)
    for (const child of inst.children) {
        writeInstance(scene, child);
    }
}
// ── Reconciler host config ─────────────────────────────────────────────────────
const hostConfig = {
    // ── Instance creation ──────────────────────────────────────────────────────
    createInstance(type, props) {
        return { type, props, children: [] };
    },
    createTextInstance() {
        throw new Error('Neutron: use <n-text text="…"> instead of raw text nodes');
    },
    appendInitialChild(parent, child) {
        parent.children.push(child);
    },
    appendChild(parent, child) {
        parent.children.push(child);
    },
    appendChildToContainer(container, child) {
        // Root-level child — write it into the scene
        writeInstance(container, child);
    },
    insertBefore(parent, child, before) {
        const idx = parent.children.indexOf(before);
        if (idx >= 0)
            parent.children.splice(idx, 0, child);
        else
            parent.children.push(child);
    },
    removeChild(parent, child) {
        const idx = parent.children.indexOf(child);
        if (idx >= 0)
            parent.children.splice(idx, 1);
    },
    removeChildFromContainer(container, child) {
        // Scene buffer is rewritten each frame — removal is implicit on next begin()
    },
    // ── Updates ────────────────────────────────────────────────────────────────
    prepareUpdate(_inst, _type, oldProps, newProps) {
        // Return true if any prop changed — drives commitUpdate
        return oldProps !== newProps;
    },
    commitUpdate(inst, _payload, _type, _old, newProps) {
        inst.props = newProps;
    },
    // ── Commit phase ───────────────────────────────────────────────────────────
    prepareForCommit(container) {
        container.begin();
        return null;
    },
    resetAfterCommit(container) {
        // All primitives written during this commit cycle — publish the frame
        container.commit();
    },
    finalizeInitialChildren() { return false; },
    // ── Container ──────────────────────────────────────────────────────────────
    getRootHostContext() { return {}; },
    getChildHostContext() { return {}; },
    getPublicInstance(inst) { return inst; },
    // ── Text (disabled — use n-text) ───────────────────────────────────────────
    shouldSetTextContent() { return false; },
    // ── Time / scheduling ──────────────────────────────────────────────────────
    scheduleTimeout: setTimeout,
    cancelTimeout: clearTimeout,
    noTimeout: -1,
    isPrimaryRenderer: false, // coexists with ReactDOM
    getCurrentEventPriority() { return DefaultEventPriority; },
    getInstanceFromNode() { return null; },
    beforeActiveInstanceBlur() { },
    afterActiveInstanceBlur() { },
    prepareScopeUpdate() { },
    getInstanceFromScope() { return null; },
    detachDeletedInstance() { },
    // ── Mutations ──────────────────────────────────────────────────────────────
    supportsMutation: true,
    supportsPersistence: false,
    supportsHydration: false,
};
// ── Reconciler instance ────────────────────────────────────────────────────────
export const NeutronReconciler = ReactReconciler(hostConfig);
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
export function createNeutronRoot(container) {
    const root = NeutronReconciler.createContainer(container, 0, // ConcurrentMode tag
    null, false, null, '', () => { }, null);
    return {
        render(element) {
            NeutronReconciler.updateContainer(element, root, null, () => { });
        },
        unmount() {
            NeutronReconciler.updateContainer(null, root, null, () => { });
        },
    };
}
