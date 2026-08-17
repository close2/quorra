# ADR 0025 — §11.4.6's two stages, by name

Status: accepted, 2026-08-11. Answers the caller's feedback §14.

## Context

`Compose::Src` is "Porter-Duff Source, **modulated by coverage**", and it is right for
the half of §11.4.6 where an element's shape *is* its coverage — which is most of a
page, including every text object §9.3.8 produces. It is wrong for the other half, and
the clause says why in one sentence:

> The existence of the knockout feature is the main reason for maintaining a separate
> shape value rather than only a single alpha that combines shape and opacity.

§11.6.4.2 gives an object's shape from its geometry alone; §11.6.4.3's soft mask and
§11.6.4.4's constant alpha are *opacity*. So a knockout element under a soft mask has
shape 1 inside its path and opacity ½, and a nested group has the shape of everything it
marks whatever alpha it is painted at. `Compose::Src` reads shape off the alpha a mark
is drawn with, which is exactly what those elements contradict.

Four corpus documents state it — `knockout_smask.pdf`, `knockout_nested.pdf`,
`knockout_nested_group_alpha.pdf`, `knockout_inner_backdrop.pdf` — and all four were
counted as *agreeing* while both of the caller's backends made the same wrong
assumption.

## Decision

**`Compose::DestOut` and `Compose::Plus`.** A caller writes §11.4.6's second stage as
`P' = (1 − f) × P + S` in one mark each: erase by the shape-only object, then add the
object. That is the smaller of the two shapes they offered — the other was a per-element
shape channel — and it is smaller here than it would be anywhere else, because **the
pipelines already exist**. The knockout lane *is* these two operators: `RectErase` and
`CoverErase` blend `(Zero, OneMinusSrcAlpha)` through the `fs_shape` entry point, and
`RectAdd`/`CoverAdd` blend `(One, One)`. This ADR is a scene vocabulary that can ask for
one of them alone.

**`DestOut` weights by shape, deliberately not by the paint's alpha.** `fs_shape` returns
the coverage under the mark's clip and ignores the paint entirely — the comment above it
already cited §11.4.7.2 for that — so a caller draws the object with every source of
opacity removed and gets §11.6.4.2's shape. Weighting by the alpha would repeat the
defect the operator exists to fix.

**Two positions refuse it.** Both already stage the clause by another route, so a staged
mark inside them applies it twice: a mark carrying a blend mode, which §11.3.5 puts in an
implicit one-element group, and a mark inside a knockout group. Refused at the builder
with `SceneError::StagedComposeUnsupported`, naming which. The builder can answer the
second because ADR 0019 taught it what a command is nested in.

## What it costs, and the one thing that cannot be refused

Measured on a wedge with a diagonal edge — axis-aligned rectangles would agree while
being wrong — a half-opaque object over an opaque backdrop, worst premultiplied
deviation from §11.4.6's line over every pixel:

| | deviation |
|---|---|
| the staged pair | **0.77 of 255** (unorm rounding) |
| the same object with source-over | **114.95 of 255** |

Source-over weights the backdrop by `1 − shape × opacity` where the clause weights it by
`1 − shape` alone. The caller pins the same phenomenon at 32 of 255 on their fixture;
the size depends on the backdrop, and either number is a wrong picture.

**`Plus` is correct only in the pair, and no scene can be refused for getting that
wrong.** Addition alone saturates: without the matching `DestOut` it drives a
premultiplied channel past its alpha, and one mark cannot tell a library whether the
other is coming. This is the first item in the vocabulary whose correctness is the
caller's obligation rather than something the builder can check, and it is stated in
`Compose::Plus`'s own documentation rather than left to be discovered. The alternative —
a shape channel per element, so one mark carries both quantities — would have kept the
obligation inside the library, at the cost of a wider instance and a change to every
lane. The caller had no preference; this is the smaller change, and the pairing is one
line in their expansion of a clause they are already expanding.

## Revisit when

A caller wants `Plus` for something that is not §11.4.6's second stage — an additive
paint mode, say. Then the saturation stops being a footnote and the shape-channel design
becomes the better one.

## Amendment, 2026-08-18 — the prose above was right and the shaders did not agree with it

The decision stands, and so does every sentence of its reasoning. **What was not true
when this was written is that the tree did what the Context says.** "§11.6.4.3's soft mask
and §11.6.4.4's constant alpha are *opacity*" is the clause's reading, and it is now the
tree's; until ADR 0066 all five lanes' `fs_shape` multiplied the mark's soft mask into the
shape they returned, so a masked element inside a knockout group was erased by
`coverage × mask`. The constant alpha was excluded and the mask was not, which is not
§11.6.4.3 under either value of the flag that governs them — Table 57 names both in one
sentence. Found by reading rather than by a failure (`doc/notes-hayro-questions.md` §2),
settled by **ADR 0066**, and measured at **138 of 255** on a wedge under a banded mask.

**Two citations in this ADR are wrong and are corrected there rather than here.** "the
comment above it already cited §11.4.7.2 for that" is accurate about the comment and the
comment was wrong: ISO 32000-2 has no §11.4.7.2 — §11.4.7 is "Page group" and has no
subclauses. The clause that keeps shape and opacity apart is **§11.3.7.2**, "Source shape
and opacity"; the clause that puts the clipping path into the shape is **§8.5.4**.
`doc/notes-mask-shape-or-opacity.md` §7 lists every site that carried the bad number.

The rest of this ADR is untouched by that: `DestOut` weighting by shape rather than by the
paint's alpha was already the clause, and it now weights by shape rather than by the mask
as well — one operator, one quantity, and the same sentence of Table 57 behind both.
