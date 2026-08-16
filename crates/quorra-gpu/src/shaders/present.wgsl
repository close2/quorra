// One finished layer put on the surface, under an affine placement and a filter
// (ADR 0056).
//
// **This shader draws no page.** Every layer it reads is a raster some earlier frame
// already finished — clause 11's transparency model was resolved when those pixels were
// made, and nothing here can move one of them into a different colour. That is why no
// clause number appears against the blending below: the pipeline's `OVER` factors put a
// finished premultiplied raster onto whatever is already on the surface, which is
// arithmetic about two rasters and not a statement about a document. `composite.wgsl` is
// where the clause lives, and it is not reachable from here.
//
// What *is* stated by a clause is the sampling point. A target pixel carries the value
// of what is under its **centre** (ISO 32000-2 §10.7.4, the same reading
// `function_lane.wgsl` cites and for the same reason), so the centre is what maps back
// through the inverse of the placement, and the layer's texel is looked up there.
//
// Two filters, resolved upstream (§4.5, integration note 1) and never decided here:
// nearest is a `textureLoad` of the texel containing the point — exact, and
// adapter-invariant; linear is the hardware sampler, whose interpolation precision is
// the driver's. Linear interpolation of a **premultiplied** raster is interpolation in
// the right space: a texture a `Target::Texture` frame produced is premultiplied (§3
// converts to straight alpha at readback and nowhere else), so the weighted sum of two
// premultiplied texels is the premultiplied form of the weighted sum of the colours.
//
// Outside the layer's own rectangle the layer contributes **nothing** — alpha zero, not
// a clamped edge texel — so a placement that puts part of the window outside the layer
// leaves the rest of the window to the layers under it and to the clear. That is the
// same convention `blit.wgsl` uses for a root smaller than its target (ADR 0039).

struct Params {
    // Inverse of the placement, in §8.3.3's layout: a, b, c, d. The placement maps the
    // layer's texel space to the surface's pixels, so its inverse maps a surface pixel
    // back to the texel that is under it.
    inv0: vec4f,
    // e, f of the same inverse, then the filter (1 = linear, 0 = nearest), then a zero
    // this shader never reads — WGSL's uniform layout rounds the struct to 16 bytes and
    // a named lane is easier to check than a gap (`shaders/layout.rs` proves the
    // offsets).
    inv1: vec4f,
    // The rectangle this pass draws, in clip space: left, top, right, bottom. The host
    // computes it from the placement (ADR 0058); see `vs_main`.
    bounds: vec4f,
    // The layer's extent in texels. The sample point is inside the layer exactly when it
    // is inside [0, source), and outside it this shader writes transparency.
    source: vec2f,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var layer_tex: texture_2d<f32>;
@group(0) @binding(2) var layer_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4f,
}

// The layer's own rectangle, as a two-triangle strip: `params.bounds` is where the
// placement puts this layer on the window and nowhere else, so the window's other
// pixels generate no fragment at all (ADR 0058). Four vertices, no vertex buffer, and
// still no placement arithmetic in this stage — the host computes the rectangle,
// because it is the side that knows how big the window is.
//
// **This stage is a bound and never a decision.** `params.bounds` is the layer's device
// corners grown outward to whole pixels and clamped to the target, so every pixel whose
// centre `layer_at` would accept is inside it; which texel a fragment gets, and whether
// it gets one at all, stays entirely `layer_at`'s. That is what makes the pass draw the
// same bytes as the full-screen triangle it replaces rather than nearly the same ones.
//
// Strip order is (left, top), (right, top), (left, bottom), (right, bottom).
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    let x = select(params.bounds.z, params.bounds.x, (vertex_index & 1u) == 0u);
    let y = select(params.bounds.w, params.bounds.y, vertex_index < 2u);
    out.position = vec4f(x, y, 0.0, 1.0);
    return out;
}

// The layer's texel under a surface pixel's centre, or transparency where the placement
// puts that centre outside the layer.
fn layer_at(p: vec2f) -> vec4f {
    let centre = p + vec2f(0.5, 0.5);
    let q = vec2f(
        params.inv0.x * centre.x + params.inv0.z * centre.y + params.inv1.x,
        params.inv0.y * centre.x + params.inv0.w * centre.y + params.inv1.y,
    );
    if any(q < vec2f(0.0)) || any(q >= params.source) {
        return vec4f(0.0);
    }
    if params.inv1.z > 0.5 {
        // The sampler's coordinates are normalised, and q is in texels: the division is
        // the whole conversion, and at a texel's centre it lands on that texel exactly.
        return textureSampleLevel(layer_tex, layer_sampler, q / params.source, 0.0);
    }
    return textureLoad(layer_tex, vec2i(floor(q)), 0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    return layer_at(floor(in.position.xy));
}
