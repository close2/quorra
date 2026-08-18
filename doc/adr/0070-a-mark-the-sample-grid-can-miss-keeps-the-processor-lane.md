# ADR 0070 — A mark the sample grid can miss keeps the processor lane

Status: accepted, 2026-08-18. Adds a **fifth** condition to ADR 0026's lane chooser and
turns `tests/thin_marks.rs`'s recorded gap into the requirement it only recorded. Declines
the area rule of `doc/notes-thin-mark-options.md` §3, which is the other half of this
decision and is written down here because a decision whose alternative's cost is not
recorded has not been made.

## Context

ADR 0016's device lane counts samples on an ordered `√n × √n` grid, so it answers *is this
sample point inside the shape* and never *does this shape intersect this pixel*. ISO 32000-2
§10.7.4 asks for the second:

> A shape shall be scan-converted by painting any pixel whose half-open square region
> intersects the shape, no matter how small the intersection is. This ensures that no shape
> ever disappears as a result of unfavourable placement relative to the device pixel grid, as
> might happen with other possible scan conversion rules. The area covered by painted pixels
> shall always be at least as large as the area of the original shape.

A 0.1-device-pixel bar swept across ten sub-pixel positions therefore **drew nothing at six
of them** on the device lane and two and a half times its own ink at the other four, where
the processor lane drew 0.10196 at all ten — byte-identical on llvmpipe and RADV, so it is
the design and not an adapter (`doc/notes-hayro-paints.md` §2).

`doc/notes-thin-mark-options.md` costed the answers on 2026-08-18 and the owner took its
§5.4. Three of its findings are the premises of this ADR and are not re-derived here:

- **Drawing 0.25 where the shape covers 0.10196 is not a violation.** The clause's second
  requirement is a floor — "at least as large as the area of the original shape" — and an
  overshoot satisfies it. It is a fidelity cost against ADR 0005's choice, nothing more.
- **Drawing nothing is a violation**, and the clause names the failure mode itself:
  "unfavourable placement relative to the device pixel grid". The last sentence's exemption
  is for zero-width strokes and reaches nothing here.
- **The population is small and magnification shrinks it**: 35 marks on 7 pages of the
  caller's 954-page corpus at scale 1, 26 on 2 pages at 4×, 16 on 1 page at 8× — because a
  stroke's width arrives already resolved in device pixels and does not follow the viewport
  — and the caller only reaches this lane past their `GPU_COVERAGE_MAGNIFICATION` of ten.

## Decision

### 1. A fifth condition, a peer of ADR 0026's four

`Encoder::take_gpu_lane` gains: **a mark whose thin axis is below the device lane's own
sample-column spacing keeps the processor lane**, whatever the four cost conditions would
have said.

### 2. The threshold is derived from `Options::coverage_samples`, not chosen

`crate::winding::sample_offsets` puts `n` samples on a `√n × √n` grid with the k-th column at
`(k + ½)/√n` of a pixel. Consecutive columns are `1/√n` apart, and **so is the wrap across a
pixel boundary**: the last column of one pixel sits `1/(2√n)` from its right edge and the
first column of the next `1/(2√n)` past it, because the grid is symmetric about the centre.
The columns are therefore one lattice of period `1/√n` across the whole device — a half-open
interval of that length contains a column wherever it lands, and one shorter than it has
placements that contain none.

So the threshold is `1/√coverage_samples`, in `encode/thin.rs::sample_column_spacing`, and it
**moves with the option**. That field is public and settable; construction rounds it down to
a square and clamps it to `4..=64`, so the grid's side is 2 to 8 and the threshold runs from
**0.5 device pixels at four samples** to **0.125 at sixty-four**. The condition therefore
narrows as a caller buys quality, which is the direction it must move in: more samples is a
grid that misses fewer marks and so fewer marks that need the other lane. A caller who sets
four samples pays for it in CPU rasterisation of everything under half a pixel; one who sets
sixty-four is diverted only under an eighth.

The spacing is computed **once per frame** into `Encoder::sample_spacing`, so the condition
itself is one float comparison at each of the two call sites rather than a square root per
mark.

### 3. What "thin axis" means, in one place

`encode/thin.rs::ThinAxis` is the only definition, and it is a newtype so the chooser cannot
be handed a tile side or a stroke width by accident. It is the **smaller** of the bounds that
apply to the mark:

- **the narrower side of its device box.** The mark lies inside that box, so across that axis
  it is nowhere wider. Exact for the axis-aligned rule a document draws most of its thin
  marks with.
- **a stroke's own resolved device width** (§8.4.3 with §8.4.3.2 and §10.7.5; §4.5 of the
  brief settles it upstream). A stroke is nowhere wider than that across its path *at any
  angle*, which is the bound a box cannot supply.

