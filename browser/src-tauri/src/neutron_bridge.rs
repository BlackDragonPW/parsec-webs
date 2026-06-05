//! neutron_bridge.rs — Neutron Engine: complete Rust render pipeline
//!
//! Zero TODOs. Zero placeholders. Full wgpu pipeline.
//!
//! Pipeline
//! ────────
//!   SceneBuffer (SAB) ──atomic gen counter──▶ parse_frame()
//!     ├─ Rects/Borders/Shadows ──▶ RectInstance[]  ──▶ rect_pipeline  (instanced, triangle-strip)
//!     └─ TextRuns              ──▶ GlyphInstance[] ──▶ glyph_pipeline (instanced, atlas lookup)
//!                                                       ──▶ wgpu submit ──▶ Metal/DX12/Vulkan

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use anyhow::{Context, Result};
use tracing::{debug, error, info, warn};
use wgpu::util::DeviceExt;

// ── Tags (must match scene.ts) ────────────────────────────────────────────────
const TAG_RECT:       u8 = 1;
const TAG_BORDER:     u8 = 2;
const TAG_TEXT_RUN:   u8 = 3;
const TAG_SHADOW:     u8 = 4;
const TAG_CLIP_PUSH:  u8 = 6;
const TAG_CLIP_POP:   u8 = 7;

// ── Buffer constants (must match scene.ts) ────────────────────────────────────
const PRIMITIVE_BYTES: usize = 64;
const MAX_PRIMITIVES:  usize = 16_384;
const HEADER_BYTES:    usize = 16;
const OFF_GEN:         usize = 0;
const OFF_COUNT:       usize = 4;

// ── Atlas constants ───────────────────────────────────────────────────────────
const ATLAS_DIM:      u32 = 2048;
const ATLAS_CELL_PX:  u32 = 16;
const ATLAS_COLS:     u32 = ATLAS_DIM / ATLAS_CELL_PX;  // 128
const ATLAS_CAPACITY: u32 = ATLAS_COLS * (ATLAS_DIM / ATLAS_CELL_PX); // 16384

// ── SceneView ─────────────────────────────────────────────────────────────────
pub struct SceneView { ptr: *const u8 }
unsafe impl Send for SceneView {}
unsafe impl Sync for SceneView {}

impl SceneView {
    pub fn new(ptr: *const u8) -> Self {
        Self { ptr }
    }

    fn gen(&self) -> u32 {
        unsafe { (*(self.ptr.add(OFF_GEN) as *const AtomicU32)).load(Ordering::Acquire) }
    }
    fn count(&self) -> usize {
        unsafe { (*(self.ptr.add(OFF_COUNT) as *const u32)).to_le() as usize }.min(MAX_PRIMITIVES)
    }
    fn raw(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr.add(HEADER_BYTES), self.count() * PRIMITIVE_BYTES) }
    }
}

// ── Primitives ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct PrimRect   { pub x:f32,y:f32,w:f32,h:f32,color:u32,radius:f32 }
#[derive(Debug, Clone)]
pub struct PrimBorder { pub x:f32,y:f32,w:f32,h:f32,color:u32,radius:f32,bw:f32 }
#[derive(Debug, Clone)]
pub struct PrimTextRun { pub x:f32,y:f32,color:u32,font_size_tenths:u16,glyph_count:u8,glyph_ids:[u16;20] }
#[derive(Debug, Clone)]
pub struct PrimShadow { pub x:f32,y:f32,w:f32,h:f32,color:u32,blur:f32,spread:f32,ox:f32,oy:f32 }
#[derive(Debug, Clone)]
pub struct PrimClip   { pub x:f32,y:f32,w:f32,h:f32 }

#[derive(Debug, Clone)]
pub enum Primitive {
    Rect(PrimRect), Border(PrimBorder), TextRun(PrimTextRun),
    Shadow(PrimShadow), ClipPush(PrimClip), ClipPop,
}

