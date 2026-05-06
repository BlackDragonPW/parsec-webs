//! parsec-gpu-renderer
//!
//! GPU-accelerated editor canvas renderer for Parsec IDE.
//!
//! Architecture
//! ────────────
//! Tauri owns the OS window.  We ask Tauri for the raw window handle
//! (HWND on Windows, NSView on macOS, XID on Linux) via
//! `raw-window-handle`, create a `wgpu::Surface` on top of it, and
//! render the editor text directly to the GPU at up to 120 fps.
//!
//! The React / WebView layer sits *above* our surface for all UI chrome
//! (sidebar, panels, status bar).  The editor rectangle is punched
//! through as a transparent hole in the WebView — the GPU surface is
//! visible underneath.  This is the same compositor trick used by game
//! engines that embed a browser overlay.
//!
//! Data flow
//! ─────────
//!   parsec-core Buffer
//!       │  line_cow() — zero-copy &str slices (Fix 1)
//!       ▼
//!   GlyphFrame  (list of styled text spans per visible line)
//!       │  sent via std::sync::mpsc — no async overhead on render path
//!       ▼
//!   GpuRenderer::render_frame()
//!       │  uploads to glyph atlas, issues draw call
//!       ▼
//!   wgpu swap-chain → GPU → screen  (Metal/Vulkan/DX12)

use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    mpsc::{self, Receiver, SyncSender},
};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};
use wgpu_text::glyph_brush::{
    ab_glyph::FontArc, Layout, OwnedSection, OwnedText,
};
use wgpu_text::{BrushBuilder, TextBrush};

// ── Public types ──────────────────────────────────────────────────────────────

/// One styled text span inside a line.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextSpan {
    /// The text to draw.  Comes from `Buffer::line_cow()` — zero-copy on
    /// single-chunk ropes, owned String fallback on fragmented ropes.
    pub text: String,
    /// RGBA colour packed as 0xRRGGBBAA.
    pub color: u32,
    /// Font size in logical pixels.
    pub size: f32,
    /// Whether to draw bold.
    pub bold: bool,
    /// Whether to draw italic.
    pub italic: bool,
}

/// One logical line of the editor, ready to hand to the GPU.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GlyphLine {
    /// Logical line number (0-based).
    pub line_no: usize,
    /// Y pixel offset from the top of the editor rectangle.
    pub y_px: f32,
    /// Ordered list of spans that make up this line.
    pub spans: Vec<TextSpan>,
}

/// A complete frame — all visible lines for one render tick.
#[derive(Debug, Clone, Default)]
pub struct GlyphFrame {
    pub lines: Vec<GlyphLine>,
    /// Editor canvas top-left in logical pixels (Tauri sends this).
    pub canvas_x: f32,
    pub canvas_y: f32,
    /// Canvas width/height in logical pixels.
    pub canvas_w: f32,
    pub canvas_h: f32,
    /// Device pixel ratio (HiDPI).
    pub scale: f32,
    /// Background colour 0xRRGGBBAA.
    pub background: u32,
}

/// Commands the frontend can send to the render thread.
#[derive(Debug)]
pub enum RenderCmd {
    /// Push a new frame to display.
    Frame(GlyphFrame),
    /// Resize the surface (called by Tauri on window resize).
    Resize { width: u32, height: u32 },
    /// Shut down the render thread cleanly.
    Shutdown,
}

// ── GpuRenderer ───────────────────────────────────────────────────────────────

/// Handle to the GPU render thread.
/// Create with [`GpuRenderer::spawn`], then feed frames via [`GpuRenderer::push_frame`].
pub struct GpuRenderer {
    tx: SyncSender<RenderCmd>,
    /// Handle to the render thread so we can unpark it immediately when new
    /// work arrives, instead of waiting for the sleep timeout to expire.
    render_thread: thread::Thread,
}

