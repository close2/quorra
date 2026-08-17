# ADR 0064 — `Options::coverage` reaches a solid paint and no other, and that is now priced

Date: 2026-08-17. Status: accepted. Closes an open question recorded on `Coverage` since
2026-08-16 by **measuring it and leaving it**, which is a decision and so has a cost to
write down.

## Context

`Options::coverage` chooses who rasterises a mark's coverage — ADR 0008's CPU scanline
rasteriser or ADR 0016's device winding lane — and ADR 0026 made that a per-mark choice by
what each would cost. `Encoder::take_gpu_lane` is consulted in exactly two places,
`encode/fill.rs`'s `fill_solid` and `encode/coverage.rs`'s `push_coverage_styled`, and
**both are the solid arm**. A shading, an image, a mesh or a §7.10.5 function paint reaches
the scratch sheet through `encode/rare.rs`'s `push_rare_coverage`, which calls
`coverage_tile` directly and never asks. So `Coverage::Gpu` draws every rare paint exactly
as `Coverage::Cpu` does.

Half of that was closed on 2026-08-16: the public rustdoc was made to *say* it, because a
claim in public API that is not true is the defect principle 6 names. The other half was
left open on purpose — "an open question with a measurement in front of it, not a decision
that has been taken".

§11.2's census (`doc/notes-census.md`, 2026-08-17) is what made it worth asking. The path
lane is 9.4 % of a page's marks but **at least 66 % of the coverage a frame rasterises at
1× and 76 % at 4×**: a tenth of the marks, two thirds of the bill. If a meaningful share of
that bill belonged to rare paints, the omission would be costing real work.

## The measurement

`doc/notes-rare-lane.md` has the instrument, the four configurations and the reconciliation.
Every coverage tile the caller's 974-document corpus seats was split by the site that seated
it; the six buckets sum to `Counters::coverage.texels` exactly on all 3 897 frames.

**Of the coverage the sheet rasterises, the part that is a rare paint's *and* is eligible to
move is 0.110 % at scale 1 and 0.629 % at 4×.**

| | scale 1 | scale 4 |
|---|---:|---:|
| all rare-painted coverage | 0.98 % of the sheet | 4.70 % |
| …of which under a residue clip (the CPU lane under either setting already) | 82 % | 83 % |
| …of which ADR 0026 keeps on the CPU anyway | 6 % | 4 % |
| **eligible** | **0.110 %**, 28 marks | **0.629 %**, 88 marks |
| eligible, as a share of what the CPU still rasterises under `Coverage::Gpu` | 1.68 % | 4.22 % |

Twenty-five pages of 954 draw a rare-painted coverage tile at all; nine draw an eligible one
at scale 1 and fourteen at 4×. The largest is 0.18 % of the corpus's coverage. For contrast,
one page with no rare paint anywhere on it — `issue1905.pdf` — is 68 % of it.

## Two findings that decide it, and only one of them is the size

**1. The eligible marks are mostly glyphs.** 56 of the 88 eligible marks at 4× are at or
below ten thousand device texels — under 100 × 100 device pixels, which at that scale is
under 25 × 25 at the page's own — and they are 11.7 % of the eligible texels. The eligible
*coverage* is a handful of large pattern fills; the eligible *marks* are reading-size text
painted with a shading. `ShowText-ShadingPattern.pdf` is the case alone on a page: 63
shading-painted glyphs, all 63 kept on the CPU lane by ADR 0026's comparison at scale 1, and
29 of them crossing it at 4×. Same glyphs.

**2. The comparison that would decide is missing its most protective condition for exactly
that population.** `take_gpu_lane` has four conditions, and the one that keeps reading-size
text off the sampled grid is the *cache* — `CacheProspect::worth_caching`, the atlas's
admission rule and the scene's placement census in one answer (ADR 0029). A rare paint's
coverage is never offered to the atlas, so `push_rare_coverage` would have to pass
`CacheProspect::TooLarge` exactly as `push_coverage_styled` does, and only the
tile-versus-triangles comparison would be left. That comparison is monotone in the
magnification: **the higher the zoom, the more of a page's shading-painted text it would
send to the device** — and the magnification is precisely when a caller switches to
`Coverage::Gpu`.

That matters because of what the device lane does to a thin mark. `Coverage::Gpu` samples a
4 × 4 ordered grid, so a 0.1-device-pixel bar falls between the columns and is drawn as
**nothing at six of ten sub-pixel positions**, while the CPU lane's analytic area draws it at
all ten (`tests/thin_marks.rs`, and `HANDOVER.md` records it as a trap). ADR 0016 states the
same trade in its own words: reading-size text "is where that difference is most visible and
is exactly where the CPU lane is already fast". A change that bought 0.1 % of a frame's
coverage and moved shading-painted glyphs onto a sampled grid at zoom would be trading the
oracle's bound (§4.5) for nothing measurable.

