# 0083 — The walk stops touching the control points

Date: 2026-08-27. Status: **accepted, and built**. ADR 0081 named the walk as the
compute lane's next wall; this is the wall measured, and its two largest terms removed.
The code is `resources.rs` (the resident control box), `encode/fill.rs`
(`corner_bounds`), and `src/compute.rs` (the deposit pass searches, the row-job table
is gone).

## The measurement that chose the targets

The caller's worst page under `Coverage::Compute`, alternating two transforms so every
frame re-encodes (their phase probe, the 890M): steady frames of ~104–160 ms split as
**encode 45–60**, upload ~25, scene ~8–13, readback ~10, the rest the device's. Inside
that encode, the largest term by inspection was `hulls.bounds` — the per-placement hull
walks every control point, and its memo (ADR 0045) keys on `(outline, linear part)`,
which on a page of **58 009 distinct outlines** hits never and turns into pure
overhead: three million point transforms and 58 009 memo inserts per frame. The second
term was the upload's row-job table: one `u32` per tile row, ~520 000 of them built and
uploaded per frame so the deposit pass could name its tile.

## The two changes

- **The control box becomes resident** (`StoredOutline::control_box`): min/max over
  every control point, taken at upload inside the same walk that already validates
  every point — so it costs nothing new, ever. The compute lane bounds a placement by
  transforming the box's four corners. For an axis-preserving transform this is the
  direct hull **to the bit** — `hull.rs`'s own monotonicity argument: correctly rounded
  `a·x + e` is monotone in `x`, so the extreme of the transformed points is the
  transformed extreme. Under rotation it is a superset, whose extra pixels read zero
  coverage and move nothing but tile extents — inside ADR 0082's contract, and the
  byte gates below did not move at all. The CPU and sampled lanes keep the exact hull:
  their tile extents are load-bearing in ways this lane's are not (the atlas keys and
  admits by them).
- **The deposit pass finds its tile by binary search** over the records' `row_start`,
  which is ascending by construction: sixteen probes on a 58k-tile frame, against a
  host-built, host-uploaded, budget-charged table of one word per row. The table, its
  build loop, its buffer and its charge are deleted.

## Measured

Same probe, same page, same adapter, steady frames:

| | before | after |
|---|---:|---:|
| encode | 45–60 ms | **16–20 ms** |
| whole frame at a new magnification | ~104–160 ms | **~85–110 ms** |

The session's whole arc for this page's zoom step: ~272 ms (`Cpu`, where this began) →
~150 (ADR 0081) → **~85–110**. The gates hold: zero pixels against the CPU lane on
RADV and llvmpipe (`tests/compute_lane.rs`, curves included), zero pixels across three
adapters in the shader-level probe, the workspace suite green.

## What remains, named with numbers

Upload ~22–25 ms (the record build, the arena's 58k probes, the buffer writes and two
submissions), encode ~16–20 (the walk's remaining per-fill costs: dispatch, clip
resolution, charges, seats, instances), and ~25 unattributed to a phase (the count
pass's stall and the device's own work, still untimestamped). The next order-of-
magnitude step is not a term but the walk itself — retained tile records replayed
under a new affine — and it should be taken with the caller's reprojection
architecture in view, as ADR 0700 already notes.