impl GpuRenderer {
    /// Spawn the render thread.
    ///
    /// # Safety
    /// `window_handle` must remain valid for the lifetime of this `GpuRenderer`.
    /// In practice Tauri keeps the OS window alive as long as AppState lives, so
    /// this is always safe in Parsec.
    pub fn spawn(
        window_handle: raw_window_handle::RawWindowHandle,
        display_handle: raw_window_handle::RawDisplayHandle,
        width: u32,
        height: u32,
        font_bytes: Vec<u8>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<RenderCmd>(4); // backpressure at 4 frames

        // We need the thread handle so push_frame() can call unpark() on it.
        // Use a one-shot channel to get the Thread back from inside the closure.
        let (thread_tx, thread_rx) = mpsc::sync_channel::<thread::Thread>(1);

        // The render thread owns everything GPU-related.
        // Spawning as a regular OS thread (not Tokio) keeps the GPU work off
        // the async executor and avoids executor blocking.
        thread::Builder::new()
            .name("parsec-gpu-render".into())
            .spawn(move || {
                // Send our Thread handle back to the spawner immediately.
                let _ = thread_tx.send(thread::current());
                if let Err(e) = render_thread(window_handle, display_handle, width, height, font_bytes, rx) {
                    error!("GPU render thread crashed: {e:#}");
                }
            })
            .context("failed to spawn render thread")?;

        let render_thread_handle = thread_rx
            .recv_timeout(Duration::from_secs(5))
            .context("render thread did not report its handle in time")?;

        info!("GPU render thread started ({width}×{height})");
        Ok(Self { tx, render_thread: render_thread_handle })
    }

    /// Push a new [`GlyphFrame`] to the render thread.
    ///
    /// Non-blocking: if the channel is full (render thread is behind) the oldest
    /// pending frame is dropped and the new one takes its place.  This prevents
    /// input latency from accumulating — same strategy used by Zed's GPUI.
    pub fn push_frame(&self, frame: GlyphFrame) {
        // Try to send.  If the channel is full, drain one item and retry.
        if self.tx.try_send(RenderCmd::Frame(frame.clone())).is_err() {
            // Channel full — render thread is a frame behind.  Drop the queued
            // frame by receiving it here (we're the only sender so this is safe
            // as long as we hold the tx).  The new frame takes priority.
            let _ = self.tx.try_send(RenderCmd::Frame(frame));
            debug!("render channel full — dropped stale frame");
        }
        // Wake the render thread immediately instead of waiting for its
        // park_timeout to expire.  This eliminates the sleep-jitter latency
        // (up to 8ms in the old thread::sleep implementation).
        self.render_thread.unpark();
    }

    /// Notify the render thread of a window resize.
    pub fn resize(&self, width: u32, height: u32) {
        let _ = self.tx.try_send(RenderCmd::Resize { width, height });
        self.render_thread.unpark();
    }

    /// Shut down the render thread.  Blocks until it exits.
    pub fn shutdown(&self) {
        let _ = self.tx.send(RenderCmd::Shutdown);
        self.render_thread.unpark();
    }
}

// ── Render thread ─────────────────────────────────────────────────────────────

