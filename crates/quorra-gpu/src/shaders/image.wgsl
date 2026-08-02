// The image lane: a decoded RGBA image mapped into the unit square (ISO 32000-2
// §8.9.5), drawn as one quad per command.
//
// The fragment's device position maps back through the inverse of the command ×
// viewport transform carried in the params, giving the unit-square (u, v) exactly —
// the same construction as the shading lane, so the drawn quad only has to *cover*
// the footprint, never trace it. §8.9.5's orientation puts the image's TOP row at
// v = 1, so the v axis flips at sampling.
//
// Coverage (ADR 0011):
// - An axis-preserving placement gets the analytic cell-overlap of the rectangle
//   lane (ADR 0005) against `image_rect`, so its edges antialias exactly; the quad
//   is the footprint expanded to pixel bounds.
// - An oblique placement paints the fragments whose centres map inside the unit
//   square — hard edges, stated as the deliberate cost of the rare case.
// - A residue clip arrives as a scratch tile multiplied in, like every lane.
//
// Filtering is the placement's **resolved** decision (§4.5, integration note 1):
// nearest goes through textureLoad (exact, adapter-invariant); linear goes through
// the hardware sampler (clamp-to-edge), whose interpolation precision is the
// driver's — the one place a §4.5 decision buys hardware variance, stated here and
// bounded by the tests' tolerance rather than hidden.

struct Params {
    // Inverse of the device transform of the unit square, §8.3.3 layout.
    inv0: vec4f, // a, b, c, d
    inv1: vec4f, // e, f, constant alpha (§11.6.4.3's CA), filter (1 = linear)
    // The image's device rectangle (exact for the axis-preserving case).
    image_rect: vec4f,
    // Quad destination rectangle, device space.
    dest: vec4f,
    // Clip rectangle (ADR 0007's four floats).
    clip: vec4f,
    // Residue-clip source: origin.xy in scratch, use_scratch, axis-preserving flag.
    coverage: vec4f,
    target_size: vec2f,
    _pad: vec2f,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var image_tex: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;
// The active soft mask (white 1x1 when absent).
@group(0) @binding(3) var soft_mask_tex: texture_2d<f32>;
// The frame's scratch, holding a rasterised residue clip when one applies.
@group(0) @binding(4) var scratch_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    let corner = vec2f(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    let pos = mix(params.dest.xy, params.dest.zw, corner);
    var out: VsOut;
    out.position = vec4f(
        pos.x / params.target_size.x * 2.0 - 1.0,
        1.0 - pos.y / params.target_size.y * 2.0,
        0.0,
        1.0,
    );
    return out;
}

// The unit-square coordinate of a device point, through the carried inverse.
fn to_unit(p: vec2f) -> vec2f {
    return vec2f(
        params.inv0.x * p.x + params.inv0.z * p.y + params.inv1.x,
        params.inv0.y * p.x + params.inv0.w * p.y + params.inv1.y,
    );
}

// Geometric coverage × clip × residue × soft mask — the element's *shape*
// (§11.4.7.2): the image's own alpha and the constant alpha are opacity, not shape,
// and stay out of this product on purpose (ADR 0011).
fn shape_at(p: vec2f, uv: vec2f) -> f32 {
    var cov: f32;
    if params.coverage.w > 0.5 {
        // Axis-preserving: the exact cell overlap with the image's rectangle.
        let o_min = max(params.image_rect.xy, p);
        let o_max = min(params.image_rect.zw, p + vec2f(1.0, 1.0));
        let e = max(o_max - o_min, vec2f(0.0, 0.0));
        cov = e.x * e.y;
    } else {
        // Oblique: painted where the centre lands inside the unit square.
        cov = f32(uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0);
    }
    if params.coverage.z > 0.5 {
        let texel = vec2i(params.coverage.xy + (p - params.dest.xy));
        cov = cov * textureLoad(scratch_tex, texel, 0).r;
    }
    let overlap_min = max(params.clip.xy, p);
    let overlap_max = min(params.clip.zw, p + vec2f(1.0, 1.0));
    let extent = max(overlap_max - overlap_min, vec2f(0.0, 0.0));
    let mask_dims = textureDimensions(soft_mask_tex);
    let mask_texel = min(vec2i(p), vec2i(mask_dims) - vec2i(1, 1));
    let mask = textureLoad(soft_mask_tex, mask_texel, 0).r;
    return cov * extent.x * extent.y * mask;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy);
    let uv = to_unit(p + vec2f(0.5, 0.5));
    let shape = shape_at(p, uv);
    // §8.9.5: top row at v = 1.
    let tex_uv = clamp(vec2f(uv.x, 1.0 - uv.y), vec2f(0.0), vec2f(1.0));
    var sample: vec4f;
    if params.inv1.w > 0.5 {
        sample = textureSampleLevel(image_tex, image_sampler, tex_uv, 0.0);
    } else {
        // Nearest: the exact texel containing the point, no sampler arithmetic.
        let dims = vec2f(textureDimensions(image_tex));
        let texel = vec2i(clamp(floor(tex_uv * dims), vec2f(0.0), dims - vec2f(1.0)));
        sample = textureLoad(image_tex, texel, 0);
    }
    // Straight-alpha samples premultiply here (§3: premultiplied internally); the
    // constant alpha (§11.6.4.3) and the image's own alpha are opacity.
    return vec4f(sample.rgb * sample.a, sample.a) * (shape * params.inv1.z);
}

// The knockout erase pass wants the shape alone (§11.4.6 with ADR 0010's algebra).
@fragment
fn fs_shape(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy);
    let uv = to_unit(p + vec2f(0.5, 0.5));
    return vec4f(0.0, 0.0, 0.0, shape_at(p, uv));
}
