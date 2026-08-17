# Four of the caller's hayro reading list, answered and gated

`doc/notes-hayro-coverage-map.md` is the coverage map; this file is the round that closed
four of its **What is open** entries — §8.7.4.3's coordinate space on a stroke and on a
glyph (their #968 and #102), thin marks (#104, #1023), a mesh drawn as the raster it is
(#3), and a function paint evaluated rather than baked (#551).

Written 2026-08-17. The standing instruction it answers is the project owner's: *make a
unit test for every issue the caller's file names, even where we already do it right.* All
four answers are "we already do it right", and **one of them is only right on one of the two
lanes** — §2 below is the finding, and it is worth more than the rest of the round.

Twenty-five tests in four new files, no source change. Every gate was verified able to fail
by planting the defect it exists to catch, named per question below. The device-touching
files were run on both adapters.

---

## 1. A shading is anchored to the page, not to the mark (#968, #102)

**Gate:** `crates/quorra-gpu/tests/shading_space.rs`, seven tests.

### The clause, and a correction to carry back

The caller's document cites §8.7.4.3 for the rule. That subclause is *Shading dictionaries*;
its NOTE 2 **names** the space — "The term target coordinate space, used in many of the
following descriptions, refers to the coordinate space into which a shading is painted" —
and then points elsewhere for what it is. The sentences that decide anything are in
**§8.7.2**, *General properties of patterns*:

> Every pattern has a pattern matrix , a transformation matrix that maps the pattern's
> internal coordinate system to the default coordinate system of the pattern's parent
> content stream (the content stream in which the pattern is defined as a resource). The
> concatenation of the pattern matrix with that of the parent content stream establishes
> the pattern coordinate space , within which all graphics objects in the pattern shall be
> interpreted.

> Changes to the page's transformation matrix that occur within the page's content stream,
> such as rotation and scaling, have no effect on the pattern; it maintains its original
> relationship to the page no matter where on the page it is used.

and in **§8.7.4.1**, which is where the independence is stated for the painting operators
the two issues are about — `f` for #968's stroke witness, `Tj` for #102:

> By setting a shading pattern as the current colour in the graphics state, a PDF content
> stream may use it with painting operators such as f (fill), S (stroke), Tj (show text),
> or Do (paint external object) with an image mask to paint a path, character glyph, or
> mask with a smooth colour transition. When a shading is used in this way, the geometry of
> the gradient fill is independent of that of the object being painted.

The caller's substantive point is exactly right; only the citation is one subclause off.
This is the same shape as the two corrections the map already carries.

### The answer

Integration note 9's shape holds. `Paint::Shading { ramp, kind, transform }` carries the
sweep in the shading's own space and `encode/rare.rs`'s `rare_paint` composes **only**
`transform` with the viewport — the shaded command's own transform is never composed in,
and `rare_paint`'s doc comment says so in as many words. The same is true of
`Paint::Function`'s `matrix` and of `Paint::Mesh`, which carries no matrix at all.

### The gate

Each test draws **the same device pixels three ways** — by the outline's own coordinates,
by a command translation, and by a translation composed with a scale — and requires the
three frames to be identical *to the byte*. That is stronger than any tolerance and it is
the clause read literally: the placement changed, the pattern did not. It is applied to a
rect-hinted fill (analytic coverage), a rasterised fill, a stroke (#968) and a repeated
glyph-sized outline (#102), with the ramp's value at each device column checked against
§8.7.4.5.3's projection plus §7.10.3's type 2 with N = 1.

### What #102 turns out to mean here, and the lane trap

**A gradient-filled glyph is not a glyph-lane mark in this tree.** `encode/fill.rs` resolves
a non-solid paint before any cache is consulted, so a shaded fill goes to the rare lane over
a rasterised coverage tile and never reaches the atlas. The gate asserts that in both
directions, which is what `doc/HANDOVER.md`'s ADR 0047 trap asks for: four placements of one
outline **filled solid** are `atlas_distinct_keys == 1` (the control — this outline at this
size genuinely is a glyph-lane mark), and the same run under a shading is
`atlas_distinct_keys == 0` with `tiles == 4`.

That is a cost, not a defect: a page whose text is gradient-filled pays a coverage tile per
glyph. It is also what the caller's own backends do, and nothing in §8.7.4.1 requires
otherwise. It is worth knowing before somebody measures a page of shaded display type.

### Verified able to fail

| defect | fails |
|---|---|
| compose the command's transform into the paint at `encode/fill.rs` and `encode/stroke.rs` | 5 of 7 |
| sample the ramp at the quad's corner rather than the fragment (`shading.wgsl`) | the two-mark continuity test |
| refuse every atlas admission (`atlas.rs`'s `prospect`) | the glyph-lane control |
| bake a shading into a flat colour in `encode_fill` | the glyph-lane assertion |

---

## 2. Thin marks — and the two lanes do **not** agree (#104, #1023)

**Gate:** `crates/quorra-gpu/tests/thin_marks.rs`, seven tests.

### What §10.7.4 supports, and what it does not

The caller's §2 is the document's most interesting entry and it invites an expectation the
clause does not carry. §10.7.4's rule, verbatim:

> A shape shall be scan-converted by painting any pixel whose half-open square region
> intersects the shape, no matter how small the intersection is. This ensures that no shape
> ever disappears as a result of unfavourable placement relative to the device pixel grid,
> as might happen with other possible scan conversion rules. The area covered by painted
> pixels shall always be at least as large as the area of the original shape. This rule
> applies both to fill operations and to strokes with non-zero width. Zero-width strokes may
> be done in an implementation-defined manner that may include fewer pixels than the rule
> implies.

Read literally that is **binary** — the pixel is *painted*, at the current colour — and it is
written for a device deciding painted-or-not. It says nothing about proportional coverage.
Two other clauses supply what it leaves out. §10.7.1's NOTE:

> The specifics of the scan conversion algorithm are not defined as part of PDF. Different
> implementations can perform scan conversion in different ways; techniques that are
> appropriate for one device could be inappropriate for another.

and §11.3.7.2's NOTE 1, which is where fractional coverage is given a meaning at all:

> Mathematically, elementary objects have "hard" edges, with a shape value of either 0.0 or
> 1.0 at every point. However, when such objects are rasterized to device pixels, the shape
> values along the boundaries can be anti-aliased, taking on fractional values representing
> fractional coverage of those pixels. When such anti-aliasing is performed, it is important
> to treat the fractional coverage as shape rather than opacity.

So the gate asserts the two things §10.7.4 decides — **the mark does not disappear**, and
**the ink is at least the shape's own area** — and asserts proportionality separately, as
ADR 0005's choice rather than as a normative requirement. Nothing in the file claims the
clause requires what it does not.

### The finding: `Coverage::Gpu` lets a hairline vanish

`Coverage::Cpu` computes the exact area a shape covers of each pixel. `Coverage::Gpu` counts
samples on an ordered 4 × 4 grid (`DEFAULT_COVERAGE_SAMPLES` = 16), so its sample columns sit
a quarter of a pixel apart. **A mark narrower than that spacing can fall entirely between two
columns and read zero.**

A 0.1-device-pixel vertical bar, 768 pixels tall, swept across ten sub-pixel positions —
total ink per row, in whole pixels:

| left edge | `Coverage::Cpu` | `Coverage::Gpu` |
|---|---|---|
| 20.0 | 0.10196 | **0** |
| 20.1 | 0.10196 | 0.25098 |
| 20.2 | 0.10196 | **0** |
| 20.3 | 0.10196 | 0.25098 |
| 20.4 | 0.10196 | **0** |
| 20.5 | 0.10196 | **0** |
| 20.6 | 0.10196 | 0.25098 |
| 20.7 | 0.10196 | **0** |
| 20.8 | 0.10196 | 0.25098 |
| 20.9 | 0.10196 | **0** |

Six of ten positions draw nothing at all; the other four draw two and a half times the ink the
shape has. The processor lane reads the shape's exact area at every position. **The two
tables are byte-identical on llvmpipe and on RADV**, so this is the design and not an
adapter.

This is precisely the disappearance §10.7.4's rule exists to forbid, and it is reachable on a
real page: a long thin rule is exactly the shape `take_gpu_lane` prefers, because its tile
area beats its triangle cost. Raising `coverage_samples` does not remove it — it halves the
width at which it starts for every quadrupling of the sample count. Only an area rule
removes it.

**Reported, not fixed**, per the round's instruction. The gate records it as a
characterisation with the clause quoted above it, so a fix moves a test rather than nothing:
`the_device_lane_lets_a_mark_between_two_sample_columns_vanish` fails if the gap ever closes
on its own, and its message says to turn it into the requirement rather than delete it
silently. A recommended `PLAN.md` bullet is at the end of this file.

### Their #1023, and the 8-bit floor

- A stroke of a stated device width lays down exactly that much ink — widths 3, 1, 0.75,
  0.5, 0.25 and 0.1 device pixels at viewport scales 1, 2 and 4, to within 2/255. The width
  is device-space and already resolved upstream (§8.4.3.2 with §10.7.5, §4.5 of the brief),
  so it does **not** follow the viewport. That is #1023's question answered.
- Both lanes finally lose a mark below **1/510 of a device pixel**, where the coverage rounds
  below half a level of ADR 0006's 8-bit store. That is a cost written down rather than a
  requirement met; the gate names the constant so a change to the store moves a test.
- The **abutting**-marks half of conflation is not tested and not solved here. The caller
  says plainly they have not solved it either.

### Verified able to fail

| defect | fails |
|---|---|
| round the analytic lane's coverage to 0 or 1 (`rect.wgsl`) | both analytic tests |
| the same in the CPU rasteriser (`raster.rs`'s `fill_mask`) | the rasterised, the stroke and the cross-lane tests |
| make `take_gpu_lane` always decline | the lane control **and** the recorded gap |
| halve the winding resolve (`winding.wgsl`) | the device-lane clause test |

The lane control deserves a note of its own: `the_device_lane_really_is_what_draws_the_tall_bar`
proves the fixture reaches the device lane by the budget trick `tests/coverage_lanes.rs`
uses — only that lane allocates the `rgba16float` winding texture, so a `max_frame_bytes` the
processor lane fits and it does not separates them by construction rather than by comparing
the pixels under test. Without it the comparison above would be one lane against itself,
which is the `m45.rs` trap.

---

## 3. A mesh is drawn as the raster it already is (#3)

**Gate:** `crates/quorra-gpu/tests/mesh_raster.rs`, seven tests.

Integration note 5 is the promise: we consume the caller's pre-rasterised mesh and never
re-triangulate it, because neither rasteriser has §8.7.4.5.5–.8's primitive and a second copy
would drift. Their #3 — Coons and tensor patches tessellated at a fixed grid, triangles
seaming at their shared edges — is what happens on the other side of that decision, and it is
settled upstream for us. Which is exactly why our half needed a gate rather than a paragraph.

In pixels the promise reduces to: **the samples uploaded are the samples drawn, at the device
pixels the upload named.** The gate says so seven ways — texel-for-texel reproduction at the
anchor; hard edges, with the first and last texels asserted present as the control for the
four absences beside them; no stretch under viewport scales 1, 2 and 4 (note 5's stated cost
drawn rather than argued — a `MeshRaster` is device-resolution, so a zoom re-uploads it); no
movement under the mark's own transform; the same anchor through the rasterised branch as
through the analytic one; a two-colour checkerboard whose target holds exactly those two
colours, which is what "no re-triangulation and no resampling" reduces to; and a sample's own
alpha reaching the target as shape per §11.3.7.2's NOTE 1.

### Verified able to fail

| defect | fails |
|---|---|
| average each mesh texel with its right neighbour (`shading.wgsl`) | 6 of 7 |
| force a sample's alpha to 1 | the seventh |
| multiply the mesh anchor by the viewport's scale (`encode/rare.rs`) | the viewport test **alone** |

The third is the one worth keeping: it fails exactly one test and nothing else, which is what
says that test is about the thing it names.

---

## 4. A function paint is evaluated, not baked (#551)

**Gate:** `crates/quorra-gpu/tests/function_resolution.rs`, four tests.

Their #551 is a shading a back end cannot express, rasterised "at a fixed low resolution
regardless of the output size", and the caller names the general shape: a paint the target
cannot express is baked at some resolution and nothing tells the baker what resolution to
use. ADR 0053's answer is that a §7.10.5 program is uploaded once, generated into a shader,
and run per fragment.

The claim worth gating is **resolution independence**, and it is not the same claim as "the
paint works": a baked grid draws a plausible picture at every scale and is wrong only in the
detail, which is §5's plausible-looking wrong page. So the assertions are ones a bake could
not pass.

§8.7.4.5.2 puts `Domain` in the shading's own space and `Matrix` from there into the target
space; §10.7.4 says the point a pixel is coloured from is its centre — "The position of the
centre of such a pixel -in other words, the point whose coordinate values have fractional
parts of one-half -shall be mapped back into source space" — so the value at device column
`i` under viewport scale `s` is the program's value at `x = (i + 0.5) / (32·s)`.

- **Every** device pixel carries that value, at scales 1, 2 and 4.
- The count of distinct bytes across the shading is 32, 64 and 128 — magnification *adds*
  detail rather than repeating it, which a grid baked at 1× cannot do.
- A discontinuity whose scene position is deliberately off the integer grid (shading `x = 0.5`
  at scene `x = 16.25`) lands at device column **16, 32 and 65**. A grid baked before the zoom
  existed would still say 64. One pixel, and it is the whole difference between evaluating and
  baking.
- The control, for `HANDOVER.md`'s newest trap: a dangling program identifier is refused by
  name, so the three assertions above are known to reach the function lane rather than a
  quieter fallback.

Both adapters, since ADR 0053 does not promise cross-adapter identity for a generated shader:
llvmpipe and RADV agree on all four. `tests/function_coverage.rs` already settles that this
paint's coverage is the processor's under either `Coverage` setting, and nothing here re-asks
it.

### Verified able to fail

| defect | fails |
|---|---|
| quantise the shading-space input to a fixed 31-cell grid (`function_lane.wgsl`) — #551's own shape | all three assertions |
| return a hole instead of `RenderError::UnknownFunction` (`encode/function.rs`) | the control |

---

## What turned out settled upstream, and where

- **The stroke's device width** — §8.4.3.2 with §10.7.5, resolved by the caller per placement
  (§4.5 of the brief). We take the width we are given, which is why #1023's question is
  answerable here at all: the gate asserts the width survives our expansion and the viewport,
  not that we chose it.
- **Hairline snapping** — `pdf_render::split_collapsed_fill` snaps a *collapsed* fill to whole
  device pixels, on the caller's side, so that both their backends inherit one answer
  (`QUORRA_HAIRLINE_MARKS.md`, answered in their 368th session). What reaches us is either a
  snapped whole-pixel run or an ordinary thin rectangle; our gate is about the second.
- **Mesh tessellation density and triangle seaming** — both of #3's named consequences are
  the caller's, taken once upstream so that a second copy cannot drift (integration note 5).
- **Colour** — a shading's ramp arrives sampled to stops with colours already device RGB
  (integration note 6), so nothing in §8.7.4.4's interpolation-space rules is ours.

## What was deliberately not done

- **The GPU lane's disappearance was not fixed.** It is a scan-conversion design decision
  with a clause behind it and a cost either way, so it is an ADR and a round of its own, not
  a quiet edit inside a gating round. §2's table is what that round starts from.
- **The abutting-marks case** is untested. It is the half of conflation the caller has not
  solved either, and a fixture for it would assert something nobody has decided.
- **Citations in the tree were not rewritten.** `paint.rs` and `encode/rare.rs` cite
  §8.7.4.3 for the shading matrix, which is the same near-miss as the caller's. The new test
  file states §8.7.2 and §8.7.4.1 with the text; a sweep of the source comments is a small
  separate change and `src/encode/` has another agent in it this round.
- **`/Interpolate` (#1310)** is the map's third open entry and was not in this round's four.

## Recommended edits, as quoted text

For `doc/PLAN.md`, **What is still open**, a new bullet:

> - **A mark thinner than a quarter of a device pixel can vanish on `Coverage::Gpu`.** The
>   device lane counts samples on an ordered 4 × 4 grid, so a mark narrower than the column
>   spacing falls between two columns and reads zero — six of ten sub-pixel positions for a
>   0.1-pixel bar, where the processor lane reads the shape's exact area at all ten, and the
>   two tables are byte-identical on llvmpipe and RADV. §10.7.4's rule exists to forbid
>   exactly that: painting every pixel the shape intersects "ensures that no shape ever
>   disappears as a result of unfavourable placement relative to the device pixel grid". No
>   sample count removes it; only an area rule does. Recorded and gated as a characterisation
>   in `tests/thin_marks.rs`, with the numbers in `doc/notes-hayro-paints.md` §2; the fix is
>   an ADR of its own.

For `doc/HANDOVER.md`, **Traps**, a new entry:

> **A sampled coverage rule and an area coverage rule disagree about what is *there*, not
> only about how much.** `Coverage::Gpu`'s 4 × 4 grid gives a 0.1-pixel bar either 0 or 0.25
> depending on where it lands; `Coverage::Cpu` gives it 0.1 everywhere.
> `tests/coverage_lanes.rs` bounds the two lanes at an eighth of a pixel *for an edge crossing
> a pixel*, which is a true statement about a shape wider than the grid and says nothing at
> all about one narrower than it. When a lane comparison is bounded, check what the bound was
> derived for before believing it covers a new shape.

For `doc/notes-hayro-coverage-map.md`, the **What is open** list — items 1, 2, 4 and 5 are
closed:

> ## What is open, in the order it is worth doing
>
> 1. **`/Interpolate` honoured, never overridden** (#1310) — integration note 1's whole point.
> 2. **No CMS is reachable** (#205 family) — a dependency assertion, cheap.
> 3. **Banding under one 8-bit level** (#60).
> 4. **`encode_threads` nested, and `Scene: Send + Sync` still asserted** (#1316, #1343).
> 5. **A shading with a transparency component** (#41).
>
> Closed 2026-08-17 by `doc/notes-hayro-paints.md`: §8.7.4.3's coordinate space on a stroke
> and on a glyph (`tests/shading_space.rs`), thin marks (`tests/thin_marks.rs`, and the
> `Coverage::Gpu` disagreement it found), a mesh drawn as the raster it is
> (`tests/mesh_raster.rs`), and a function paint evaluated rather than baked
> (`tests/function_resolution.rs`).

And in that file's **§3 — shadings** table, the #968 row's citation:

> | #968 — a gradient **clipped incorrectly on a stroke** | **§8.7.2**, not §8.7.4.3 — see correction 3 below. A shading's coordinates are the space of the page at the time the pattern's parent content stream began, *not* the space in force when the paint is used | `tests/shading_space.rs` |

with a third correction beside the two already there:

> 3. **Their §3 cites §8.7.4.3 for a shading's coordinate space.** §8.7.4.3's NOTE 2 only
>    *names* that space — "The term target coordinate space, used in many of the following
>    descriptions, refers to the coordinate space into which a shading is painted" — and points
>    at §8.7.2 for what it is. The sentence they want is §8.7.2's: "Changes to the page's
>    transformation matrix that occur within the page's content stream, such as rotation and
>    scaling, have no effect on the pattern; it maintains its original relationship to the page
>    no matter where on the page it is used." §8.7.4.1 states the same independence for the
>    painting operators, naming `f`, `S` and `Tj` together — which is why one gate covers
>    #968 and #102 both. Their substantive point stands unchanged.
