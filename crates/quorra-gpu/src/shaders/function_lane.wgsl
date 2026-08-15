// The function lane: one quad painted by a §7.10.5 program the device evaluates
// (ISO 32000-2 §8.7.4.5.2's type 1 shading, ADR 0053).
//
// This file is **not a module of its own**. It is the fixed half of a shader whose
// other half is generated per program: `function::generate` emits the operator
// library and `quorra_function_evaluate`, and `pipeline::function` appends this text
// to it. So every pipeline compiled for a program compiles this too, and the two
// halves are separated exactly where a program stops being one and starts being a
// placement: nothing below knows which program it is calling, and nothing above knows
// where on the page it is being called.
//
// It is the shading lane's twin (`shading.wgsl`) with one difference that matters: the
// colour is computed rather than looked up, so there is no paint texture and the
// per-fragment work is the program. Coverage, the clip rectangle and the soft mask are
// the same in both, because those belong to the *mark* rather than to the paint.

struct Params {
    // Inverse of the shading-space → device transform, §8.3.3 layout: a, b, c, d in
    // `inv0`, then e, f. This is §8.7.4.5.2's `Matrix` composed with the viewport and
    // inverted: the fragment has a device pixel and the program wants the shading's own
    // space, so the mapping runs the other way from the one the paint states.
    inv0: vec4f, // a, b, c, d
    inv1: vec4f, // e, f, unused, unused
    // §8.7.4.5.2's `Domain`, as (min x, max x, min y, max y) — the pair of bounds for
    // one axis side by side, which is the order the generated function reads them in.
    domain: vec4f,
    // §7.10.1's `Range`, low bounds then high, one lane per output component.
    range_low: vec4f,
    range_high: vec4f,
    // §8.7.4.5.2's `Background`, straight-alpha. An absent one is vec4f(0), which is
    // the clause's "left unpainted" — see `function::background_rgba`.
    background: vec4f,
    // Quad destination rectangle, device space.
    dest: vec4f,
    // Coverage source: origin.xy in scratch, use_scratch, unused.
    coverage: vec4f,
    // Analytic coverage rectangle (the shape itself when no scratch tile).
    coverage_rect: vec4f,
    // Clip rectangle.
    clip: vec4f,
    target_size: vec2f,
    // The device-space corner this attachment's texel (0, 0) is (ADR 0036). The vertex
    // stage subtracts it before dividing by `target_size`; the fragment stage adds it
    // back to recover the device pixel it is shading, because clip rectangles, tile
    // lookups and masks are all stated in device space.
    origin: vec2f,
    // Where the active soft mask sits (ADR 0037): its device corner in .xy, its size in
    // texels in .zw.
    mask_rect: vec4f,
    // What that mask holds outside `mask_rect`, in .x. A size of (0, 0) is an absent
    // mask, and then this is 1 and admits everything.
    mask_outside: vec4f,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var scratch_tex: texture_2d<f32>;
// The active soft mask, realised at its own plan's rectangle.
@group(0) @binding(2) var soft_mask_tex: texture_2d<f32>;

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

// This pass draws one shading under one mask, so the placement is in its uniform.
fn soft_mask_at(p: vec2f) -> f32 {
    return soft_mask_value(params.mask_rect, params.mask_outside.x, p);
}

struct VsOut {
    @builtin(position) position: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    let corner = vec2f(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    let pos = mix(params.dest.xy, params.dest.zw, corner);
    var out: VsOut;
    out.position = vec4f(
        (pos.x - params.origin.x) / params.target_size.x * 2.0 - 1.0,
        1.0 - (pos.y - params.origin.y) / params.target_size.y * 2.0,
        0.0,
        1.0,
    );
    return out;
}

// Coverage × clip × soft mask at the fragment's cell — the geometric part of the
// weight, and textually the shading lane's, because it is the same quantity.
fn base_weight(p: vec2f) -> f32 {
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
    return cov * extent.x * extent.y * soft_mask_at(p);
}

// The straight-alpha paint at a device pixel: the program's own answer for the point
// that pixel's centre maps to.
//
// ISO 32000-2 §10.7.4 is why the centre rather than the corner — a pixel carries the
// value of the function at its centre — and the caller's ADR 0339 replaced a sampled
// grid with the device's own grid for exactly that reason, which is what makes this
// lane worth having.
//
// The returned `.a` is coverage in the clause's sense: 1 inside the domain rectangle,
// and whatever `Background` carries outside it, which is 0 when there is none. The
// discard is the generated function's own first instruction (§8.7.4.5.2), so nothing
// here decides it.
fn paint_at(p: vec2f) -> vec4f {
    let centre = p + vec2f(0.5, 0.5);
    let inv = params.inv0;
    let q = vec2f(
        inv.x * centre.x + inv.z * centre.y + params.inv1.x,
        inv.y * centre.x + inv.w * centre.y + params.inv1.y,
    );
    return quorra_function_evaluate(
        q.x,
        q.y,
        params.domain,
        params.range_low.xyz,
        params.range_high.xyz,
        params.background,
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy) + params.origin;
    let straight = paint_at(p);
    return vec4f(straight.rgb * straight.a, straight.a) * base_weight(p);
}

// The knockout erase pass wants the shape alone (§11.4.6, ADR 0010). A pixel the paint
// does not mark knocks nothing out: outside the domain rectangle with no `Background`,
// §8.7.4.5.2 says the point is "left unpainted", and an unpainted point has no shape.
//
// The test is `> 0.0` on the paint's alpha, and it carries one consequence worth
// stating: a `Background` whose own alpha is zero is indistinguishable here from an
// absent one, because `function::background_rgba` encodes both as vec4f(0). That is the
// encoding's deliberate collapse — the two paint the same pixels — and §11.4.7.2's
// shape/opacity distinction is preserved everywhere it is observable: a background of
// alpha 0.4 marks full shape at four tenths opacity, exactly as a ramp stop of that
// alpha does.
@fragment
fn fs_shape(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy) + params.origin;
    let straight = paint_at(p);
    if straight.a <= 0.0 {
        return vec4f(0.0);
    }
    return vec4f(0.0, 0.0, 0.0, base_weight(p));
}
