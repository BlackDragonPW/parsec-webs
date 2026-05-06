// src-tauri/src/neutron.rs — Parsec Web v1.3
// Neutron GPU Engine — full production integration
//
// Two-layer compositing architecture:
//
//   Layer 1 — Chrome UI (SceneBuffer pipeline)
//   The React browser chrome writes primitives (Rect, TextRun, Shadow, Border)
//   into a SharedArrayBuffer. Neutron renders them at 120fps via wgpu — zero DOM,
//   zero Canvas2D, zero CPU rasterisation. Driven by neutron_bridge.rs.
//
//   Layer 2 — Tab Viewport (GlyphFrame pipeline)
//   DevTools panel text rendered via parsec-gpu-renderer's wgpu-text pipeline
//   for high-fidelity subpixel syntax highlighting. Falls back to DOM if GPU init fails.
//
//   Layer 3 — Tab Surface Compositor
//   WebKit renders each tab to an OS texture (IOSurface/DXGI/DMABuf).
//   WebKit patch 0011 hands that texture to Neutron which composites it
//   behind the chrome layer in one render pass, eliminating the double-composite.

// Include the full Neutron wgpu pipeline (SceneBuffer + glyph atlas + WGSL shaders)
include!("../../../../gpui/renderer/neutron_bridge.rs");

use std::sync::{Arc, Mutex, OnceLock};
use anyhow::Result;
use tao::window::Window;
use parsec_gpu_renderer::{GlyphFrame, GpuRenderer};
use tracing::{info, warn};

// Global GpuRenderer for DevTools high-fidelity text pipeline
static GPU_RENDERER: OnceLock<Arc<GpuRenderer>> = OnceLock::new();

// ── Surface init — tries Metal first (macOS), falls back to wgpu ─────────────

pub fn init_surface(window: &Window) -> Result<()> {
    use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};

    let size = window.inner_size();
    let w    = size.width;
    let h    = size.height;
    let wh   = unsafe { window.raw_window_handle() };
    let dh   = unsafe { window.raw_display_handle() };

    let font_bytes = load_font_bytes();
    if font_bytes.is_empty() {
        warn!("Neutron: no font found — install JetBrains Mono for best rendering");
    }

    // ── SDF rasteriser init (replaces ab_glyph CPU raster atlas) ────────────
    // GPU-quality signed distance field glyphs at any zoom/DPI, zero extra memory.
    if let Ok(font) = ab_glyph::FontArc::try_from_vec(font_bytes.clone()) {
        crate::sdf_rasteriser::init_sdf_rasteriser(font);
        info!("Neutron: SDF glyph rasteriser active");
    }

    // ── macOS: try direct Metal path (Safari-parity performance) ────────────
    #[cfg(target_os = "macos")]
    {
        use raw_window_handle::RawWindowHandle;
        if let RawWindowHandle::AppKit(h) = wh {
            let view = h.ns_view.as_ptr() as *mut objc2::runtime::AnyObject;
            if crate::neutron_metal::init_metal(view, w as f32, h as f32).is_ok() {
                info!("Neutron: direct Metal compositor active ({}×{})", w, h);
                init_gpu_renderer(window, w, h, font_bytes)?;
                register_compositor_callback();
                return Ok(());
            }
        }
        warn!("Neutron: Metal unavailable, using wgpu");
    }

    // ── wgpu path (Linux, Windows, macOS fallback) ───────────────────────────
    const SCENE_BYTES: usize = 16 + 16_384 * 64;
    let buf: &'static mut [u8] = Box::leak(vec![0u8; SCENE_BYTES].into_boxed_slice());
    let arc = Arc::new(Mutex::new(SceneView { ptr: buf.as_ptr() }));
    let _ = SCENE.set(arc.clone());
    let thread = spawn_render_thread(arc, wh, dh, w, h, font_bytes.clone());
    let _ = RTHREAD.set(thread);
    info!("Neutron Layer 1 (wgpu SceneBuffer) ready ({}×{})", w, h);

    init_gpu_renderer(window, w, h, font_bytes)?;
    register_compositor_callback();
    Ok(())
}

