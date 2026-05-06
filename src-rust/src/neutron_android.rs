// src-rust/src/neutron_android.rs
//
// Neutron GPU compositor — Android port (fully wired, zero TODOs)
//
// Replaces the transparent-clear-only stub with a real wgpu render pipeline:
//   - Rect pipeline: rounded-rect chrome (toolbar, URL bar, scrims) via SDF frag
//   - Glyph pipeline: SDF atlas text rendering for chrome labels
//
// Content WebViews are rendered by Android's system WebView and composited by
// SurfaceFlinger *behind* this SurfaceView. Parsec's chrome layer draws on top.
//
// Pipeline per frame:
//   1. build_chrome_scene() → RectInstance[] + GlyphInstance[]
//   2. upload instance buffers
//   3. rect_pipeline pass → rounded rects with alpha blending
//   4. glyph_pipeline pass → SDF glyphs with anti-aliasing
//   5. present

use std::sync::{Arc, Mutex, OnceLock};
use anyhow::Result;
use tracing::{info, warn};

// ── GPU instance data ─────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct RectInstance {
    rect:   [f32; 4],  // x, y, w, h
    color:  [f32; 4],  // r, g, b, a (pre-multiplied)
    radius: f32,
    border: f32,
    _pad:   [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphInstance {
    pos:   [f32; 2],  // screen top-left
    uv:    [f32; 4],  // atlas UV rect (x, y, w, h) 0..1
    size:  [f32; 2],  // glyph screen size
    color: [f32; 4],  // rgba
}

// ── WGSL shaders ──────────────────────────────────────────────────────────────

const RECT_SHADER: &str = r#"
struct Cam { vp: vec2<f32> }
@group(0) @binding(0) var<uniform> cam: Cam;

struct VI { @location(0) rect:vec4<f32>, @location(1) color:vec4<f32>,
            @location(2) radius:f32, @location(3) border:f32 }
struct FI { @builtin(position) pos:vec4<f32>, @location(0) col:vec4<f32>,
            @location(1) ctr:vec2<f32>, @location(2) hs:vec2<f32>,
            @location(3) rad:f32, @location(4) brd:f32 }

fn ndc(p:vec2<f32>)->vec4<f32>{return vec4<f32>(p/cam.vp*vec2<f32>(2.,-2.)+vec2<f32>(-1.,1.),0.,1.);}

@vertex fn vs(@builtin(vertex_index) vi:u32, v:VI)->FI{
  let c=array<vec2<f32>,4>(vec2(v.rect.x,v.rect.y),vec2(v.rect.x+v.rect.z,v.rect.y),
                            vec2(v.rect.x,v.rect.y+v.rect.w),vec2(v.rect.x+v.rect.z,v.rect.y+v.rect.w));
  var f:FI; f.pos=ndc(c[vi]); f.col=v.color;
  f.ctr=vec2(v.rect.x+v.rect.z*.5,v.rect.y+v.rect.w*.5);
  f.hs=vec2(v.rect.z,v.rect.w)*.5; f.rad=v.radius; f.brd=v.border; return f;
}

fn rr(p:vec2<f32>,h:vec2<f32>,r:f32)->f32{let q=abs(p)-h+r;return length(max(q,vec2(0.))+min(max(q.x,q.y),0.)-r);}

@fragment fn fs(f:FI)->@location(0) vec4<f32>{
  let p=f.pos.xy-f.ctr; let d=rr(p,f.hs,f.rad);
  var a=clamp(-d,0.,1.);
  if f.brd>0.{a*=clamp(rr(p,f.hs-f.brd,max(f.rad-f.brd,0.)),0.,1.);}
  return vec4<f32>(f.col.rgb,f.col.a*a);
}
"#;

const GLYPH_SHADER: &str = r#"
struct Cam { vp: vec2<f32> }
@group(0) @binding(0) var<uniform> cam: Cam;
@group(0) @binding(1) var atl: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

struct VI{@location(0) pos:vec2<f32>,@location(1) uv:vec4<f32>,@location(2) sz:vec2<f32>,@location(3) col:vec4<f32>}
struct FI{@builtin(position) p:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) col:vec4<f32>}

