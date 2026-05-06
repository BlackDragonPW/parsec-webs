// src-rust/src/sdf_rasteriser.rs
//
// Signed-Distance-Field glyph rasteriser — Android port.
// Merges: fixed build's SdfAtlas structure + perfect-build's real
// generate_sdf / find_atlas_space / rasterize_glyph logic.
//
// The atlas (2048×512 R8) is uploaded once to the GPU texture.
// URL bar / toolbar / tab-title labels are rendered via this pipeline.
// Content WebView text is rendered by Android System WebView directly.

use std::collections::HashMap;
use std::sync::Mutex;
use once_cell::sync::OnceCell;
use ttf_parser::{Face, OutlineBuilder};

// ── Public types ──────────────────────────────────────────────────────────────

/// UV coordinates + advance metrics for a single glyph slot in the atlas.
#[derive(Debug, Clone, Copy)]
pub struct GlyphMetrics {
    pub advance_width: f32,
    pub bearing_x:     f32,
    pub bearing_y:     f32,
    pub width:         f32,
    pub height:        f32,
    /// Normalised UV rect [0,1] into the R8 atlas texture.
    pub uv_x: f32,
    pub uv_y: f32,
    pub uv_w: f32,
    pub uv_h: f32,
}

// ── Atlas ─────────────────────────────────────────────────────────────────────

pub struct SdfAtlas {
    pub width:    u32,
    pub height:   u32,
    /// Raw R8 pixel data uploaded to GPU.
    pub data:     Vec<u8>,
    /// Cache: (font_id, codepoint) → packed metrics.
    pub glyphs:   HashMap<(u32, u32), GlyphMetrics>,
    // Row-based shelf packing state.
    cursor_x:  u32,
    cursor_y:  u32,
    row_h:     u32,
}

impl SdfAtlas {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            width: w, height: h,
            data: vec![0u8; (w * h) as usize],
            glyphs: HashMap::new(),
            cursor_x: 0, cursor_y: 0, row_h: 0,
        }
    }

    /// Pack a glyph bitmap into the atlas using shelf-packing.
    /// Returns the (atlas_x, atlas_y) origin of the packed slot, or None if full.
    fn pack(&mut self, glyph_w: u32, glyph_h: u32) -> Option<(u32, u32)> {
        let padding = 2u32;
        let slot_w = glyph_w + padding * 2;
        let slot_h = glyph_h + padding * 2;

        // Advance to next row if needed.
        if self.cursor_x + slot_w > self.width {
            self.cursor_y += self.row_h;
            self.cursor_x = 0;
            self.row_h = 0;
        }

        if self.cursor_y + slot_h > self.height {
            return None; // Atlas full — caller should evict or grow.
        }

        let x = self.cursor_x + padding;
        let y = self.cursor_y + padding;
        self.cursor_x += slot_w;
        if slot_h > self.row_h { self.row_h = slot_h; }
        Some((x, y))
    }

    /// Write SDF bitmap into the atlas at (ax, ay).
    fn blit(&mut self, sdf: &[u8], ax: u32, ay: u32, gw: u32, gh: u32) {
        for row in 0..gh {
            for col in 0..gw {
                let dst = ((ay + row) * self.width + ax + col) as usize;
                let src = (row * gw + col) as usize;
                if dst < self.data.len() && src < sdf.len() {
                    self.data[dst] = sdf[src];
                }
            }
        }
    }
}

// ── Global singleton ──────────────────────────────────────────────────────────

static ATLAS: OnceCell<Mutex<SdfAtlas>> = OnceCell::new();

pub fn init() {
    ATLAS.get_or_init(|| Mutex::new(SdfAtlas::new(2048, 512)));
    tracing::info!("SDF atlas initialised (2048×512 R8)");
}

// ── SDF generation ────────────────────────────────────────────────────────────

const SDF_RADIUS: f32 = 8.0;
const SDF_SPREAD: f32 = 6.0;

