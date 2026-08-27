# 0086 — The collapsed fill's marks are the encode's

Date: 2026-08-27. Status: **accepted, and built** — the second boundary amendment on
ADR 0084's road, after ADR 0085's stroke width, and asked for by a measurement on the
caller's side: their ADR 0702 built page-space scenes whose survival across viewports
turns on nothing in the scene reading the view, and on their worst page ~20 of 58 009
fills read it — hairline rulings whose §10.7.4 marks were split out *before* the scene,
sized and floored on one placement's pixel grid. Identically at 0.55× and at 2.4×, so
no zoom outruns them: the split's placement had to move to where the placement is
known, or that page could never keep a scene.

## What changed

A fill whose subpath encloses no area — extent exactly zero along exactly one axis —
now deposits ISO 32000-2 §10.7.4's mark, and the *encode* places it, per viewport:

> A shape shall be scan-converted by painting any pixel whose half-open square region
> intersects the shape, no matter how small the intersection is. This ensures that no
> shape ever disappears as a result of unfavourable placement relative to the device
> pixel grid […] A zero-width or zero-height rectangle paints a line 1 pixel wide.
> (ISO 32000-2 §10.7.4)

Three parts, each where its inputs are:

- **Upload** finds the collapsed subpaths once (`resources.rs`, `collapsed_marks`),
  because whether a subpath collapses is a property of its control points and of
  nothing else — the same walk and the same exact-equality test as the caller's
  `subpath_extents` and `Extent::collapse`, over control points as well as endpoints
  (a cubic lies in its control hull). The table rides `StoredOutline` beside ADR
  0083's `control_box`, priced with the segments, empty for almost every outline.
- **Encode** places each mark under the composed placement (`encode/fill.rs`,
  `encode_collapsed_marks`): under an axis-preserving invertible placement, the run
  of whole device pixels the collapsed axis passes through — `floor(v)` to
  `floor(v) + 1`, the other axis keeping the subpath's own extent — as an
  axis-aligned device rectangle, which takes ADR 0007's analytic lane where a solid
  rectangle would and a coverage quad where it cannot (a residue clip, a rare
  paint). Otherwise, the band: one device pixel (`1 / max_stretch`) about the
  subpath's own line, through the coverage lane. The arithmetic mirrors the caller's
  `pdf_render::collapsed` statement for statement, as ADR 0085's width resolution
  mirrors their `device_width` — with one stated difference: this side emits the
  snapped rectangle directly in device space, where the caller's split must
  round-trip it through the placement's inverse because its output is path geometry.
  The difference is ulps on coordinates that are exact integers here, inside ADR
  0082's contract.
- **The fill itself is untouched.** A collapsed subpath contributes zero winding
  under either rule, so filling the original path draws exactly what filling the
  caller's split remainder drew — which is what lets a caller upload the *original*
  outline and keep its pointer-identity cache key.

No API changed. The rule is unconditional for `Command::Fill`, solid and rare paints
alike (the clause names the shape, not the ink); `Command::Rect` — this library's own
vocabulary, which no document emits — is not touched.

## Why the encode, and not a flag, and not the caller

The caller's pre-split was correct at exactly one placement, which was fine while
their scenes were placement-baked and fatal the day they stopped being. A flag on the
fill ("apply §10.7.4 here") was considered and dropped: the rule is the standard's
statement about scan conversion itself, every real caller would set it on every fill,
and a caller who genuinely wants a degenerate subpath to vanish is a caller this
library has never had. Adopting the clause outright makes every lane §10.7.4-conformant
for zero-area subpaths — which the lanes were not: an exact scanline computes zero
coverage for them, and until now a hand-built scene with a `10159 0 re`-shaped outline
drew that ruling on the caller's backends and nothing here.

## What it cost, and where the seams are

Two tests stated the old behaviour and were amended, not deleted: `no_ink.rs`'s
zero-area fill is now *diagonal* collinear points — outside this table's test exactly
as it is outside the caller's `collapsed.rs`, whose module comment names the diagonal
case as the stated absence — and `zero_extents.rs`'s flat path now asserts the mark
*and* still holds what it always held, that the degenerate coverage tile underneath
charges nothing and stops no frame. The pixel gates are `tests/collapsed_fills.rs`:
the clause's own cases (the floored row, the boundary row, the quarter turn, the
shear's band, the sliver left to its coverage, the point left to §8.5.3.3.1), plus
the one this table exists for — **one scene, three viewports, three rows**.

## Held by

The full suite (620), clippy pedantic clean. The caller's side — deleting their
split at the quorra boundary and un-marking collapsed fills as view-consuming — is
their ADR 0703, behind the pin as always.
