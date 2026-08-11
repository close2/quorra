# ADR 0021 — The scratch sheet is as wide as it is used, and is charged for what it is

Status: accepted, 2026-08-11. Takes the item `doc/PLAN.md` recorded as "the next thing
to do here" when ADR 0016's winding-texture pricing was fixed.

## Context

Coverage tiles — path fills too large for the atlas, clip residues, strokes, and the
GPU lane's winding tiles — are packed onto one R8 sheet by a shelf packer, and the sheet
is created and uploaded once a frame.

**The packing width is the device's maximum dimension on purpose.** A narrower one
refuses real pages whose coverage is well inside the byte budget; 2 048 did exactly that
and the caller reported it (their feedback §3). The comment at the packer's construction
says the dimension is "capacity, not commitment".

`finish` made it commitment. It produced a sheet of the *packing* width, padded the
staging buffer to it, and `upload_scratch` created and uploaded a texture that size. On
this machine that is 16 384 texels a row, so:

- a page with one 180-pixel tile allocated and moved **2.95 MB** to carry 32 KB;
- the GPU coverage lane, whose winding target takes its extent from the same sheet at
  eight bytes a texel (ADR 0016), paid **23.6 MB** for the same tile;
- five real corpus pages moved 34, 39, 69, 81 and 143 MB of R8 per frame for between
  28 KB and 99 MB of tile area.

And the frame budget did not see any of it: tiles were charged their own area, so the
largest scene-derived allocation a page of path work makes was the one number nobody
counted — the reverse of what principle 3 asks.

## Decision

**Narrow at `finish` to the widest shelf cursor.** Every tile is placed left of that
cursor, so the region kept is exactly the region written and no tile moves: the
coordinates both lanes recorded stay valid. The written rows are restrided in place,
which is a copy of the used area — cheaper than uploading the padding it removes.
Capacity is unchanged: the packer still places tiles across the full device dimension,
so §3's refusal does not come back.

**Charge the sheet, once, when its extent is known.** Tiles keep their running charge —
they are the CPU-side staging that is really allocated, and a runaway page must be
refused before it materialises — and the difference between the sheet's own bytes and
what the tiles already paid is charged at the end, beside the winding texture's. Shelf
gaps are allocated bytes and are now priced as such.

## What it buys, measured

Best of nine, release, RADV, 1191×1684 `Readback`, closed curves past `MAX_GLYPH_DIM`
so their coverage lands on the sheet:

| fixture | before | after |
|---|---|---|
| GPU lane, 8 blobs of 80 px | 10.54 MB, **3.00 ms** | 0.46 MB, **1.96 ms** |
| GPU lane, 40 blobs of 80 px | 10.74 MB, 3.48 ms | 2.30 MB, 2.87 ms |
| GPU lane, 40 blobs of 180 px | 43.50 MB, 5.00 ms | 15.14 MB, 4.56 ms |
| CPU lane, 40 blobs of 180 px | 4.33 MB, 7.16 ms | 1.49 MB, 6.73 ms |

The first row is the one that matters most: a page with a little path content is the
shape of the caller's median page, and it is a third of the frame. The saving is
allocation and bandwidth, not shader time — `Timings::execute` is unmoved on these
fixtures, which is what says where the cost was.

**On the caller's 957-page corpus gate, no page changed verdict** (914 agree, 35 differ,
8 refused, 17 not comparable, before and after). The wall clocks in that run are too
noisy on this machine to attribute a percentage to, and are not quoted here for that
reason; the deterministic counter is `Counters::bytes_uploaded` above.

## What it does not fix

**The sheet's height is still the sum of the shelves.** A page whose tiles are a few
tall shapes leaves gaps between shelves that narrowing cannot reach; a better packer
(or a second sheet) is a different change with a different measurement.

**The sheet is still one texture for the whole frame.** Damage patching scissors the
passes that read it, not the upload that fills it.

## Revisit when

A page's `bytes_uploaded` is large next to its tile area again. The instrument is in the
counter, and `tests/scratch_sheet.rs` holds the ratio against the device dimension so a
regression fails rather than being reported.
