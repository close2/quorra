# 0044 — The flatness bound is the tighter of two

Date: 2026-08-14. Status: accepted. Answers the caller's `QUORRA_FEEDBACK.md` §21.2.

## The defect

A filled circle of diameter 0.5 or 1.0 device pixels deposited 36 % less ink than its own
area. The caller measured it through a whole page of their stack; `raster.rs`'s own test
module measures it through `fill_mask` alone, with the new bound made inert, and the two
instruments agree to the digit:

| diameter | ink, `fill_mask` | ink, the caller | `π·r²` | error | chords a turn |
|---:|---:|---:|---:|---:|---:|
| 0.5 | 0.1255 | 0.1255 | 0.1963 | −36.09 % | 4 |
| 1.0 | 0.5020 | 0.5020 | 0.7854 | −36.09 % | 4 |
| 2.0 | 2.8196 | 2.8235 | 3.1416 | −10.25 % | 8 |

Two instruments producing one number is what identifies the mechanism as **flattening**
rather than anything in the compositor: 0.5 is the inscribed square of a unit circle
exactly, and 2.8284 is the inscribed octagon.

## The clause, and a citation corrected

The report cites §10.7.3 for "each output device may have internal limits on the maximum
and minimum tolerances attainable". That sentence is genuinely there — but §10.7.3 is
**Smoothness tolerance**, the allowable *colour* error of a piecewise-linear approximation
to a shading. Flatness is **§10.7.2**, and its own NOTE 2 says the two are only similar:
"flatness is measured in device-dependent units of pixel width, whereas smoothness is
measured as a fraction of colour component range."

The report's conclusion survives the correction, by a stronger sentence in the right
clause. §10.7.2:

> The flatness tolerance controls the maximum permitted distance in device pixels between
> the mathematically correct path and an approximation constructed from straight line
> segments

and

> PDF processors may choose to ignore any flatness tolerance specified within a PDF file.

So the number is licensed *and* it is ours — we need not honour a document's `i` or `FL` at
all, which is why `FLATTEN_TOLERANCE` is a constant of this library (ADR 0008).

The same clause is also where the licence stops, and this ADR turns on it. §10.7.2, NOTE 2:

> Although the figure exaggerates the difference between the curved and flattened paths for
> the sake of clarity, the purpose of the flatness tolerance is to control the precision of
> curve rendering, not to draw inscribed polygons. If the parameter's value is large enough
> to cause visible straight line segments to appear, the result is unpredictable.

At a diameter of one pixel the flattening does not approximate the circle: it *is* the
inscribed square, which is the one thing the note names. Nothing had to be invented to say
where a quarter pixel stops being a tolerance — the clause says it.

## The arithmetic

**What a polygon costs.** A regular *n*-gon inscribed in a circle of radius *r* has area
`(n/2)·r²·sin(2π/n)`, so it covers `(n/2π)·sin(2π/n)` of the circle:

| *n* | ratio | short by |
|---:|---:|---:|
| 4 | `2/π` = 0.63662 | 36.34 % |
| 8 | `2√2/π` = 0.90032 | 9.97 % |
| 16 | `(8/π)·sin(π/8)` = 0.974495 | 2.55 % |
| 32 | `(16/π)·sin(π/16)` = 0.993587 | 0.64 % |

The first two rows are the measured table above, to within the byte the coverage is rounded
to. That is the model earning the right to be used for the rest.

**Where the chords come from.** A circle is four cubics, each a quarter-arc with controls
`k·r` along the tangents, `k = 4(√2 − 1)/3`. For an arc of half-angle `α` the controls sit
`(4/3)·r·tan(α/2)·sin(α)` from the chord, which is what `flatten_cubic` measures:

| half-angle | chords a turn | control distance |
|---|---:|---:|
| `π/4` | 4 | 0.390524·*r* |
| `π/8` | 8 | 0.101476·*r* |
| `π/16` | 16 | 0.025624·*r* |
| `π/32` | 32 | 0.006424·*r* |

Against a fixed 0.25 px that is 4 chords up to *r* = 0.640, 8 up to 2.464, 16 up to 9.84 and
32 up to 39.3. So a circle's chord count goes as `√r` and its area error as `1/n² ≈ 0.25/r`
— **the error is worst exactly where the shape is smallest**, which is the caller's sentence
about it and is the opposite of how a tolerance is usually chosen.

## The decision

**A cubic's flatness bound is the tighter of a distance in device pixels and a fraction of
the curve's own device extent.**

```
tolerance = min(FLATTEN_TOLERANCE, RELATIVE_FLATTEN_TOLERANCE × extent)
```

with `extent` the diagonal of the cubic's four control points' bounding box, in device
space, measured once per cubic in `flatten` and carried down the subdivision.
`RELATIVE_FLATTEN_TOLERANCE` is `1/32`, exactly representable.

Three choices inside that sentence, each with its reason:

- **`min`, so the bound can only tighten.** No curve loses a chord; no page gets coarser
  anywhere. The absolute bound and ADR 0008's argument for it are untouched.
