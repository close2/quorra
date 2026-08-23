// The GPU coverage lane: winding accumulation, then the fill rule (ADR 0016).
//
// Evan Wallace's *Easy Scalable Text Rendering on the GPU* (2016). Two passes:
//
//   1. `vs_winding`/`fs_winding` draw one triangle per outline segment from a shared
//      anchor, plus one Loop-Blinn control triangle per curve, into a float target
//      with additive blending. What accumulates at a sample is the outline's
//      **winding number** there, ISO 32000-2 section 8.5.3.3, and nothing else.
//   2. `vs_resolve`/`fs_resolve` draw one quad per tile, apply the tile's fill rule to
//      each sample of a pixel, and add the fraction that passed to the scratch sheet.
//      Coverage leaves as one r8unorm byte, which is what every downstream lane
//      already expects of that sheet.
//
// Three deliberate departures from the article, all recorded in ADR 0016:
//
// - **Signed accumulation, not parity.** Wallace adds 1/255 per triangle and calls a
//   pixel inside when the total is odd, which is the even-odd rule (8.5.3.3.3) and
//   only that rule. A float target holds the sign, so `fs_winding` emits +1 or -1 by
//   triangle orientation and both of 8.5.3.3's rules follow from the same number.
//   Non-zero is the PDF default and cannot be expressed otherwise.
// - **Samples in channels, not in the bits of one byte.** His packing was a 2016 WebGL
//   necessity — 8-bit colour buffers were all there was. Four rgba16float channels
//   hold four signed windings exactly (f16 is exact on integers to 2048, and no page
//   winds a path two thousand times), and a frame runs the pair of passes once per
//   group of four samples.
// - **Sample positions are ours, not the driver's.** An ordered grid stated here,
//   rather than a multisample pattern the adapter chooses, is what keeps coverage
//   identical on every adapter — the promise ADR 0006 measured and ADR 0008 protects.

struct Globals {
    // The scratch sheet's size in pixels, which `vs_resolve` maps its tile quads into.
    //
    // The R8 sheet both coverage lanes share, and the resolve pass's attachment — not
    // the winding target, whose size is `pane_size` below and whose relationship to the
    // sheet is `pane_origin`. The accumulate pass has no use for this number at all.
    sheet_size: vec2f,
    // Where this draw's sample sits inside the pixel, in pixels, relative to its
    // centre. The ordered grid lives on the CPU (`outline::sample_offsets`), because
    // the vertex stage should not have to know which of the frame's draws it is.
    sample_offset: vec2f,
    // Which channel this draw accumulates into, 0..3. Written as a mask by the
    // fragment stage so that one pipeline serves all four: a channel this draw does
    // not own receives exactly zero and additive blending leaves it alone.
    channel: vec4f,
    // The rectangle of the sheet this pass is drawing: its top-left corner in sheet
    // texels, and its extent.
    //
    // The winding target holds one pane at a time, not the whole sheet (ADR 0028): it
    // is scratch — accumulated, resolved, and dead — so its size is a choice rather
    // than a consequence of the page. Sheet texel `pane_origin` is texel (0, 0) of the
    // attachment, and **three** places have to agree about that: `vs_winding` subtracts
    // it before mapping to clip space, `fs_winding` adds it back to test a fragment
    // against its own tile, and `fs_resolve` subtracts it again when it reads back.
    // Any one of them alone is the caller's §11 defect wearing a different offset —
    // ADR 0027 shipped with the second missing, and every band after the first drew
    // nothing at all.
    pane_origin: vec2f,
    pane_size: vec2f,
}

@group(0) @binding(0) var<uniform> globals: Globals;

struct WindingIn {
    // Device pixel position on the winding sheet.
    @location(0) position: vec2f,
    // Loop-Blinn implicit coordinates: the quadratic is u^2 - v = 0.
    @location(1) uv: vec2f,
    // The tile this triangle belongs to (min.xy, max.xy) in sheet pixels. Carried per
    // vertex so one draw call holds every tile of a frame: a chord triangle fans from
    // an anchor and can cover far more sheet than its own tile, so something has to
    // stop it, and a scissor would mean one draw call per tile.
    @location(2) clip: vec4f,
}

struct WindingOut {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
    @location(1) @interpolate(flat) clip: vec4f,
}

@vertex
fn vs_winding(input: WindingIn) -> WindingOut {
    var out: WindingOut;
    // Moving the geometry by minus the sample offset is the same as moving the sample
    // by plus it, and this way the rasteriser's own pixel centres stay the sample
    // grid — no per-fragment arithmetic, and the offset cannot drift between the
    // triangle test and the coverage test because there is only one of them.
    let placed = input.position - globals.sample_offset - globals.pane_origin;
    // Sheet space to pane space: the attachment holds the rectangle at `pane_origin` of
    // `pane_size` texels, so a triangle belonging to another pane maps outside clip
    // space and is dropped by the rasteriser. The host draws only this pane's tiles'
    // triangles, so that is a backstop rather than the mechanism — but a chord triangle
    // fans from an anchor and may reach a long way outside its own tile, which is
    // exactly what the backstop is for.
    let ndc = vec2f(
        placed.x / globals.pane_size.x * 2.0 - 1.0,
        1.0 - placed.y / globals.pane_size.y * 2.0,
    );
    out.position = vec4f(ndc, 0.0, 1.0);
    out.uv = input.uv;
    out.clip = input.clip;
    return out;
}

