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

**Skeleton.** The workspace, the tooling, the module map and the requirements are in
place; no rendering is implemented yet. `doc/PLAN.md` says what happens in which order
and what has to be measured before each part is designed.

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