/// Outline collector used with ttf-parser.
struct ContourCollector {
    /// Flat list of (x, y) edge sample points.
    points: Vec<(f32, f32)>,
    cur:    (f32, f32),
}

impl ContourCollector {
    fn new() -> Self { Self { points: Vec::new(), cur: (0.0, 0.0) } }
}

impl OutlineBuilder for ContourCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.cur = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        // Sample the segment.
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            self.points.push((
                self.cur.0 + t * (x - self.cur.0),
                self.cur.1 + t * (y - self.cur.1),
            ));
        }
        self.cur = (x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (sx, sy) = self.cur;
        for i in 1..=12 {
            let t = i as f32 / 12.0;
            let u = 1.0 - t;
            self.points.push((
                u * u * sx + 2.0 * u * t * x1 + t * t * x,
                u * u * sy + 2.0 * u * t * y1 + t * t * y,
            ));
        }
        self.cur = (x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (sx, sy) = self.cur;
        for i in 1..=16 {
            let t = i as f32 / 16.0;
            let u = 1.0 - t;
            self.points.push((
                u*u*u*sx + 3.0*u*u*t*x1 + 3.0*u*t*t*x2 + t*t*t*x,
                u*u*u*sy + 3.0*u*u*t*y1 + 3.0*u*t*t*y2 + t*t*t*y,
            ));
        }
        self.cur = (x, y);
    }
    fn close(&mut self) {}
}

/// Ray-casting point-in-glyph test (winding-number based, works for most TrueType glyphs).
fn point_in_glyph(pts: &[(f32, f32)], px: f32, py: f32) -> bool {
    let n = pts.len();
    if n < 2 { return false; }
    let mut winding = 0i32;
    let mut j = n - 1;
    for i in 0..n {
        let (ax, ay) = pts[j];
        let (bx, by) = pts[i];
        if ay <= py {
            if by > py && (bx - ax) * (py - ay) - (by - ay) * (px - ax) > 0.0 {
                winding += 1;
            }
        } else if by <= py && (bx - ax) * (py - ay) - (by - ay) * (px - ax) < 0.0 {
            winding -= 1;
        }
        j = i;
    }
    winding != 0
}

/// Minimum distance from (px, py) to any sampled edge point.
fn distance_to_edge(pts: &[(f32, f32)], px: f32, py: f32) -> f32 {
    pts.iter().fold(f32::MAX, |min, &(ex, ey)| {
        let d = ((ex - px).powi(2) + (ey - py).powi(2)).sqrt();
        d.min(min)
    })
}

