# quorra

A GPU renderer for **documents**, in Rust, on wgpu.

Not a general 2D vector renderer pointed at documents — the other way round. Its fast
paths assume that most of a page is the same few glyph outlines repeated at many
sub-pixel phases plus axis-aligned rectangles, and it treats general curve filling as
the rare case rather than the uniform one. It expresses the transparency model of
ISO 32000-2 clause 11 natively, because that is the part an SVG-shaped renderer cannot
be patched into.

The name is Tron: Legacy's Quorra, the last ISO. This library implements one.

## Status

**All nine milestones are done, and the viewer renders through this library.**
`render-quorra` in the PDF viewer's workspace implements their `Rasterizer` over quorra,
and their window presents through the surface tier with no readback anywhere.

Every scene command draws. Analytic rectangles with rectangular clips at zero device cost
(ADR 0007); a glyph atlas with the settable 1/16-pixel quantum (ADR 0009); a general path
lane — both fill rules, strokes with caps/joins/miters, non-rectangular clip residues —
over one deterministic CPU coverage rasteriser, with a GPU winding lane where it pays
(ADRs 0008, 0016, 0026); clause 11 natively — isolated and non-isolated groups, all
sixteen §11.3.5 blend modes in-shader, per-element knockout, soft masks byte-agreed with
the caller's CPU reduction on all 256 inputs (ADRs 0010, 0019, 0032, 0033); images,
ramp shadings and pre-rasterised meshes as uniform-driven quads (ADR 0011); damage
honoured exactly against a retained texture target (ADR 0012); and a `RetainedScene`
whose unchanged frame replays instead of re-encoding (ADR 0048). A frame is drawn or
refused by name, never approximated.

**Against the brief's own success criterion** (§6.2: a third of a multi-threaded
`tiny-skia`'s 5.9 ms on a dense text page at 1191×1684, presenting to a surface): 1.816 ms
on RADV, of which the GPU is about 4 %. The caller's 974-document corpus agrees with their
CPU oracle on 934 of 956 comparable pages at scale 1.

`doc/PLAN.md` carries the design and what is true today, `doc/history/` how it got there,
`doc/adr/` why each decision was taken.

## Layout

| | |
|---|---|
| `crates/quorra-scene` | What is to be drawn. No device, no `wgpu` dependency — see ADR 0001. |
| `crates/quorra-gpu` | The device, the pipelines, the atlas, the frame. |
| `crates/quorra` | The facade a caller depends on. |
| `doc/RENDER_LIBRARY.md` | The brief. Written by the consuming PDF viewer, with a measurement behind every requirement. |
| `doc/PLAN.md` | Milestones, and the questions each one has to settle by measurement. |
| `doc/adr/` | Every non-obvious decision, with its reasoning and its cost. |

## What it will not do

Colour management, font loading, shaping, text layout, SVG, hit testing, or any
document format at all — it never sees a PDF byte. The full list, with reasons, is
`doc/RENDER_LIBRARY.md` §9.

## Licence

MIT. See `LICENSE`.