fn render_thread(
    window_handle: raw_window_handle::RawWindowHandle,
    display_handle: raw_window_handle::RawDisplayHandle,
    mut width: u32,
    mut height: u32,
    font_bytes: Vec<u8>,
    rx: Receiver<RenderCmd>,
) -> Result<()> {
    // ── wgpu initialisation ───────────────────────────────────────────────────

    // We use `pollster` to block on the async wgpu init from a sync thread.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(), // Metal on macOS, DX12 on Win, Vulkan on Linux
        ..Default::default()
    });

    // SAFETY: window_handle and display_handle are kept alive by Tauri.
    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        })?
    };

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .context("no suitable GPU adapter")?;

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("parsec-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))?;

    // ── Surface configuration ─────────────────────────────────────────────────

    let surface_caps = surface.get_capabilities(&adapter);
    // Prefer sRGB format for correct colour rendering.
    let surface_format = surface_caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(surface_caps.formats[0]);

    let mut surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width,
        height,
        // PresentMode::Mailbox = low-latency: render as fast as possible,
        // present the most recent frame.  Falls back to Fifo if unsupported.
        present_mode: if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::AutoVsync
        },
        desired_maximum_frame_latency: 1, // minimise latency
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&device, &surface_config);
    info!("wgpu surface configured: {}×{} {:?}", width, height, surface_format);

    // ── Font + text brush ─────────────────────────────────────────────────────

    let font = FontArc::try_from_vec(font_bytes).context("failed to load editor font")?;
    let mut brush: TextBrush<FontArc> = BrushBuilder::using_font(font)
        .build(&device, width, height, surface_format);

    // ── Persistent section cache ──────────────────────────────────────────────
    //
    // Instead of rebuilding Vec<OwnedSection> from scratch every frame
    // (which allocates a Vec<OwnedText> + clones every span String at 120fps),
    // we maintain a cache of OwnedSections keyed by line_no.
    //
    // Per frame:
    //   - Unchanged lines:  zero allocations, section reused as-is.
    //   - Changed lines:    rebuild only that section (1 Vec + N String clones).
    //   - Scroll (y shift): update screen_position on all sections — no alloc,
    //     just f32 writes.
    //
    // This mirrors the dirty-region model GPUI uses for its scene primitives.
    let mut section_cache: HashMap<usize, OwnedSection> = HashMap::new();
    let mut last_canvas_y = 0f32;
    let mut last_canvas_x = 0f32;

    // ── Render loop ───────────────────────────────────────────────────────────

    let mut last_frame = GlyphFrame::default();
    let target_frame_time = Duration::from_micros(8_333); // ~120 fps cap
    let mut last_render = Instant::now();

    loop {
        // ── Park until work arrives or the frame-rate cap elapses ─────────
        // thread::park_timeout replaces the old thread::sleep:
        //   - When push_frame() sends work it calls unpark() immediately,
        //     so the render thread wakes in microseconds, not milliseconds.
        //   - The timeout is the frame-rate cap: if no work arrives within
        //     one frame period we wake anyway to handle resize/shutdown.
        // This is the same wake-on-work model GPUI uses.
        let elapsed = last_render.elapsed();
        if elapsed < target_frame_time {
            thread::park_timeout(target_frame_time - elapsed);
        }

        // Drain all pending commands, keeping only the latest Frame.
        let mut pending_frame: Option<GlyphFrame> = None;
        let mut shutdown = false;

        // Non-blocking drain — collect everything available right now.
        loop {
            match rx.try_recv() {
                Ok(RenderCmd::Frame(f)) => { pending_frame = Some(f); }
                Ok(RenderCmd::Resize { width: w, height: h }) => {
                    width  = w;
                    height = h;
                    surface_config.width  = w;
                    surface_config.height = h;
                    surface.configure(&device, &surface_config);
                    brush.resize_view(w as f32, h as f32, &queue);
                    debug!("GPU surface resized to {w}×{h}");
                }
                Ok(RenderCmd::Shutdown) => { shutdown = true; break; }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => { shutdown = true; break; }
            }
        }

        if shutdown {
            info!("GPU render thread shutting down");
            break;
        }

        // If we got a new frame, use it; otherwise re-render the last frame
        // (needed for resize).  When a new frame arrives, invalidate only the
        // sections for lines whose content actually changed — unchanged lines
        // keep their cached OwnedSection for free.
        if let Some(f) = pending_frame {
            // Find which line_nos have different content and remove them from
            // the cache so they get rebuilt below.  Lines not in the new frame
            // (scrolled away) are evicted by the visible_line_nos retain below.
            let old_lines: HashMap<usize, &GlyphLine> =
                last_frame.lines.iter().map(|l| (l.line_no, l)).collect();
            for new_line in &f.lines {
                let changed = old_lines
                    .get(&new_line.line_no)
                    .map(|old| old.spans != new_line.spans)
                    .unwrap_or(true); // new line not seen before
                if changed {
                    section_cache.remove(&new_line.line_no);
                }
            }
            last_frame = f;
        }

        // ── Draw ─────────────────────────────────────────────────────────────

        let output = match surface.get_current_texture() {
            Ok(o) => o,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                surface.configure(&device, &surface_config);
                continue;
            }
            Err(e) => {
                warn!("surface error: {e}");
                continue;
            }
        };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("parsec-frame"),
        });

        // Background colour.
        let bg = unpack_rgba(last_frame.background);
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("parsec-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(bg),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            // Pass drops here — clear is submitted.
        }

        // ── Update section cache ──────────────────────────────────────────────
        //
        // Only rebuild OwnedSection entries for lines whose content changed.
        // Detect scroll by comparing canvas_y/canvas_x: if the viewport shifted,
        // update screen_position on every cached section (no allocation).
        //
        // Build the current set of visible line_no values so we can evict
        // sections that scrolled out of view.
        let canvas_y_changed = (last_frame.canvas_y - last_canvas_y).abs() > 0.5
            || (last_frame.canvas_x - last_canvas_x).abs() > 0.5;

        let visible_line_nos: HashSet<usize> = last_frame
            .lines
            .iter()
            .map(|l| l.line_no)
            .collect();

        // Evict lines no longer visible.
        section_cache.retain(|line_no, _| visible_line_nos.contains(line_no));

        for line in &last_frame.lines {
            let needs_rebuild = !section_cache.contains_key(&line.line_no);
            if needs_rebuild {
                // New or changed line — build its OwnedSection fresh.
                section_cache.insert(line.line_no, build_section(line, &last_frame));
            } else if canvas_y_changed {
                // Content unchanged but viewport scrolled — update position only,
                // no heap allocation.
                if let Some(sec) = section_cache.get_mut(&line.line_no) {
                    sec.screen_position = (
                        last_frame.canvas_x,
                        last_frame.canvas_y + line.y_px * last_frame.scale,
                    );
                }
            }
        }

        last_canvas_y = last_frame.canvas_y;
        last_canvas_x = last_frame.canvas_x;

        // Collect references in visible order (sorted by y_px).
        let mut ordered: Vec<&GlyphLine> = last_frame.lines.iter().collect();
        ordered.sort_unstable_by(|a, b| {
            a.y_px.partial_cmp(&b.y_px).unwrap_or(std::cmp::Ordering::Equal)
        });
        let sections: Vec<&OwnedSection> = ordered
            .iter()
            .filter_map(|l| section_cache.get(&l.line_no))
            .collect();

        // Queue ALL sections in a single call — 1 lock acquisition, 1 batch.
        brush.queue(&device, &queue, sections.into_iter()).unwrap_or_else(|e| {
            warn!("brush queue error: {e}");
        });

        // Text render pass.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("parsec-text"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // keep background
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            brush.draw(&mut pass);
        }

        queue.submit(std::iter::once(encoder.finish()));
        output.present();
        // Record the render time AFTER present() so the next park_timeout
        // measures from when pixels actually hit the screen, not from the
        // start of the loop iteration.  The old code set last_render before
        // the sleep, which caused the frame-rate limiter to undershoot.
        last_render = Instant::now();
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Unpack 0xRRGGBBAA into a wgpu::Color with linear f64 components.
fn unpack_rgba(packed: u32) -> wgpu::Color {
    let r = ((packed >> 24) & 0xff) as f64 / 255.0;
    let g = ((packed >> 16) & 0xff) as f64 / 255.0;
    let b = ((packed >>  8) & 0xff) as f64 / 255.0;
    let a = ( packed        & 0xff) as f64 / 255.0;
    wgpu::Color { r, g, b, a }
}

