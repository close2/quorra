# ADR 0027 — The winding target holds a band, and the lane's crossover is measured

Status: accepted, 2026-08-12. Takes the piece ADR 0026 recorded as the next work on this
lane, and re-derives that ADR's criterion against the measurement banding made possible.

## Context

ADR 0026 stopped the GPU coverage lane from taking tiny glyphs, and named what was left:
a page of large shapes still refused, because the winding target was sized from the whole
scratch sheet at eight bytes a texel. Sixty shapes of 500 pixels asked for 359 MB against
a 256 MiB budget and could not be drawn — on a lane whose entire purpose is large shapes.

## Decision

### 1. The target holds one band of the sheet at a time

The winding target is **scratch**: triangles accumulate into it, `fs_resolve` turns it
into the R8 sheet, and it is dead. Nothing needs it to hold the whole sheet at once. So
the sheet's rows are cut into bands — greedy over tiles sorted by their top row, never
splitting a tile, aiming at [`BAND_BYTES`] (16 MiB) — and each band is accumulated and
resolved before the next begins.

Both stages learn the band, and that is the delicate part of this change, because the
same agreement in a different form is the caller's §11 defect: `vs_winding` subtracts the
band's first row before mapping to clip space, and `fs_resolve` subtracts it again when
it reads the target back. One without the other draws the right shape in the wrong place.

**The vertices are not sorted.** Each band draws every triangle in the frame and the
shader maps the ones outside the band out of clip space. That costs vertex work and saves
permuting the largest buffer in the frame; the *tiles* are sorted, so a band's resolve is
one draw of a contiguous instance range.

### 2. The lane's crossover is a measured constant, not a cost comparison

ADR 0026 compared two byte counts — coverage against triangles — and refused to state a
constant. Banding let both lanes be measured on the same pages, and the comparison was
wrong in magnitude: at 200 and 500 pixels the GPU lane was two to four times *slower*
overall, because its winding traffic is proportional to the sheet and its per-tile
overheads only amortise once a tile is big.

RADV, sixteen samples, whole frames, cold devices:

| tile | texels | CPU lane | GPU lane |
|---|---|---|---|
| 200 × 260 | 52 000 | **73 ms** | 275 ms |
| 500 × 650 | 325 000 | **71 ms** | 146 ms |
| 700 × 910 | 637 000 | 97 ms | **26 ms** |
| 900 × 1170 | 1 053 000 | 113 ms | **20 ms** |

So `GPU_LANE_MIN_AREA` is half a megapixel, inside the bracket those rows leave, and the
triangle comparison of ADR 0026 stays as a floor beneath it. The trade is CPU *time*
against device *bandwidth*, and no count of bytes expresses it — which is why this ADR
states a constant where its predecessor would not.

## What it buys

Pages that could not be drawn now draw, and the lane is taken where it wins:

| page | before | after |
|---|---|---|
| 60 shapes of 500 px | **refused**, 359 MB | drawn — and on the CPU lane, which is faster for it |
| 40 shapes of 700 px | CPU 97 ms | **GPU 26 ms** |
| 24 shapes of 900 px | CPU 113 ms | **GPU 20 ms** |
| a page of small glyphs | CPU (ADR 0026) | unchanged |

## What it does not fix

**A page of very large shapes can still refuse under `Coverage::Gpu` where `Cpu` draws
it.** Thirty shapes of 1 200 pixels want 309 MB: the bands are bounded, but a band spans
the *sheet's* width, which both lanes share, and thirty tiles of 1 560 rows make a sheet
no band budget can help. The honest fix is for the target to span the GPU lane's own
tiles rather than the sheet's full width, and it is not attempted here.

`tests/coverage_lanes.rs` needed its fixtures magnified for this ADR — at 48 pixels every
comparison in it would now be the CPU lane against itself, passing without testing
anything. They are drawn at 16× through the viewport, which left all three stated
lane-difference bounds intact: exact agreement where no edge crosses, 32 of 255 for a
straight edge, 96 for a curved one, unchanged at the new scale.

## Revisit when

The width question above is taken, which changes what a band costs and so may move the
crossover again. Re-derive the table rather than keeping the constant: it is a
measurement, and it has already moved once.