## Decision

**`Options::coverage` selects the coverage producer for a solid fill or stroke, and for
nothing else. That stays true, and it is a decision rather than an omission.**

Three things follow, and all three are in this round:

1. **The public rustdoc on `Coverage` stops calling it an open question** and states the
   number instead. A caller reading it now learns what the limitation costs, not only that
   it exists.
2. **The claim is gated for the paints it names.** `tests/function_coverage.rs` asserted the
   equality for `Paint::Function` through `encode_fill`'s door alone — one of four paints and
   one of two doors, which is `tests/shader_copies.rs`'s shape (it named 8 shaders where the
   tree had 10, compared five and passed). `tests/rare_lane_coverage.rs` adds the shading arm
   through **both** doors, with a solid-painted control on the same geometry so the equality
   cannot be an equality between one lane and itself.
3. **No `Counters` field is added.** `coverage` (ADR 0057), `lanes` (the census) and
   `atlas_overflow_tiles` (ADR 0063) answer every question a caller has; a fourth would be
   the duplicate the census round already refused.

## The cost of leaving it, written down

- **A caller who sets `Coverage::Gpu` does not get it for 1.7 % (scale 1) to 4.2 % (4×) of
  the coverage the processor still rasterises.** On the fourteen pages that have such a mark,
  and on six of them it is the whole of the page's sheet — though those sheets are 74 k to
  508 k texels, which no lane choice rescues.
- **A page of large shading-filled paths at high magnification is the shape this leaves
  behind**, and it is a real shape (`pattern_text_embedded_font.pdf`, 694 697 texels at 4×).
  It pays the CPU rasteriser every frame of a zoom gesture, which is the exact case ADR 0016
  exists for. The corpus says it is 0.18 % of the corpus's coverage; it does not say it is
  0.18 % of *that user's* frame.
- **A second lane rule to remember.** The tree now has two paints-versus-lanes rules — the
  residue rule, which is on the `Gpu` variant, and this one — and neither is visible from a
  call site. Both are stated on `Coverage` and both are gated.

## What is not decided here

- **Whether a residue could ever multiply on the device.** That is ADR 0016's recorded gap
  and it is four fifths of all rare-painted coverage — a far larger prize than this round's,
  and it would also unlock the *solid* marks the residue rule currently forces onto the CPU.
  Nothing here bears on it.
- **Whether the rare lane's fills should be flattened later.** A rare fill arrives at
  `push_rare_coverage` already flattened, so even if it took the device it would gain
  ADR 0016's rasterising saving and not its scale-independence. That is a change to the fill
  arm, and a round that reopens this one has to take it too or it will measure the smaller
  half.

## Revisit when

Any one of these, and each is a number rather than a feeling:

- **The residue multiply moves to the device.** The eligible population then grows by the
  350 residue-clipped rare marks — 82 % of all rare-painted coverage — and this ADR's
  arithmetic is about a fifth of the question it was.
- **A caller reports a page**, or a corpus row shows one, where rare-painted coverage is a
  material share of a *frame* rather than of a corpus. The instrument is
  `Counters::coverage` against a page, and `doc/notes-rare-lane.md` §8 is the split.
- **The device lane gains an area rule** rather than a sample grid, which removes finding 2
  entirely: it is the only thing that makes moving glyph-shaped marks to the device safe,
  and `tests/thin_marks.rs` is where it would be observed.
- **The census's population moves off first pages.** Shading-painted artwork is exactly the
  content that sits past page one, and this measurement cannot see it
  (`doc/notes-rare-lane.md` §9).

## Recorded, so it is not re-derived

**The omission is an oversight, not a design constraint**, and the evidence is in the tree's
own structure rather than in an argument: `push_gpu_tile` takes its seat from
`ScratchPacker::reserve`, the same one door onto the sheet `pack_scratch` uses; the winding
tiles are resolved into the frame's R8 scratch texture during the upload phase, before any
draw pass is recorded; and `shading.wgsl` reads coverage as
`textureLoad(scratch_tex, coverage.xy + (p − dest.xy))` from that same texture. The device
lane's output *is* the R8 tile a rare paint is drawn through. The only obstacle is that
`push_gpu_tile` reserves, draws the triangles **and** emits the solid quad in one function,
where the rare lane needs the first two and builds its own op.

That was verified by making the change — about fifty lines in `coverage_placement` — and
running it: it draws, and its frames differ from the CPU lane's only in the antialiased
edges (419 pixels on each of two marks). The patch served as this round's forced defect and
was reverted. A later round must not re-derive the structural question, and must not be told
it cannot be done.
