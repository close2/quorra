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

**M8 — every scene command draws, and damage is honoured.** A device (headless and
surface-attached,
presenting real frames — proven under Xvfb), three target kinds, and the full
drawing vocabulary: analytic rectangles with rectangular clips at zero device cost
(ADR 0007); a glyph atlas with the settable 1/16-pixel quantum (5 933 fills → 107
cached tiles → 1.0 ms/frame at window scale on RADV; ADR 0009); a general path lane —
fills under both rules, strokes with caps/joins/miters, non-rectangular clip
residues — over one deterministic CPU coverage rasteriser (ADR 0008); clause 11
natively — isolated groups, all sixteen §11.3.5 blend modes in-shader, per-element
knockout, soft masks byte-agreed with the caller's CPU reduction on all 256 inputs
(ADR 0010); and the rare-case lanes — images with resolved per-placement filtering,
axial/radial ramp sweeps with clause-derived bytes, pre-rasterised meshes — as
uniform-driven quads (ADR 0011; the dense page plus a figure load: 0.63 ms/frame on
RADV). The scene vocabulary validates loudly per §4.7; resources are
upload-once/reference-many under stated budgets; frames report timestamped phase
timings and refuse rather than approximate. Two of the brief's open questions are
answered by measurement: the readback is ~90% of an offscreen frame's cost on the
real GPU, and cross-adapter byte identity is not achievable through the
fixed-function raster path — but is achievable wherever the arithmetic is ours,
which is how the compositor and the paint lanes are built (ADR 0006). Damage
against a retained texture target is honoured exactly — scissored rendering,
rectangle patching, nothing outside the list touched (ADR 0012; a caret blink's
execute drops 3× on RADV) — and the pipeline-cache `unsafe` exception was weighed
and declined with the benchmark in hand (ADR 0013). `doc/PLAN.md` carries the
design, the records, and what happens next. The swap into the caller's tree (M9) is
under way: `render-quorra` in the viewer's workspace implements their `Rasterizer`
over this library and passes their cross-backend and real-page suites at their
Vello backend's own thresholds — what remains is the viewer-ui wiring, the corpus
run, and the windowed session.

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