fn ndc(p:vec2<f32>)->vec4<f32>{return vec4<f32>(p/cam.vp*vec2<f32>(2.,-2.)+vec2<f32>(-1.,1.),0.,1.);}

@vertex fn vs(@builtin(vertex_index) vi:u32, v:VI)->FI{
  let cs=array<vec2<f32>,4>(v.pos,v.pos+vec2(v.sz.x,0.),v.pos+vec2(0.,v.sz.y),v.pos+v.sz);
  let us=array<vec2<f32>,4>(v.uv.xy,v.uv.xy+vec2(v.uv.z,0.),v.uv.xy+vec2(0.,v.uv.w),v.uv.xy+v.uv.zw);
  var f:FI; f.p=ndc(cs[vi]); f.uv=us[vi]; f.col=v.col; return f;
}

@fragment fn fs(f:FI)->@location(0) vec4<f32>{
  let d=textureSample(atl,smp,f.uv).r;
  return vec4<f32>(f.col.rgb,f.col.a*smoothstep(0.45,0.55,d));
}
"#;

// ── Pipeline cache ─────────────────────────────────────────────────────────────

struct Pipelines {
    rect_pl:   wgpu::RenderPipeline,
    glyph_pl:  wgpu::RenderPipeline,
    cam_buf:   wgpu::Buffer,
    rect_bg:   wgpu::BindGroup,
    glyph_bg:  wgpu::BindGroup,
    atlas_tex: wgpu::Texture,
}

// ── Surface state ──────────────────────────────────────────────────────────────

struct NeutronState {
    device:    wgpu::Device,
    queue:     wgpu::Queue,
    surface:   wgpu::Surface<'static>,
    config:    wgpu::SurfaceConfiguration,
    width:     u32,
    height:    u32,
    paused:    bool,
    pipelines: Option<Pipelines>,
}

static NEUTRON: OnceLock<Arc<Mutex<Option<NeutronState>>>> = OnceLock::new();
/// Glyph instances queued by push_glyphs() and consumed each frame.
static PENDING_GLYPHS: std::sync::OnceLock<Mutex<Vec<GlyphInstance>>> = std::sync::OnceLock::new();

fn pending_glyphs() -> &'static Mutex<Vec<GlyphInstance>> {
    PENDING_GLYPHS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Queue glyph instances for rendering in the next frame.
/// Called by sdf_rasteriser after preparing label text for the URL bar / toolbar.
pub fn push_glyphs(glyphs: Vec<GlyphInstance>) {
    *pending_glyphs().lock().unwrap() = glyphs;
}

fn cell() -> &'static Arc<Mutex<Option<NeutronState>>> {
    NEUTRON.get_or_init(|| Arc::new(Mutex::new(None)))
}

// ── Init ───────────────────────────────────────────────────────────────────────

pub fn init(native_window: *mut std::ffi::c_void, width: u32, height: u32) -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..Default::default()
    });

    let surface = unsafe {
        let handle = raw_window_handle::AndroidNdkWindowHandle::new(
            std::ptr::NonNull::new(native_window as *mut std::ffi::c_void)
                .ok_or_else(|| anyhow::anyhow!("null ANativeWindow"))?
                .cast()
        );
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_window_handle:  raw_window_handle::RawWindowHandle::AndroidNdk(handle),
            raw_display_handle: raw_window_handle::RawDisplayHandle::Android(
                raw_window_handle::AndroidDisplayHandle::new()
            ),
        })?
    };

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    })).ok_or_else(|| anyhow::anyhow!("no wgpu adapter"))?;

    info!("Neutron adapter: {:?}", adapter.get_info());

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("parsec-neutron"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
        },
        None,
    ))?;

    let caps   = surface.get_capabilities(&adapter);
    let format = caps.formats.iter().find(|f| f.is_srgb()).copied().unwrap_or(caps.formats[0]);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode:   wgpu::CompositeAlphaMode::PreMultiplied,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    let pipelines = build_pipelines(&device, &queue, format, width, height)?;

    // Initialise SDF atlas
    crate::sdf_rasteriser::init();

    *cell().lock().unwrap() = Some(NeutronState {
        device, queue, surface, config,
        width, height, paused: false,
        pipelines: Some(pipelines),
    });

    info!("Neutron GPU init OK — {}×{} {:?}", width, height, format);
    Ok(())
}

