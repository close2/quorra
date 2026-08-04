# ADR 0015 — Commands are culled against the target before their geometry is built

Status: accepted, 2026-08-04.

## Context

ADR 0012 recorded, as a cost rather than an oversight, that encode walks the whole
scene whatever is visible: "commands are not culled against damage… culling needs
bounds we already compute". The lever went unused because at 1× it wins nothing — a
page laid out for the window has nothing outside the window.

Zoom is where that stops being true, and the caller found it. A viewer magnifies by
handing the same page a larger transform and a window that has not changed size: at
20× a 1191×1684 window shows about 1/400 of the page, and the encoder was flattening
all of the other 399/400. So a frame got *more* expensive the further in a person
zoomed, when it should have got cheaper — the opposite of what the interaction
implies, and the reason a zoomed page in the viewer stopped being interactive well
before it stopped being legible.

Every lane already drew into `bounds ∩ clip ∩ target` and no further (`coverage_tile`,
`encode_rect`, `encode_image` each intersect exactly those three). The work being paid
for was building geometry that the very next step discarded.

## Decision

**Test `bounds ∩ clip ∩ target` before building anything, and count what that
rejects.**

- The coverage lanes (fill, stroke, oblique rectangle) test the outline's
  convex-hull bounds under the composed transform, inflated by `CULL_MARGIN` = 2
  device pixels. Both pixels are mechanisms, not padding: the glyph lane rasterises
  at a *quantised* sub-pixel phase, up to one pixel from the transform its bounds came
  from, and every coverage tile expands to whole pixels by `floor`/`ceil`. Flattening
  needs no allowance — a flattened point lies on the curve, which lies inside the
  hull.
- A stroke's bound grows by its own reach, `width/2 × miter_limit`: the width is
  device-space (§4.5 resolved it per placement), a miter may carry a corner that far
  from the outline (§8.4.3.5), and a cap reaches half the width, which a limit of at
  least 1 already covers. This is the one place where an outline's hull is not what
  marks pixels.
- The analytic rectangle and image lanes need no margin at all: they intersect
  exactly the region they draw. The rectangle lane also *clips* its geometry to the
  target, which is sound because `target_rect` has integer corners — an edge it
  introduces falls on a pixel boundary and so changes no pixel's coverage.
- A **solid** fill or stroke is rejected before the implicit one-element group a
  non-Normal blend would wrap it in (§11.3.5), which is the expensive half of an
  off-screen blended command.
- A **shaded** fill waits until `shaded_geometry` has resolved its paint.
  Deliberately: an unknown ramp or mesh id must refuse by name wherever the fill
  happens to land. **Visibility may not decide validity** — a scene whose acceptance
  depended on where the viewer was looking would be a worse defect than the work
  culling saves.
- `Counters::commands_culled` reports the count, so the saving is measured rather
  than assumed, and a caller can see how much of a page its window did not need.

This is not §5's forbidden silence. The test establishes that the command could mark
no pixel, so the frame is byte-for-byte the frame that would have built the command
and thrown it away. `tests/cull.rs` asserts exactly that equality.

## Measured

`examples/zoom.rs`, release, RADV (Radeon 890M), 5 933 glyph-lane fills over 107
outlines at 1191×1684, interleaved A/B against the pre-change binary, best of five per
row. **The machine was under load** (a corpus run on ~20 cores), so the wall figures
are pessimistic and the *ratios* carry the finding; the counts are exact.

| magnification | encode before | encode after | commands culled |
|---|---|---|---|
| 1× | 0.76 ms | 0.84 ms | 0 of 5 933 |
| 4× | 0.75 ms | 0.31 ms | 5 340 |
| 20× | 8.2 ms | 6.8 ms | 5 903 |
| 100× | 5.8 ms | 3.1 ms | 5 930 |

And the case a viewer actually spends its frames in — a **zoom gesture**, 1× to 20×
over 24 frames, where every frame carries a new transform so no cached tile helps,
scored by its *worst* frame because that is what a person sees:

| | encode | wall |
|---|---|---|
| before | 156 ms | 168 ms |
| after | **9.3 ms** | 16.4 ms |

Segments flattened per frame at 20×: 29 665 → 150.

## Costs, stated

- **A page with nothing outside the target encodes about 6–10% slower** (5 933
  commands, 0.76 → 0.81–0.84 ms): the test runs once per command and wins nothing
  there. Writing it on scalars instead of through `Rect::intersection` measured the
  same, so the clear construction is the one that stays.
- **A group is not culled as a unit.** An off-screen transparency group still plans a
  child layer; its contents cull individually inside it. Culling the group needs a
  bound over its commands, which nothing computes yet — a recorded lever, as ADR 0012
  recorded this one.
- **Damage is not a cull.** A frame with a valid damage list still encodes every
  command that reaches the target, not only those reaching the damage rectangles.
  ADR 0012's lever remains open, and it must stay conditional on the damage actually
  being honoured (`Surface` and `Readback` targets redraw fully).

## What is left, and what it is not

At 20× the residue after culling is **6.8 ms of encode for 30 commands** — that is not
walking, it is the CPU rasterising 30 glyph tiles of roughly 290×290 px, every frame,
cached by nothing: past `MAX_GLYPH_DIM` (128 px) a glyph never enters the atlas.
Raising that constant to 2048 as a probe took 20× from 6.8 ms to **0.25 ms** of encode
and the frame to 0.61 ms, which identifies the cost beyond argument.

It is *not*, however, an argument for a GPU coverage lane (ADR 0008's recorded lever):
the win came from **caching**, not from where the arithmetic ran. The open question is
therefore an atlas policy for large tiles — what a tile may cost against the 8 MiB
budget, whether a tile clipped to the visible region may be cached at all, and how a
zoom gesture's per-frame key churn is kept from thrashing the atlas it fills. That
decision needs its own ADR and its own measurement; this one stops at proving the
9.35 ms it was hiding behind was not glyph rasterisation.

## What holds it

`tests/cull.rs`: off-target commands of every lane counted and byte-identical output;
a stroke whose width reaches in from outside still draws (columns 0 and 1 at full
alpha, column 2 untouched); a fill straddling the edge keeps `round(0.5 × 255)` on the
half-covered column; a clipped-away command counted and marking nothing; an unknown
ramp refusing both in sight and out of it. `tests/perf_gate.rs`: a deterministic gate
on the count — 0 culled at 1×, exactly 5 909 at 20× — which needs no wall clock and so
cannot flake on a loaded runner.