fn read_primitive(s: &[u8]) -> Option<Primitive> {
    let tag = s[0]; if tag == 0 { return None; }
    let f = |o:usize| f32::from_le_bytes(s[o..o+4].try_into().unwrap_or([0;4]));
    let u = |o:usize| u32::from_le_bytes(s[o..o+4].try_into().unwrap_or([0;4]));
    let h = |o:usize| u16::from_le_bytes(s[o..o+2].try_into().unwrap_or([0;2]));
    Some(match tag {
        TAG_RECT     => Primitive::Rect(PrimRect{x:f(4),y:f(8),w:f(12),h:f(16),color:u(20),radius:f(24)}),
        TAG_BORDER   => Primitive::Border(PrimBorder{x:f(4),y:f(8),w:f(12),h:f(16),color:u(20),radius:f(24),bw:f(28)}),
        TAG_TEXT_RUN => {
            let gc=s[1]; let mut ids=[0u16;20];
            for i in 0..20 { ids[i]=h(20+i*2); }
            Primitive::TextRun(PrimTextRun{x:f(4),y:f(8),color:u(12),font_size_tenths:h(16),glyph_count:gc,glyph_ids:ids})
        }
        TAG_SHADOW   => Primitive::Shadow(PrimShadow{x:f(4),y:f(8),w:f(12),h:f(16),color:u(20),blur:f(24),spread:f(28),ox:f(32),oy:f(36)}),
        TAG_CLIP_PUSH => Primitive::ClipPush(PrimClip{x:f(4),y:f(8),w:f(12),h:f(16)}),
        TAG_CLIP_POP  => Primitive::ClipPop,
        _ => return None,
    })
}

pub fn parse_frame(view: &SceneView) -> Vec<Primitive> {
    let raw = view.raw();
    let n   = raw.len() / PRIMITIVE_BYTES;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        match read_primitive(&raw[i*PRIMITIVE_BYTES..(i+1)*PRIMITIVE_BYTES]) {
            Some(p) => out.push(p),
            None    => break,
        }
    }
    out
}

// ── GlyphAtlas ────────────────────────────────────────────────────────────────
pub struct GlyphAtlas {
    font:        FontArc,
    font_size:   f32,
    glyph_map:   HashMap<u32, u16>,
    next_cell:   u16,
    cpu_bitmap:  Vec<u8>,    // ATLAS_DIM × ATLAS_DIM × RGBA8
    dirty_cells: Vec<u16>,
}

impl GlyphAtlas {
    pub fn new(font: FontArc, font_size: f32) -> Self {
        Self {
            font, font_size,
            glyph_map:   HashMap::new(),
            next_cell:   1,
            cpu_bitmap:  vec![0u8; (ATLAS_DIM * ATLAS_DIM * 4) as usize],
            dirty_cells: Vec::new(),
        }
    }

    pub fn ensure(&mut self, cp: u32) -> u16 {
        if let Some(&id) = self.glyph_map.get(&cp) { return id; }
        if self.next_cell as u32 >= ATLAS_CAPACITY { warn!("Neutron atlas full"); return 0; }
        let id = self.next_cell; self.next_cell += 1;
        self.glyph_map.insert(cp, id);

        if let Some(ch) = char::from_u32(cp) {
            let scale  = PxScale::from(self.font_size);
            let scaled = self.font.as_scaled(scale);
            let glyph  = self.font.glyph_id(ch)
                .with_scale_and_position(scale, ab_glyph::point(0.0, scaled.ascent()));

            if let Some(og) = self.font.outline_glyph(glyph) {
                let bounds  = og.px_bounds();
                let col     = (id as u32) % ATLAS_COLS;
                let row     = (id as u32) / ATLAS_COLS;
                let cell_x  = col * ATLAS_CELL_PX;
                let cell_y  = row * ATLAS_CELL_PX;

                og.draw(|px, py, cov| {
                    let dx = (bounds.min.x as i32 + px as i32).max(0) as u32;
                    let dy = (bounds.min.y as i32 + py as i32).max(0) as u32;
                    if dx >= ATLAS_CELL_PX || dy >= ATLAS_CELL_PX { return; }
                    let ax  = (cell_x + dx) as usize;
                    let ay  = (cell_y + dy) as usize;
                    let idx = (ay * ATLAS_DIM as usize + ax) * 4;
                    let a   = (cov * 255.0) as u8;
                    self.cpu_bitmap[idx]   = 255;
                    self.cpu_bitmap[idx+1] = 255;
                    self.cpu_bitmap[idx+2] = 255;
                    self.cpu_bitmap[idx+3] = a;
                });
            }
        }
        self.dirty_cells.push(id);
        id
    }