/// Generate an SDF bitmap for a single glyph outline at `px_size`.
/// Returns (bitmap, width, height) in R8 format where 128 = exactly on the edge.
fn generate_sdf(face: &Face, glyph_id: ttf_parser::GlyphId, px_size: f32) -> Option<(Vec<u8>, u32, u32)> {
    let bb = face.glyph_bounding_box(glyph_id)?;
    let units = face.units_per_em() as f32;
    let scale = px_size / units;

    let pw = ((bb.x_max - bb.x_min) as f32 * scale).ceil() as u32 + 2;
    let ph = ((bb.y_max - bb.y_min) as f32 * scale).ceil() as u32 + 2;
    if pw == 0 || ph == 0 { return None; }

    let mut collector = ContourCollector::new();
    face.outline_glyph(glyph_id, &mut collector);

    // Transform collected points to pixel space.
    let ox = bb.x_min as f32 * scale;
    let oy = bb.y_min as f32 * scale;
    let pts: Vec<(f32, f32)> = collector.points.iter()
        .map(|&(x, y)| (x * scale - ox, y * scale - oy))
        .collect();

    let mut sdf = vec![0u8; (pw * ph) as usize];
    for row in 0..ph {
        for col in 0..pw {
            let px = col as f32 + 0.5;
            let py = row as f32 + 0.5;
            let inside = point_in_glyph(&pts, px, py);
            let dist   = distance_to_edge(&pts, px, py);
            // Map dist to [0,255]; 128 = on edge.
            let signed = if inside { dist } else { -dist };
            let normalised = (signed / SDF_SPREAD * 128.0 + 128.0).clamp(0.0, 255.0) as u8;
            sdf[(row * pw + col) as usize] = normalised;
        }
    }

    Some((sdf, pw, ph))
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Rasterise a glyph (if not already cached) and return its atlas metrics.
/// `font_data` is the raw TTF/OTF bytes; `font_id` is a caller-assigned handle.
pub fn rasterize_glyph(
    font_data: &[u8],
    font_id:   u32,
    codepoint: u32,
    px_size:   f32,
) -> Option<GlyphMetrics> {
    {
        let atlas = ATLAS.get()?.lock().unwrap();
        if let Some(m) = atlas.glyphs.get(&(font_id, codepoint)) {
            return Some(*m);
        }
    }

    // Parse font and find glyph.
    let face = Face::parse(font_data, 0).ok()?;
    let ch   = char::from_u32(codepoint)?;
    let gid  = face.glyph_index(ch)?;

    let units  = face.units_per_em() as f32;
    let scale  = px_size / units;
    let adv    = face.glyph_hor_advance(gid)? as f32 * scale;
    let bear_x = face.glyph_hor_side_bearing(gid).unwrap_or(0) as f32 * scale;
    let bb     = face.glyph_bounding_box(gid);
    let bear_y = bb.map(|b| b.y_max as f32 * scale).unwrap_or(0.0);
    let g_w    = bb.map(|b| (b.x_max - b.x_min) as f32 * scale).unwrap_or(0.0);
    let g_h    = bb.map(|b| (b.y_max - b.y_min) as f32 * scale).unwrap_or(0.0);

    // Generate SDF bitmap.
    let (sdf, bw, bh) = generate_sdf(&face, gid, px_size).unwrap_or_else(|| {
        // Whitespace / control chars: use a blank slot.
        (vec![0u8; 1], 1, 1)
    });

    // Pack into atlas.
    let mut atlas = ATLAS.get()?.lock().unwrap();
    let (ax, ay) = atlas.pack(bw, bh)?;
    atlas.blit(&sdf, ax, ay, bw, bh);

    let aw = atlas.width  as f32;
    let ah = atlas.height as f32;

    let metrics = GlyphMetrics {
        advance_width: adv,
        bearing_x: bear_x, bearing_y: bear_y,
        width: g_w, height: g_h,
        uv_x: ax as f32 / aw,
        uv_y: ay as f32 / ah,
        uv_w: bw as f32 / aw,
        uv_h: bh as f32 / ah,
    };

    atlas.glyphs.insert((font_id, codepoint), metrics);
    Some(metrics)
}

/// Layout a text string into a list of (pen_x, pen_y, GlyphMetrics) quads.
pub fn layout_text(
    font_data: &[u8],
    text: &str,
    font_id: u32,
    px_size: f32,
    ox: f32, oy: f32,
) -> Vec<(f32, f32, GlyphMetrics)> {
    let mut out = Vec::new();
    let mut x = ox;
    for ch in text.chars() {
        if let Some(m) = rasterize_glyph(font_data, font_id, ch as u32, px_size) {
            out.push((x + m.bearing_x, oy - m.bearing_y, m));
            x += m.advance_width;
        }
    }
    out
}

/// Measure the advance width of a string without laying it out.
pub fn measure_text(font_data: &[u8], text: &str, font_id: u32, px_size: f32) -> f32 {
    text.chars()
        .filter_map(|c| rasterize_glyph(font_data, font_id, c as u32, px_size))
        .map(|m| m.advance_width)
        .sum()
}

/// Return a reference to the raw atlas pixel data (for GPU upload).
pub fn atlas_data() -> Option<Vec<u8>> {
    Some(ATLAS.get()?.lock().unwrap().data.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atlas_initialises() {
        init();
        let a = ATLAS.get().unwrap().lock().unwrap();
        assert_eq!(a.data.len(), 2048 * 512);
    }
}