Every bound is an **upper** bound on the mark's thickness, and where two apply the smaller is
taken — because the two errors are not symmetric. Over-stating thinness costs a mark the
device lane, which is CPU rasterisation for a mark that would have been drawn correctly
anyway: a cost. Under-stating it leaves a mark on a lane that can lose it: a §10.7.4
violation.

### 4. The residual, stated here rather than discovered later

**A hairline at 45° given as a fill is not covered.** A thin parallelogram submitted as a
`Fill` has a device box far wider than the mark and no width of its own to be read instead,
so it reads thick and keeps the device lane. It does not *vanish* there — it crosses many
pixels and catches a sample column in some of them — it **dots**: uneven coverage along its
length where the processor lane's would be even. The same shape of error, quieter: a curved
mark's box here is its *control hull's*, which over-states a thin curve's extent.

A 45° **stroke** is caught, because its own width bounds it at every angle. That asymmetry is
the whole reason the stroke width is threaded through `push_coverage` at all, and
`a_turned_hairline_stroke_is_declined_by_its_own_width` is what holds it.

The corpus says the residual is small: `issue12810.pdf`'s 29 375 sub-quarter-pixel strokes
are the corpus's largest such population and exactly **one** of them takes the device lane,
and after the change no page outside the processor lane's own differing set remains
(`doc/notes-thin-mark-options.md` §2.4, and the matrix below).

## The inversion this records, and its price

ADR 0026's title is that the lane is chosen by what each lane would **cost**. This condition
is not a cost comparison, and it is deliberately allowed to overrule one: it declines a lane
that is cheaper for a mark it cannot represent. That is an inversion of that ADR's principle
and it is recorded as one rather than folded in quietly.

Its price is that those marks are rasterised on the CPU. Stated as a measurement rather than
an expectation:

- **The count is 35 marks at scale 1, 26 at 4× and 16 at 8×**, over the caller's whole
  corpus, measured by the probe of `doc/notes-thin-mark-options.md` §2.1.
- **Each of them is under a quarter of a device pixel across by construction**, so its tile
  is one or two pixels wide and among the smallest the sheet ever holds.