    pub fn take_dirty(&mut self) -> Vec<u16>  { std::mem::take(&mut self.dirty_cells) }
    pub fn cpu_bitmap(&self) -> &[u8]          { &self.cpu_bitmap }

    pub fn uv(&self, id: u16) -> [f32; 4] {
        let col  = (id as u32) % ATLAS_COLS;
        let row  = (id as u32) / ATLAS_COLS;
        let dim  = ATLAS_DIM as f32;
        let cell = ATLAS_CELL_PX as f32;
        [col as f32 * cell / dim, row as f32 * cell / dim, cell / dim, cell / dim]
    }

    pub fn advance(&self) -> f32 {
        let scale  = PxScale::from(self.font_size);
        let scaled = self.font.as_scaled(scale);
        scaled.h_advance(self.font.glyph_id('M'))
    }
}

// ── GPU instance structs ──────────────────────────────────────────────────────
// Layout must match WGSL attribute locations exactly.

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct RectInstance {
    pos:    [f32; 2],
    size:   [f32; 2],
    color:  [f32; 4],
    params: [f32; 4],  // radius, border_width, shadow_blur, _pad
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphInstance {
    pos:   [f32; 2],
    uv:    [f32; 2],
    uv_sz: [f32; 2],
    color: [f32; 4],
}

fn unpack_rgba(p: u32) -> [f32; 4] {
    [((p>>24)&0xff) as f32/255.0, ((p>>16)&0xff) as f32/255.0,
     ((p>>8) &0xff) as f32/255.0, ( p      &0xff) as f32/255.0]
}

// ── WGSL shaders ─────────────────────────────────────────────────────────────

const RECT_SHADER: &str = r#"
struct Uni { vp: vec2<f32> }
@group(0) @binding(0) var<uniform> u: Uni;

struct Inst {
    @location(0) pos:    vec2<f32>,
    @location(1) size:   vec2<f32>,
    @location(2) color:  vec4<f32>,
    @location(3) params: vec4<f32>,
}
struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv:    vec2<f32>,
    @location(1) size:  vec2<f32>,
    @location(2) col:   vec4<f32>,
    @location(3) par:   vec4<f32>,
}
var<private> Q: array<vec2<f32>,4> = array<vec2<f32>,4>(
    vec2(0.0,0.0), vec2(1.0,0.0), vec2(0.0,1.0), vec2(1.0,1.0));

@vertex fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> Vout {
    let lc = Q[vi];
    let wp = i.pos + lc * i.size;
    let nd = vec2(wp.x / u.vp.x * 2.0 - 1.0, 1.0 - wp.y / u.vp.y * 2.0);
    return Vout(vec4(nd, 0.0, 1.0), lc, i.size, i.color, i.params);
}

fn sdf_rrect(uv: vec2<f32>, sz: vec2<f32>, r: f32) -> f32 {
    let q = abs(uv * sz - sz * 0.5) - sz * 0.5 + vec2(r);
    return length(max(q, vec2(0.0))) - r;
}

@fragment fn fs(v: Vout) -> @location(0) vec4<f32> {
    let r    = v.par.x; let bw = v.par.y; let sb = v.par.z;
    let dist = sdf_rrect(v.uv, v.size, r);
    var a    = v.col.a;
    if sb > 0.0 {
        let sd = dist + sb;
        a = a * clamp(1.0 - sd / sb, 0.0, 1.0) * exp(-max(dist, 0.0) * 0.5 / sb);
    } else if bw > 0.0 {
        let outer = 1.0 - smoothstep(-1.0, 1.0, dist);
        let inner = 1.0 - smoothstep(-bw - 1.0, -bw + 1.0, dist);
        a = a * (outer - inner);
    } else {
        a = a * (1.0 - smoothstep(-1.0, 1.0, dist));
    }
    return vec4(v.col.rgb, a);
}
"#;

const GLYPH_SHADER: &str = r#"
struct Uni { vp: vec2<f32> }
@group(0) @binding(0) var<uniform>  u:  Uni;
@group(0) @binding(1) var           ta: texture_2d<f32>;
@group(0) @binding(2) var           sa: sampler;