- **Per cubic, not per outline.** One path can hold a page border and a one-pixel dot, and
  the dot must not inherit the border's extent. (`outline.rs`, whose quadratic conversion
  runs *before* any transform, takes the whole outline's extent instead, and says so — it
  has no device scale to be relative to anything else.)
- **Measured once and carried down, not recomputed per half.** A bound recomputed on each
  sub-curve shrinks with it, and the recursion then terminates at a fixed chord *angle*
  applied to shapes of every size — which is a different decision, wider by far, and priced
  below.

### The caller's two remedies are one mechanism

The report offers "a tolerance stated as a fraction of the *shape*" or "simply floored at a
few segments per full turn". They are the same thing seen from two ends. For a quarter-arc
of radius *r* the extent is `r√2`, so the bound is `ρ·r√2`; the stopping half-angle solves
`(2/3)·r·α² ≈ ρ·r√2`, giving `α ≈ √(3ρ√2/2)` — **no *r* in it**. A relative tolerance *is*
a floor on chords per turn, and choosing `ρ` is choosing the floor.

`ρ = 1/32` puts the floor at 16. The window that does is wide: every `ρ` from about 1/14 to
1/54 gives a four-cubic circle exactly 16 chords, and 1/32 sits inside it with a factor of
two of margin each way, so nothing here balances on a knife edge.

### What actually changes

The relative term is inert wherever `extent/32 ≥ 0.25` — a control polygon 8 device pixels
across, which for a quarter-arc (`r√2`) is *r* ≥ 5.66. For a *circle* the count moves by
less still, because the subdivision is binary: it only moves where today's count is below
16, and by the table above that is *r* < 2.464, a circle under five device pixels across.
Between 2.464 and 5.66 the bound tightens and the answer is 16 either way.

So the population that pays is curves whose whole control polygon is under 8 device pixels
across *and* whose bend is large enough that the tighter number forces one more split.
Everything else in a frame is byte-identical, and that is asserted rather than argued
(`a_large_curve_keeps_the_segment_count_it_had`).

**That population is not exotic, and the corpus says so.** A glyph outline at body size is
13 device pixels tall and its bowls and shoulders are cubics two to five pixels across —
squarely inside the 8-pixel crossover. Fifteen corpus pages of ordinary prose moved onto
the oracle for this change (below). The reading to take from that is not that the fix is
bigger than advertised, but that **text is exactly the population a floor reaches**, which
is the argument against raising it further.

## Why sixteen chords and not thirty-two

The floor and its reach are tied: a floor of *N* chords necessarily changes every circle
smaller than the radius at which a quarter pixel already yields *N*. Sixteen reaches
*r* < 2.464 — the caller's rows and almost nothing else. **Thirty-two would reach
*r* < 9.84**: every curve under twenty device pixels across, which is where body text lives.
It would roughly double the flattening of every glyph outline on a frame whose §6.2 gap is
already entirely CPU-side (recording is 1.90–2.32 ms of a 2.84–3.38 ms dense-text frame,
ADR 0043), to buy an accuracy the device cannot show:

- **The byte is the floor.** A mark of diameter *d* touches about `(d+1)²` pixels and each
  is rounded by `round(cov × 255)`, so quantisation alone costs a *d* = 0.5 circle up to
  4.0 % of its ink, a *d* = 1 up to 1.0 % and a *d* = 2 up to 0.56 %. Sixteen chords'
  2.55 % is at that floor for the first two.
- **This file already stands at 2 %.** `ARC_STEP` = 0.35 rad fixes a stroke's round caps
  and joins at `⌈π/0.35⌉` = 9 steps a half turn — 18 a full turn, **2.02 % of a disc's
  area**. A page draws a filled circle and a round cap side by side. A fill held to 0.64 %
  beside a cap held to 2.02 % spends segments on an accuracy its neighbour does not have,
  and raising the floor to 32 would mean moving `ARC_STEP` too.

**The honest residual**, since it is the row that does not fit the argument: at *d* = 2 the
mark keeps 2.76 % of error against its own 0.56 % quantisation floor. That is above what
that mark can show. It is 10.25 % today, the alternative that would remove it costs the
text lane, and this sentence exists so the cost is written down rather than discovered.

## What it costs

- **Four min/max pairs and a `hypot` per cubic**, in `flatten`, on every frame that takes
  the CPU coverage lane — against a subdivision that is four recursions at its cheapest. It
  is not measurable beside the work it guards and it is not claimed to be free.
- **A curve under 8 device pixels across can gain a subdivision level.** Bounded, and by
  geometry rather than by hope: a control point is never further from its chord than the
  control polygon's diagonal, and each split divides that distance by about four, so the
  relative bound is met within three levels *whatever the curve's size*. The depth-16 cap
  therefore still guards only the absolute branch.
- **The glyph atlas is keyed on the outline and its phase, not on chord count**, so a glyph
  that gains chords pays for them once per cached tile rather than once per placement.

## Verification

`raster.rs`'s test module, on the arithmetic above rather than on any observed output:

