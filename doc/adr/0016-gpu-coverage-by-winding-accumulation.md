# ADR 0016 — GPU coverage by winding accumulation (Wallace's method)

Status: accepted, 2026-08-05. Landed complete: `Options::coverage` selects the lane,
and the crossover below says when a caller should.

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

## Measured: the lane is right

On llvmpipe, from `src/winding.rs`'s tests: an aligned square is 255 inside and 0
outside; an edge through a pixel's centre reads exactly **128** — with a 4×4 grid the
sample columns sit at ±0.125 and ±0.375, so two of four are inside, 8 of 16 samples,
`round(0.5 × 255)`; nested same-wound squares fill under non-zero and hollow under
even-odd, which is what proves the sign survived accumulation.

## The crossover, measured

`examples/zoom.rs` (`cargo run --release -p quorra-gpu --example zoom -- gpu`), RADV,
the dense 5 933-fill page at 1191×1684. Encode and wall, best of five, machine under
light load:

| magnification | CPU encode / wall | GPU encode / wall |
|---|---|---|
| 1× | **0.85 / 1.12 ms** | 4.9 / 15.2 ms |
| 4× | **0.34 / 0.54 ms** | 0.63 / 2.5 ms |
| 20× | 8.4 / 12.5 ms | **0.26 / 1.9 ms** |
| 100× | 3.9 / 7.4 ms | **0.25 / 2.2 ms** |

**The crossover is between 4× and 20×**, and the shape of it is what the two designs
predict: the CPU lane has an atlas and pays per *pixel rasterised*, so it wins while
glyphs are small and reused; the GPU lane has no atlas and pays per *triangle*, so it
wins as soon as a glyph is large enough to cost more to fill than to describe. At 20×
the GPU lane is **6.6× faster on the wall clock** and its encode is 32× cheaper; at 1×
it is 13× slower, which is why `Coverage::Cpu` remains the default and why the choice
belongs to a caller that knows its magnification.

Two things the measurement changed while it was being taken, both recorded because
they were found rather than designed:

- **The winding texture is kept between frames.** ADR 0012 declined a texture pool
  "until a measurement says otherwise"; allocating and zero-initialising 8 bytes ×
  2.5 million texels every frame cost **10.7 ms of a 15 ms frame** at 20×, which is
  more than the rasterising this lane exists to avoid. One texture, grown to the
  largest sheet a frame has needed, took the same frame to 1.9 ms. It is still charged
  to every frame that uses it: what a frame *needs* is what a budget is about, not what
  happens to be resident.
- **The conversion tolerance is relative to the outline, not absolute.** An absolute
  1e-4 gave a 14-unit glyph the allowance of a 600-unit page border — 32 quadratics per
  cubic where 4 are inside a thousandth of the glyph.

## Where the lanes differ, and which is right

Held against each other in `tests/coverage_lanes.rs`:

- **Where no edge crosses a pixel, they agree exactly.** 255 is 255 and 0 is 0 in both;
  a difference there would be a defect, not a quantisation.
- **On a straight edge they differ by at most 32 of 255**, which is the sample grid and
  nothing else: four columns at ⅛, ⅜, ⅝, ⅞ answer any edge to within an eighth of a
  pixel. The fixture measures 12.
- **On a curved edge they differ by up to 96, and the CPU lane is the one that is
  wrong.** `raster.rs` flattens to `FLATTEN_TOLERANCE` — a quarter pixel — and a chord
  cuts *inside* a convex curve, so its shape is up to that much smaller than the one
  the caller submitted. The GPU lane does not flatten: it draws the quadratics the
  upload converted to, within 8×10⁻⁵ units of the cubic (`outline.rs`'s own test).
  Tightening `FLATTEN_TOLERANCE` to 0.004 takes the worst difference from 62 to *zero
  pixels over 20*, which is what identifies the flattening as the whole of the
  disagreement rather than something in the new lane.

That last one is the honest summary of the trade: the GPU lane is **more** accurate
about the shape and **less** accurate about the pixel.

## What the lane does not do

- **Residue clips take the CPU lane.** A non-rectangular clip multiplies into coverage
  bytes on the CPU (`residue_product`), and there is no pass that does the same on the
  device. Such a command falls back, and both kinds of tile land on one sheet — the
  upload writes bytes, the passes draw beside them, and each tile's quad covers only
  its own rectangle so neither disturbs the other. `one_sheet_carries_both_producers`
  is the test that proves it.
- **No atlas stands in front of it**, which is most of why 1× costs what it does. A
  cache keyed on the device transform is exactly what a zoom gesture defeats, so the
  lane was built without one on purpose; whether a *scale-independent* cache (tiles
  keyed by outline and sub-pixel phase alone) could pay for itself is an open question
  and would be its own ADR.
- **Soft masks and layers are untouched.** They consume coverage; they do not produce
  it.

## What holds it

`src/outline.rs`'s tests: a degree-elevated quadratic converts back to its own control
point; an open contour gains its closing chord (§8.5.3.1); a contour that cannot
enclose area emits nothing, with the rule stated geometrically rather than guessed; a
chord triangle's implicit coordinates can never discard it. `src/winding.rs`'s tests:
the four above, plus the sample grid being ordered and balanced about the pixel centre.