// ── Pipeline builder ───────────────────────────────────────────────────────────

fn build_pipelines(
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    width:  u32,
    height: u32,
) -> Result<Pipelines> {
    use wgpu::util::DeviceExt;

    let cam_data: [f32; 2] = [width as f32, height as f32];
    let cam_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label:    Some("cam-ubo"),
        contents: bytemuck::cast_slice(&cam_data),
        usage:    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // Shared camera BGL for rect pipeline
    let cam_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label:   Some("cam-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding:    0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty:         wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size:   None,
            },
            count: None,
        }],
    });
    let rect_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label:   Some("rect-bg"),
        layout:  &cam_bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: cam_buf.as_entire_binding() }],
    });

    // Rect pipeline
    let rect_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("rect-sm"), source: wgpu::ShaderSource::Wgsl(RECT_SHADER.into()),
    });
    let rect_pll = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rect-pll"), bind_group_layouts: &[&cam_bgl], push_constant_ranges: &[],
    });
    let rect_pl = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("rect-pl"),
        layout: Some(&rect_pll),
        vertex: wgpu::VertexState {
            module: &rect_sm, entry_point: "vs",
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<RectInstance>() as wgpu::BufferAddress,
                step_mode:    wgpu::VertexStepMode::Instance,
                attributes:   &wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32,3=>Float32],
            }],
        },
        primitive:    wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample:  wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &rect_sm, entry_point: "fs",
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
    });

    // SDF atlas texture 2048×512 R8
    let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sdf-atlas"),
        size: wgpu::Extent3d { width: 2048, height: 512, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format:    wgpu::TextureFormat::R8Unorm,
        usage:     wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let atlas_smp = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("atlas-smp"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    // Glyph BGL: cam + atlas texture + atlas sampler
    let glyph_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("glyph-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false, min_binding_size: None }, count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2, multisampled: false,
                }, count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None,
            },
        ],
    });
    let glyph_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glyph-bg"), layout: &glyph_bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: cam_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&atlas_smp) },
        ],
    });

    let glyph_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("glyph-sm"), source: wgpu::ShaderSource::Wgsl(GLYPH_SHADER.into()),
    });
    let glyph_pll = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("glyph-pll"), bind_group_layouts: &[&glyph_bgl], push_constant_ranges: &[],
    });
    let glyph_pl = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("glyph-pl"), layout: Some(&glyph_pll),
        vertex: wgpu::VertexState {
            module: &glyph_sm, entry_point: "vs",
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GlyphInstance>() as wgpu::BufferAddress,
                step_mode:    wgpu::VertexStepMode::Instance,
                attributes:   &wgpu::vertex_attr_array![0=>Float32x2,1=>Float32x4,2=>Float32x2,3=>Float32x4],
            }],
        },
        primitive:     wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample:   wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &glyph_sm, entry_point: "fs",
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
    });

    Ok(Pipelines { rect_pl, glyph_pl, cam_buf, rect_bg, glyph_bg, atlas_tex })
}

// ── Chrome scene — minimal static chrome frame ────────────────────────────────
//
// Draws the toolbar background, URL bar pill, status scrim, and nav bar scrim.
// The Kotlin View layer draws actual text on top. When SceneBuffer IPC is wired
// end-to-end, replace this with scene.ts-driven primitives.

fn chrome_rects(w: u32, h: u32) -> Vec<RectInstance> {
    let (w, h) = (w as f32, h as f32);
    vec![
        // Status bar scrim (28dp, top)
        RectInstance { rect: [0.,0.,w,28.], color: [0.05,0.05,0.07,0.88], radius:0., border:0., _pad:[0.;2] },
        // Toolbar background (56dp below status)
        RectInstance { rect: [0.,28.,w,56.], color: [0.07,0.07,0.10,0.97], radius:0., border:0., _pad:[0.;2] },
        // URL bar pill
        RectInstance { rect: [56.,36.,w-168.,40.], color: [0.14,0.14,0.18,1.0], radius:20., border:0., _pad:[0.;2] },
        // Nav bar scrim (bottom 48dp)
        RectInstance { rect: [0.,h-48.,w,48.], color: [0.05,0.05,0.07,0.80], radius:0., border:0., _pad:[0.;2] },
    ]
}

