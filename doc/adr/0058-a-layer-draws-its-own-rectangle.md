# 0058 — A layer draws its own rectangle

Date: 2026-08-17. Status: **accepted, and built**.

Supersedes one bullet of ADR 0056's costs — *"a full-screen triangle per layer … a
transformed bounding quad is available later if a measurement ever asks for it"*. This is
the measurement, and it asked. The round notes are `doc/notes-present-quad.md`; the code
is `src/shaders/present.wgsl`, `src/present/layer.rs`, `src/present/pass.rs`,
`src/pipeline/spec.rs` and `src/pipeline/layouts.rs`; the proof is
`examples/present_thread/`, extended rather than replaced.

## Context

`Presenter::present(&[Layer])` drew a full-screen triangle for every layer and answered
the placement in the fragment stage: a pixel the layer does not reach maps outside
`[0, source)` and the shader returns transparency. That is correct, and it costs one
fragment of the **whole window** per layer whatever the layer's size.

ADR 0056 declined to change it for the right reason at the time — nothing had measured it.
`HANDOVER.md` carried the debt in one sentence: *on a page-sized layer they are the same,
and on a small chrome layer they are not.*

### The instrument, chosen before the number

ADR 0052's seam decides this: **a claim about "how many" is a count and is exact; a claim
about "how fast" is a duration and this machine cannot measure one.** What is exact here is
how many fragments each arrangement shades — arithmetic over the placements, with no
adapter in it. Device timestamps were taken as well, round-robin in one process, and they
are context rather than argument. One of their cells (llvmpipe, the page-only row) reads
the rectangle 14 % *slower* than the triangle it is 4 % smaller than, which is what a
duration on this machine is worth.

## The counts, at the caller's own window

2048 × 2560 — `pdf-viewer/doc/QUORRA_NONBLOCKING_RENDER.md` §2's 1280 × 1600 window at a
device scale of 1.6 — with the page, selection, sidebar and modal card of `PLAN.md`'s M9
entry, sized from the caller's own tree:

