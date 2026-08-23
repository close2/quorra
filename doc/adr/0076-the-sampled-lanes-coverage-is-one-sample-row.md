# ADR 0076 — The sampled lane's coverage is one sample row, and the clause it misses is priced rather than hidden

Status: accepted, 2026-08-23. Answers question 2 of the caller's `QUORRA_FEEDBACK.md` §31,
which asked "is the gpu lane's y coverage quantised, and to what". It is, to `1/√n` of a
pixel; and the answer that matters is not the number but that **the quantisation breaks
ISO 32000-2 §10.7.4 in a way ADR 0070's condition does not reach and cannot be widened to
reach.** This ADR states the bound, records the non-conformance as one, declines the only
fix available today, and names what would overturn that.

Measurements: `doc/notes-sampled-lane-quantum.md`. Instrument:
`crates/quorra-gpu/examples/lane_placement/`.

**Measured against `take_gpu_lane`, re-measured on `ae03b55`.** ADR 0075 split that function
into `Encoder::gpu_lane_admissible` — the cheap four, including ADR 0070's thin axis — and
`Encoder::triangles_under_coverage`, ADR 0026's byte comparison, which is the floor this ADR
argues against. Neither one's arithmetic changed, and the whole sweep re-run after the rebase
is identical to the character. The new names are used below.

## Context

### What the lane does, from the code

`crate::winding::sample_offsets` lays `n = Options::coverage_samples` samples on a
`√n × √n` ordered grid and puts the k-th at `((k mod √n) + ½)/√n − ½` across and
`((k / √n) + ½)/√n − ½` down. Across the whole device those rows are one lattice of period
`p = 1/√n`. `winding.wgsl`'s `fs_resolve` counts how many of a pixel's samples the fill rule
admits and stores `covered / n`.

For an **axis-aligned** band the consequence is arithmetic rather than empirical: every
pixel row the band reaches contributes `√n` samples for each lattice point it holds, so

- the band's **ink** is `k · p`, where `k` is the number of lattice points in the half-open
  interval the band occupies — either `⌊w/p⌋` or `⌈w/p⌉`;
- the band's **centroid** is the plain mean of those lattice points, and nothing else;
- a pixel row holding no lattice point of the band receives **exactly zero**.

`p` is 0.5 at four samples, **0.25 at the default sixteen**, and 0.125 at sixty-four.

### The clause, verbatim

> A shape shall be scan-converted by painting any pixel whose half-open square region
> intersects the shape, no matter how small the intersection is. This ensures that no shape
> ever disappears as a result of unfavourable placement relative to the device pixel grid,
> as might happen with other possible scan conversion rules. The area covered by painted
> pixels shall always be at least as large as the area of the original shape. This rule
> applies both to fill operations and to strokes with non-zero width. Zero-width strokes may
> be done in an implementation-defined manner that may include fewer pixels than the rule
> implies.
>
> — ISO 32000-2:2020 §10.7.4

with NOTE 1 under it, which is the sentence that makes the first requirement bite at a
boundary rather than at an interior:

> Normally, the intersection of two regions is defined as the intersection of their
> interiors. However, for purposes of scan conversion, a filling region is considered to
> intersect every pixel through which its boundary passes, even if the interior of the
> filling region is empty.

`tests/thin_marks.rs`'s module comment already reads two of those sentences as decidable in
an anti-aliased device's units, and ADR 0070 turned the first into a lane condition — read
**per mark**: no shape disappears. This ADR is the same sentence read **per pixel**, which is
how the clause states it, and there the lane does not hold it.

### The measurement, and the trap that hid it for three rounds

A 0.878-device-pixel band — the caller's own `issue16500.pdf` witness — swept through a
whole pixel of position, on the sampled lane, at three sample counts
(`doc/notes-sampled-lane-quantum.md` §3):

| samples | pitch | distinct inks the sweep produced | worst ink error | worst placement error |
|---|---|---|---|---|
| 4 | 0.5 | 0.5020, 1.0039 | **−0.3760** | −0.3108 |
| 16 | 0.25 | **0.7529**, 1.0039 | +0.1259 | +0.1757 |
| 64 | 0.125 | 0.8784, 1.0039 | +0.1259 | −0.1216 |

Two rungs at every count, a pitch apart, at exactly the lattice `p·k`. **0.7529 is
192/255, which is the caller's 0.753 to the byte** — their table's most alarming row is this
lane's arithmetic seen from their tree, and nothing else.

