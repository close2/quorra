# 0096 — Flatten-from-quadratics, built, measured, and declined

Date: 2026-09-01. Status: **accepted — the third and last of ADR 0091's kernel
experiments, built whole and reverted on its numbers.** The caller's
`doc/todo/46-the-kernel-floor.md` named this the only untried structural idea with
the zoom step's magnitude; it was built honestly, held green through the whole
suite, and the step did not move. The numbers are the deliverable; the code is
deliberately not, exactly as ADR 0092 put it — and this file is what keeps the next
round from re-buying the idea on the same hope.

## What was built

Everything the idea asks for, end to end, in `compute.rs`:

- **The arena carried the quadratic form.** `encode_segments` converted §8.5.2.2's
  cubics through `outline::push_cubic` — the GPU triangle lane's own conversion,
  within `QUAD_TOLERANCE` (1e-4) of the outline's extent — so the two device lanes
  carried one geometry. Tags: 0 move, 1 line, 2 quad (5 words), 3 close, and later
  4 (see the tolerance lesson below).
- **The kernel flattened stackless.** A quadratic's second derivative is the
  constant `2·(p0 − 2c + p2)`, so its chord count is closed-form:
  `n = ⌈½·√(|p0 − 2c + p2| / tol)⌉` meets the tolerance that the cubic recursion
  met by splitting — no per-thread stack where `flatten_cubic` held ~170 scalars of
  function-scope arrays (ADR 0091's occupancy suspicion), and the count pass added
  `n` without evaluating a single point.
- **The contract was restated, not skipped.** Under ADR 0082 the Cpu↔Compute
  comparison became a derived band — each lane's polyline within §10.7.2's 0.25 px
  of the curve it flattens, the two curves within `QUAD_TOLERANCE · extent ·
  stretch` of each other, so divergence confined to a boundary's own neighbourhood
  and one summation step elsewhere. A shared `tests/common/band.rs` gated it;
  `compute_lane.rs` kept byte-for-byte on a lines-only mosaic; the whole 632-test
  suite was green on both adapters before anything was measured.

## The numbers (890M, headless, `tmp/Entwurf.pdf` p1, 58 010 commands, ADR 0767's own sequence, arms interleaved in one sitting, load < 1.1)

| arm | warm step | count pass | emit+deposit | first compute frame | accounted MB |
|---|---|---|---|---|---|
| cubic kernel (HEAD) | **62.0–66.7** | 13.2–19.9 | 27.8–29.9 | 341–343 | 177 |
| quads, per-quad tolerance | 65.9–66.1 | **9.7–9.9** | 35.7–36.6 | 454–476 | 229 |
| quads, per-cubic tolerance | **62.5–63.7** | 12.4–13.0 | 29.8–30.5 | 444–450 | 184 |

And one split taken by temporarily ending the coverage query at the emit pass, on
the final quad arm: **emit alone 16.4–16.8 ms, deposit ≈ 13.5, count 12.5**.

## What the numbers say

- **Occupancy was not the wall.** Removing the whole 170-scalar stack — not
  shrinking it, removing it, with the count pass reduced to closed-form arithmetic —
  moved the count pass 30–45% and the *step* not at all. The passes are bound by
  the arena walk (one thread per tile streaming ~85 MB of segments, serially,
  twice) and by edge traffic (emit writes ~64 MB of edges; deposit re-reads them
  per row), not by register pressure. ADR 0091's suspicion is hereby measured and
  refuted; its two design debts stay open and stay listed there.
- **The tolerance is the cubic's, not each quad's — the round's one transferable
  lesson.** `raster/flatten.rs` reads ADR 0044's relative flatness term off the
  whole curve's control polygon. Read per quad, the polygon is a fraction of its
  cubic's, the tolerance tightens by roughly the square root of the split, and the
  frame paid ~1.7× the edges and +28% of emit+deposit — the middle row above. The
  fix (a tag-4 header carrying the cubic's outline-space diagonal, scaled by the
  transform's larger singular value in the kernel) brought the edges back and cost
  the count pass ~2.6 ms of extra arena reads, which is its own small proof of
  where the time goes.
- **What the change would have cost to keep**: +100–130 ms on the first compute
  frame of a worst page (the cubic→quad conversion at residency plus a third more
  arena to upload), +4% resident arena, and the byte-for-byte Cpu↔Compute
  contract — the sharpest canary both projects' gates own — traded for a band, all
  for a step change inside the run-to-run noise.

## Decision

Reverted, whole. The compute lane keeps `raster/flatten.rs`'s cubic recursion,
statement for statement, and `tests/compute_lane.rs` keeps holding whole frames
byte-equal. What remains of the kernel-floor arc after three measured declines
(ADR 0092's two and this): the floor is the **serial per-tile arena walk and the
edge traffic**, so the ideas with the step's magnitude are structural — fewer
bytes walked (a packed arena format), more parallelism per tile (segments across
threads with a device-side scan), or fewer edges deposited (tile-local runs) — and
each is a design of its own, not an evening's experiment. The caller's
`doc/todo/46` carries the re-pricing.
