// The compositor: one finished layer painted onto its backdrop, exactly once.
//
// ISO 32000-2 §11.4.5: a transparency group's elements draw onto a fully transparent
// backdrop; the finished group is then composited with the group's backdrop under
// the group's constant alpha and blend mode. This pass is that sentence: it reads
// the backdrop layer and the group layer, applies §11.3.6's compositing formula with
// one of §11.3.5's sixteen blend functions, and writes the result — no fixed-function
// blending (the target state is REPLACE), so the arithmetic below is the whole
// definition and is identical on every adapter up to the final store conversion
// (ADR 0006).
//
// The group's own clip rectangle (ADR 0007), an optional clip-residue mask and an
// optional soft mask (§11.6.4.3) all modulate the group's alpha and shape here,
// which is where §11.4.5 applies them.

struct Params {
    // §11.3.5's mode, numbered in the BlendMode enum's declaration order.
    mode: u32,
    // The group's constant alpha (§11.4.5).
    alpha: f32,
    // 1 for §11.4.4's non-isolated group, whose layer was seeded with this backdrop
    // and is interpolated back onto it; 0 for §11.4.5's isolated group.
    non_isolated: u32,
    // §11.4.6's stage this group *is*, when it is one (ADR 0033): 0 the ordinary
    // composite below, 1 the erase `P' = (1 − f) × P`, 2 the deposit `P' = P + S`.
    // A caller writes a knockout element whose shape is not its coverage as two groups
    // under these, because §11.6.4.2 makes a group's shape the union of its elements'
    // and no fill can state that.
    compose: u32,
    // Device-space clip rectangle for the composited group (min.xy, max.xy).
    clip: vec4f,
    // Clip-residue mask placement: region min in .xy, scratch texel origin in .zw.
    // Region empty (min >= max in `clip_residue_rect`) means no residue.
    residue: vec4f,
    residue_rect: vec4f,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var backdrop_tex: texture_2d<f32>;
@group(0) @binding(2) var src_tex: texture_2d<f32>;
// The group's soft mask at device resolution (white 1x1 when absent).
@group(0) @binding(3) var soft_mask_tex: texture_2d<f32>;
// The frame's scratch image, holding the clip-residue mask when present.
@group(0) @binding(4) var scratch_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4f,
}

// One full-target triangle: three vertices, no buffers.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index >> 1u) * 4 - 1);
    out.position = vec4f(x, y, 0.0, 1.0);
    return out;
}

// §11.3.5.3's Lum: the weighted luminosity of a colour. The clause's own
// coefficients (0.30, 0.59, 0.11) — not Rec. 709's, not sRGB's.
fn lum(c: vec3f) -> f32 {
    return 0.30 * c.r + 0.59 * c.g + 0.11 * c.b;
}

// §11.3.5.3's ClipColor: pull components back into gamut around the luminosity.
fn clip_color(c_in: vec3f) -> vec3f {
    var c = c_in;
    let l = lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    if n < 0.0 {
        c = l + (c - l) * l / (l - n);
    }
    if x > 1.0 {
        c = l + (c - l) * (1.0 - l) / (x - l);
    }
    return c;
}

// §11.3.5.3's SetLum: shift to a target luminosity, then clip.
fn set_lum(c: vec3f, l: f32) -> vec3f {
    return clip_color(c + (l - lum(c)));
}

// §11.3.5.3's Sat.
fn sat(c: vec3f) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

// §11.3.5.3's SetSat: rescale the mid component between min and max, per the
// clause's case analysis over the component ordering.
fn set_sat(c: vec3f, s: f32) -> vec3f {
    let mn = min(c.r, min(c.g, c.b));
    let mx = max(c.r, max(c.g, c.b));
    if mx <= mn {
        return vec3f(0.0);
    }
    // Each component maps to 0, s, or the rescaled middle.
    return (c - mn) * s / (mx - mn);
}

// §11.3.5.2's SoftLight auxiliary D.
fn soft_light_d(b: f32) -> f32 {
    if b <= 0.25 {
        return ((16.0 * b - 12.0) * b + 4.0) * b;
    }
    return sqrt(b);
}

fn hard_light_channel(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b * (2.0 * s); // Multiply(b, 2s)
    }
    let s2 = 2.0 * s - 1.0;
    return b + s2 - b * s2; // Screen(b, 2s - 1)
}

fn color_dodge_channel(b: f32, s: f32) -> f32 {
    // §11.3.5.2 ColorDodge: 0 stays 0; otherwise min(1, b / (1 - s)), with s = 1
    // saturating to 1.
    if b <= 0.0 {
        return 0.0;
    }
    if s >= 1.0 {
        return 1.0;
    }
    return min(1.0, b / (1.0 - s));
}

fn color_burn_channel(b: f32, s: f32) -> f32 {
    // §11.3.5.2 ColorBurn: 1 stays 1; otherwise 1 - min(1, (1 - b) / s), with s = 0
    // saturating to 0.
    if b >= 1.0 {
        return 1.0;
    }
    if s <= 0.0 {
        return 0.0;
    }
    return 1.0 - min(1.0, (1.0 - b) / s);
}

fn soft_light_channel(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    }
    return b + (2.0 * s - 1.0) * (soft_light_d(b) - b);
}

