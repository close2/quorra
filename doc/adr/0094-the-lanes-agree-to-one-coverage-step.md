# 0094 — The lanes agree to one coverage step, and that is now the statement

Date: 2026-08-27. Status: **accepted** — ADR 0093's chartered follow-up, resolved by
*stating* the divergence under the relaxed contract (ADR 0082) rather than chasing
it, with the diagnosis that says why chasing would be wrong.

## The diagnosis

ADR 0093's gate found pure `Coverage::Cpu` and `Coverage::Compute` frames differing
on a fixture of shallow near-horizontal edges laid on near-integer rows — a shape the
zero-pixel suites' fixtures never produced. Pixel inspection shows the whole
difference is **one alpha step**: a ramp of coverages stepping 25/255 per pixel lands
each value on a byte boundary, the scanline rounds one way and the deposit the other,
consistently along the run (and a `0` against a `1` at the run's tail, whose
straight-alpha un-premultiply then reads as a large *colour* delta over the same
one-step ink difference).

Both conversions use round-half-up (`(cov * 255.0).round()` against
`floor(cov * 255.0 + 0.5)`), and the deposit's arithmetic mirrors `raster/fill.rs`
statement for statement — but not in the same **order**. The scanline accumulates per
tile; the deposit accumulates slabs along a sheet row. Float addition is not
associative, so a true coverage within an ulp of a boundary legitimately rounds
apart. That is a property of summation order, not a slip in either mirror; making the
orders identical would mean making the two lanes one implementation, which is exactly
what the cross-lane comparison exists to avoid.

## The statement, and its gate

Where the lanes disagree at all, they disagree by **at most one coverage step**, at
any scale — measured across 0.6× to 2.3× at 5–40 pixels per 128² frame, worst alpha
delta 1 at every scale. `tests/lane_bound.rs` is the statement's gate: a divergence
past one step is a defect, not more of the same. The existing zero-pixel suites keep
their exact assertions — their fixtures genuinely produce byte-identical frames, and
staying exact keeps them the sharpest canary for real regressions.

Colour is gated *through* alpha: straight-alpha output divides premultiplied ink by
the pixel's own alpha, so a one-step alpha difference on a nearly transparent pixel
amplifies into tens of colour levels describing the same ink. The gate therefore
holds colour equal wherever alpha is equal, and bounds the ink where it is not.