- `a_circle_deposits_its_own_area_at_every_size` — diameters 0.5, 1.0 and 2.0, ink summed
  from the coverage bytes, held to `π·r²` from below by the 16-gon's 2.55 % and from both
  sides by half a coverage step per pixel the mark can touch. It draws **0.1922, 0.7647 and
  3.0549** against 0.1963, 0.7854 and 3.1416: **−2.14 %, −2.63 % and −2.76 %**, from
  −36.09 %, −36.09 % and −10.25 %. With `RELATIVE_FLATTEN_TOLERANCE` made inert the same
  test reproduces the caller's table and fails, which is the check that it measures the
  reported defect and not something beside it.
- `a_large_curve_keeps_the_segment_count_it_had` — a circle of radius 20 flattens to
  exactly 32 chords, each of its cubics reports `cubic_tolerance == FLATTEN_TOLERANCE`
  (the `min` is not binding on anything 28 pixels across), and circles of diameter 0.5, 1,
  2 and 4 all flatten to exactly 16. The perf posture is a pinned number, not a paragraph.

**The caller's corpus, one copy of their tree, both runs of each pair the same hour,
flipping only the `[patch]` between a `git worktree` at `d53c1c8` and this change:**

| scale | | agree | differ | refused | not comparable |
|---|---|---:|---:|---:|---:|
| 1 | `d53c1c8` | 919 | 35 | 2 | 18 |
| 1 | this change | **934** | **20** | 2 | 18 |
| 4 | `d53c1c8` | 935 | 11 | 5 | 23 |
| 4 | this change | **936** | **10** | 5 | 23 |

**Fifteen pages moved onto the oracle and none moved off it.** They are prose:
`tracemonkey.pdf` and its six annotated variants, `bug1885505`, `bug1992868`,
`chrome-text-selection-markedContent`, `issue14438`, `issue15012`, `issue18911`,
`issue19239`, `issue7014`, `issue7492` — all of them differing by a mean of about 1.52 of
255 before and by nothing at all after. Every one of the twenty pages that still differ
either did not move at all (nine of them, to the last digit of mean, worst tile, position,
fraction and SSIM — the shapes this bound does not reach) or improved on **both** mean and
SSIM. No page's verdict moved away from agreement.

At scale 4 one page moved on — `issue16316.pdf` — none moved off, and the five refusals are
the same five documents by name, so the coverage sheet's ceiling is where it was.

**Two numbers moved the wrong way and they are recorded rather than summarised away.** At
scale 1, `issue12295.pdf`'s worst tile went 7.96 → 8.12 while its mean went 1.6159 → 1.6032
and its SSIM 0.91801 → 0.91823, in the same tile at (128, 128). At scale 4, `inks.pdf` went
0.0564 → 0.0572 on the mean and 15.35 → 17.29 on the worst tile, with SSIM moving in its
fifth decimal. `inks.pdf` is 822 curve operators under 24 strokes of 2.25 to 9 units, which
names the mechanism: for a **stroked** path the flattening is not only the fill boundary, it
is the polygon `stroke_polylines` expands — more chords means more segment quads and more
joins, whose union under the non-zero rule converges on the true swept region from a
chunkier one. Both pages are moving toward the shape and, on one tile, away from the other
rasteriser's polygon. That is principle 5 costing something visible, and it is the right
direction: the oracle flattens too, to a tolerance of its own, and a polygon closer to the
true curve is not thereby closer to *another* polygon.

Both runs also fail the caller's own ratchet — the list of pages they expect to differ,
pinned at the release they consume (`a7babab`, twenty-five commits back). **The baseline
fails it identically**, so the failure is that list going stale, and this change staled it
further in the direction it wants to go.

## What this does not touch

- **The GPU coverage lane**, which does not flatten at all — ADR 0016 converts cubics to
  quadratics once at upload and draws Loop and Blinn's implicit test, so it has no chord
  count for a floor to set. `PLAN.md`'s 2026-08-05 entry measured the two lanes differing
  by up to 96 of 255 on a curved shape and attributed it to this flattening; that entry did
  not go on to ask whether the flattening was *wrong* rather than merely coarser, and on a
  sub-pixel mark it was.
- **`ARC_STEP`**, and the 2.02 % a round cap or join carries. It is a decision of the same
  kind, made in the same file, and it should move only with a measurement of its own.
- **`FLATTEN_TOLERANCE` itself.** ADR 0008 chose the quarter pixel and §10.7.2 licenses it;
  a large curve's chord is still permitted 0.1875 px inside the true edge, and nothing here
  argues about that.

## Revisit when

**The flattening cost of a dense text page is measured.** The corpus says glyph curves are
in the population this reaches; nothing here says what the extra chords cost, because the
instrument for it — `examples/surface_measure.rs`'s `geometry` phase — needs the real GPU
and the corpus's own clocks are unusable at this machine's load. It is a bounded cost (at
most one extra split below 8 device pixels, three levels at the very worst) on a phase
measured at 0.16–0.18 ms of a 2.84–3.38 ms frame, and that is the honest size of what is
unmeasured. Raising the floor to 32 is the decision that waits on this number.

And if `ARC_STEP` ever tightens, this floor tightens with it, for the reason above.
