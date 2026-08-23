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
// optional soft mask (§11.6.4.3) all reach the group here, which is where §11.4.5
// applies them — and they do not all reach it the same way. The mask and the constant
// are §11.3.7.2's opacity inputs and multiply; the clip is a **set**, and §8.5.4
// intersects it with the group's own shape (ADR 0074).

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
    // Clip-residue mask placement: the scratch texel origin in .zw. `.xy` is unread and
    // is written as zero — the region's own corner is `residue_rect.xy`, which is where
    // `residue_value` takes it from. An empty `residue_rect` (min >= max) means no
    // residue at all.
    residue: vec4f,
    residue_rect: vec4f,
    // Where this pass's attachment, the child it reads and the backdrop copy it reads sit
    // in device space, and how big the child is (ADR 0036, ADR 0038).
    //
    // A layer is allocated at its plan's bounds, so four spaces meet in this pass: the
    // attachment (the parent's region), device space (what every rectangle in this
    // uniform is stated in), the child's own texture, and the copy of the pixels this
    // pass is about to cover. `origin` converts the first to the second, `child_origin`
    // and `backdrop_origin` the second to the last two. Outside the child's rectangle
    // there is nothing to read, and nothing is what a plan contributes there — which is
    // why the pass is scissored to the child and never asked to shade beyond it.
    origin: vec2f,
    child_origin: vec2f,
    child_size: vec2f,
    backdrop_origin: vec2f,
    // Where the group's soft mask sits (ADR 0037): its device corner in .xy, its size
    // in texels in .zw.
    mask_rect: vec4f,
    // What that mask holds outside `mask_rect`, in .x: the reduce's output for a
    // transparent pixel, since the mask's group marks nothing out there. A size of
    // (0, 0) is an absent mask, and then this is 1 and admits everything.
    mask_outside: vec4f,
    // 1 when this layer's alpha *is* §11.6.4.2's group shape — which the encoder proves
    // from the group's own commands, by finding no opacity input below 1.0 anywhere
    // inside it (`every_opacity_is_one`). That is what decides whether the clip below
    // meets the group by §8.5.4's intersection or by the product a compositor that
    // cannot tell shape from opacity is left with (ADR 0074).
    alpha_is_shape: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var backdrop_tex: texture_2d<f32>;
@group(0) @binding(2) var src_tex: texture_2d<f32>;
// The group's soft mask, realised at its own plan's rectangle.
@group(0) @binding(3) var soft_mask_tex: texture_2d<f32>;
// The frame's scratch image, holding the clip-residue mask when present.
@group(0) @binding(4) var scratch_tex: texture_2d<f32>;

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

// This pass composites one group under one mask, so the placement is in its uniform.
fn soft_mask_at(p: vec2f) -> f32 {
    return soft_mask_value(params.mask_rect, params.mask_outside.x, p);
}

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
    // §11.3.5.2 HardLight: Multiply(b, 2s) where the *source* is at or below 0.5,
    // Screen(b, 2s − 1) above it. The switch is on s, which is what distinguishes it
    // from Overlay — the same function with its two arguments exchanged.
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
    // §11.3.5.2 SoftLight: below s = 0.5 the backdrop is darkened toward b², above it
    // lightened toward D(b) — the clause's auxiliary, `soft_light_d` above. The two
    // halves meet at b when s = 0.5, so the function is continuous across the switch.
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

// The clip in force at this blit, as **one region** rather than as two factors.
//
// A chain is one clipping path in the graphics state — ISO 32000-2 §8.5.4: "After the
// path has been painted, the clipping path in the graphics state shall be set to the
// intersection of the current clipping path and the newly constructed path" — and this
// pass receives it split in two only because the rectangular links resolve into a
// rectangle at encode time (ADR 0007) while the rest rasterise into the residue. Putting
// the two halves back together owes the clause an intersection, which is `min` and not a
// product (ADR 0030). The two differ only where both halves are fractional in one pixel;
// `residue_value` is exactly 1 wherever there is no residue, and `min` leaves the
// rectangle alone there.
fn clip_at(p: vec2f) -> f32 {
    return min(clip_coverage(p), residue_value(p));
}