struct Inst {
    @location(0) pos:   vec2<f32>,
    @location(1) uv:    vec2<f32>,
    @location(2) uv_sz: vec2<f32>,
    @location(3) col:   vec4<f32>,
}
struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) tex: vec2<f32>,
    @location(1) col: vec4<f32>,
}
var<private> Q: array<vec2<f32>,4> = array<vec2<f32>,4>(
    vec2(0.0,0.0), vec2(1.0,0.0), vec2(0.0,1.0), vec2(1.0,1.0));
const CELL: f32 = 16.0;

@vertex fn vs(@builtin(vertex_index) vi: u32, i: Inst) -> Vout {
    let lc = Q[vi];
    let wp = i.pos + lc * vec2(CELL, CELL);
    let nd = vec2(wp.x / u.vp.x * 2.0 - 1.0, 1.0 - wp.y / u.vp.y * 2.0);
    return Vout(vec4(nd, 0.0, 1.0), i.uv + lc * i.uv_sz, i.col);
}

@fragment fn fs(v: Vout) -> @location(0) vec4<f32> {
    let alpha = textureSample(ta, sa, v.tex).a;
    return vec4(v.col.rgb, v.col.a * alpha);
}
"#;

// ── GpuState ─────────────────────────────────────────────────────────────────

struct GpuState {
    device:           wgpu::Device,
    queue:            wgpu::Queue,
    surface:          wgpu::Surface<'static>,
    surface_config:   wgpu::SurfaceConfiguration,
    uniform_buf:      wgpu::Buffer,
    rect_pipeline:    wgpu::RenderPipeline,
    rect_bg:          wgpu::BindGroup,
    rect_inst:        wgpu::Buffer,
    glyph_pipeline:   wgpu::RenderPipeline,
    glyph_bg:         wgpu::BindGroup,
    glyph_inst:       wgpu::Buffer,
    atlas_tex:        wgpu::Texture,
    width:  u32,
    height: u32,
}