// One triangle's contribution to the winding number at this sample.
//
// `front_facing` is the whole of the sign: a chord triangle wound the same way as its
// contour adds one, wound the other way subtracts one, and a Loop-Blinn control
// triangle inherits the same rule — which is why the bulge of a curve is added when
// the control point lies outside the chord and taken away when it lies inside,
// without this shader or the geometry that fed it having to decide which case it is.
@fragment
fn fs_winding(input: WindingOut, @builtin(front_facing) front: bool) -> @location(0) vec4f {
    // Outside its own tile this triangle is somebody else's business. The test is on
    // the fragment's own coordinate, which the offset above already moved, so a tile
    // boundary falls in the same place for every sample of the frame.
    //
    // The fragment's coordinate is in the *attachment*, so the pane's origin goes back
    // on before it is compared with a tile stated in sheet texels. Leaving it off is
    // not a small error: every tile of every pane but the first sits below the pane's
    // own origin, fails this test at every fragment, and draws nothing (ADR 0028 —
    // ADR 0027 shipped exactly that).
    let p = input.position.xy + globals.pane_origin;
    if p.x < input.clip.x || p.y < input.clip.y || p.x >= input.clip.z || p.y >= input.clip.w {
        discard;
    }
    // Loop and Blinn's implicit test. A chord triangle carries (0, 1) at every corner,
    // where this holds everywhere, so it is never discarded and needs no branch.
    if input.uv.x * input.uv.x > input.uv.y {
        discard;
    }
    let winding = select(-1.0, 1.0, front);
    return globals.channel * winding;
}

// The resolve pass: one quad per tile, so that a frame pays for the sheet it packed
// rather than for the sheet it allocated, and so that each tile carries its own fill
// rule (a rule is a property of the fill command, not of the sheet).
struct TileIn {
    // The tile in scratch pixels (min.xy, max.xy).
    @location(0) rect: vec4f,
    // x: fill rule, 0 = non-zero (8.5.3.3.2), 1 = even-odd (8.5.3.3.3).
    // y: how many samples the frame takes in total, which is what each group of four
    //    is a quarter of.
    @location(1) rule_and_samples: vec2f,
}

struct TileOut {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) rule_and_samples: vec2f,
}

@vertex
fn vs_resolve(@builtin(vertex_index) vertex_index: u32, tile: TileIn) -> TileOut {
    let corner = vec2f(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    let pos = tile.rect.xy + corner * (tile.rect.zw - tile.rect.xy);
    let ndc = vec2f(
        pos.x / globals.sheet_size.x * 2.0 - 1.0,
        1.0 - pos.y / globals.sheet_size.y * 2.0,
    );
    var out: TileOut;
    out.position = vec4f(ndc, 0.0, 1.0);
    out.rule_and_samples = tile.rule_and_samples;
    return out;
}

@group(1) @binding(0) var winding_tex: texture_2d<f32>;

// Whether a sample's winding number puts it inside the path.
//
// ISO 32000-2 section 8.5.3.3.2 (non-zero): inside where the winding number is not
// zero. Section 8.5.3.3.3 (even-odd): inside where it is odd. What accumulated is a
// sum of +1 and -1 contributions, so it is an integer up to float rounding; rounding
// before the test is what makes both comparisons exact rather than thresholds.
fn inside(winding: f32, rule: f32) -> bool {
    let turns = round(winding);
    if rule < 0.5 {
        return turns != 0.0;
    }
    return abs(turns) % 2.0 == 1.0;
}

@fragment
fn fs_resolve(input: TileOut) -> @location(0) vec4f {
    // The quad is positioned in *sheet* space, because that is what the coverage sheet
    // is; the winding texture holds this pane alone, so the read is the same texel less
    // the pane's origin (ADR 0028).
    let texel = vec2i(input.position.xy) - vec2i(globals.pane_origin);
    let windings = textureLoad(winding_tex, texel, 0);
    var covered = 0.0;
    for (var channel = 0u; channel < 4u; channel = channel + 1u) {
        if inside(windings[channel], input.rule_and_samples.x) {
            covered = covered + 1.0;
        }
    }
    // The frame's coverage is the fraction of *all* its samples that landed inside,
    // and this pass contributes the four it holds. Additive blending sums the groups.
    //
    // **Once per group of four, not once per frame** — this comment claimed the latter
    // until 2026-08-23 and it was false for every frame above four samples. The sheet is
    // `r8unorm`, so each group's store rounds the running sum to 1/255, and a frame of
    // `n` samples pays `n/4` of those roundings rather than one. Measured by
    // `examples/lane_placement` at the default sixteen: three sample rows of four read
    // back as 192/255 = 0.7529 where a single quantisation of 3/4 would give 191/255 =
    // 0.7490, and a full pixel reads 1.0039. The drift is bounded by half a level per
    // group, `n/8` levels in all, and ADR 0076 prices it against the ¼-pixel quantum
    // the sample count itself imposes — which is thirty times larger and is what
    // actually decides this lane's fidelity.
    return vec4f(covered / input.rule_and_samples.y, 0.0, 0.0, 0.0);
}
