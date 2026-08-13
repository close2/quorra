// A rectangle of one texture copied into another, unchanged.
//
// Two callers, and they want the same pass for different reasons. **The final hand-off**:
// a frame that needed internal layers (groups, masks, non-Normal blends) accumulates the
// page in a texture the compositor can read, and this moves it onto the target — which
// may be a swapchain texture that cannot be sampled. **The composite's backdrop**: a pass
// cannot read its own attachment, so the pixels a child is about to be composited onto are
// copied out first, at the child's size rather than the page's (ADR 0038).
//
// REPLACE, no blending, no conversion: the page stays rendered onto transparency (§3; the
// caller composites onto the medium), and between two `Rgba8Unorm` textures a `textureLoad`
// and a store is exact.

struct Placement {
    // The source texel this pass's texel (0, 0) reads. Zero for a copy between two
    // textures of one size, which is what the seed is; the child's rectangle inside its
    // parent's, for the composite's backdrop (ADR 0038); and **negative** for the
    // hand-off, whose destination is the whole target while its source is only what the
    // page marks (ADR 0039).
    origin: vec2f,
    // The source's extent in texels. Outside it there is nothing to copy, and this pass
    // writes **transparency** there rather than leaving the attachment alone — a page is
    // rendered onto transparency (§3), so that is what a full redraw puts outside what it
    // marked, and the damage contract says a patched rectangle must equal a full redraw.
    size: vec2f,
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> placement: Placement;

struct VsOut {
    @builtin(position) position: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index >> 1u) * 4 - 1);
    out.position = vec4f(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let at = floor(in.position.xy) + placement.origin;
    if any(at < vec2f(0.0)) || any(at >= placement.size) {
        return vec4f(0.0);
    }
    return textureLoad(src_tex, vec2i(at), 0);
}
