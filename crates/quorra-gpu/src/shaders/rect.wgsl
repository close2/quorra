// The rectangle lane: exact analytic coverage for axis-aligned rectangles.
//
// RENDER_LIBRARY.md §6.4: a rectangle is not a path. No tiling, no binning, no edge
// list — one instanced quad per rectangle, and the fragment stage computes the exact
// area of overlap between the rectangle and each pixel's unit cell.
//
// Anti-aliasing by exact pixel-area coverage is a deliberate choice of ours, recorded
// in doc/adr/0005: ISO 32000-2 does not define anti-aliasing (§10.7.4 discusses only
// image interpolation), so the choice is documented as a choice rather than presented
// as normative.
//
// Determinism (§4.6, doc/adr/0005): every operation below is plain IEEE 754 f32
// arithmetic — floor, ceil, min, max, multiply, divide — with no derivatives, no
// subgroup operations, and no ordering dependence. Quad edges land on integers and
// pixel centres on half-integers, so no fragment is ever on a rasterisation tie.

struct Globals {
    // Target size in pixels. Only the vertex stage needs it, for the NDC mapping.
    target_size: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
// The active soft mask at device resolution (§11.5's soft clip), a 1x1 white
// stand-in when absent. Bindings 0 and 1 of this group belong to the coverage
// lane's shader; the layout is shared so both lanes bind one group.
@group(1) @binding(2) var soft_mask_tex: texture_2d<f32>;

struct Instance {
    // Device-space rectangle: min.xy in .xy, max.xy in .zw. Already transformed and
    // ordered on the CPU (encode.rs), which is what keeps this shader a pure evaluator.
    @location(0) rect: vec4<f32>,
    // Premultiplied device RGB. Premultiplication happened once, at encode time —
    // straight alpha at the boundary, premultiplied internally (§3).
    @location(1) color: vec4<f32>,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: Instance) -> VsOut {
    // Four strip vertices; corner in {0,1}².
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    // The quad covers every pixel the rectangle touches: expanded outward to the
    // enclosing pixel grid so that partially-covered border pixels get fragments.
    let quad_min = floor(instance.rect.xy);
    let quad_max = ceil(instance.rect.zw);
    let pos = mix(quad_min, quad_max, corner);
    // Device pixel space (y down, origin top-left) to NDC (y up). The scene-to-device
    // transform, including the page's y flip, was applied on the CPU; this is only the
    // fixed pixels-to-NDC map.
    let ndc = vec2<f32>(
        pos.x / globals.target_size.x * 2.0 - 1.0,
        1.0 - pos.y / globals.target_size.y * 2.0,
    );
    var out: VsOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.rect = instance.rect;
    out.color = instance.color;
    return out;
}

// Coverage = area of (rect ∩ pixel cell), times the soft mask (§11.5.1's soft
// clip). Each extent is in [0, 1] by construction, so the product is the exact
// area — no approximation at corners, where the two partial extents multiply.
fn coverage_at(in: VsOut) -> f32 {
    // position.xy is the pixel centre (k + 0.5 exactly); floor recovers the pixel's
    // integer corner, and the pixel's cell is [px, px+1) × [py, py+1).
    let px = floor(in.position.xy);
    let overlap_min = max(in.rect.xy, px);
    let overlap_max = min(in.rect.zw, px + vec2<f32>(1.0, 1.0));
    let extent = max(overlap_max - overlap_min, vec2<f32>(0.0, 0.0));
    let mask_dims = textureDimensions(soft_mask_tex);
    let mask_texel = min(vec2i(px), vec2i(mask_dims) - vec2i(1, 1));
    let mask = textureLoad(soft_mask_tex, mask_texel, 0).r;
    return extent.x * extent.y * mask;
}

// Premultiplied source scaled by coverage; the fixed-function blend is
// (ONE, ONE_MINUS_SRC_ALPHA), the premultiplied over operator.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color * coverage_at(in);
}

// The element's SHAPE alone (§11.4.7.2: object shape ∧ clip ∧ mask shape — the
// paint's alpha is opacity, not shape). The knockout erase pass scales the backdrop
// by (1 − shape) through the (ZERO, ONE_MINUS_SRC_ALPHA) blend; the add pass then
// deposits shape · element (ADR 0010's two-pass knockout).
@fragment
fn fs_shape(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(coverage_at(in));
}
