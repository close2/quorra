// The shading lane: axial and radial ramp sweeps (ISO 32000-2 §8.7.4.5.2/.3) and
// the caller's pre-rasterised meshes, painted through a coverage source.
//
// The sweep parameter t is computed in the *shading's own space*: the fragment's
// device position maps back through the inverse of the command × viewport transform
// carried in the params, so a sheared or rotated placement sweeps correctly. The
// ramp was sampled to straight-RGBA texels on the CPU at upload (deterministic);
// t indexes it with textureLoad — no sampler, no filtering, adapter-invariant.
//
// Coverage comes either from the frame's scratch (a rasterised fill/stroke shape,
// ADR 0008) or analytically from the coverage rectangle for the rectangle case, and
// is modulated by the clip rectangle and soft mask like every lane.

struct Params {
    // Inverse device transform, §8.3.3 layout: a, b, c, d, e, f.
    inv0: vec4f, // a, b, c, d
    inv1: vec4f, // e, f, kind (0 axial, 1 radial, 2 mesh), extend flags packed
    // Axial: start.xy, end.xy. Radial: start.xy, end.xy. Mesh: left, top, 0, 0.
    geo0: vec4f,
    // Radial: start_radius, end_radius. Others unused.
    geo1: vec4f,
    // Quad destination rectangle, device space.
    dest: vec4f,
    // Coverage source: origin.xy in scratch, use_scratch, unused.
    coverage: vec4f,
    // Analytic coverage rectangle (the shape itself when no scratch tile).
    coverage_rect: vec4f,
    // Clip rectangle.
    clip: vec4f,
    target_size: vec2f,
    _pad: vec2f,
}

@group(0) @binding(0) var<uniform> params: Params;
// The ramp (RAMP_RESOLUTION x 1) for axial/radial, or the mesh raster.
@group(0) @binding(1) var paint_tex: texture_2d<f32>;
@group(0) @binding(2) var scratch_tex: texture_2d<f32>;
@group(0) @binding(3) var soft_mask_tex: texture_2d<f32>;

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

// The sweep parameter, or -1 when the point is outside an unextended range —
// §8.7.4.5.2/.3: where extension is off, nothing is painted there at all, which is
// not the same as painting the end colour.
fn sweep_t(p_shading: vec2f) -> f32 {
    let kind = params.inv1.z;
    // Extend flags packed as bits: 1 = beyond start, 2 = beyond end.
    let bits = i32(params.inv1.w);
    let extend_start = (bits & 1) != 0;
    let extend_end = (bits & 2) != 0;
    if kind < 0.5 {
        // §8.7.4.5.2: the projection onto the axis.
        let d = params.geo0.zw - params.geo0.xy;
        let len_sq = dot(d, d);
        if len_sq <= 0.0 {
            return -1.0;
        }
        var t = dot(p_shading - params.geo0.xy, d) / len_sq;
        if t < 0.0 {
            if !extend_start { return -1.0; }
            t = 0.0;
        }
        if t > 1.0 {
            if !extend_end { return -1.0; }
            t = 1.0;
        }
        return t;
    }
    // §8.7.4.5.3: the largest s with |p − c(s)| = r(s), r(s) >= 0, where centre and
    // radius interpolate linearly between the two circles.
    let cd = params.geo0.zw - params.geo0.xy;
    let rd = params.geo1.y - params.geo1.x;
    let pd = p_shading - params.geo0.xy;
    let a = dot(cd, cd) - rd * rd;
    let b = dot(pd, cd) + params.geo1.x * rd;
    let c = dot(pd, pd) - params.geo1.x * params.geo1.x;
    var s: f32;
    if abs(a) < 1e-6 {
        if abs(b) < 1e-12 {
            return -1.0;
        }
        s = c / (2.0 * b);
    } else {
        let disc = b * b - a * c;
        if disc < 0.0 {
            return -1.0;
        }
        let root = sqrt(disc);
        let s1 = (b + root) / a;
        let s2 = (b - root) / a;
        // The larger s whose radius is non-negative (§8.7.4.5.3's preference).
        s = max(s1, s2);
        if params.geo1.x + s * rd < 0.0 {
            s = min(s1, s2);
            if params.geo1.x + s * rd < 0.0 {
                return -1.0;
            }
        }
    }
    if s < 0.0 {
        if !extend_start { return -1.0; }
        s = 0.0;
    }
    if s > 1.0 {
        if !extend_end { return -1.0; }
        s = 1.0;
    }
    return s;
}