**Why no gate saw it, and why no round did either.** Two separate reasons, and the second was
found by forcing the first.

`tests/thin_marks.rs` sweeps `SUB_PIXEL_WIDTHS = [0.75, 0.5, 0.25,
…]`, and every one of those at or above the default spacing is a multiple of ¼ — of the
pitch itself. A band whose width the pitch divides contains the same count of lattice points
*wherever it lands*, so its ink is exact at every position and the clause's floor holds by
arithmetic rather than by fidelity. The file's assertion of that floor on the device lane has
therefore never been able to fail. This is the third appearance of one trap — a sweep whose
step divides the quantity under test measures that quantity's fixed points — after the glyph
phase at 1/16 (`doc/notes-glyph-phase-carry.md` §2) and `--check`'s own four steps of ¼,
found in this round.

**And the round that went looking had reached the lane without knowing it.**
`examples/lane_placement`'s previous revision concluded that a stroked hairline "takes the
path lane under both settings" and never reached the sampled grid at all. Its stroke row was
on the sampled grid: `LaneCounts::path` counts the processor's tiles and the winding lane's
together — deliberately, since "they produce the same tile on the same sheet and differ only
in who drew it" — so the lane *name* agreed while the rasterisers did not. Re-measured at that
revision's own geometry, the stroke's sampled column snaps to the ¼ grid at ±0.1071 where its
processor column is exact to 0.0019. **A counter that names two producers with one word cannot
answer a question about which producer drew a mark**, which is why this round's reachability
check is behavioural — the ink and the placement against the grid's own pitch — and why the
`Reading::lane` field in the instrument now says so at its own definition.

### What actually fails, and under which reading

**§10.7.4's first sentence fails under every reading of the clause.** Counted over the same
sweep — a pixel row whose exact area inside the shape is not zero, which the processor lane
inks, receiving **exactly zero** from the sampled lane:

| samples | pitch | positions with an unpainted pixel |
|---:|---:|---|
| 4 | 0.5 | **19** of 38 |
| 16 | 0.25 | **10** of 38 |
| 64 | 0.125 | **5** of 38 |

**The fraction is the pitch**, and that is arithmetic rather than a fit: a pixel row holds no
lattice point when the band's part in it is shorter than the distance from the boundary to
the first sample row, `pitch/2`, and there are two edges — so `2 · pitch/2 = pitch` of every
whole pixel of placement. **A quarter of all sub-pixel placements at the default sample
count.**

NOTE 1 puts that pixel inside the requirement explicitly: the band's boundary passes through
it. It is not painted. Under the clause's own binary vocabulary that is a pixel the rule says
shall be painted and is not; under the anti-aliased reading it is a coverage of zero where the
shape has area.

**§10.7.4's third sentence fails under the anti-aliased reading**, which is the one this tree
took in `tests/thin_marks.rs` and in ADR 0070: 0.7529 of ink for 0.878 of shape is not "at
least as large as the area of the original shape". Under the strictly binary reading it
passes vacuously, because one whole painted pixel is more area than a 0.878-pixel band has.

**The shape does not disappear**, which is the requirement ADR 0070 bought and which holds:
the band is 3.5 times the pitch, so it always catches lattice points somewhere. The caller's
own sentence — "it does not disappear, so §10.7.4 is not broken" — is right about the
disappearance and wrong about the clause, and the difference is that the clause is stated per
**pixel** and their reading was per **mark**.

### No threshold removes it

ADR 0070's fifth condition diverts a mark whose thin axis is below `p`. The obvious response
is to raise that threshold. It does not work, and the arithmetic says why: the ink error is
`p·k − w`, whose magnitude reaches nearly `p` for any `w` just above a multiple of `p`,
**independently of how large `w` is**. A ten-pixel band is as capable of drawing 9.75 as a
one-pixel band is of drawing 0.75. A threshold bounds the error only *relative* to the mark:
at width `T` the relative undershoot is `p/T`, so holding a page of rules to five per cent
would divert everything under five device pixels to the processor — which is every rule, most
glyphs, and the lane's reason for existing.

The same arithmetic disposes of "more samples". Sixty-four samples halve the pitch twice and
cost four times the pass pairs; the table above shows the error at sixty-four is still +0.1259
of a pixel, because the failing case is the *other* rung and that rung is `⌈w/p⌉·p − w`, which
does not shrink with `p` at all for a fixed `w`. Only the undershooting rung shrinks.