// The group's premultiplied raster met with that clip (§8.5.4, §10.7.4, ADR 0074).
//
// §10.7.4 states the meeting as a set operation: "Subsequent painting operations shall
// affect a region that is the intersection of the set of pixels defined by the clipping
// region with the set of pixels for the region to be painted." Where `alpha_is_shape`
// holds — the encoder proved this raster's alpha to be the group's shape — that
// intersection is `min` per pixel,
// and `S ∩ C = S` wherever `S ⊆ C` — a clip that contains the group takes nothing from
// it, including from its own anti-aliased boundary.
//
// A clip cuts a shape and never touches a colour, so the straight colour is unchanged
// and only the alpha moves: the premultiplied vector is rescaled by the ratio of the two
// alphas. The division is safe because `s.a > c` implies `s.a > 0` — `c` is an area and
// a coverage byte, never negative.
//
// Where it does not hold the alpha is shape times opacity, the two are not separable in
// one raster, and `min` would paint a half-transparent group at full coverage wherever
// the clip admits more than its alpha. The product stays there: exact wherever the clip
// admits all or none of a pixel, and the square of the truth where two edges share one.
fn meet_clip(s: vec4f, c: f32) -> vec4f {
    if params.alpha_is_shape == 0u {
        return s * c;
    }
    if s.a <= c {
        return s;
    }
    return vec4f(s.rgb * (c / s.a), c);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    // Device space, which is what `clip`, `residue` and the mask are stated in; the
    // attachment's own texel is `pi` (ADR 0036).
    let p = floor(in.position.xy) + params.origin;
    let b = textureLoad(backdrop_tex, vec2i(p - params.backdrop_origin), 0);
    // The child's texture holds only its own rectangle of the frame.
    let child = p - params.child_origin;
    let inside = all(child >= vec2f(0.0)) && all(child < params.child_size);
    var s = select(vec4f(0.0), textureLoad(src_tex, vec2i(child), 0), inside);

    // The group's **opacity** inputs, which §11.3.7.2 multiplies in as many words —
    // "The three opacity inputs shall be multiplied together, producing an intermediate
    // value called the source opacity" — with §11.6.4.4's constant ("the nonstroking
    // alpha constant shall also be applied when painting a transparency group's results
    // onto its backdrop") and §11.6.4.3's mask as two of the three. The clip is in
    // neither of that subclause's two products, which is why it is not here.
    let q = params.alpha * soft_mask_at(p);
    // The clip, as the region §8.5.4 makes it, and the group met with it.
    let c = clip_at(p);
    let clipped = meet_clip(s, c);

    // The staged stages, before §11.4.4's interpolation and §11.3.6's formula, because
    // they replace both: this group is one half of somebody's expansion of §11.4.6 and
    // not an object being composited with a backdrop.
    //
    // The erase weight is the group's own **alpha**, which is its shape when the caller
    // draws the shape half opaque — the only way a group's shape can reach a raster
    // (Table 140's group alpha is not carried here). The clip meets that shape and the
    // opacity scales it, because a group's constant alpha and soft mask are the caller's
    // statements about this half (ADR 0033, ADR 0066).
    if params.compose == 1u {
        return b * (1.0 - clipped.a * q);
    }
    if params.compose == 2u {
        return b + clipped * q;
    }

    // §11.4.4, the non-isolated group. `s` holds E(B): this group's elements
    // composited onto the very backdrop `b` holds, because the layer was seeded with
    // it. The clause's Result step removes that backdrop's contribution by dividing by
    // Table 140's group alpha — which this raster does not carry — and §11.3.3 then
    // multiplies it straight back in when the result is composited with the same
    // backdrop under Normal. The two cancel, leaving one interpolation, exact for
    // every backdrop alpha and every blend mode *inside* the group (ADR 0019; the
    // builder refuses the three cases where the cancellation is false).
    //
    // **The clip is a weight here and not a set**, and it has to be: `s` is not the
    // group, it is the group over this backdrop, so its alpha is the backdrop's as well
    // and no part of it is the group's shape. `alpha_is_shape` is therefore never set for
    // a non-isolated group, whatever its elements are (ADR 0074) — and this branch takes
    // `s` rather than `clipped` so that the reading is stated here as well as encoded.
    if params.non_isolated != 0u {
        return mix(b, s, q * c);
    }

    s = clipped * q;

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