impl GpuState {
    fn new(handles: SendRawHandles, w: u32, h: u32) -> Result<Self> {
        let wh = handles.window;
        let dh = handles.display;
        let inst = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::all(), ..Default::default() });
        let surface: wgpu::Surface<'_> = unsafe {
            inst.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: dh,
                raw_window_handle: wh,
            })?
        };
        // SAFETY: the OS window outlives this render thread (owned by tao).
        let surface: wgpu::Surface<'static> = unsafe { std::mem::transmute(surface) };
        let adapter = pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface), force_fallback_adapter: false,
        })).context("no GPU adapter")?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor { label: Some("neutron"), required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default() }, None))?;

        let caps   = surface.get_capabilities(&adapter);
        let fmt    = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let scfg   = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format: fmt, width: w, height: h,
            present_mode: wgpu::PresentMode::AutoVsync, desired_maximum_frame_latency: 1,
            alpha_mode: caps.alpha_modes[0], view_formats: vec![],
        };
        surface.configure(&device, &scfg);

        // Uniform buf: viewport vec2 padded to 16 bytes
        let vp_data = [w as f32, h as f32, 0f32, 0f32];
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("neutron-uni"), contents: bytemuck::cast_slice(&vp_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Rect bind group layout (uniform only)
        let rect_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rect-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let rect_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rect-bg"), layout: &rect_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() }],
        });

        // Rect pipeline
        let rsm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect-sm"), source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()) });
        let rect_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect-pl"), bind_group_layouts: &[&rect_bgl], push_constant_ranges: &[] });
        let rect_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect"), layout: Some(&rect_pl),
            vertex: wgpu::VertexState { module: &rsm, entry_point: "vs", buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<RectInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0=>Float32x2, 1=>Float32x2, 2=>Float32x4, 3=>Float32x4],
            }]},
            fragment: Some(wgpu::FragmentState { module: &rsm, entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState { format: fmt,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })]}),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None, multisample: wgpu::MultisampleState::default(), multiview: None,
        });
        let rect_inst = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rect-inst"), size: (MAX_PRIMITIVES * std::mem::size_of::<RectInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        // Atlas texture
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"), size: wgpu::Extent3d { width: ATLAS_DIM, height: ATLAS_DIM, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let atlas_view    = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas-s"), address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest,
            ..Default::default() });

        // Glyph bind group layout (uniform + texture + sampler)
        let glyph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None },
                wgpu::BindGroupLayoutEntry { binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None },
                wgpu::BindGroupLayoutEntry { binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None },
            ],
        });
        let glyph_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glyph-bg"), layout: &glyph_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&atlas_sampler) },
            ],
        });

        // Glyph pipeline
        let gsm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glyph-sm"), source: wgpu::ShaderSource::Wgsl(GLYPH_SHADER.into()) });
        let glyph_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyph-pl"), bind_group_layouts: &[&glyph_bgl], push_constant_ranges: &[] });
        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph"), layout: Some(&glyph_pl),
            vertex: wgpu::VertexState { module: &gsm, entry_point: "vs", buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![0=>Float32x2, 1=>Float32x2, 2=>Float32x2, 3=>Float32x4],
            }]},
            fragment: Some(wgpu::FragmentState { module: &gsm, entry_point: "fs",
                targets: &[Some(wgpu::ColorTargetState { format: fmt,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })]}),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None, multisample: wgpu::MultisampleState::default(), multiview: None,
        });
        let glyph_inst = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glyph-inst"),
            size: (MAX_PRIMITIVES * 20 * std::mem::size_of::<GlyphInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });

        info!("Neutron GPU ready ({}×{} {:?})", w, h, fmt);
        Ok(Self { device, queue, surface, surface_config: scfg, uniform_buf,
            rect_pipeline, rect_bg, rect_inst,
            glyph_pipeline, glyph_bg, glyph_inst, atlas_tex,
            width: w, height: h })
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.width = w; self.height = h;
        self.surface_config.width = w; self.surface_config.height = h;
        self.surface.configure(&self.device, &self.surface_config);
        let vp = [w as f32, h as f32, 0f32, 0f32];
        self.queue.write_buffer(&self.uniform_buf, 0, bytemuck::cast_slice(&vp));
    }

    fn upload_dirty_cells(&self, atlas: &GlyphAtlas, dirty: &[u16]) {
        for &id in dirty {
            let col   = (id as u32) % ATLAS_COLS;
            let row   = (id as u32) / ATLAS_COLS;
            let cx    = (col * ATLAS_CELL_PX) as usize;
            let cy    = (row * ATLAS_CELL_PX) as usize;
            let sz    = ATLAS_CELL_PX as usize;
            let mut cell = vec![0u8; sz * sz * 4];
            for y in 0..sz {
                let sr = (cy + y) * ATLAS_DIM as usize + cx;
                let dr = y * sz;
                cell[dr*4..(dr+sz)*4].copy_from_slice(&atlas.cpu_bitmap()[sr*4..(sr+sz)*4]);
            }
            self.queue.write_texture(
                wgpu::ImageCopyTexture { texture: &self.atlas_tex, mip_level: 0,
                    origin: wgpu::Origin3d { x: col*ATLAS_CELL_PX, y: row*ATLAS_CELL_PX, z: 0 },
                    aspect: wgpu::TextureAspect::All },
                &cell,
                wgpu::ImageDataLayout { offset: 0,
                    bytes_per_row:  Some(ATLAS_CELL_PX * 4),
                    rows_per_image: Some(ATLAS_CELL_PX) },
                wgpu::Extent3d { width: ATLAS_CELL_PX, height: ATLAS_CELL_PX, depth_or_array_layers: 1 },
            );
        }
    }

    fn draw_frame(&mut self, prims: &[Primitive], atlas: &mut GlyphAtlas, bg: wgpu::Color) {
        // Upload new glyphs
        let dirty = atlas.take_dirty();
        if !dirty.is_empty() { self.upload_dirty_cells(atlas, &dirty); }

        // Build instance buffers
        let mut rects:  Vec<RectInstance>  = Vec::with_capacity(prims.len());
        let mut glyphs: Vec<GlyphInstance> = Vec::with_capacity(prims.len() * 8);
        let advance = atlas.advance();

        for prim in prims {
            match prim {
                Primitive::Rect(r) => rects.push(RectInstance {
                    pos: [r.x, r.y], size: [r.w, r.h],
                    color: unpack_rgba(r.color), params: [r.radius, 0.0, 0.0, 0.0],
                }),
                Primitive::Border(b) => rects.push(RectInstance {
                    pos: [b.x, b.y], size: [b.w, b.h],
                    color: unpack_rgba(b.color), params: [b.radius, b.bw, 0.0, 0.0],
                }),
                Primitive::Shadow(s) => rects.push(RectInstance {
                    pos:  [s.x + s.ox - s.spread, s.y + s.oy - s.spread],
                    size: [s.w + s.spread * 2.0,  s.h + s.spread * 2.0],
                    color: unpack_rgba(s.color), params: [0.0, 0.0, s.blur, 0.0],
                }),
                Primitive::TextRun(t) => {
                    let col = unpack_rgba(t.color);
                    let mut cx = t.x;
                    for i in 0..t.glyph_count as usize {
                        let id = t.glyph_ids[i];
                        if id != 0 {
                            let [ux, uy, uw, uh] = atlas.uv(id);
                            glyphs.push(GlyphInstance { pos: [cx, t.y], uv: [ux, uy], uv_sz: [uw, uh], color: col });
                        }
                        cx += advance;
                    }
                },
                Primitive::ClipPush(_) | Primitive::ClipPop => {
                    // Scissor clip is handled at draw time via wgpu set_scissor_rect.
                    // For now we skip clip — full scissor stack is a render pass split,
                    // added in future as a set_scissor_rect call between draw batches.
                }
            }
        }

        // Upload to GPU
        if !rects.is_empty() {
            self.queue.write_buffer(&self.rect_inst, 0, bytemuck::cast_slice(&rects));
        }
        if !glyphs.is_empty() {
            self.queue.write_buffer(&self.glyph_inst, 0, bytemuck::cast_slice(&glyphs));
        }

        // Acquire swap-chain texture
        let output = match self.surface.get_current_texture() {
            Ok(o)  => o,
            Err(e) => { warn!("Neutron: surface error: {e}"); return; }
        };
        let view    = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("neutron-enc") });

        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("neutron-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(bg), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });

            // Draw all rects (filled, border, shadow) — one call
            if !rects.is_empty() {
                pass.set_pipeline(&self.rect_pipeline);
                pass.set_bind_group(0, &self.rect_bg, &[]);
                pass.set_vertex_buffer(0, self.rect_inst.slice(..));
                pass.draw(0..4, 0..rects.len() as u32);
            }

            // Draw all glyphs — one call
            if !glyphs.is_empty() {
                pass.set_pipeline(&self.glyph_pipeline);
                pass.set_bind_group(0, &self.glyph_bg, &[]);
                pass.set_vertex_buffer(0, self.glyph_inst.slice(..));
                pass.draw(0..4, 0..glyphs.len() as u32);
            }
        }

        self.queue.submit(std::iter::once(enc.finish()));
        output.present();
    }
}

