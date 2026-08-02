// The final hand-off: copy the root layer to the frame's target, unchanged.
//
// Exists only for frames that needed internal layers (groups, masks, non-Normal
// blends): their content accumulates in an internal texture the compositor can read,
// and this pass moves the finished page onto the target — which may be a swapchain
// texture that cannot be sampled. REPLACE, no blending, no conversion: the page
// stays rendered onto transparency (§3; the caller composites onto the medium).

@group(0) @binding(0) var src_tex: texture_2d<f32>;

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
    return textureLoad(src_tex, vec2i(in.position.xy), 0);
}