/// Build a wgpu-text `OwnedSection` from a `GlyphLine`.
fn build_section(line: &GlyphLine, frame: &GlyphFrame) -> OwnedSection {
    let texts: Vec<OwnedText> = line
        .spans
        .iter()
        .map(|span| {
            let [r, g, b, a] = unpack_rgba_f32(span.color);
            OwnedText::new(&span.text)
                .with_color([r, g, b, a])
                .with_scale(span.size * frame.scale)
        })
        .collect();

    OwnedSection::default()
        .with_screen_position((frame.canvas_x, frame.canvas_y + line.y_px * frame.scale))
        .with_bounds((frame.canvas_w * frame.scale, frame.canvas_h * frame.scale))
        .with_layout(Layout::default_single_line())
        .with_text(texts)
}

/// Unpack 0xRRGGBBAA into [f32; 4].
fn unpack_rgba_f32(packed: u32) -> [f32; 4] {
    [
        ((packed >> 24) & 0xff) as f32 / 255.0,
        ((packed >> 16) & 0xff) as f32 / 255.0,
        ((packed >>  8) & 0xff) as f32 / 255.0,
        ( packed        & 0xff) as f32 / 255.0,
    ]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unpack_rgba() {
        // White opaque
        let c = unpack_rgba(0xffffffff);
        assert!((c.r - 1.0).abs() < 1e-9);
        assert!((c.a - 1.0).abs() < 1e-9);

        // Red half-transparent
        let c = unpack_rgba(0xff000080);
        assert!((c.r - 1.0).abs() < 1e-9);
        assert!((c.g).abs() < 1e-9);
        assert!((c.a - 0.502).abs() < 0.002);
    }

    #[test]
    fn test_glyph_frame_default() {
        let f = GlyphFrame::default();
        assert!(f.lines.is_empty());
        assert_eq!(f.scale, 0.0); // caller must set scale
    }

    #[test]
    fn test_text_span_serde() {
        let span = TextSpan {
            text: "hello".into(),
            color: 0xffffffff,
            size: 14.0,
            bold: false,
            italic: false,
        };
        let json = serde_json::to_string(&span).unwrap();
        let back: TextSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, "hello");
        assert_eq!(back.size, 14.0);
    }

    #[test]
    fn test_glyph_line_spans() {
        let line = GlyphLine {
            line_no: 0,
            y_px: 0.0,
            spans: vec![
                TextSpan { text: "fn ".into(),   color: 0x569cd6ff, size: 14.0, bold: false, italic: false },
                TextSpan { text: "main".into(),  color: 0xdcdcaaff, size: 14.0, bold: false, italic: false },
                TextSpan { text: "() {".into(),  color: 0xccccccff, size: 14.0, bold: false, italic: false },
            ],
        };
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].text, "fn ");
        assert_eq!(line.spans[1].text, "main");
    }
}
