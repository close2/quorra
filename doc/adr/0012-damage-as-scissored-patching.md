# ADR 0012 — Damage as scissored rendering plus rectangle patching

Status: accepted, 2026-08-02. Landed with M8.

## Context

§6.5 of the brief: a caret blink must not cost a page. `Viewport::damage` is a
contract, not a hint — the caller guarantees the scene changed only inside the
listed rectangles, and a device that honours the list may touch nothing outside it,
because a stale region outside damage is undetectable downstream. Two facts shaped
the mechanism: only a `Target::Texture` has retained contents at all (a swapchain's
previous texture is not guaranteed, a `Readback` frame starts fresh); and drawing a
scene *over* its own previous rendering double-composites every translucent pixel,
so "load and redraw the damaged commands" is wrong on arrival.

## Decision

**Render internally under a scissor; patch with REPLACE blits.** A valid damage
list against a `Texture` target renders the frame through the compositor's root
texture — even a flat frame — with **every internal pass scissored to the damage
bounding box**, then blits each damage rectangle onto the target with `LoadOp::Load`
and the blend-free blit pipeline. Correctness rests on two invariants:

- **Every pass in this pipeline is pixel-local**: a fragment reads attachments only
  at its own coordinate (lane draws, §11.3.6 composites, §11.5 reductions; images
  and meshes read *uploaded* textures, which are whole). Pixels outside the scissor
  are therefore never inputs to pixels inside it, and skipping them changes
  nothing that is blitted.
- **The blit replaces.** Inside a rectangle the target receives exactly what a full
  redraw would have put there; outside, no fragment is ever generated. No
  compositing against retained contents happens anywhere, so nothing can
  double-composite.

The rest of the contract: a `Surface` or `Readback` target redraws fully and says
so in a `Report` naming the target kind; a malformed rectangle (NaN, inverted) is
refused as `RenderError::InvalidDamage` with its index — a guessed region risks
exactly the stale frame damage exists to prevent (§4.7); a list that clamps to
nothing touches no pixel at all.

## Measured

Dense 5 933-command page, 1191×1684, retained texture target, one 12×18 damage
rect: **RADV execute 0.136 → 0.047 ms** (wall 0.46 → 0.37 ms, fixed submit cost
dominating); **llvmpipe 4.2 → 1.6 ms/frame** — the software rasteriser is
pixel-bound, so the scissor pays there most.

## Costs, stated

Encode still walks the whole scene (~0.1 ms on this page): commands are not culled
against damage, and a patched flat frame pays the root texture pair it renders
through. Both are recorded levers, not oversights — culling needs bounds we already
compute, and the pair could shrink to the damage bbox. **No texture pool** backs
the internal textures: creation is inside the measured 0.37 ms frame, so a pool has
nothing measurable to win at this scale; that decision (and any shrink policy a
pool would need) waits for a measurement that says otherwise.

## What holds it

`tests/m8.rs`: inside-equals-full-redraw and outside-survives-byte-for-byte in one
fixture whose scenes differ everywhere; disjoint rects with an untouched gap;
a layered (grouped) scene through the same machinery; the full-redraw `Report` on
`Readback`; refusal by index; the clamps-to-nothing no-op.