### A second, much smaller thing found beside it

The sheet is `r8unorm` and the resolve pass stores into it **once per group of four
samples**, not once per frame: a frame of `n` samples pays `n/4` roundings to 1/255 with
additive blending between them. `winding.wgsl` claimed the opposite in a comment — "the
single quantisation to a byte happens once, at the store" — and that was true of a
four-sample frame and of no other. Measured at the default sixteen: three sample rows read
back as 192/255 = 0.7529 where one quantisation of ¾ would give 191/255 = 0.7490, and a full
pixel reads 1.0039. The drift is bounded by half a level per group, `n/8` levels in all —
four hundredths of a pitch at sixteen samples, thirty times smaller than the quantum above
it. The comment is corrected; the arithmetic is not changed.

## Decision

### 1. The bound is stated, and it is one pitch

For an axis-aligned mark on `Coverage::Gpu`, against the exact area ADR 0005 defines:

- **ink** is a multiple of `1/√coverage_samples` and within one of those of the shape's area,
  in either direction;
- **placement** — the coverage centroid — is within one pitch, and within **half** a pitch
  where the mark's width is a multiple of the pitch, which is the case a hairline of exactly
  one device pixel is;
- **a pixel the shape intersects may receive zero**, which is the non-conformance above and
  is not bounded by anything smaller than "it happens".

`1/√n` and not a written-down 0.25: the bound follows `Options::coverage_samples`, exactly as
ADR 0070's threshold does, and for the same reason.

### 2. It is recorded as a non-conformance, not as a tolerance

`Coverage::Gpu`'s own rustdoc carries it, because that enum is where a caller chooses this
lane and CLAUDE.md's rule is that a cost belongs where the choice is made rather than in a
note somebody has to find. The wording says which sentence of §10.7.4 is not met, so that a
reader of the API is not left to infer it from a bound.

Principle 6 asks whether a lane that cannot draw what was asked should **refuse**. It should
not, here, and the reason is that the refusal would be of the setting rather than of a scene:
sampled coverage is what `Coverage::Gpu` *is*, every mark on it is subject to this, and a
backend that refused every mark would be a backend with one lane. What principle 6 does
require is that the caller can tell — which is what §2 above is for, and it is why the answer
is a documented property of the setting rather than a silent one.

### 3. Routing does not change this round

The lane chooser is untouched — `gpu_lane_admissible` and `triangles_under_coverage`, which
ADR 0075 split out of `take_gpu_lane` without changing either one's arithmetic. Two reasons,
and the first is the binding one:

- **The only removal is the area rule ADR 0070 declined**, and that ADR's price stands
  unchanged: a new pipeline pair, a new shader, a new vertex format, ADR 0026's criterion
  re-derived, ADR 0006's cross-adapter identity re-measured against a *float* accumulation
  where today's is integer, and the loss of `Options::coverage_samples`'s meaning as public
  API. It is a milestone.
- **No threshold change may be made without a corpus column**, and this round did not run
  one — the corpus is run from one copy of the caller's tree and another session held it.
  "Genuinely faster is decided by measurement" cuts both ways: a lane condition widened on
  arithmetic alone is the constant ADR 0027 picked and ADR 0029 had to unpick.

### 4. The gate stops asserting the clause at the quantiser's fixed points

`tests/thin_marks.rs` gains `OFF_LATTICE_WIDTHS = [0.878, 0.6, 0.31]` — none a multiple of
¼, ½ or ⅛, so none a fixed point of any grid `Options::coverage_samples` admits — and
`the_device_lanes_ink_is_quantised_to_one_sample_row`, which sweeps them across ten sub-pixel
positions and asserts the honest two-sided bound. §10.7.4's **first** requirement is asserted
there, because the mark still cannot disappear; its **area floor** is not asserted of the
device lane, because it does not hold and asserting it only at widths the pitch divides is
what produced three rounds of false confidence.

`assert_the_clause_holds` splits so that the disappearance half is reusable and the floor half
carries, at its own site, the sentence naming which lanes it is true of.

## Consequences

- **The caller's §31 question 2 is answered with a number and a clause**: the quantum is
  `1/√coverage_samples`, 0.25 device pixels at the default, and their 0.753 is 192/255 — three
  sample rows plus the four byte roundings of §"A second, much smaller thing" above.
- **Their `bug1863910.pdf` witness is the same arithmetic**: 0.500/0.500 is two sample rows
  each, and reads 0.502 here for the same reason.
