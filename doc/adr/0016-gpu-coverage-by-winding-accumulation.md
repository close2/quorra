# ADR 0016 — GPU coverage by winding accumulation (Wallace's method)

Status: accepted for the lane and its proof, 2026-08-05. **Not yet wired into the
encoder** — see "What is landed" below, which is the part of this ADR that decays
first.

## Context

ADR 0008 chose a CPU scanline rasteriser as the single producer of coverage bytes, and
named what would overturn it: "a page profile where per-frame CPU rasterisation
dominates". ADR 0015's measurement found that profile, and it is the one a person
reaches by zooming. Past `MAX_GLYPH_DIM` a glyph never enters the atlas, so at 20×
magnification a frame spends **6.8 ms rasterising thirty glyphs**, every frame, cached
by nothing; a probe that let those tiles into the atlas took the same frame to 0.25 ms,
which identifies the cost beyond argument.

Two things follow. The cost is *caching*, not the CPU — so a GPU rasteriser is not
automatically the answer. But the cache cannot help the case that actually hurts: a
zoom *gesture* gives every frame a new transform, so every key is new, and no cache of
transformed coverage can ever be warm during the interaction a person is performing.
A producer whose cost does not depend on the transform at all is the only thing that
helps there.

## Decision

**Add a second coverage producer that rasterises outlines on the GPU by Evan Wallace's
method** (*Easy Scalable Text Rendering on the GPU*, 2016), and keep the CPU one.

The algorithm, as landed:

- **One triangle per segment, fanned from an anchor.** Summed with a sign per triangle
  orientation, the fan covers a point exactly `winding` times — ISO 32000-2 §8.5.3.3's
  winding number, and nothing else.
- **One extra triangle per curve**, kept only where Loop and Blinn's implicit test
  `u² < v` holds. It adds the bulge between chord and curve when the control point lies
  outside, and takes the bite out when it lies inside, *purely by its own orientation*
  — neither the shader nor the geometry that fed it decides which case it is.
- **Additive blending into a float target**, then a resolve pass that applies the fill
  rule to each sample and averages.
- **Cubics become quadratics once, at upload** (`outline.rs`), with a bound: a cubic is
  within `√3/36 · |d|` of the quadratic through the same ends with control
  `¾(c₁+c₂) − ¼(p₀+p₃)`, where `d` is the third difference. `d` is exactly zero when
  the cubic is a degree-elevated quadratic, so an already-quadratic curve converts
  exactly, and splitting at ½ divides `d` by eight.

**Nothing in that depends on the device scale.** That is the whole point, and the
difference from `raster.rs`, whose flattening tolerance is in device pixels.

### Three departures from the article, and why

1. **Signed accumulation, not parity.** Wallace adds 1/255 per triangle and calls a
   pixel inside when the total is odd. That is §8.5.3.3.3's even-odd rule and *only*
   that rule, because an unsigned buffer cannot hold a winding number's sign. §8.5.3.3.2's
   non-zero rule is PDF's default, so parity was never enough for us. An
   `rgba16float` target holds four signed windings exactly — `f16` is exact on integers
   to 2048, four hundred times any winding a real page produces — and both rules fall
   out of the same number. `tests::the_two_fill_rules_differ_where_the_clause_says_they_do`
   is the proof: nested same-wound squares, filled under one rule and hollow under the
   other, on the same geometry.
2. **Samples in channels, not in the bits of a byte.** His packing of eight samples
   into an 8-bit colour buffer was a 2016 WebGL necessity. Four channels hold four
   samples with no packing and no bound on the winding; a frame wanting sixteen samples
   runs the pair of passes four times, clearing the winding texture between rounds and
   adding each round's quarter into the sheet. **Sample count costs time, never
   memory** — the right trade here, because the GPU is idle (`execute` is tens of
   microseconds) and memory is what a zoomed page runs out of.
3. **The sample grid is ours, not the driver's.** An ordered grid stated in `winding.rs`
   rather than a multisample pattern the adapter chooses is what keeps coverage
   identical on every adapter. ADR 0006 measured that promise and ADR 0008 protects it;
   a lane that took MSAA sample positions from the driver would have spent it.

### Both lanes, and why that is not a hedge

The two produce the same artefact — an R8 tile in the same sheet — so everything
downstream (the quad lanes, clips, knockout, the compositor) cannot tell which one ran.
The choice is a device option, not a second code path through the renderer.

Keeping the CPU lane is not indecision. Its coverage is the *exact* area of the pixel a
shape covers, analytically, 256 levels (ADR 0005); the GPU lane counts samples, and
sixteen samples give seventeen levels. Reading-size text is where that difference is
most visible and is exactly where the CPU lane is already fast, because the atlas is
warm. The lanes have opposite cost curves and the crossover is a magnification.

## Measured

The lane itself, on llvmpipe, from `src/winding.rs`'s tests: an aligned square is 255
inside and 0 outside; an edge through a pixel's centre reads exactly **128** — with a
4×4 grid the sample columns sit at ±0.125 and ±0.375, so two of four are inside, 8 of
16 samples, `round(0.5 × 255)`; nested same-wound squares fill under non-zero and
hollow under even-odd.

The numbers that motivate it are ADR 0015's: 20× costs 6.8 ms of encode for thirty
commands, and a 1×→20× gesture's worst frame is 9.3 ms of encode with every tile cold.

**What is not yet measured is the only number that decides how this is used**: the
magnification at which the GPU lane overtakes the CPU one on a real page. It cannot be
measured until the encoder integration below exists.

## What is landed, and what is not

Landed and tested: the geometry (`outline.rs` — conversion, contour closure, the fan
and control triangles), the shaders (`shaders/winding.wgsl`), the two pipelines, and
the frame pass (`winding.rs`) that turns a sheet of triangles into the R8 coverage
texture. It is exercised end to end on a real device.

**Not landed: the encoder does not route anything to it.** `encode.rs` still sends every
fill to `raster.rs`. Wiring it needs three things, none of them speculative:

- the scratch packer reserving a tile's space *without* bytes, so both producers can
  share one sheet layout;
- the frame budget pricing the winding texture and the vertex buffers before they are
  allocated (`Sheet::device_bytes` exists for exactly this and has no caller yet);
- **residue clips**, which multiply a non-rectangular clip into the coverage mask on
  the CPU. Until that happens on the GPU too, a command under such a clip has to take
  the CPU lane, and a frame can need both sheets at once — which is the one place where
  "two producers, one artefact" is not yet true.

Recording the gap rather than closing it quietly is the point: a reader of this tree
should not have to find out from a dead-code warning that a lane is not reachable.

## What holds it

`src/outline.rs`'s tests: a degree-elevated quadratic converts back to its own control
point; an open contour gains its closing chord (§8.5.3.1); a contour that cannot
enclose area emits nothing, with the rule stated geometrically rather than guessed; a
chord triangle's implicit coordinates can never discard it. `src/winding.rs`'s tests:
the four above, plus the sample grid being ordered and balanced about the pixel centre.
