# 0080 — The compute lane exists, and flattening is its measured wall

Date: 2026-08-26. Status: **accepted, and built**. ADR 0079 chose the direction; this is
the first increment: `Coverage::Compute`, a third lane that runs `raster/fill.rs`'s exact
scanline arithmetic on the device. The code is `src/compute.rs` (the sheet and the
dispatch), `Place::Compute` through `encode/parallel.rs` and its commit, the routing in
`encode/fill.rs`, and the seam in `device/staging.rs`; the gate is
`tests/compute_lane.rs`.

## What was built

- **The shader is the port the determinism probe proved** (ADR 0079), lifted unchanged:
  one invocation per tile row, exact signed trapezoid deposits, the serial prefix sum,
  the CPU's own rounding — with the frame around it new. The encoder's parallel phase
  still flattens (and expands strokes' neighbours' — strokes themselves keep the CPU
  lane in this increment, as do rare paints and residue-clipped marks); what a compute
  job's worker no longer does is run the scanline pass. It hands back the closed edge
  list in tile space, shifted with `fill_mask`'s own subtraction so the shader's
  arithmetic starts from the same bits.
- **Routing**: under `Coverage::Compute`, every solid fill without a residue clip takes
  the lane — **no atlas in front of it**, which is the deliberate trade: a still page of
  repeated glyphs re-rasterises per frame, and in exchange nothing about the lane's cost
  depends on what a cache holds.
- **The bytes reach the sheet without a per-tile call**: texture → buffer, one dispatch,
  buffer → texture — the whole image twice, never a copy per tile (ADR 0078's lesson).
  Bytes are OR'd into the zero-seeded image with `atomicOr`, deterministic because every
  byte has exactly one writer. Mixed sheets work by construction: the CPU lane's tiles
  ride the seed copy.
- **Counted before allocated** (principle 3): edges, accumulator floats and row jobs are
  charged at each commit, the aligned image buffer at `finish`; the lane's `u32`
  counters are unreachable for any budget those charges pass. The pipeline compiles on
  the first frame that takes the lane — the startup path never pays (§7).

## The gate

`tests/compute_lane.rs`: a mosaic of abutting jittered quads, self-crossing stars under
both fill rules, and a stroke sharing the sheet, rendered under `Cpu` and `Compute` on
every adapter this machine has — **zero pixels of 65 536 differ, on RADV and on
llvmpipe**, and a replayed retained frame repeats its bytes. The comparison is honest
because the tiny-atlas trick keeps the CPU lane on the path lane, where both lanes
rasterise under the full device transform; a fill the atlas caches rasterises at a
quantised phase and may differ in the last unorm step, which is ADR 0009's doing, not
this lane's.

So the lane inherits the CPU rasteriser's two strongest properties at once: §10.7.4's
no-disappearance (exact area, no sample lattice, no thin-mark guard needed — the sampled
`Gpu` lane's recorded non-conformance does not apply here) and the oracle's byte
identity, on this machine's adapters.

## The measurement, and the wall it names

AMD Radeon 890M (RADV), release, `examples/zoom` (dense glyph page, 5 933 fills of 107
outlines) and the caller's worst page (58 009 unique fills, 3.0 M segments) through
their backend, fastest-of-N:

| | `Cpu` | `Compute` |
|---|---:|---:|
| dense text, still at 1× (atlas warm) | **1.07 ms** | 3.07 ms |
| dense text, zoom sweep worst frame's encode (every tile cold) | 8.84 ms | **3.56 ms** |
| dense text at 100× (past the atlas), encode | 3.56 ms | **0.41 ms** |
| the 58 009-fill page, cold frame | 272 ms | 290 ms |
| the 58 009-fill page, same transform again | **~90 ms** | ~265 ms |

Three readings, each the design's own prediction confirmed or bounded:

1. **Where the scanline pass was the cost, the device halves it or better** — the zoom
   gesture on text, and anything past the atlas's reach.
2. **A still page belongs to the atlas.** 1.07 against 3.07 ms is the cache's 20–60×
   argument (ADR 0029) surviving intact; the lane does not replace `Cpu` as a default,
   it exists beside it.
3. **On the worst page the lane moves nothing, and the reason is now a measurement:
   flattening is the wall.** The 58k-fill page spends its frame turning 3.0 M segments
   into polylines — CPU work in every lane this library has — and the scanline pass the
   dispatch absorbed was the smaller term. The next increment is therefore **GPU
   flattening from the resident outlines** (whose quadratic forms ADR 0075 already keeps
   resident), which is also what makes the per-frame edge upload (sixteen bytes per
   flattened point, ~48 MB on this page) disappear rather than merely batch.

## Costs, written down

One dispatch and two full-sheet copies per frame that takes the lane, inside the upload
phase and currently invisible to `Timings::execute` (the pass carries no timestamps yet);
per-frame edge extraction and upload, priced above and charged to the budget; `atomicOr`
per covered byte. A driver that refuses the shader says so through the uncaptured-error
channel, exactly as a warm-set pipeline's refusal does.

## Not done, deliberately

Strokes, rare paints and residue tiles keep the CPU lane (each is its own seam);
no hybrid policy that keeps the atlas for repeated glyphs while the lane takes the rest —
that policy should be designed against the flattening increment, not before it;
no persistent device-side edge buffers — they arrive with resident flattening, where they
stop being per-frame at all.