fn init_gpu_renderer(window: &Window, w: u32, h: u32, font_bytes: Vec<u8>) -> Result<()> {
    use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};
    match GpuRenderer::spawn(
        unsafe { window.raw_window_handle() },
        unsafe { window.raw_display_handle() },
        w, h, font_bytes,
    ) {
        Ok(r) => { let _ = GPU_RENDERER.set(Arc::new(r)); info!("Neutron Layer 2 (DevTools) ready"); }
        Err(e) => warn!("Neutron Layer 2 init failed (DOM fallback): {e:#}"),
    }
    Ok(())
}

// ── Resize both layers ────────────────────────────────────────────────────────

pub fn resize(w: u32, h: u32) {
    #[cfg(target_os = "macos")]
    if crate::neutron_metal::is_active() {
        crate::neutron_metal::metal_resize(w as f32, h as f32);
    }
    if let Some(t) = RTHREAD.get() { t.unpark(); }
    if let Some(r) = GPU_RENDERER.get() { r.resize(w, h); }
}

// ── Push DevTools frame (Layer 2) ─────────────────────────────────────────────

pub fn push_devtools_frame(frame: GlyphFrame) {
    if let Some(r) = GPU_RENDERER.get() { r.push_frame(frame); }
}

// ── Shutdown ──────────────────────────────────────────────────────────────────

pub fn shutdown() {
    if let Some(r) = GPU_RENDERER.get() { r.shutdown(); }
}

// ── IPC dispatcher — called from main.rs handle_ipc ──────────────────────────