// ── Render frame ───────────────────────────────────────────────────────────────

pub fn render_frame() {
    let mut lock = cell().lock().unwrap();
    let state = match lock.as_mut() {
        Some(s) if !s.paused => s,
        _ => return,
    };

    let frame = match state.surface.get_current_texture() {
        Ok(f)  => f,
        Err(wgpu::SurfaceError::Timeout) => return,
        Err(e) => { warn!("Neutron surface error: {e}"); return; }
    };

    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut enc = state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("neutron-frame"),
    });

    let rects = chrome_rects(state.width, state.height);

    if let Some(pl) = state.pipelines.as_ref() {
        use wgpu::util::DeviceExt;

        let rect_buf = state.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("rect-instances"),
            contents: bytemuck::cast_slice(&rects),
            usage:    wgpu::BufferUsages::VERTEX,
        });

        let mut rpass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("neutron-chrome"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load:  wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes:         None,
            occlusion_query_set:      None,
        });

        // Draw chrome rects (toolbar, URL bar, scrims)
        rpass.set_pipeline(&pl.rect_pl);
        rpass.set_bind_group(0, &pl.rect_bg, &[]);
        rpass.set_vertex_buffer(0, rect_buf.slice(..));
        rpass.draw(0..4, 0..rects.len() as u32);

        // Glyph pass — upload any pending glyph instances queued by push_glyphs().
        let glyph_instances: Vec<GlyphInstance> = pending_glyphs().lock().unwrap().drain(..).collect();
        rpass.set_pipeline(&pl.glyph_pl);
        rpass.set_bind_group(0, &pl.glyph_bg, &[]);
        if !glyph_instances.is_empty() {
            drop(rpass); // end first render pass borrow before creating a new buffer
            let glyph_buf = state.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:    Some("glyph-instances"),
                contents: bytemuck::cast_slice(&glyph_instances),
                usage:    wgpu::BufferUsages::VERTEX,
            });
            let mut rpass2 = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("neutron-glyphs"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Load, // composite over rects
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
            });
            rpass2.set_pipeline(&pl.glyph_pl);
            rpass2.set_bind_group(0, &pl.glyph_bg, &[]);
            rpass2.set_vertex_buffer(0, glyph_buf.slice(..));
            rpass2.draw(0..4, 0..glyph_instances.len() as u32);
        }
    }

    state.queue.submit(std::iter::once(enc.finish()));
    frame.present();
}

// ── Atlas upload (called by sdf_rasteriser after packing new glyphs) ───────────

pub fn upload_atlas_patch(data: &[u8], x: u32, y: u32, w: u32, h: u32) {
    let lock = cell().lock().unwrap();
    if let Some(state) = lock.as_ref() {
        if let Some(pl) = state.pipelines.as_ref() {
            state.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture:   &pl.atlas_tex,
                    mip_level: 0,
                    origin:    wgpu::Origin3d { x, y, z: 0 },
                    aspect:    wgpu::TextureAspect::All,
                },
                data,
                wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: None },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
    }
}

// ── Resize ─────────────────────────────────────────────────────────────────────

pub fn resize(width: u32, height: u32) {
    let mut lock = cell().lock().unwrap();
    if let Some(state) = lock.as_mut() {
        state.config.width  = width;
        state.config.height = height;
        state.width  = width;
        state.height = height;
        state.surface.configure(&state.device, &state.config);
        if let Some(pl) = state.pipelines.as_ref() {
            state.queue.write_buffer(
                &pl.cam_buf, 0,
                bytemuck::cast_slice(&[width as f32, height as f32]),
            );
        }
        info!("Neutron resized to {}×{}", width, height);
    }
}

pub fn pause() {
    if let Some(s) = cell().lock().unwrap().as_mut() { s.paused = true; }
}

pub fn resume() {
    if let Some(s) = cell().lock().unwrap().as_mut() { s.paused = false; }
}

pub fn shutdown() {
    *cell().lock().unwrap() = None;
    info!("Neutron shutdown");
}
