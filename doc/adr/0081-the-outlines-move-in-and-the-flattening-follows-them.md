# 0081 — The outlines move in, and the flattening follows them

Date: 2026-08-26. Status: **accepted, and built**. ADR 0080 named its own successor —
"GPU flattening from the resident outlines" — and this is it, hours later, because the
wall it named was measured rather than suspected. The code is `src/compute.rs`
rewritten around a [`SegmentArena`] and a three-pass chain; the encoder's side shrank
to a record (`encode/fill.rs::fill_compute`), and ADR 0080's job plumbing through the
parallel seam came back out — the lane no longer needs the walk's threads at all.

## What changed

Under `Coverage::Compute` the encoder now records, per solid fill, **a tile record and
nothing else**: seat, rectangle, transform, rule, outline id. No flattening, no edge
extraction, no per-frame geometry on the host. The device:

- keeps every outline's segments **resident in one arena buffer**, uploaded on the
  first frame that draws it and addressed by range ever after — grown by doubling with
  a device-side copy, entries dropped when their outline is released (the id may be
  reissued; the words become counted `holes`), the growth charged to the frame that
  caused it;
- runs **count → emit → deposit**: one invocation per tile walks its segments under
  its transform with `raster/flatten.rs`'s own arithmetic — the `a·x + c·y + e`
  transform, exact midpoint halving made iterative (right half pushed first, so the
  emission order is the recursion's), the flatness cross-products, the degenerate-chord
  epsilon branch, the depth cap — first counting the edges, then, after an exact
  allocation, writing them; the deposit pass is ADR 0080's scanline shader unchanged;
- pays **one readback per frame** for the counts, which is what "count then allocate"
  costs when the counter is the device: the edge buffer is allocated exactly and
  checked against `max_frame_bytes` before it exists, and a magnification that makes
  more geometry than the budget holds is refused by name (principle 6), not guessed at.

**The one stated divergence from the CPU's arithmetic**: `cubic_tolerance` takes
`√(w² + h²)` where `flatten.rs` takes `f32::hypot`, which WGSL does not have. The two
differ by at most an ulp of the diagonal; it can matter only for a cubic whose flatness
test lands inside that ulp of its boundary; and the gate below holds whole frames
byte-equal over fixtures built to walk every flattening branch — wavy cubics, a loop
whose chord is a point, a sub-pixel curve in the relative-tolerance regime. If a
fixture ever finds the boundary, the resolution is ADR 0077's: share the arithmetic,
by its own ADR.

## The gate

`tests/compute_lane.rs`, now with curves: **zero pixels differ against the CPU lane on
RADV and on llvmpipe**, whole frames, both fill rules, a stroke sharing the sheet, and
a replayed retained frame repeats its bytes. The tile is now the control hull's rather
than the flattened geometry's — a superset whose extra pixels read zero coverage —
which moves seats and sheet area and not one composited pixel, and the gate is what
says so.

## The measurement

AMD Radeon 890M (RADV), release, same instruments as ADR 0080; "zoom step" means every
cached picture is useless, which is the gesture the lane exists for:

| | `Cpu` | ADR 0080 | this |
|---|---:|---:|---:|
| dense text, zoom sweep worst frame's encode | 8.84 ms | 3.56 ms | **0.93 ms** |
| dense text at 100×, encode | 3.56 ms | 0.41 ms | **0.24 ms** |
| dense text, still at 1× (atlas warm), wall | **1.07 ms** | 3.07 ms | 2.10 ms |
| 58 009-fill page, frame at a new magnification | ~272 ms | ~290 ms | **~150 ms** |
| 58 009-fill page, still (atlas warm), wall | **~90 ms** | ~265 ms | ~140 ms |

Three readings:

1. **The wall ADR 0080 named is down.** The worst page's zoom step nearly halves
   against the CPU lane, and the dense page's cold sweep is now under a millisecond of
   encode — flattening has left the host entirely, and with it the per-frame edge
   upload (the ~48 MB became an ~84 MB arena paid once and ~3.7 MB of tile records per
   frame).
2. **The atlas keeps its home turf**, still: a page standing still is cheapest under
   the cache, on both page shapes. The lane is for motion; the hybrid policy that
   takes each regime's winner is a design for the round that has both numbers, which
   is now this one's successor.
3. **The next wall is the walk.** ~90–100 ms of the worst page's ~150 is `recording` —
   clip resolution, culling, instance building, one command at a time on the host —
   which is ADR 0368's floor arriving on schedule. Moving *that* is the retained-scene
   question (tile records that survive a transform change), and it should be designed
   with the caller's reprojection architecture in view rather than under it.

## Costs, written down

The arena: resident device memory equal to every drawn outline's segments (~84 MB for
the worst page's 3.0 M segments), counted, with `holes` counting what release leaves
behind until compaction is worth measuring. The readback: one submit boundary and a
blocking map per frame that takes the lane — a fence-and-retry replacement is named,
unbuilt. Two whole-sheet copies per frame and `atomicOr` per covered byte, as before.
The count and emit passes each walk the segments once, so flattening arithmetic runs
twice per frame on the device — cheaper than one host walk by the table above, and
collapsible later by persisting per-tile counts across frames whose transform is
unchanged.
