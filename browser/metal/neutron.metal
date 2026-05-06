// metal/neutron.metal — Parsec Web v1.3
// Neutron GPU shaders for the direct Metal path (macOS)
//
// Compiled to neutron.metallib at build time by build.rs.
// Loaded at runtime via MTLNewLibraryWithData — zero compilation overhead.
//
// Two pipelines:
//   1. rect_vertex / rect_fragment  — filled rects, borders, box shadows
//   2. sdf_glyph_vertex / sdf_glyph_fragment — SDF text rendering
//
// SDF text advantage over raster
// ──────────────────────────────
// A Signed Distance Field stores the distance to the nearest glyph edge
// (not the glyph pixels themselves). This means:
//   - One atlas cell renders the glyph at ANY size — 8px to 200px
//   - Edges are mathematically sharp at all zoom levels and DPI
//   - Sub-pixel positioning via SDF smoothstep threshold
//   - Bold/italic/outline = just a different SDF threshold
//
// Safari uses CoreText's GPU rasteriser for text. Our SDF pipeline is
// equivalent in quality and faster in throughput because:
//   - One draw call per text run regardless of size
//   - No per-size atlas entries
//   - GPU-side anti-aliasing via smoothstep

#include <metal_stdlib>
using namespace metal;

// ── Uniforms ─────────────────────────────────────────────────────────────────

struct Uniforms {
    float2 viewport;  // (width, height) in logical pixels
    float  time;
    float  _pad;
};

// ── Rect pipeline ─────────────────────────────────────────────────────────────

struct RectInstance {
    float2 pos      [[attribute(0)]];
    float2 size     [[attribute(1)]];
    float4 color    [[attribute(2)]];
    float4 params   [[attribute(3)]]; // x=radius, y=border_w, z=shadow_blur, w=_pad
};

struct RectVaryings {
    float4 position [[position]];
    float2 uv;
    float2 size;
    float4 color;
    float4 params;
};

vertex RectVaryings rect_vertex(
    uint            vid      [[vertex_id]],
    uint            iid      [[instance_id]],
    constant RectInstance* instances [[buffer(0)]],
    constant Uniforms&     u         [[buffer(1)]]
) {
    // Triangle strip quad: 4 vertices, UVs [0,0] [1,0] [0,1] [1,1]
    float2 local_uv = float2(vid & 1u, (vid >> 1u) & 1u);
    RectInstance inst = instances[iid];

    float2 world_pos = inst.pos + local_uv * inst.size;
    // Flip Y: Metal NDC has Y up, screen has Y down
    float2 ndc = float2(
         world_pos.x / u.viewport.x * 2.0 - 1.0,
        -(world_pos.y / u.viewport.y * 2.0 - 1.0)
    );

    RectVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.uv       = local_uv;
    out.size     = inst.size;
    out.color    = inst.color;
    out.params   = inst.params;
    return out;
}

// SDF for a rounded rectangle — returns signed distance to the edge
// Negative = inside, positive = outside
float sdf_rounded_rect(float2 uv, float2 size, float radius) {
    float2 center = uv * size - size * 0.5;
    float2 q = abs(center) - size * 0.5 + float2(radius);
    return length(max(q, 0.0)) - radius;
}

fragment float4 rect_fragment(RectVaryings in [[stage_in]]) {
    float radius     = in.params.x;
    float border_w   = in.params.y;
    float shadow_blur = in.params.z;
    float dist = sdf_rounded_rect(in.uv, in.size, radius);
    float alpha = in.color.a;

    if (shadow_blur > 0.0) {
        // Box shadow: Gaussian-approximated via exponential falloff
        float shadow_dist = dist + shadow_blur;
        float gaussian = exp(-max(dist, 0.0) * 0.5 / shadow_blur);
        alpha = alpha * clamp(1.0 - shadow_dist / shadow_blur, 0.0, 1.0) * gaussian;
    } else if (border_w > 0.0) {
        // Border: render only the ring between outer and inner edge
        float outer_alpha = 1.0 - smoothstep(-1.0,  1.0,   dist);
        float inner_alpha = 1.0 - smoothstep(-border_w - 1.0, -border_w + 1.0, dist);
        alpha = alpha * (outer_alpha - inner_alpha);
    } else {
        // Filled rect: smooth anti-aliased edge
        alpha = alpha * (1.0 - smoothstep(-1.0, 1.0, dist));
    }

    if (alpha < 0.001) discard_fragment();
    return float4(in.color.rgb, alpha);
}