- **`Coverage::Cpu` is unaffected in every respect.** The processor lane computes exact area
  (ADR 0005) and this ADR does not touch it. The caller's default lane and everything below
  their magnification of ten is outside all of the above.
- **The corpus is predicted to move by nothing**, and the prediction is stated so it can be
  wrong: no lane condition, shader arithmetic or encoder path changed, so a `Coverage::Gpu`
  column should be byte-identical either side of this ADR. What changed is one WGSL comment,
  one enum's rustdoc, one example and one test file.
- **A cost that is now visible**: a page of hairlines on `Coverage::Gpu` can show a stripe —
  the caller's own words — because neighbouring rules at different sub-pixel phases land on
  different rungs. This ADR does not fix that; it makes it a thing a reader of `Coverage` is
  told about before choosing.

## What holds it

- `tests/thin_marks.rs::the_device_lanes_ink_is_quantised_to_one_sample_row` — three widths
  the pitch does not divide, ten positions each, the two-sided bound and the disappearance
  requirement, on a device.
- `examples/lane_placement`, `--check` in CI: the ladder is asserted to lie on the lattice
  `p·k` at every sample count, the ink and placement bounds are asserted against `1/√n`
  computed from the sample count rather than written down, and **reachability** is asserted —
  a run in which the mark never reached the sampled lane fails by name, naming the triangle
  floor and the sweep's own step as the two things that put it there.
- `crates/quorra-gpu/examples/lane_placement/witness.rs` — the caller's published §31.2 table,
  with this lane's grid arithmetic applied to it, run on every invocation.

### The defect forced

`the_device_lanes_ink_is_quantised_to_one_sample_row` was written against a bound of half a
pitch first, which the four-sample column of the instrument then contradicted at −0.3760.
That is the derivation being corrected by a measurement rather than the measurement being
fitted to a derivation, and it is why the bound in §1 is one pitch and not half of one.

Forced afterwards, one at a time, on the finished tree:

| # | the defect, forced | what went red |
|---|---|---|
| 1 | `OFF_LATTICE_WIDTHS` replaced by `SUB_PIXEL_WIDTHS`' three lattice widths | `the_device_lanes_ink_is_quantised_to_one_sample_row`'s fixed-point guard: *"0.75 is 0.000 of a pitch from a multiple of the grid's 0.25, so it is a fixed point of the quantiser and this sweep would assert nothing"* |
| 2 | the device-lane bound tightened to §10.7.4's floor, `ink >= w − 1/255` | the same test, at the first position of the first width: *"width 0.878: 0.7529412 of ink"* — the clause failing, on a device |
| 3 | `fixture::ALONG` cut to 48, below the triangle floor | `lane_placement`'s reachability assertion, naming the floor and the sweep's step |
| 4 | ADR 0073's carry removed from `GlyphPlacement::of` — `% q` without `ix += 1.0` | `lane_placement`'s **processor**-column bound: *"the cpu column moved a hairline by −0.9844 device pixels, past the quantum's own bound of 0.0312"*. The assertion this round inherited is intact and still fails on the defect it was written for |

**Defect 3 was forced at 128 first and did not go red**, and that is where §"the trap" above
gained its second half. `triangles_under_coverage` is handed the mark's **own unclipped
device box**, not
the visible tile, and the instrument's rule runs past both edges of the target — so at
`ALONG` = 128 the box is `4 × 132` = **528** texels. That clears the stroke's 384 and fails
the fill's 576. The previous round's stroke row was on the sampled lane and was read as the
processor's, because `LaneCounts::path` is the name of both rasterisers and no counter
separates them. `doc/notes-sampled-lane-quantum.md` §1.1 has the measurement.

## Revisit when

- **The area rule is built.** It removes all of §"What actually fails" at once, and ADR
  0070's "Revisit when" already names what would justify it: a page from the caller's product,
  past magnification ten, where a sub-pixel mark is a material part of what a frame shows. A
  table of hairlines is that page, and §31 is the first sighting of one.
- **A corpus column at `Coverage::Gpu` is run with ADR 0070's threshold raised**, which is the
  measurement §3 declines to make without. It would say what a threshold costs in pages rather
  than in arithmetic, and it is cheap: one constant, one column.
- **A caller sets `coverage_samples` away from sixteen in earnest.** Everything above follows
  the option; nothing above was measured on a device at any other value until this round, which
  measured three.
