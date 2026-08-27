# 0089 — The image filter and its reduction resolve at the encode

Date: 2026-08-27. Status: **accepted, and built** — the §4.5 amendment pattern a
third time, after the stroke width (ADR 0085) and the collapsed fill (ADR 0086), and
for the third time because a scene that carries a placement-resolved answer is true
at exactly one placement: any page with a picture on it could never keep its
page-space scene across a zoom (the caller's ADR 0702 ledger named images first among
the view-consumers that remained).

## What changed

[`ImageFilter`] gains `Auto { interpolate }`: the image command crosses the boundary
as its own samples plus ISO 32000-2 §8.9.5.3's `/Interpolate` flag, and the encode
resolves, per placement:

- **the filter**, by the caller's own rule mirrored statement for statement
  (`raster/reduce.rs::smoothed`): the flag always filters; a magnified image without
  it draws flat rectangles; a reduced one keeps the filter on;
- **the reduction**, the caller's documented area-averaging departure from §10.7.4
  (their ADR 0025): where a device pixel gathers two or more samples, the encode
  names integer factors (`reduce::reduction`, mirroring their `factor` and
  `Reduction`) and the op draws a resident **reduced variant** the device realises
  once per `(image, factors)` and keeps for its life — `reduce::area_averaged`
  mirrors their premultiplied sums, proportional band boundaries and round-to-nearest
  to the byte; the arithmetic is integer throughout, so "mirrored" here means
  *byte-identical*, not close.

`Nearest` and `Linear` behave exactly as before; the presenter's layer filter maps
`Auto` to a smoothed tap, since a presented layer has no document placement to
resolve against. What is deliberately not mirrored is the caller's rayon split: a
reduction runs once per `(image, factors)` for the device's life here, against once
per placement change upstream, so a 2700×3450 photograph pays its ~20 ms once, on the
first frame that minifies it past a new integer factor.

## Held by

`tests/auto_image_filter.rs` — the minified checkerboard is the blocks' mean, the
magnified one is flat rectangles, and **one scene resolves per viewport**, which is
the property the variant exists for — beside `raster/reduce.rs`'s unit gates on the
mirrored arithmetic (the mean at the byte, the filter rule at both ends, transparency
carrying no colour). The full suite (629), clippy pedantic clean; the caller's sixty
`render-quorra` oracle gates pass against the mirror unmodified, which is the
cross-tree byte-fidelity check. Record replay composes: an image command is a `Slow`
record (ADR 0087), so a replayed frame re-resolves the filter and the factors under
its own viewport.