// Coverage × clip × soft mask at the fragment's cell — the geometric part of the
// weight, shared by both entry points.
fn base_weight(p: vec2f) -> f32 {
    // Coverage: a scratch tile's byte, or the analytic rectangle's cell overlap.
    var cov: f32;
    if params.coverage.z > 0.5 {
        let texel = vec2i(params.coverage.xy + (p - params.dest.xy));
        cov = textureLoad(scratch_tex, texel, 0).r;
    } else {
        let o_min = max(params.coverage_rect.xy, p);
        let o_max = min(params.coverage_rect.zw, p + vec2f(1.0, 1.0));
        let e = max(o_max - o_min, vec2f(0.0, 0.0));
        cov = e.x * e.y;
    }
    let overlap_min = max(params.clip.xy, p);
    let overlap_max = min(params.clip.zw, p + vec2f(1.0, 1.0));
    let extent = max(overlap_max - overlap_min, vec2f(0.0, 0.0));
    let mask_dims = textureDimensions(soft_mask_tex);
    let mask_texel = min(vec2i(p), vec2i(mask_dims) - vec2i(1, 1));
    let mask = textureLoad(soft_mask_tex, mask_texel, 0).r;
    return cov * extent.x * extent.y * mask;
}

// The straight-alpha paint at the fragment, or a negative alpha sentinel where the
// shading paints nothing at all (unextended sweep, or outside the mesh raster).
fn paint_at(p: vec2f) -> vec4f {
    if params.inv1.z > 1.5 {
        // Mesh: the pre-rasterised samples sit at absolute device pixels.
        let dims = vec2f(textureDimensions(paint_tex));
        let texel = p - params.geo0.xy;
        if texel.x < 0.0 || texel.y < 0.0 || texel.x >= dims.x || texel.y >= dims.y {
            return vec4f(0.0, 0.0, 0.0, -1.0);
        }
        return textureLoad(paint_tex, vec2i(texel), 0);
    }
    // Map back to the shading's own space, then sweep.
    let centre = p + vec2f(0.5, 0.5);
    let inv = params.inv0;
    let q = vec2f(
        inv.x * centre.x + inv.z * centre.y + params.inv1.x,
        inv.y * centre.x + inv.w * centre.y + params.inv1.y,
    );
    let t = sweep_t(q);
    if t < 0.0 {
        return vec4f(0.0, 0.0, 0.0, -1.0);
    }
    // The ramp was sampled on the CPU to the texture's own width (4096 texels —
    // fine enough that a banded shading's hard boundary sits within an eighth
    // of a pixel on a page-spanning axis): index by rounding, exactly.
    let entries = f32(textureDimensions(paint_tex).x - 1u);
    let index = i32(round(t * entries));
    return textureLoad(paint_tex, vec2i(index, 0), 0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy);
    let straight = paint_at(p);
    if straight.a < 0.0 {
        return vec4f(0.0);
    }
    return vec4f(straight.rgb * straight.a, straight.a) * base_weight(p);
}

// The knockout erase pass wants the shape alone (§11.4.6, ADR 0010). Where an
// unextended shading paints nothing, no mark is made and nothing knocks out —
// shape 0, not shape-with-zero-opacity (ADR 0011 records the reading of
// §11.4.7.2). A mesh raster's own alpha is antialiased triangle coverage and so
// counts as shape.
@fragment
fn fs_shape(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy);
    let straight = paint_at(p);
    if straight.a < 0.0 {
        return vec4f(0.0);
    }
    var shape = base_weight(p);
    if params.inv1.z > 1.5 {
        shape = shape * straight.a;
    }
    return vec4f(0.0, 0.0, 0.0, shape);
}
