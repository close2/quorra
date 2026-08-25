# 0078 — The atlas flush pays per call, and the sheet pays once

Date: 2026-08-25. Status: **accepted, and built**. The code is `crates/quorra-gpu/src/atlas.rs`
(the sheet, `DirtyRows`, `take_dirty`/`rows`/`mark_dirty`) and
`crates/quorra-gpu/src/device/staging.rs` (`flush_atlas_tiles`). **It moves no pixel**: every
readback gate passed unchanged before and after, which is the property the whole change hangs on —
the texels a span uploads are the texels the tiles held.

## Context

`flush_atlas_tiles` uploaded the frame's new glyph tiles one `queue.write_texture` at a time —
natural, because a tile is what the packer produces and a `write_texture` takes exactly one
rectangle. What that shape charges is wgpu's fixed per-call price — validation, a staging
allocation, a scheduled copy — once per tile, and the price is the adapter's to set.

On this machine's RADV it is small and was never noticed. On the caller's Windows machine it is
not: their trace of `tmp/Entwurf.pdf` (58 003 fills, essentially every one a distinct outline, so
a cold frame inserts a tile per fill) shows the transfer phase spending **6 409 ms moving
4 907 244 bytes** on DX12 — ~110 µs a call, into which the 85-byte average payload vanishes. The
same loop on RADV costs ~65 ms and on llvmpipe 71–92 ms (their `doc/todo/44` §6), which is what
kept it invisible here: **the defect is the call count, and only an adapter that prices calls
highly makes the count legible.**

## Decision

**The atlas keeps a CPU sheet of its own texels, and the flush uploads dirty row spans of it —
one `write_texture` per span, never one per tile.**

- `insert` writes the tile into the sheet (allocated on first insert, never at construction — the
  startup path pays nothing, §7) and marks its rows dirty. The transient per-tile `Vec` clone the
  old pending list took is gone with the list.
- Spans are full-width, disjoint, and coalesced when they touch. Full-width is what makes the
  upload a *borrowed slice* of the sheet at the sheet's own stride, with no repacking; the texels
  a span carries beyond its tiles are bytes the sheet already holds (zero, or earlier tiles
  restated over themselves), so the width costs bandwidth only — and bandwidth is not what the
  per-call price was made of.
- A cold frame appends shelves contiguously, so its spans merge to **one**: the 58 003-tile frame
  above becomes a single call moving ~5 MB, which is milliseconds on any adapter.
- `reset` zeroes the sheet and drops the spans with the layout they were packed for, exactly as it
  dropped the pending list.

**The cost, written down:** one atlas of bytes held CPU-side — 8 MiB at the default budget,
bounded by the same `2048 × max_dimension` cap as the texture it mirrors. It is the price of
being able to restate any row at any time, which is what lets a later insert into a shared shelf
upload that shelf's rows without knowing which earlier tiles sit beside it.

## The measurement

`cargo run --release -p quorra-gpu --example zoom`, AMD Radeon 890M (RADV, Vulkan),
`Coverage::Cpu`, the dense glyph page — the sweep row is the one this change is about, since a
sweep's every frame rasterises every visible tile cold:

| | before | after |
|---|---:|---:|
| sweep 1×→20×, worst frame's upload | **22.980 ms** | **9.816 ms** |
| held 1×, upload | 0.015 ms | 0.009 ms |
| held 1×, whole wall | 1.129 ms | 1.069 ms |

RADV is the adapter on which this matters *least* — its per-call price is why the loop survived 77
ADRs. The caller's DX12 number is the one the change is for, and it is theirs to confirm: ~58 000
calls at ~110 µs became a handful of spans.

## What this does not change

The retained-encode contract is untouched: a replayed frame encodes nothing, inserts nothing, and
owes no spans, exactly as it owed no pending tiles. Determinism is untouched: the sheet's bytes
are a pure function of the insertions, and unwritten texels are zero on both sides of the change —
the same bytes wgpu zero-initialised the texture to.