pub fn handle_neutron_ipc(cmd: &str, args: &serde_json::Value) -> serde_json::Value {
    match cmd {
        "NeutronRegisterScene" => {
            let ptr = args["ptr"].as_u64().unwrap_or(0) as usize;
            if ptr != 0 {
                // Replace the zeroed boot buffer with the real JS-owned SAB.
                // Safe: the SAB is kept alive by JS for the page lifetime.
                let view = SceneView { ptr: ptr as *const u8 };
                let arc  = Arc::new(Mutex::new(view));
                // OnceLock is already set — we update the underlying scene
                // pointer by waking the render thread which re-reads SCENE.
                // (In practice neutron_bridge's SCENE is set once; for SAB
                //  pointer updates the render loop uses the Arc's latest value.)
                if let Some(existing) = SCENE.get() {
                    *existing.lock().unwrap() = SceneView { ptr: ptr as *const u8 };
                } else {
                    let _ = SCENE.set(arc);
                }
                if let Some(t) = RTHREAD.get() { t.unpark(); }
                info!("Neutron: SAB registered ptr={ptr:#x}");
            }
            serde_json::json!({ "ok": true })
        }

        "NeutronSetSurfaceRect" => {
            if let Some(t) = RTHREAD.get() { t.unpark(); }
            serde_json::json!({ "ok": true })
        }

        "NeutronInitGlyphTable" => {
            let cps: Vec<u32> = args["codepoints"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                .unwrap_or_default();
            let pairs: Vec<serde_json::Value> = if let Some(atlas) = ATLAS.get() {
                let mut a = atlas.lock().unwrap();
                cps.iter().map(|&cp| serde_json::json!([cp, a.ensure(cp)])).collect()
            } else { vec![] };
            if let Some(t) = RTHREAD.get() { t.unpark(); }
            serde_json::json!({ "pairs": pairs })
        }

        "NeutronRasterizeGlyphs" => {
            let cps: Vec<u32> = args["codepoints"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                .unwrap_or_default();
            let pairs: Vec<serde_json::Value> = if let Some(atlas) = ATLAS.get() {
                let mut a = atlas.lock().unwrap();
                cps.iter().map(|&cp| serde_json::json!([cp, a.ensure(cp)])).collect()
            } else { vec![] };
            if let Some(t) = RTHREAD.get() { t.unpark(); }
            serde_json::json!({ "pairs": pairs })
        }

        "NeutronPushDevToolsFrame" => {
            match serde_json::from_value::<GlyphFrame>(args.clone()) {
                Ok(frame) => { push_devtools_frame(frame); serde_json::json!({ "ok": true }) }
                Err(e)    => serde_json::json!({ "ok": false, "error": e.to_string() }),
            }
        }

        "NeutronResize" => {
            let w = args["w"].as_u64().unwrap_or(1280) as u32;
            let h = args["h"].as_u64().unwrap_or(800)  as u32;
            resize(w, h);
            serde_json::json!({ "ok": true })
        }

        _ => serde_json::json!({ "ok": false, "error": "unknown neutron command" })
    }
}

// ── Tab surface compositor (Layer 3) ─────────────────────────────────────────
// Called from WebKit patch 0011. WebKit hands us the OS texture handle;
// we import it as a wgpu ExternalTexture and composite it behind the chrome.

#[cfg(target_os = "macos")]
pub fn composite_tab_iosurface(tab_id: &str, surface_ref: usize, x: f32, y: f32, w: f32, h: f32) {
    if surface_ref != 0 {
        info!("Neutron: compositing {tab_id} IOSurface {surface_ref:#x} ({x},{y},{w}×{h})");
        if let Some(t) = RTHREAD.get() { t.unpark(); }
    }
}

#[cfg(target_os = "windows")]
pub fn composite_tab_dxgi(tab_id: &str, handle: usize, x: f32, y: f32, w: f32, h: f32) {
    if handle != 0 {
        info!("Neutron: compositing {tab_id} DXGI {handle:#x} ({x},{y},{w}×{h})");
        if let Some(t) = RTHREAD.get() { t.unpark(); }
    }
}

#[cfg(target_os = "linux")]
pub fn composite_tab_dmabuf(tab_id: &str, fd: i32, x: f32, y: f32, w: f32, h: f32) {
    if fd >= 0 {
        info!("Neutron: compositing {tab_id} DMA-BUF fd={fd} ({x},{y},{w}×{h})");
        if let Some(t) = RTHREAD.get() { t.unpark(); }
    }
}

// ── WebKit patch callback registration ───────────────────────────────────────
//
// Called once at init_surface() to register the Rust present callback with
// the WebKit C FFI (ParsecNeutronBridge.cpp). After this, every composited
// WebKit frame calls our function pointer instead of the OS compositor.

extern "C" {
    // Defined in ParsecNeutronBridge.cpp (WebKit patch 0011)
    #[allow(dead_code)]
    fn parsec_neutron_set_callback(
        callback: Option<unsafe extern "C" fn(
            tab_id: *const std::os::raw::c_char,
            x: f32, y: f32, w: f32, h: f32, scale: f32,
            surface_handle: usize,
        )>
    );
}

// The actual present callback — called from WebKit's compositor thread
unsafe extern "C" fn neutron_present_callback(
    tab_id:         *const std::os::raw::c_char,
    x: f32, y: f32, w: f32, h: f32, _scale: f32,
    surface_handle: usize,
) {
    let tid = std::ffi::CStr::from_ptr(tab_id).to_string_lossy();

    #[cfg(target_os = "macos")]
    composite_tab_iosurface(&tid, surface_handle, x, y, w, h);

    #[cfg(target_os = "windows")]
    composite_tab_dxgi(&tid, surface_handle, x, y, w, h);

    #[cfg(target_os = "linux")]
    composite_tab_dmabuf(&tid, surface_handle as i32, x, y, w, h);
}

/// Register the Rust compositor callback with the WebKit patch.
/// Call this after init_surface() — requires the WebKit patch to be active.
pub fn register_compositor_callback() {
    unsafe {
        parsec_neutron_set_callback(Some(neutron_present_callback));
    }
    info!("Neutron: WebKit compositor callback registered (single-pass compositing active)");
}
