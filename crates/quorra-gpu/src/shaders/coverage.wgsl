// The coverage-quad lane: glyphs from the atlas, paths from the per-frame scratch.
//
// RENDER_LIBRARY.md §6.3: on a GPU the blitter for cached coverage "is a textured
// quad, which is the natural primitive". One instanced quad per command; the fragment
// stage fetches the CPU-rasterised coverage byte (raster.rs, ADR 0008) and modulates
// it by the analytic clip-rectangle coverage of ADR 0007.
//
// Determinism (§4.6): texels are fetched with textureLoad — an exact fetch, no
// sampler, no filtering, no derivatives — and the coverage bytes were rasterised on
// the CPU, so they are identical on every adapter; only the fixed-function blend and
// store conversion vary, inside ADR 0006's stated bound.

struct Globals {
    target_size: vec2f,
    // The device-space corner this attachment's texel (0, 0) is (ADR 0036).
    //
    // A plan renders into a texture the size of *its own* content rather than the
    // target's, so device space and attachment space differ by this. Two places have to
    // agree about it and they are the whole of the mapping: the vertex stage subtracts it
    // before dividing by `target_size`, and the fragment stage adds it back to recover
    // the device pixel it is shading — clip rectangles, tile lookups and masks are all
    // stated in device space and would otherwise be read at the wrong place. Zero for the
    // frame's root, which renders into the target itself.
    origin: vec2f,
}

// Where the active soft mask sits and what it is elsewhere (ADR 0037). Per mask
// rather than per region, which is why it is in this group and not in `Globals`:
// the region is one pass's, and a pass draws batches under different masks.
struct MaskPlace {
    // Device corner of the mask's texel (0, 0) in .xy, its size in texels in .zw.
    rect: vec4f,
    // What the mask holds outside `rect`, in .x: the reduce's output for a
    // transparent pixel, since the mask's group marks nothing out there. A size of
    // (0, 0) is an absent mask, and then this is 1 and admits everything.
    outside: vec4f,
}

@group(0) @binding(0) var<uniform> globals: Globals;
// The persistent glyph atlas (M4) and the frame's scratch masks (M5), both r8unorm.
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var scratch_tex: texture_2d<f32>;
// The active soft mask (§11.5's soft clip), realised at its own plan's rectangle.
@group(1) @binding(2) var soft_mask_tex: texture_2d<f32>;
@group(1) @binding(3) var<uniform> mask_place: MaskPlace;

// The soft mask at a device pixel, given where the mask sits (ADR 0037). Identical in
// all six shaders that sample a mask; WGSL has no include, so the copies are kept
// textually the same, and tests/shader_copies.rs fails the build when they drift. The
// placement is an argument rather than a global because it reaches each lane in a
// different uniform; `soft_mask_tex` is the one name all six bind it under.
fn soft_mask_value(rect: vec4f, outside: f32, p: vec2f) -> f32 {
    let local = p - rect.xy;
    if any(local < vec2f(0.0)) || any(local >= rect.zw) {
        return outside;
    }
    return textureLoad(soft_mask_tex, vec2i(local), 0).r;
}

// This lane draws batches under different masks, so the placement is its own uniform.
fn soft_mask_at(p: vec2f) -> f32 {
    return soft_mask_value(mask_place.rect, mask_place.outside.x, p);
}

struct Instance {
    // Integer device pixel of the tile's top-left corner.
    @location(0) dest_min: vec2f,
    // Tile size in pixels.
    @location(1) size: vec2f,
    // Texel origin in the source texture, plus the source selector in .z
    // (0 = atlas, 1 = scratch). .w pads the slot.
    @location(2) tex_origin_source: vec4f,
    // Premultiplied device RGB (§3: premultiplied internally).
    @location(3) color: vec4f,
    // Device-space clip rectangle (min.xy, max.xy) — ADR 0007's four floats,
    // applied per pixel here because a coverage sample cannot be pre-intersected
    // the way the rectangle lane's geometry can.
    @location(4) clip: vec4f,
}

struct VsOut {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) dest_min: vec2f,
    @location(1) @interpolate(flat) tex_origin_source: vec4f,
    @location(2) @interpolate(flat) color: vec4f,
    @location(3) @interpolate(flat) clip: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, instance: Instance) -> VsOut {
    let corner = vec2f(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    let pos = instance.dest_min + corner * instance.size;
    let ndc = vec2f(
        (pos.x - globals.origin.x) / globals.target_size.x * 2.0 - 1.0,
        1.0 - (pos.y - globals.origin.y) / globals.target_size.y * 2.0,
    );
    var out: VsOut;
    out.position = vec4f(ndc, 0.0, 1.0);
    out.dest_min = instance.dest_min;
    out.tex_origin_source = instance.tex_origin_source;
    out.color = instance.color;
    out.clip = instance.clip;
    return out;
}

// The element's SHAPE at this fragment: the CPU-rasterised coverage byte times the
// analytic clip-rectangle overlap of the pixel's unit cell (same formula as
// rect.wgsl, ADR 0005/0007) — §11.6.4.2's object shape met with §8.5.4's clip. The
// soft mask is *not* in it (ADR 0066): Table 57's alpha source flag reads the mask as
// opacity by default, so it belongs to `q` and not to `f`.
fn shape_at(in: VsOut, p: vec2f) -> f32 {
    let local = vec2i(p - in.dest_min);
    let texel = vec2i(in.tex_origin_source.xy) + local;
    var cov: f32;
    if in.tex_origin_source.z < 0.5 {
        cov = textureLoad(atlas_tex, texel, 0).r;
    } else {
        cov = textureLoad(scratch_tex, texel, 0).r;
    }
    let overlap_min = max(in.clip.xy, p);
    let overlap_max = min(in.clip.zw, p + vec2f(1.0, 1.0));
    let extent = max(overlap_max - overlap_min, vec2f(0.0, 0.0));
    return cov * extent.x * extent.y;
}

// The device pixel this fragment shades (rect.wgsl's `pixel_of`).
fn pixel_of(in: VsOut) -> vec2f {
    return floor(in.position.xy) + globals.origin;
}

// The source alpha the mark is drawn with: shape times §11.6.4.3's mask opacity.
fn coverage_at(in: VsOut) -> f32 {
    let p = pixel_of(in);
    return shape_at(in, p) * soft_mask_at(p);
}

// Premultiplied source scaled by coverage; fixed-function (ONE, ONE_MINUS_SRC_ALPHA).
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    return in.color * coverage_at(in);
}

// The element's shape alone, for the knockout erase pass (see rect.wgsl's fs_shape).
@fragment
fn fs_shape(in: VsOut) -> @location(0) vec4f {
    return vec4f(shape_at(in, pixel_of(in)));
}
