# ADR 0030 — A clip chain is one region, so its links intersect

Status: accepted, 2026-08-12. Answers the question the caller's `QUORRA_FEEDBACK.md` §18
put to this side, from the clause rather than from their answer.

## Context

The caller changed how their own rasteriser composes a clip chain — from a product of the
links' coverages to `min` — and told us, because their cross-backend gate would otherwise
report the difference as ours. They asked one thing: **say what our rule is, and whether
§10.7.4 changes it.**

Our rule was a product. `Encoder::residue_product` multiplied each non-rectangular link
into the last with `(a·b + 127) / 255`, which on a chain of *n* restatements of one clip
raises an antialiased boundary to the *n*-th power. Their ladder: one page under *n*
identical clips paints its edge at 0.5, 0.25, 0.125, 0.0625 — each rung the one above it
halved. Their witness page, `issue21346.pdf`, states one device rectangle six times over
(a `W n`, three `/BBox` clips under §8.10.1 step c, the mark's own path and a §11.6.5 mask
group's) and painted its edge at **0.041** where the geometry is 0.827; `poppler` and
`ghostscript` give 1.000, `mupdf` 0.755.

## What the specification says

Read from ISO 32000-2:2020 rather than from their quotation of it. Three sentences decide
it, and the first is the one that settles the chain:

§8.5.4, on what a clipping path operator does:

> After the path has been painted, the clipping path in the graphics state shall be set
> to the intersection of the current clipping path and the newly constructed path.

§8.5.4 again, on what a clip does to a shape value:

> The effective shape is the intersection of the object’s intrinsic shape with the
> clipping path; the source shape value shall be 0.0 outside this intersection.

§10.7.4, on scan conversion:

> For clipping, the clipping region consists of the set of pixels that would be included
> by a fill operation. Subsequent painting operations shall affect a region that is the
> intersection of the set of pixels defined by the clipping region with the set of pixels
> for the region to be painted.

And §11.3.7.2's NOTE 1, which is why there are fractions here at all:

> Mathematically, elementary objects have "hard" edges, with a shape value of either 0.0
> or 1.0 at every point. However, when such objects are rasterized to device pixels, the
> shape values along the boundaries can be anti-aliased, taking on fractional values
> representing fractional coverage of those pixels.

**A chain is not a stack of boundaries in the model at all.** The graphics state holds one
clipping path, and each `W` replaces it with the intersection of two *paths*. Rasterising
each link separately is our convenience; the fractional values that then appear are, by
NOTE 1, the rasterisation of one hard-edged region. Nothing in the standard composes two
fractional coverages — the place where a genuine product of shape values lives is §11.5's
soft mask, which is a different mechanism with its own clause, and importing its
arithmetic into clipping is what a product does.

## Decision

**The links of a residue chain intersect: `min`, per pixel.**

Two properties follow, and the first is the clause's own:

- **Idempotence.** Intersecting a region with itself is the region, so restating a clip
  must change nothing. `min` has that; a product cannot. `tests/m3.rs`'s
  `a_clip_stated_again_admits_exactly_what_it_did` states one diagonal clip up to five
  times and holds every frame to the first byte for byte — it reads 64 of 255 under the
  old rule.
- **It is never further from the truth.** The exact value is the area of the intersection
  of the regions inside the pixel, which lies in `[max(0, a+b−1), min(a, b)]`. `min` is
  that interval's upper end and the product is inside it, so `min` is never *below* the
  product and is exact wherever two boundaries coincide or nest — which is what a chain's
  links are.

**Where the clip meets the mark, the product stays**, and that is a choice rather than a
consequence. §8.5.4 asks for an intersection there too, but a mark's edge and its clip's
edge are two unrelated boundaries far more often than they are one restated region, and
for unrelated boundaries the product is the estimator that assumes exactly that. Neither
estimate is the clause's area, and only a conflation-free rasteriser would be.

## What it costs, and what it does not buy

**Nothing measurable on the caller's corpus, in either direction.** 957 pages, their new
oracle, three configurations — product chain, `min` chain, `min` chain *and* `min` mark —
all read 915 agree, 37 differ, 5 refused, and not one page's mean, worst tile or SSIM
moves. `issue21346.pdf`, their witness, agrees with the oracle under all three: the
difference their rasteriser measures at its clip boundary is below what this gate reports.

So this is a decision taken from the clause with a measurement that declines to arbitrate
— which is the honest order, and is stated here so that nobody re-runs the corpus
expecting the change to show and reads a null as a mistake. What it buys is the property:
a page cannot lose its edges by restating a clip, and a chain's depth is no longer
something the picture depends on.

## Revisit when

A page is found where the mark-meets-clip site is the one that matters — the `W n`
followed by a fill of *the same non-rectangular path* is the case, since there the two
boundaries are one region and the product squares it. That wants its own fixture and its
own measurement; the corpus does not contain one that this gate can see. And if
conflation-free rasterisation is ever on the table, both sites become the same exact
answer and this ADR is superseded rather than amended.
