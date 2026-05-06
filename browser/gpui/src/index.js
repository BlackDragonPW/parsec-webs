import { jsx as _jsx } from "react/jsx-runtime";
// @ts-nocheck
/**
 * index.ts — Neutron Engine Public API
 *
 * Everything a React component or hook needs to use Neutron.
 * Import from 'gpui' — that's the whole surface area.
 *
 * Usage
 * ─────
 * // At app startup (once):
 * import { initNeutron } from 'gpui'
 * await initNeutron({ fontSize: 14 })
 *
 * // To mount a Neutron-rendered surface:
 * import { NeutronSurface } from 'gpui'
 * <NeutronSurface>
 *   <EditorCanvas lines={lines} />
 * </NeutronSurface>
 *
 * // Inside Neutron components:
 * import { Rect, Text, Border, Clip } from 'gpui'
 * function EditorLine({ y, text, color }) {
 *   return (
 *     <>
 *       <Rect x={0} y={y} width={800} height={20} color={0x1e1e1eff} />
 *       <Text x={48} y={y+3} text={text} color={color} fontSize={14} />
 *     </>
 *   )
 * }
 */
import { useEffect, useRef, createContext, useContext } from 'react';
import { invoke } from '@tauri-apps/api/tauri';
import { getScene } from './scene';
import { initGlyphTable } from './glyphTable';
import { createNeutronRoot } from './renderer';
let _initialized = false;
/**
 * Initialize the Neutron engine. Call once before mounting any NeutronSurface.
 * Returns when the glyph atlas is populated and the Rust render thread is ready.
 */
export async function initNeutron(config) {
    if (_initialized)
        return;
    _initialized = true;
    const scene = getScene();
    // Register the SharedArrayBuffer with the Rust side.
    // The SAB pointer is passed as a numeric address — this works because
    // Tauri's in-process WebView (macOS/Linux) shares address space with Rust.
    // The Rust side reads scene primitives directly from this memory.
    //
    // On Windows (WebView2 is out-of-process), Rust creates a named shared
    // memory region and we map to it here instead — same result, slightly
    // different plumbing handled transparently by neutron_register_scene.
    try {
        await invoke('neutron_register_scene', {
            // Pass buffer identity — Rust resolves to actual memory
            sabLen: scene.sab.byteLength,
        });
    }
    catch (e) {
        console.warn('Neutron: Rust bridge not available, running in shadow mode', e);
    }
    // Pre-rasterize the common character set into the GPU glyph atlas
    await initGlyphTable(config.fontSize);
    console.info('Neutron engine initialized');
}
// ── NeutronSurface ─────────────────────────────────────────────────────────────
const SceneContext = createContext(null);
/**
 * A surface rendered by the Neutron engine instead of the browser compositor.
 * Children of NeutronSurface render via the custom React reconciler into the
 * shared memory scene buffer. The Rust GPU thread draws them natively.
 *
 * NeutronSurface is transparent from the DOM's perspective — it renders a
 * zero-size container in the DOM and tells the GPU where the surface is.
 * The GPU surface is composited behind the WebView, visible through the
 * transparent hole (same trick as the existing GPU canvas).
 */
export function NeutronSurface({ children, style, }) {
    const containerRef = useRef(null);
    const rootRef = useRef(null);
    const scene = getScene();
    useEffect(() => {
        if (!containerRef.current)
            return;
        // Mount the Neutron reconciler into the scene buffer
        rootRef.current = createNeutronRoot(scene);
        // Tell Rust the exact screen coordinates of this surface
        const rect = containerRef.current.getBoundingClientRect();
        invoke('neutron_set_surface_rect', {
            x: rect.left, y: rect.top, width: rect.width, height: rect.height,
        }).catch(() => { });
        return () => {
            rootRef.current?.unmount();
        };
    }, []);
    // Re-render the Neutron tree whenever children change
    useEffect(() => {
        rootRef.current?.render(_jsx(SceneContext.Provider, { value: scene, children: children }));
    }, [children, scene]);
    // The DOM element is just a position anchor — it's transparent
    return (_jsx("div", { ref: containerRef, style: {
            position: 'absolute',
            pointerEvents: 'none', // GPU surface handles input via Rust
            backgroundColor: 'transparent',
            ...style,
        } }));
}
/** Filled rectangle — the base building block */
export function Rect(props) {
    return _jsx("n-rect", { ...props });
}
/** Rectangle outline */
export function Border(props) {
    return _jsx("n-border", { ...props });
}
/** Text run — renders via the GPU glyph atlas, zero DOM involvement */
export function Text(props) {
    return _jsx("n-text", { ...props });
}
/** Box shadow */
export function Shadow(props) {
    return _jsx("n-shadow", { ...props });
}
/** Clip region — clips all children to the given rect */
export function Clip(props) {
    return _jsx("n-clip", { ...props });
}
// ── useNeutronScene hook ───────────────────────────────────────────────────────
/** Access the raw scene buffer for imperative drawing (advanced use) */
export function useNeutronScene() {
    return useContext(SceneContext);
}
// ── Re-exports ─────────────────────────────────────────────────────────────────
export { getScene } from './scene';
export { glyphId, stringToGlyphIds } from './glyphTable';