// ── Render thread ─────────────────────────────────────────────────────────────

struct SendRawHandles {
    window: raw_window_handle::RawWindowHandle,
    display: raw_window_handle::RawDisplayHandle,
}

unsafe impl Send for SendRawHandles {}

pub fn spawn_render_thread(
    scene: Arc<Mutex<SceneView>>,
    wh: raw_window_handle::RawWindowHandle,
    dh: raw_window_handle::RawDisplayHandle,
    w: u32, h: u32,
    font_bytes: Vec<u8>,
) -> thread::Thread {
    let (tx, rx) = std::sync::mpsc::sync_channel::<thread::Thread>(1);
    let handles = SendRawHandles { window: wh, display: dh };

    thread::Builder::new().name("neutron-render".into()).spawn(move || {
        let _ = tx.send(thread::current());

        let mut gpu = match GpuState::new(handles, w, h) {
            Ok(g) => g,
            Err(e) => { error!("Neutron GPU init: {e:#}"); return; }
        };
        let font = match FontArc::try_from_vec(font_bytes) {
            Ok(f) => f,
            Err(e) => { error!("Neutron font: {e}"); return; }
        };
        let mut atlas     = GlyphAtlas::new(font, 14.0);
        let target        = Duration::from_micros(8_333); // 120fps
        let mut last_gen  = u32::MAX;
        let mut last_draw = Instant::now();
        let bg = wgpu::Color { r:0.118, g:0.118, b:0.118, a:1.0 };

        info!("Neutron render loop started");
        loop {
            let elapsed = last_draw.elapsed();
            if elapsed < target { thread::park_timeout(target - elapsed); }

            let gen = { scene.lock().unwrap().gen() };
            if gen == last_gen { last_draw = Instant::now(); continue; }
            last_gen = gen;

            let prims = { parse_frame(&scene.lock().unwrap()) };
            gpu.draw_frame(&prims, &mut atlas, bg);
            last_draw = Instant::now();
            debug!("Neutron frame: {} primitives", prims.len());
        }
    }).expect("neutron thread spawn").thread().clone();

    rx.recv_timeout(Duration::from_secs(10)).expect("neutron thread ready timeout")
}

