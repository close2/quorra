# ADR 0026 — The coverage lane is chosen by what each lane would cost

Status: accepted, 2026-08-12. Fixes a refusal the corpus profile found; supersedes the
part of ADR 0016 that made the lane a per-frame choice alone.

## Context

ADR 0016 gave `Coverage::Gpu` its argument: nothing in the GPU lane depends on the
magnification, so a zoom gesture — where every cached tile is cold on every frame —
costs the same as standing still. The lane was a per-frame setting, and the crossover
was described as "a cliff at `MAX_GLYPH_DIM`", where a glyph stops entering the atlas.

`MAX_GLYPH_DIM` is gone (ADR 0024), and the corpus profile then showed what the setting
does to a real page. On the corpus's largest — 66 309 commands over 65 978 distinct
outlines — `Coverage::Gpu` **refuses the frame**:

```
frame needs 922319928 scene-derived bytes, over the stated budget of 268435456
```

Broken down: 82.8 MB of winding texture, 1.6 MB of tile records, and **821 MB of
vertices**. That is the number that matters, and it is not the texture anybody would
have suspected.

**The GPU lane costs an outline's triangles per placement, whatever the tile's size.**
Measured: a nine-pixel glyph of eight curves is 387 vertices — **12.4 KB** — against
roughly **150 bytes** of coverage for the same tile. Eighty times more, per glyph, on
the page shape most documents are made of. The same outline at 300 pixels costs the
same 12.4 KB against 90 KB of coverage, and there the device should obviously draw it.

## Decision

**A command takes the GPU lane when its tile's coverage would cost more than its
triangles.** Both numbers are known at encode time: `width × height` bytes of R8 against
`triangle_count × 3 × WindingVertex::STRIDE`. No constant, no dimension, no threshold
anybody has to tune — each shape decides for itself, and a shape with few curves crosses
earlier than a fussy one.

`Coverage::Gpu` therefore stops meaning "use the GPU lane for everything" and starts
meaning "use it where it pays". The two conditions that were already there stay: the
caller must ask, and a residue clip still forces the CPU lane because nothing on the
device multiplies a residue yet.

`QuadOutline::triangle_count` was `#[cfg(test)]`. It is the criterion now, which is a
better fate for a number than being an assertion's helper.

## What it buys, measured

Release, RADV, cold device, 1191×1684:

| page | `Coverage::Cpu` | `Coverage::Gpu` before | `Coverage::Gpu` after |
|---|---|---|---|
| corpus max, 66 309 commands, no reuse | 387 ms | **refused**, 922 MB | 397 ms — the CPU lane, chosen per command |
| dense text, 4 320 commands | 6 ms | 17 ms, 62.5 MB uploaded | 5 ms, 0.5 MB |
| 200 shapes of 200 px, no reuse | encode **121 ms** | — | encode **3 ms** |
| 60 shapes of 500 px, no reuse | encode 368 ms | refused, 359 MB | refused, 359 MB |

The third row is ADR 0016's promise arriving: forty times less encode on the shape the
lane was built for. The second row is the defect this ADR fixes — a page of ordinary
text was paying 62.5 MB and twelve extra milliseconds to be drawn *worse* (sampled
coverage rather than exact). The first row is a page that could not be drawn at all.

## What it does not fix

**The last row.** Sixty large shapes still refuse, and now the reason is visible rather
than hidden behind the vertex count: the winding texture is sized from the whole scratch
sheet at eight bytes a texel, so a sheet of large tiles is hundreds of megabytes. The
winding target is *scratch* — it is resolved into the R8 sheet and then dead — so it
does not need to hold the sheet at once. Processing the sheet in bands, with a winding
texture bounded by a stated size and one resolve per band, would make the GPU lane's
memory O(band) instead of O(page).

That is the next piece of work on this lane. It is deliberately not in this ADR: it
touches the pass that produced the caller's §11 defect — a frame that drew the *wrong
glyph* after a larger one — and it wants its own change, its own tests and its own
measurement rather than being folded into a policy fix.

## Revisit when

The banding above lands, which changes what "the GPU lane costs" means for a large tile
and so may move the crossover. The criterion should then be re-derived rather than
kept: it is two costs compared, and one of them will have changed.