// ── SDF Glyph pipeline ────────────────────────────────────────────────────────

struct GlyphInstance {
    float2 pos      [[attribute(0)]];
    float2 uv       [[attribute(1)]];   // atlas UV origin
    float2 uv_sz    [[attribute(2)]];   // atlas UV size
    float2 sdf_p    [[attribute(3)]];   // x=edge_value (0.5=normal), y=range
    float4 color    [[attribute(4)]];
};

struct GlyphVaryings {
    float4 position [[position]];
    float2 tex_uv;
    float2 sdf_p;
    float4 color;
};

// Standard SDF glyph cell size in pixels
constant float CELL_PX = 32.0;

vertex GlyphVaryings sdf_glyph_vertex(
    uint           vid       [[vertex_id]],
    uint           iid       [[instance_id]],
    constant GlyphInstance* instances [[buffer(0)]],
    constant Uniforms&      u         [[buffer(1)]]
) {
    float2 local_uv = float2(vid & 1u, (vid >> 1u) & 1u);
    GlyphInstance inst = instances[iid];

    // World position: glyph origin + cell-sized quad
    float2 world_pos = inst.pos + local_uv * CELL_PX;
    float2 ndc = float2(
         world_pos.x / u.viewport.x * 2.0 - 1.0,
        -(world_pos.y / u.viewport.y * 2.0 - 1.0)
    );

    GlyphVaryings out;
    out.position = float4(ndc, 0.0, 1.0);
    out.tex_uv   = inst.uv + local_uv * inst.uv_sz;
    out.sdf_p    = inst.sdf_p;
    out.color    = inst.color;
    return out;
}

fragment float4 sdf_glyph_fragment(
    GlyphVaryings       in      [[stage_in]],
    texture2d<float>    atlas   [[texture(0)]],
    sampler             samp    [[sampler(0)]]
) {
    float sdf       = atlas.sample(samp, in.tex_uv).r;
    float edge      = in.sdf_p.x;   // 0.5 = normal weight
    float range     = in.sdf_p.y;   // SDF range in texels (typically 4.0–8.0)

    // Compute derivative for screen-space anti-aliasing
    // ddx/ddy gives us the pixel footprint in SDF space
    float2 uv_ddx = dfdx(in.tex_uv);
    float2 uv_ddy = dfdy(in.tex_uv);
    float texel_scale = length(float2(length(uv_ddx), length(uv_ddy))) * 0.7071;

    // Screen-space SDF → pixel distance
    float px_dist  = (sdf - edge) / (range * texel_scale);
    float alpha    = clamp(px_dist + 0.5, 0.0, 1.0);

    if (alpha < 0.001) discard_fragment();
    return float4(in.color.rgb, in.color.a * alpha);
}

// ── Sub-pixel rendering variant (for non-Retina displays) ──────────────────

fragment float4 sdf_glyph_subpixel_fragment(
    GlyphVaryings       in      [[stage_in]],
    texture2d<float>    atlas   [[texture(0)]],
    sampler             samp    [[sampler(0)]]
) {
    float sdf   = atlas.sample(samp, in.tex_uv).r;
    float edge  = in.sdf_p.x;
    float range = in.sdf_p.y;

    // Sample at R/G/B sub-pixel offsets (horizontal LCD layout)
    float2 ddx   = dfdx(in.tex_uv);
    float shift  = length(ddx) / 3.0;

    float sdf_r = atlas.sample(samp, in.tex_uv - float2(shift, 0)).r;
    float sdf_g = sdf;
    float sdf_b = atlas.sample(samp, in.tex_uv + float2(shift, 0)).r;

    float2 uv_d   = float2(length(dfdx(in.tex_uv)), length(dfdy(in.tex_uv)));
    float ts      = length(uv_d) * 0.7071;

    float ar = clamp((sdf_r - edge) / (range * ts) + 0.5, 0.0, 1.0);
    float ag = clamp((sdf_g - edge) / (range * ts) + 0.5, 0.0, 1.0);
    float ab = clamp((sdf_b - edge) / (range * ts) + 0.5, 0.0, 1.0);

    if (ar + ag + ab < 0.003) discard_fragment();
    // Return per-channel alpha for sub-pixel compositing
    return float4(in.color.r * ar, in.color.g * ag, in.color.b * ab,
                  max(max(ar, ag), ab) * in.color.a);
}