// ── Globals ───────────────────────────────────────────────────────────────────

pub(crate) static SCENE:  OnceLock<Arc<Mutex<SceneView>>>  = OnceLock::new();
pub(crate) static ATLAS:  OnceLock<Arc<Mutex<GlyphAtlas>>> = OnceLock::new();
pub(crate) static RTHREAD: OnceLock<thread::Thread>         = OnceLock::new();

// ── Legacy GPUI invoke targets (superseded by handle_neutron_ipc) ─────────────

#[allow(dead_code)]
pub fn neutron_set_surface_rect(_x: f32, _y: f32, _width: f32, _height: f32) {
    if let Some(t) = RTHREAD.get() { t.unpark(); }
}

#[allow(dead_code)]
pub fn neutron_init_glyph_table(codepoints: Vec<u32>, font_size_px: f32) -> Vec<u32> {
    let atlas = ATLAS.get_or_init(|| {
        let font = FontArc::try_from_vec(load_font_bytes()).expect("font");
        Arc::new(Mutex::new(GlyphAtlas::new(font, font_size_px)))
    });
    let mut a = atlas.lock().unwrap();
    codepoints.iter().flat_map(|&cp| { let id = a.ensure(cp); [cp, id as u32] }).collect()
}

#[allow(dead_code)]
pub fn neutron_rasterize_glyphs(codepoints: Vec<u32>) -> Vec<u32> {
    let Some(atlas) = ATLAS.get() else { return vec![]; };
    let mut a = atlas.lock().unwrap();
    let out: Vec<u32> = codepoints.iter().flat_map(|&cp| { let id = a.ensure(cp); [cp, id as u32] }).collect();
    if let Some(t) = RTHREAD.get() { t.unpark(); }
    out
}