// §11.3.5's B(Cb, Cs), by mode. The numbering matches quorra-scene's BlendMode
// declaration order, which matches the clause's own table order.
fn blend(mode: u32, cb: vec3f, cs: vec3f) -> vec3f {
    switch mode {
        case 0u: { return cs; }                                   // Normal
        case 1u: { return cb * cs; }                              // Multiply
        case 2u: { return cb + cs - cb * cs; }                    // Screen
        case 3u: {                                                // Overlay = HardLight swapped
            return vec3f(
                hard_light_channel(cs.r, cb.r),
                hard_light_channel(cs.g, cb.g),
                hard_light_channel(cs.b, cb.b),
            );
        }
        case 4u: { return min(cb, cs); }                          // Darken
        case 5u: { return max(cb, cs); }                          // Lighten
        case 6u: {                                                // ColorDodge
            return vec3f(
                color_dodge_channel(cb.r, cs.r),
                color_dodge_channel(cb.g, cs.g),
                color_dodge_channel(cb.b, cs.b),
            );
        }
        case 7u: {                                                // ColorBurn
            return vec3f(
                color_burn_channel(cb.r, cs.r),
                color_burn_channel(cb.g, cs.g),
                color_burn_channel(cb.b, cs.b),
            );
        }
        case 8u: {                                                // HardLight
            return vec3f(
                hard_light_channel(cb.r, cs.r),
                hard_light_channel(cb.g, cs.g),
                hard_light_channel(cb.b, cs.b),
            );
        }
        case 9u: {                                                // SoftLight
            return vec3f(
                soft_light_channel(cb.r, cs.r),
                soft_light_channel(cb.g, cs.g),
                soft_light_channel(cb.b, cs.b),
            );
        }
        case 10u: { return abs(cb - cs); }                        // Difference
        case 11u: { return cb + cs - 2.0 * cb * cs; }             // Exclusion
        case 12u: { return set_lum(set_sat(cs, sat(cb)), lum(cb)); } // Hue
        case 13u: { return set_lum(set_sat(cb, sat(cs)), lum(cb)); } // Saturation
        case 14u: { return set_lum(cs, lum(cb)); }                // Color
        default: { return set_lum(cb, lum(cs)); }                 // Luminosity (15)
    }
}

// The pixel-cell overlap with the group's clip rectangle (ADR 0005's coverage rule,
// applied to the clip as ADR 0007 defers it here for composited groups).
fn clip_coverage(p: vec2f) -> f32 {
    let overlap_min = max(params.clip.xy, p);
    let overlap_max = min(params.clip.zw, p + vec2f(1.0, 1.0));
    let extent = max(overlap_max - overlap_min, vec2f(0.0, 0.0));
    return extent.x * extent.y;
}

// The clip-residue mask value at this pixel: 1 when no residue region is set,
// the scratch byte inside the region, 0 outside it.
fn residue_value(p: vec2f) -> f32 {
    let rect = params.residue_rect;
    if rect.x >= rect.z || rect.y >= rect.w {
        return 1.0;
    }
    if p.x < rect.x || p.x >= rect.z || p.y < rect.y || p.y >= rect.w {
        return 0.0;
    }
    let texel = vec2i(params.residue.zw + (p - rect.xy));
    return textureLoad(scratch_tex, texel, 0).r;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let p = floor(in.position.xy);
    let pi = vec2i(p);
    let b = textureLoad(backdrop_tex, pi, 0);
    var s = textureLoad(src_tex, pi, 0);

    // §11.4.5: the group's constant alpha; §11.6.4.3: its soft mask; ADR 0007: its
    // clip. All scale the group's premultiplied contribution uniformly.
    let mask_dims = textureDimensions(soft_mask_tex);
    let mask_texel = min(pi, vec2i(mask_dims) - vec2i(1, 1));
    let soft = textureLoad(soft_mask_tex, mask_texel, 0).r;
    let w = params.alpha * soft * clip_coverage(p) * residue_value(p);

    // The staged stages, before §11.4.4's interpolation and §11.3.6's formula, because
    // they replace both: this group is one half of somebody's expansion of §11.4.6 and
    // not an object being composited with a backdrop.
    //
    // The erase weight is the group's own **alpha**, which is its shape when the caller
    // draws the shape half opaque — the only way a group's shape can reach a raster
    // (Table 140's group alpha is not carried here). `w` scales it because a group's
    // constant alpha, soft mask and clip are the caller's statements about this half.
    if params.compose == 1u {
        return b * (1.0 - s.a * w);
    }
    if params.compose == 2u {
        return b + s * w;
    }

    // §11.4.4, the non-isolated group. `s` holds E(B): this group's elements
    // composited onto the very backdrop `b` holds, because the layer was seeded with
    // it. The clause's Result step removes that backdrop's contribution by dividing by
    // Table 140's group alpha — which this raster does not carry — and §11.3.3 then
    // multiplies it straight back in when the result is composited with the same
    // backdrop under Normal. The two cancel, leaving one interpolation, exact for
    // every backdrop alpha and every blend mode *inside* the group (ADR 0019; the
    // builder refuses the three cases where the cancellation is false).
    if params.non_isolated != 0u {
        return mix(b, s, w);
    }

    s = s * w;

    let ab = b.a;
    let as_ = s.a;
    // Straight colours for B(); a fully transparent side contributes no colour, and
    // its straight colour is irrelevant because its weight below is zero.
    let cb = select(b.rgb / vec3f(ab), vec3f(0.0), ab <= 0.0);
    let cs = select(s.rgb / vec3f(as_), vec3f(0.0), as_ <= 0.0);
    let mixed = blend(params.mode, cb, cs);

    // §11.3.6's compositing formula, written premultiplied:
    //   co = as·(1−ab)·Cs + ab·(1−as)·Cb + as·ab·B(Cb, Cs)
    //   ao = as + ab·(1−as)
    let co = as_ * (1.0 - ab) * cs + ab * (1.0 - as_) * cb + as_ * ab * mixed;
    let ao = as_ + ab * (1.0 - as_);
    return vec4f(co, ao);
}
