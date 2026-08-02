# ADR 0007 — Rectangular clips resolve at encode time, to four floats and an intersection

Status: accepted, 2026-08-02. Landed with M3.

## Context

`RENDER_LIBRARY.md` §6.4: most clips are axis-aligned rectangles — the caller's page 6
states one clipping rectangle 303 times — and "a clip that is an axis-aligned
rectangle should not become an R8 mask texture; it should become four floats and a
comparison." The brief imagines the comparison in the fragment shader; the question
this ADR answers is *where* those four floats are applied.

Two facts shaped the answer. First, the caller's clips arrive as outlines (its display
list has no rectangle type), so the rectangle must be *recognised* — done once, at
upload (`axis_aligned_rect`, stored as a hint on the resident outline). Second, for
the rectangle lane specifically, the geometry being clipped is itself an axis-aligned
rectangle, and rectangle ∩ rectangle is a rectangle.

## Decision

**Chains resolve on the CPU, per frame, memoised; the rectangle lane applies the clip
by intersection before anything reaches the GPU.**

- At encode, each referenced chain folds to one device-space rectangle: per link,
  recognised rectangle × axis-preserving composed transform, intersected with its
  parent's resolution. Memoisation across shared prefixes keeps the caller's
  3 608-chain worst page linear; parent ids are smaller than child ids by
  construction, so the walk cannot cycle.
- A rectangle-lane command under a clip draws `fill ∩ clip` — computed in exactly the
  `min`/`max` arithmetic a per-pixel shader comparison would use, so the pixels are
  identical and the device cost of a rectangular clip is **zero**: no scissor state,
  no batch cut, no extra instance bytes.
- An empty resolution (disjoint links, or a degenerate rectangle outline) admits
  nothing: the instance is dropped, the frame is legitimate. Distinct from an absent
  clip, and tested as distinct.
- `Counters::clip_distinct_regions` counts resolved *regions* (by the four floats'
  bit patterns), never identifiers: 303 identical clip states count as 1 — the
  caller's ADR 0132 lesson, applied where it was learned.
- A chain link that is not a recognised rectangle under an axis-preserving transform
  is refused by name (`NotYetDrawable`, "M5's clip residue") — the R8 residue mask is
  the path lane's work, and until it exists a hole-with-a-sentence beats a plausible
  lie (§5).

## Consequences

- The per-pixel clip comparison the brief sketched is *deferred, not rejected*: the
  glyph lane's atlas quads (M4) cannot pre-intersect their coverage, so M4 introduces
  the shader-side clip rectangle — with these same resolved four floats as its input.
  This ADR fixes the resolution point (CPU, memoised, region-counted); the
  application point is per lane.
- The recogniser is strict on purpose: lines only, four axis-aligned edges, exact
  coordinate equality. A rectangle drawn as collinear cubics is not recognised and
  takes the residue path — correct, merely slower, and a measurement at M5 can argue
  for loosening it if real corpora contain such clips.
- The rectangle lane's determinism story is unchanged: the intersection runs in the
  same f32 arithmetic on every host, and the M3 fixtures hold the result to the CPU
  reference within ADR 0006's bound and to same-adapter byte identity exactly.