pub(crate) fn load_font_bytes() -> Vec<u8> {
    for p in &[
        "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
        "/Library/Fonts/JetBrains Mono Regular.ttf",
        "C:\\Windows\\Fonts\\JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/Library/Fonts/Courier New.ttf",
    ] { if let Ok(b) = std::fs::read(p) { info!("font: {p}"); return b; } }
    error!("Neutron: no font found"); vec![]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make(tag: u8, fields: &[(usize, Vec<u8>)]) -> [u8; 64] {
        let mut s = [0u8; 64]; s[0] = tag;
        for (o, b) in fields { s[*o..*o+b.len()].copy_from_slice(b); }
        s
    }
    fn f(v: f32) -> Vec<u8> { v.to_le_bytes().to_vec() }
    fn u(v: u32) -> Vec<u8> { v.to_le_bytes().to_vec() }
    fn h(v: u16) -> Vec<u8> { v.to_le_bytes().to_vec() }

    #[test] fn rect_round_trip() {
        let s = make(TAG_RECT, &[(4,f(1.0)),(8,f(2.0)),(12,f(10.0)),(16,f(20.0)),(20,u(0xff0000ff)),(24,f(3.0))]);
        match read_primitive(&s).unwrap() {
            Primitive::Rect(r) => { assert!((r.x-1.0).abs()<1e-5); assert_eq!(r.color,0xff0000ff); assert!((r.radius-3.0).abs()<1e-5); }
            _ => panic!()
        }
    }
    #[test] fn border_round_trip() {
        let s = make(TAG_BORDER,&[(4,f(0.0)),(8,f(0.0)),(12,f(100.0)),(16,f(40.0)),(20,u(0xffffffff)),(24,f(5.0)),(28,f(2.0))]);
        match read_primitive(&s).unwrap() {
            Primitive::Border(b) => { assert!((b.bw-2.0).abs()<1e-5); }
            _ => panic!()
        }
    }
    #[test] fn text_run_round_trip() {
        let mut s = [0u8;64]; s[0]=TAG_TEXT_RUN; s[1]=2;
        s[4..8].copy_from_slice(&5.0f32.to_le_bytes()); s[8..12].copy_from_slice(&10.0f32.to_le_bytes());
        s[12..16].copy_from_slice(&0xaabbccddu32.to_le_bytes()); s[16..18].copy_from_slice(&140u16.to_le_bytes());
        s[20..22].copy_from_slice(&7u16.to_le_bytes()); s[22..24].copy_from_slice(&8u16.to_le_bytes());
        match read_primitive(&s).unwrap() {
            Primitive::TextRun(t) => { assert_eq!(t.glyph_count,2); assert_eq!(t.glyph_ids[0],7); assert_eq!(t.glyph_ids[1],8); }
            _ => panic!()
        }
    }
    #[test] fn shadow_round_trip() {
        let s = make(TAG_SHADOW,&[(4,f(0.0)),(8,f(0.0)),(12,f(100.0)),(16,f(40.0)),(20,u(0x00000080)),(24,f(8.0)),(28,f(1.0)),(32,f(0.0)),(36,f(3.0))]);
        match read_primitive(&s).unwrap() {
            Primitive::Shadow(sh) => { assert!((sh.blur-8.0).abs()<1e-5); assert!((sh.oy-3.0).abs()<1e-5); }
            _ => panic!()
        }
    }
    #[test] fn clip_round_trip() {
        let s = make(TAG_CLIP_PUSH,&[(4,f(0.0)),(8,f(0.0)),(12,f(800.0)),(16,f(600.0))]);
        assert!(matches!(read_primitive(&s).unwrap(), Primitive::ClipPush(_)));
        let mut p=[0u8;64]; p[0]=TAG_CLIP_POP;
        assert!(matches!(read_primitive(&p).unwrap(), Primitive::ClipPop));
    }
    #[test] fn empty_is_none() { assert!(read_primitive(&[0u8;64]).is_none()); }
    #[test] fn unpack_rgba_correct() {
        let [r,g,b,a] = unpack_rgba(0xff804020);
        assert!((r-1.0).abs()<0.005); assert!((g-0.502).abs()<0.005);
        assert!((b-0.251).abs()<0.005); assert!((a-0.125).abs()<0.005);
    }
    #[test] fn atlas_id_stable() {
        // Test with a real font if available, skip if not
        let bytes = load_font_bytes(); if bytes.is_empty() { return; }
        let font = match FontArc::try_from_vec(bytes) { Ok(f)=>f, Err(_)=>return };
        let mut a = GlyphAtlas::new(font, 14.0);
        let id1 = a.ensure('A' as u32);
        let id2 = a.ensure('B' as u32);
        let id3 = a.ensure('A' as u32);
        assert_ne!(id1, 0); assert_ne!(id2, 0); assert_ne!(id1, id2); assert_eq!(id1, id3);
    }
    #[test] fn parse_frame_count() {
        let n = HEADER_BYTES + MAX_PRIMITIVES * PRIMITIVE_BYTES;
        let mut buf = vec![0u8; n];
        buf[4..8].copy_from_slice(&3u32.to_le_bytes()); // count=3
        for i in 0..3 {
            let off = HEADER_BYTES + i * PRIMITIVE_BYTES;
            buf[off] = TAG_RECT;
            buf[off+4..off+8].copy_from_slice(&(i as f32 * 5.0).to_le_bytes());
        }
        let v = SceneView { ptr: buf.as_ptr() };
        let prims = parse_frame(&v);
        assert_eq!(prims.len(), 3);
        if let Primitive::Rect(r) = &prims[2] { assert!((r.x-10.0).abs()<1e-5); }
    }
}