| arrangement | full-screen triangle | rectangle | share |
|---|---:|---:|---:|
| window-sized overlays (the caller's shape today) | 20 971 520 | 19 210 251 | 91.6 % |
| content-sized overlays | 20 971 520 | 10 270 775 | **49.0 %** |
| content-sized overlays, no modal | 15 728 640 | 5 027 895 | **32.0 %** |
| one page layer, reprojected at 1.25× | 5 242 880 | 5 022 720 | 95.8 % |

Read as waste rather than as saving: of a four-layer present today, **1 766 624 fragments
(8.4 %) are shaded to no effect** in the first arrangement and **10 711 584 (51.1 %)** in
the second.

**Both rows are real and they are not the same claim.** The caller's overlays today are
window-pixel display lists — `viewer-ui`'s `highlight_list` and `Notice::draw` each build
one at the window's size, because their present composes everything into one scene. Ported
unchanged into layer textures, they are window-sized textures and this ADR buys 8.4 %. A
host that sizes each overlay texture to what it holds gets 51 %. **Which of those happens
is the host's decision, and the point of this ADR is that today the decision buys them
nothing at all**: the pass costs `layers × window` fragments no matter how small a layer
is. That is CLAUDE.md's first instrumentation rule with the sign flipped — a cost the API
hides from the person who could remove it.

## Decision

**Each layer draws its own rectangle.** `Params` gains `bounds: vec4f` — the layer's
device corners in clip space — and `vs_main` picks a corner of it per vertex index, as a
four-vertex triangle strip. Nothing else about the pass changes.

### The rectangle is a bound, never a decision

The fragment stage keeps its bounds test, unchanged, and stays the only thing that decides
which pixel gets which texel. The vertex stage only says which pixels are worth asking
about. Two properties make that safe, and both are structural rather than measured:

- **It is the axis-aligned box of the placement's parallelogram**, so it contains the
  parallelogram whatever the linear part does — including rotation and shear, where a
  transformed quad's edges would have to be trusted against the shader's inverse map.
- **It is grown outward to whole pixels**, one pixel each side. A pixel is enormous beside
  any rounding difference between a rasteriser's edge functions and the shader's inverse,
  and integral edges in clip space put no pixel centre on a boundary at all.

The rectangle is therefore never smaller than the set of pixels the old arrangement shaded
to some effect, and — clamped to the target — never larger than the window. **The pass
cannot cost more than it did, for any placement.** That is arithmetic, not a benchmark.

### In `f64`, and that is the interesting part

A placement this contract *accepts* can still put a corner outside `f32`: `a = 1e38` has a
finite determinant, so `Layer::inverse` admits it, and a 64-texel row under it reaches
6.4e39. In `f32` that is an infinity, and an infinite vertex is a primitive a driver
discards — a window drawn with nothing on it and no error, which is principle 6's third
state wearing an exponent. The corners are computed in `f64`, where a sum of three
products of finite `f32`s cannot leave the range, and clamped to the target before they
become `f32` again. So there is no non-finite case to have an opinion about, **no new
refusal**, and the enormous placement draws the whole window exactly as it used to.

### `Placed`, so the order cannot be got wrong

`Layer::check` now returns the inverse *and* the bounds together. The bound's arithmetic
assumes a finite invertible placement; returning it from the function that establishes
that is cheaper than a comment saying so.

### The uniform is visible to both stages now

`present`'s bind group entry moves from `FRAGMENT` to `QUAD_UNIFORM` (both stages), which
is the shape every lane already uses. This was found by the warm-set tests within a minute
of the first build, refusing the pipeline by name — the same "stage plumbing at zero
first" trap `HANDOVER.md` records from ADR 0028, caught this time by a gate.

## Determinism: the same bytes, and it was checked as bytes

The two arrangements were drawn into the same 2048 × 2560 target and read back:
**0 differing pixels of 5 242 880**, in all four arrangements, on RADV *and* llvmpipe.
§1.7 is untouched for the older reason too — nothing on this path draws a page, and the
corpus and the oracle both use `Target::Readback`.

`examples/present_thread` gains **the page's own first row and column**: the placement puts
page texel (0, 0) at device (64, 32), so column 63 and row 31 are the window's clear and
column 64 and row 32 are the page. Those four points are the rectangle's boundary from
both sides, and they were needed: with the bound eroded by one pixel instead of dilated,
every assertion that example already had went on passing and these failed —
`the page's left edge: the window shows [0, 0, 0], the scene says [51, 102, 204]`. The
five `device_bounds` unit tests fail under the same defect, and the `Params` layout gate
fails with `bounds` and `inv1` exchanged.

## The costs, written down

- **A rotated placement pays for its bounding box**, up to twice its own area, where a
  transformed parallelogram would have paid for its shape. Nothing in the caller's
  arrangement rotates a layer — a reprojection is a scale and a translation — and the box
  is what makes the "never a decision" property hold without trusting edge arithmetic. If
  a host ever rotates a layer and the count matters, the parallelogram is the change, and
  it needs the overflow story this one gets for free.
- **The outward pixel is fragments nobody needs**: 5 355 and 10 839 on the two four-layer
  arrangements, **0.03 % and 0.11 %** of what the pass draws. It buys the property above
  and it is cheaper than reasoning about a tie.
- **The uniform grows by 16 bytes per layer**, written once per present per layer, and the
  vertex stage now reads it.
- **The win depends on what the host does.** 8.4 % of the fragments at the caller's overlay
  shape today, 51 % if they size their layer textures, nothing at all for a single
  page-sized layer under a reprojection — which is the case ADR 0056 exists for. Anyone
  quoting one of those numbers should quote which.
- **What the pass costs inside a real frame is still not ours to say.** `Xvfb` reports a
  refresh of 0.00 here, so what share of a refresh a present takes is the owner's
  measurement on the display that states one. The notes say exactly what to ask it.

  **Taken on 2026-08-17. It is 4.4 % of a refresh, and this ADR's own guess about itself
  was right** — see the amendment below.

## Consequences

- Public API: none. `Layer`, `Presenter::present`, `PresentCost` and every refusal are
  unchanged; a host recompiles and sees nothing.
- The present pipeline is the first in this crate whose vertex stage is not either a
  full-screen triangle or an instanced buffer. `Spec::strip` and `QUAD_UNIFORM` already
  existed for it.
- ADR 0056's cost bullet is superseded and says so there.

## Amendment, 2026-08-17 — the share of a refresh, and it is the value this ADR predicted

The decision stands and the counts above are unchanged; **the one number this ADR left open
has been measured** on the owner's `eDP-1` — 2880 × 1800 at 119.96 Hz, one refresh
**8.34 ms**. The round is `doc/notes-present-rate.md`; the instrument is
`examples/present_thread/rate.rs` and `examples/present_thread/arrangement.rs`, which CI
runs.

**The pass is 0.37 ms — 4.4 % of a refresh — at the caller's four layers.** It could not be
timed at their 2048 × 2560 window because 2560 rows do not fit an 1800-row display, so it was
measured at **1280 × 1600**, their window at a device scale of 1, where the window-sized
overlay arrangement is **7 506 609 fragments of 8 192 000 — 91.6 %, this ADR's own share to
the decimal.**

Two instruments, and they agree:

- **A count, and it is the exact half.** Sixteen copies of the whole four-layer arrangement
  in one present — 120 105 744 fragments — still land on every refresh, in four runs out of
  four; thirty-two never do. So one present is **at most 1/16 of a refresh, 0.52 ms**.
- **A slope, and it is the indicative half.** `Fifo` floors the interval at the refresh until
  the work exceeds it, so the difference between the two loaded rows divides out everything
  that does not scale with the layer count: **0.367, 0.369, 0.378 and 0.469 ms per copy**.
  The minimum is the number; the outlier is the run whose load average reached 17.40.
  8.31 / 0.367 = 22.6 copies is where the crossing should be, and it is observed between 16
  and 32.

**Scaled to the caller's own window by this ADR's own fragment counts** — 49 ps a fragment,
and this row is a *model*, labelled as one:

| at 2048 × 2560 | fragments | modelled | of a refresh |
|---|---:|---:|---:|
| before this ADR — four full-screen triangles | 20 971 520 | 1.03 ms | 12.4 % |
| window-sized overlays, their shape today | 19 210 251 | 0.95 ms | 11.4 % |
| content-sized overlays, if they size their textures | 10 270 775 | 0.51 ms | 6.1 % |

**So this ADR bought them 0.09 ms — 1.0 % of a refresh — at the arrangement they have, and
0.53 ms, 6.3 %, at the one they could have.** The decision section said exactly this about
itself before the number existed: *"today the decision buys them nothing at all … a cost the
API hides from the person who could remove it."* The milliseconds are not the value; **that
sizing a layer now pays is.**

**And one caveat of `doc/notes-present-quad.md` §3 resolves, in the favourable direction.**
That round timed the pass into an offscreen `Rgba8Unorm` target and warned that *"a surface's
image may differ in tiling or compression, so these are the pass's shape rather than the
window's cost"*. It read 1.44 – 1.49 ms for window-sized overlays at 2048 × 2560, which is
76 ps a fragment against the 49 ps measured here on the swapchain — **the window is about
1.5× cheaper per fragment than the texture**, and the gap is if anything understated, because
the marginal copy in this round also carries four bind groups and four uniform buffers of
host work that the timestamped pass did not include. **The offscreen figure must not be
quoted as the window's cost**; it overstates it by half again.
