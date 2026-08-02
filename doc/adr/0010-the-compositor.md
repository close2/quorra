# ADR 0010 — The compositor: layers, one blend formula, two-pass knockout

Status: accepted, 2026-08-02. Landed with M6.

## Context

Clause 11 is the reason this library exists (brief §0). Three properties had to hold
at once: groups composite as a unit onto transparency (§11.4.1/.5); shape is not
opacity, and knockout replaces by shape (§11.4.6, §4.1); and the sixteen blend modes
of §11.3.5 need the backdrop as an input, which `wgpu` has no framebuffer-fetch for.
ADR 0006's finding also bound: any arithmetic the fixed-function unit owns diverges
across adapters, so the less of clause 11 it owns, the tighter the determinism story.

## Decision

**A layer tree, executed with in-shader compositing (`compose.rs`).**

- Each group is a `LayerPlan` rendered into its own premultiplied texture; the
  finished layer composites onto its parent exactly once through `composite.wgsl`,
  which implements §11.3.6's formula and all sixteen §11.3.5 B() functions — target
  blend state REPLACE, so the whole computation is our stated f32 arithmetic, not
  the driver's. A parent ping-pongs between two textures across composites (a pass
  cannot read its own attachment). Group alpha, clip rectangle, clip residue and
  soft mask all apply at the composite, where §11.4.5 puts them.
- **An element with a non-Normal blend is an implicit one-element group** — one
  machine for §11.3.5 rather than two. Inside a knockout group the wrap is skipped:
  §11.3.6 with αb = 0 degenerates every mode to Normal against the transparent
  initial backdrop.
- **Knockout is two fixed-function passes per element**: erase — `fs_shape` emits
  the element's *shape* (coverage × clip × mask; paint alpha is opacity, not shape,
  §11.4.7.2) under factors (ZERO, 1−α), scaling the backdrop by (1 − shape) — then
  add — the paint under (ONE, ONE), depositing shape·element. Algebra:
  `d·(1−f) + f·s` is §11.4.6's replacement exactly, and the passes must interleave
  strictly per element: batching erases before adds computes
  `d(1−f₁)(1−f₂) + f₁s₁ + f₂s₂`, which loses the `f₁s₁(1−f₂)` term wherever
  knockout elements overlap. `Compose::Src` outside a knockout group takes the same
  two passes (§4.1 applies it per element).
- **Soft masks realise first**, in id order (the builder lets a mask reference only
  earlier masks, so dependencies are acyclic by construction): the mask group runs
  through the same layer machinery, then `reduce.wgsl` mirrors the caller's
  `SoftMask::value` operation-for-operation — integer demultiply, `fma` in their
  order, the clause's 0.30/0.59/0.11, round-clamp, the 256-entry table — and writes
  `byte/255`, which lands on the exact R8 level on every driver. `tests/m6.rs` holds
  all 256 bytes of both rules, a non-black backdrop and an inverting transfer, and
  they agree **exactly**.
- **Flat frames skip all of it**: a root with no children and no masks draws
  straight into the target, so M1's measured fast path is untouched. Layered frames
  pay one final blit (the target may be an unsampleable swapchain texture).
- **Budgets**: internal layer and mask textures are priced before creation
  (2 × RGBA per plan + 1 × R8 per mask, full target size) against
  `Options::max_frame_bytes`; the refusal names both numbers.

## Costs, stated

Full-target layers are the simple-correct choice, not the cheap one: a group's bbox
could bound its textures and composites. Recorded as the first optimisation for a
measurement to justify, likely alongside M8's damage work. Timestamp queries bracket
first-to-last pass (an empty end-stamp pass closes the frame), so `execute` still
gates; per-pass attribution beyond that is future work if a regression ever needs it.

## What holds it

`tests/m6.rs`: the sixteen modes against a clause-transcribed reference; the knockout
diagonal against §11.4.6's own formula; group alpha once; isolation (a group is not
per-element blending); the 256-byte mask agreement; compositor determinism per
adapter and the cross-adapter bound.