- **A count is the honest statement here, because this machine cannot measure the duration**
  (`doc/HANDOVER.md`'s wall-clock trap, and ADR 0052's seam between "how many" and "how
  fast"). Nothing in the corpus matrix below moved a refusal or a page's coverage
  accounting, which is the arithmetic half that *is* machine-independent.

The encode side pays one float comparison per solid fill and per stroke that reaches the
chooser, and `Coverage::Cpu` — the caller's default — pays nothing at all: the setting test
short-circuits ahead of it, as it did before.

## What it buys, measured on the caller's corpus

One copy of their tree taken 2026-08-18, base and change run in the same sitting against it,
per-page lines compared rather than only the totals (`doc/HANDOVER.md`'s corpus trap).

<<MATRIX>>

> **Corrected 2026-08-18.** Two things about this section, both found by a later round and
> left in place rather than rewritten.
>
> **The matrix was never transcribed.** `<<MATRIX>>` above and `<<DEFECTS>>` below are the
> round's own placeholders, merged unreplaced. They are not filled in from here: the
> settlement round measured one lane at one scale, and writing the other rows from anything
> but a run is the failure `doc/notes-release-matrix.md` exists to prevent.
>
> **This round's report explained its scale-4 exit 101 by saying the caller's `REFUSED_AT_FOUR`
> "does not list `issue18032.pdf`, which ADR 0069 began refusing two commits before mine".
> Both halves are wrong.** That ratchet does list `issue18032.pdf`, and has since the caller's
> five-hundred-and-twelfth session (their ADR 0327, 2026-08-08, eight days before ADR 0069);
> the page is refused by `render-quorra`'s own §11.4.6 check *before* a `quorra_scene::Scene`
> is built, so `SceneError::KnockoutElementGroupUnsupported` cannot reach it, and its message
> appears nowhere in either column. The one element of the failing assertion's difference is
> `bug1703683_page2_reduced.pdf`, which ADR 0057 moved from **refused to drawn** — the
> opposite direction, and the caller's outstanding re-baseline that the three matrices before
> this one already record. **No corpus page moved from drawn to refused.** Evidence, both
> revisions in one copy of their tree at `829d7faa`:
> `doc/notes-release-matrix.md`, "A refusal that did not move".

## Why the area rule was declined

`doc/notes-thin-mark-options.md` §3 sketched an exact-area rule on the device lane, and §4
priced what it does to public API. It is not built, and the reasons are recorded here because
this ADR is where a later round will look for them.

- **It costs ADR 0016 its reason for existing.** Loop and Blinn's `u² < v` answers *inside or
  outside at a point*; it is a sample test by construction, so an area rule cannot use it.
  The realistic form of the rule flattens to a device-space tolerance instead — and ADR 0016's
  own sentence is "**Nothing in that depends on the device scale.** That is the whole point,
  and the difference from `raster.rs`, whose flattening tolerance is in device pixels."
  Flattening per frame at a device tolerance is exactly the 6.8 ms-a-frame cost ADR 0015
  measured at 20× and exactly what a zoom gesture defeats.
- **It trades the one fidelity advantage the device lane has for the one it lacks.** ADR 0016
  measured the lanes differing by up to 96 of 255 on a curved edge **with the processor lane
  wrong**, because a chord cuts inside a convex curve at `FLATTEN_TOLERANCE`'s quarter pixel.
  An area rule that flattens inherits that error.
- **It is a milestone, not a round**: a new pipeline pair, a new shader, a new vertex format,
  ADR 0026's criterion re-derived for flattened edges, ADR 0006's cross-adapter identity
  re-measured (the current promise is protected by the accumulation being *integer*; an area
  accumulation is float), and a corpus run.
- **It makes `Options::coverage_samples` meaningless, which is public API** (§4 of the notes):
  under an area rule there are no samples, so the field must be deprecated, made to select
  between two lanes, or refuse a non-default value — each of them a decision, and one of them
  a `Report` for a field that was legal in the previous release.
- **And the condition already reaches the whole of the visible prize.** The matrix above is
  the processor lane's own verdict for that scale; an area rule would arrive at the same
  place at that price.

What would overturn it is in the notes' §6 and is not re-stated: a page from the caller's
*product*, past magnification ten, where a sub-quarter-pixel mark is a material part of what
a frame shows. The corpus's answer today is one page.

ADR 0064's objection to sending rare paints to the device — that magnification would put
shading-painted text on this grid — is **not** dissolved by this ADR: that arm never asks
`take_gpu_lane` at all, so nothing here changes what a rare paint does. It would be dissolved
by an area rule, and the notes price what that unlocks at 0.11 % of a frame's coverage at
scale 1 and 0.63 % at 4×, with 82 % of rare-painted coverage still behind the residue
multiply that neither change touches.

## What holds it

In `crates/quorra-gpu/tests/thin_marks.rs`, on a device, and in `encode/thin.rs`'s own unit
tests for the arithmetic:

- `a_mark_below_the_sample_spacing_is_drawn_at_every_position_on_both_lanes` — the clause on
  **both** lanes: every sub-pixel width in the file's sweep, at ten sub-pixel positions, on
  `Coverage::Cpu` and `Coverage::Gpu`. Below the spacing it also requires the ink to be the
  shape's exact area to one 8-bit rounding, which is the stronger claim: not "the device lane
  got better" but "the mark is drawn by the producer that can draw it".
- `the_lane_is_declined_exactly_below_the_sample_spacing` — the lane itself, read from
  `Counters::bytes_uploaded` rather than from pixels. A fixture whose subject is a lane choice
  must assert the lane (`doc/HANDOVER.md`'s `m45.rs` trap), and one whose subject is a
  *difference* must measure the rejected reading and require it to miss
  (`tests/mask_shape_or_opacity.rs`'s pattern): the bar **at** the spacing must still take the
  device lane, so a condition that declined every thin mark fails here.
- `a_turned_hairline_stroke_is_declined_by_its_own_width` — the half of `ThinAxis` a box
  cannot reach, with the box-only reading measured and required to miss.
- `encode::thin::tests` — the spacing at every sample count the option admits including both
  ends of the clamp, the boundary being *below* rather than *at*, the stroke width being read
  where it is the narrower bound, and a non-finite thin axis declining nothing.

**The instrument, and why it needs no constant.** The processor lane *uploads* its rasterised
tile into the sheet; the device lane draws its own and uploads a winding target instead. So
for one scene and one viewport `Coverage::Gpu` reports **exactly** the `Coverage::Cpu` figure
when no mark took the device lane, and strictly more when one did — an equality and a strict
inequality, rather than a threshold read off a run.

<<DEFECTS>>

## Revisit when

- **ADR 0026's criterion is re-derived** (its own "Revisit when": the winding-texture banding
  changes what the device lane costs for a large tile). This condition sits ahead of that
  comparison and is unaffected by its outcome, but the *order* of the five should be re-read
  when the four change.
- **A caller sets `coverage_samples` away from sixteen in earnest.** The threshold follows the
  option by construction, but the corpus matrix below was taken at sixteen and the population
  it measures is a function of the threshold.
- **The 45° filled hairline stops being a residual.** The instrument is
  `doc/notes-thin-mark-options.md` §2.1's probe with the thin axis taken from the mark's own
  geometry rather than its box; it costs minutes, and today's answer is one corpus mark.
