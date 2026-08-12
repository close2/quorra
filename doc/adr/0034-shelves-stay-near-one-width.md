# ADR 0034 — Shelves stay near one width, so the sheet stays near square

Status: accepted, 2026-08-12. Takes the item ADR 0021 recorded as what it did not fix.

## Context

ADR 0021 narrowed the scratch sheet to the width its shelves reached and wrote down what
was left:

> What is left is the sheet's *height* — gaps between shelves that narrowing cannot reach
> — and that is a packer question with its own measurement.

Here is the measurement. `issue16287.pdf` at 4× packs six tiles of roughly 2 000 × 500 and
commits a sheet of **6 026 × 2 406 for 6.93 M texels of tiles — 48 % used**. The layout
says why: the first shelf takes two tiles and grows to 6 026, and the next three take one
tile each because none of the remaining tiles fits the open shelf's height rule. A sheet is
a rectangle, so those three shelves each pay 3 900 empty columns.

`Test-plusminus.pdf` at 4× is the same shape at 29 tiles: 9 298 × 6 300 for 29 M texels,
**50 % used**.

## Decision

**A shelf may grow to `√(2 × placed area)` rather than to the packing width**, floored at
the widest tile seen and capped at the device dimension. Past that, the packer opens a new
shelf instead of extending the open one.

`√(2A)` is the side of the square a shelf packing of area `A` fills at the efficiency
shelves actually achieve, so it is the width that minimises `w × h` for the sheet as a
whole rather than for the shelf in front of it. It is computed from what has been placed,
which makes it grow with the frame — and a shelf packed under an earlier, smaller target is
never invalidated by that, because a cursor that stopped growing only leaves room a later
tile may still take.

The same six tiles now commit **2 224 × 3 293 — 95 % used**, and the 29 become 72 %.

## What it buys, and what it does not

**Bytes, which is the currency refusals are counted in.** The sheet is charged against
`max_frame_bytes` (ADR 0021 made the gaps charged too, which is what made this visible),
and it is bounded on each side by the adapter's dimension — so halving a page's sheet both
frees budget and postpones `ScratchExhausted`, which is three of the caller's twelve
refusals at 4×.

**Not time, measured.** `Test-plusminus.pdf` at 4× — the page whose sheet falls from 58.6 M
texels to 40.4 M — takes 344, 344 and 380 ms with this change and 291, 331 and 372 without
it, three runs each. The whole sheet is uploaded every frame, so 18 M texels of upload
disappear and the clock does not notice: the page's time is elsewhere. An optimisation with
no measured time is worth stating as exactly that.

**And no page of the corpus changes verdict.** 915 agree / 37 differ / 5 refused at scale 1
and 925 / 16 / 11 at scale 4, both identical to the packing this replaces, with the same
pages refusing for the same reasons. `issue16287.pdf` still refuses 4 % over the frame
budget, because its 279 MB is layer textures rather than sheet: the packing frees 7 MB of a
shortfall of 11.

## Revisit when

A page is found whose refusal the packing decides — the shape to look for is many tiles of
*varied* height, where shelf packing is weakest and this heuristic is only a partial
answer. The full answer is to place tiles by size rather than in encounter order, which
needs the positions to be assigned after the walk rather than during it; the quads and the
GPU lane's triangles both record sheet coordinates as they are packed, so that is a
two-pass encode and a larger change than this one. It is worth pricing the day a page needs
it and not before.
