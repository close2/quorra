# The sampled lane's quantum, and the offset that was never per command — round notes, 2026-08-23

Opened to answer the two questions the caller's `QUORRA_FEEDBACK.md` §31 asks. **Both are
answered**, and neither answer is the one the question expected:

- **Question 2** — is the sampled lane's y coverage quantised, and to what — yes, to
  `1/√coverage_samples` of a device pixel, 0.25 at the default sixteen. Their witness value
  0.753 is 192/255, which is three sample rows plus four byte roundings, exactly. The answer
  that matters is not the number: the quantisation **breaks ISO 32000-2 §10.7.4's first
  sentence**, per pixel — at **a quarter of all sub-pixel placements** at the default sample
  count a pixel the shape reaches is left entirely unpainted — and no widening of ADR 0070's
  condition can reach it. ADR 0076.
- **Question 1** — is the default lane's per-command offset intended — **it did not
  reproduce, and their own published table says it is not per command.** Two free values fit
  all six of their numbers to 0.0014 of a pixel. What their table shows is a difference in
  the *device transform* the two backends were handed, of about one part in nine hundred plus
  a sixth of a pixel — and their sampled-lane column is that same geometry put through the
  quantiser above, which is why it looked exact.

And a third thing, which was found by trying to make this round's own new assertion fail:
**yesterday's round had a hairline on the sampled lane and read it as the processor's**,
because `LaneCounts::path` is the name of both rasterisers. §1.1.

The instrument is `crates/quorra-gpu/examples/lane_placement/`, which this round split into
four modules and gave two more phases.

## 1. The correction this round was opened for, verified in the source

Yesterday's note (`doc/notes-glyph-phase-carry.md` §3) claimed that on a default atlas the two
coverage settings are the same lane for a hairline, "because `take_gpu_lane` declines the
device lane for anything `worth_caching`". The caller's §37.4 says that is true of a solid
fill and false of a stroke. **It is.** `Encoder::push_coverage_styled` passes
`CacheProspect::TooLarge` at the call site — its own comment says why, "the atlas caches
outlines by key, not polylines" — and `CacheProspect::worth_caching` answers `false` for
`TooLarge` unconditionally. So that condition cannot decline anything for any stroke.

> **Names, after ADR 0075.** This round was measured against `take_gpu_lane` and rebased onto
> `ae03b55`, where lazy quadratics split that function at the seam it already had:
> `Encoder::gpu_lane_admissible` is the cheap four (setting, residue chain, atlas prospect,
> ADR 0070's thin axis) and `Encoder::triangles_under_coverage` is the byte comparison below.
> **The arithmetic is identical**, and every number in this note was re-measured after the
> rebase and did not move. The `worth_caching` clause above is now
> `gpu_lane_admissible`'s third; the floor below is `triangles_under_coverage`.

What kept the previous round's hairline off the sampled grid was the **triangle floor**, now
`Encoder::triangles_under_coverage` and then the last clause of `take_gpu_lane`:

```rust
let area = u64::from(width).saturating_mul(u64::from(height));
let triangle_bytes = (triangles as u64) * 3 * crate::outline::WindingVertex::STRIDE;
area >= triangle_bytes
```

`STRIDE` is 32, and `width`/`height` here are `tile_side` over the mark's **own unclipped
device bounds**, not over the visible tile. A stroke's expansion is four points and so four
triangles (`append_polyline_triangles` fans one per edge), which is 384 bytes; the six-point
band the instrument fills is six triangles, 576. A rule one device pixel thick has a box of
`length × 2` texels. **So a mark shorter than 192 device pixels can never take the sampled
lane, and one shorter than 288 can never take it as a fill, whatever the setting says.**

The fixture's `ALONG` is 512 now, and that one number is the whole of what made this round's
measurements possible. It is documented as a lane condition rather than as a canvas size,
because that is what it is.

### 1.1 And the previous round *did* reach the sampled lane, without knowing it

This was found by forcing `ALONG` back to 128 to check that the reachability assertion could
fail — it did not, and the reason is worth more than the assertion was.

The rule runs past both edges of the target, so its length is `2 × (ALONG + 4)` and its box is
`4 × (ALONG + 4)` texels. At the previous instrument's 128 that is **528**, which is exactly
the number `doc/notes-glyph-phase-carry.md` §3 quotes — and 528 clears the **stroke's** 384
while failing the **fill's** 576. So of that round's four rows, the fill rows were answered by
the processor under both settings and **the stroke row was not**: its sampled column was the
winding lane all along. Measured here at `ALONG` = 128, seven positions:

```
  offset    geometry    cpu lane     error   lane    gpu lane     error   lane
  0.1429     80.1429     80.1431   +0.0003   path     80.2500   +0.1071   path
  0.2857     80.2857     80.2843   -0.0014   path     80.2500   -0.0357   path
  0.4286     80.4286     80.4294   +0.0008   path     80.5000   +0.0714   path
  0.8571     80.8571     80.8569   -0.0003   path     80.7500   -0.1071   path
```

Two rasterisers, one lane **name**: `LaneCounts::path` counts the processor's tiles and the
winding lane's together, by design and with a documented reason, and there is no counter that
separates them. That is what turned "the mark took the path lane under both settings" into
"the two settings are the same rasteriser for it", and it is why this round's reachability
check is a *behavioural* one — the ink and the placement against the grid's own pitch — rather
than a counter read.

The correction to yesterday's note is therefore in two parts, not one: its first bullet is
wrong about *why* the settings agreed (the caller's §37.4), and its third bullet — "this round
did not get a hairline onto [the sampled lane]" — is wrong that they did agree. The hairline
was on it, in the row the note called exact.

## 2. The instrument

`examples/lane_placement/` — `main.rs` (the phases and the assertions), `fixture.rs` (the
pictures and the devices), `measure.rs` (what is read off a raster), `witness.rs` (arithmetic
on the caller's published table). Four phases:

| phase | picture | question |
|---|---|---|
| 1 placement | one rule, swept through a whole pixel of position | both lanes' placement |
| 2 grid | one rule of a width the sample lattice does not divide, at three sample counts | 2 |
| 3 graph paper | six rules, six commands, at the caller's own pitch and CTM | 1 |
| 4 witness | their §31.2 table, put through this lane's own grid arithmetic | 1 |

Measured on llvmpipe, 37 positions per row plus ADR 0073's carry position, target 512 × 128.

## 3. Question 2 — the quantum, and what it breaks

### 3.1 The arithmetic, from the code

`winding::sample_offsets` puts `n` samples on a `√n × √n` grid, the k-th row at `(k + ½)/√n`
of a pixel. Across the device those rows are **one lattice of period `p = 1/√n`**.
`winding.wgsl`'s `fs_resolve` counts the samples of a pixel the fill rule admits and stores
`covered / n`. For an axis-aligned band that gives three consequences, none of them empirical:

- its **ink** is `k · p`, `k` being the lattice points inside it — `⌊w/p⌋` or `⌈w/p⌉`;
- its **centroid** is the plain mean of those points and can be nothing else;
- a pixel row holding none of them receives **exactly zero**.

### 3.2 The sweep

A 0.878-device-pixel rule — the caller's `issue16500.pdf` witness total — as a stroke, swept
through one pixel of position, both settings side by side:

| samples | pitch `p` | distinct sampled inks | worst ink error | worst placement error |
|---:|---:|---|---:|---:|
| 4 | 0.5 | 0.5020, 1.0039 | **−0.3760** | −0.3108 |
| 16 | 0.25 | **0.7529**, 1.0039 | +0.1259 | +0.1757 |
| 64 | 0.125 | 0.8784, 1.0039 | +0.1259 | −0.1216 |

The processor column reads 0.8784 at every position of every row, which is 0.878 met by an
eight-bit store.

Two rungs at every count, one pitch apart, on the lattice `p·k`. **0.7529 is 192/255 — the
caller's 0.753 to the byte.** And the row split at the positions that produce it, from the
sixteen-sample sweep:

```
  offset   cpu ink      row    row+1   gpu ink      row    row+1  centroid
  0.3243    0.8784   0.1137   0.7647    0.7529   0.7529   0.0000   +0.1757
  0.3514    0.8784   0.0863   0.7922    0.7529   0.7529   0.0000   +0.1487
  0.3784    0.8745   0.0588   0.8157    0.7529   0.7529   0.0000   +0.1216
```

which is their table, reproduced from our own arithmetic:

| | row 141 | row 142 | total |
|---|---|---|---|
| their oracle | 0.439 | 0.439 | 0.878 |
| their gpu lane | **0.753** | **0.000** | **0.753** |

### 3.3 The bound is one pitch, and the derivation that said half was corrected by the run

The first version of this round's assertion said half a pitch. The four-sample column
contradicted it at −0.3760 and the arithmetic explains why: `k` is `⌊w/p⌋` or `⌈w/p⌉` and
those are a whole pitch apart, so the error reaches `p·⌊w/p⌋ − w`, which for a band just over
a pitch is nearly a whole pitch. Half a pitch is right only where `p` divides `w` — the count
is then fixed and only the lattice's phase moves — which is the case a hairline of exactly one
device pixel is, and is what phase 1 measures at ±0.1216.

Writing this down because it is the round's own instance of the thing it is about: a bound
derived on paper, contradicted by a sweep at a sample count the derivation had not been run
at.

### 3.4 What it breaks

ISO 32000-2 §10.7.4, verbatim, with the NOTE that makes the first sentence bite at a
boundary:

> A shape shall be scan-converted by painting any pixel whose half-open square region
> intersects the shape, no matter how small the intersection is.

> NOTE 1 … for purposes of scan conversion, a filling region is considered to intersect every
> pixel through which its boundary passes, even if the interior of the filling region is
> empty.

At 0.878 pixels of width and sixteen samples, the sampled lane leaves **a pixel row whose
exact area inside the shape is 0.1137 at exactly zero** — the row split above, where the
processor lane inks two rows and the sampled lane one. The band's boundary passes through
that pixel. It is not painted. That fails under the clause's own binary vocabulary and under
the anti-aliased reading alike.

Counted over the sweep, per sample count:

| samples | pitch `p` | positions with a pixel the shape reaches and the lane did not paint |
|---:|---:|---|
| 4 | 0.5 | **19** of 38 |
| 16 | 0.25 | **10** of 38 |
| 64 | 0.125 | **5** of 38 |

**The fraction is the pitch itself**, and it is arithmetic rather than a fit: a pixel row
holds no lattice point when the band's part in it is shorter than the distance from the
boundary to the first sample row, `p/2`, and a band has two edges — so `2 · p/2 = p` of every
whole pixel of placement. **A quarter of all sub-pixel placements at the default sixteen.**
That is a much larger population than "a hairline at an unfavourable phase", which is what a
reader of §31 would have guessed, and it is the number this round would put first.

The clause's third sentence — "[t]he area covered by painted pixels shall always be at least
as large as the area of the original shape" — fails under the anti-aliased reading this tree
took in `tests/thin_marks.rs` and in ADR 0070: 0.7529 for 0.878. Under the binary reading it
passes vacuously.

**The mark does not disappear**, which is what ADR 0070 bought, and the caller's own sentence
— "it does not disappear, so §10.7.4 is not broken" — is right about that and wrong about the
clause. The difference is that the clause is stated per *pixel* and their reading was per
*mark*.

### 3.5 Why no threshold fixes it

The ink error is `p·k − w`, whose magnitude reaches nearly `p` for any `w` just above a
multiple of `p`, **independently of `w`**. A ten-pixel band can draw 9.75 as readily as a
one-pixel band can draw 0.75. Raising ADR 0070's threshold bounds the error only *relative* to
the mark — five per cent needs a five-pixel threshold, which is every rule and most glyphs on
the processor lane and the sampled lane's reason for existing gone.

More samples does not fix it either, and the table above is the evidence: at sixty-four
samples the worst error is still +0.1259, because the failing rung is `⌈w/p⌉·p − w` and that
one does not shrink with `p` for a fixed `w`.

The only removal is the area rule ADR 0070 priced as a milestone. ADR 0076 declines it again,
records the bound, and says what would overturn the decision.

### 3.6 A second, smaller thing: the byte is written once per group of four

`winding.wgsl` claimed "the single quantisation to a byte happens once, at the store". It does
not. The sheet is `r8unorm` and the resolve pass runs once per group of four samples with
additive blending, so a frame of `n` samples pays `n/4` roundings. Measured at sixteen:

- three sample rows read **192/255 = 0.7529**, where one quantisation of ¾ gives 191/255 =
  0.7490. The measurement says 192, so the roundings compound.
- a full pixel reads **1.0039**, one level over.

Bounded by half a level per group, `n/8` levels in all — four hundredths of a pitch at the
default, thirty times smaller than the quantum above it. The comment is corrected; the
arithmetic is not changed, because the cost of changing it is a wider sheet and the quantum
above it dominates by thirty to one.

## 4. Question 1 — the per-command offset, which is not per command

### 4.1 It did not reproduce

Phase 3 builds their construction as closely as this tree can state it: six rules, **one
`SceneBuilder::stroke` per rule** so that each is its own drawing command, each carrying its
position through its own `Affine::scale(0.317180616).then(translate)` the way a `q … cm … S …
Q` does, at the pitch their §31.2 derives from the document. Both axes, and two device widths
— theirs literally (0.5 user units under that CTM is **0.1586** device pixels) and their
prose's ("about one device pixel wide").

**One correction to that pitch, and it is theirs.** §31.2 writes "`52.0277778 ×
0.317180616 = 16.5013`". The product is **16.5022026**. The difference is 0.0009 per rule and
0.0045 over the six, which is small but not nothing in a table whose subject is a
hundredth-of-a-pixel drift — and it makes their oracle's measured pitch of 16.500 out by
0.0022 per rule rather than by 0.0013. This instrument uses the product rather than the
printed figure.

At one device pixel, the default lane:

```
 rule    geometry    cpu lane     error    gpu lane     error
    0     33.0000     33.0000   +0.0000     33.0000   +0.0000
    1     49.5022     49.5039   +0.0017     49.5000   -0.0022
    2     66.0044     66.0059   +0.0015     66.0000   -0.0044
    3     82.5066     82.5078   +0.0012     82.5000   -0.0066
    4     99.0088     99.0098   +0.0010     99.0000   -0.0088
    5    115.5110    115.5118   +0.0007    115.5000   -0.0110
cpu lane: measured pitch 16.5024 against the document's 16.5022
gpu lane: measured pitch 16.5000 against the document's 16.5022
```

**Our default lane is exact to 0.0017 device pixels** — a byte, not a placement. Their table
has it out by up to 0.122. Identical in the other axis.

The sampled column is worth reading too: it reports a pitch of **exactly 16.5000** for a
document that says 16.5022, with the error growing to −0.0110 over six rules. That is §3's
quantiser, and it is the shape a reader would call "a scale error" — which is the shape their
table attributes to our *default* lane.

### 4.2 At the width their sentence's own arithmetic gives, the two settings are one lane

At 0.1586 device pixels both columns are identical to the fourth decimal and both report the
`path` lane, because ADR 0070's fifth condition diverts anything below `p = 0.25` to the
processor under either setting. So **whatever the rules on `bug1743245.pdf` are, they are not
0.1586 device pixels wide** — at that width their two lanes could not differ at all, and their
§37.4 measures them differing by a mean of 2.5978. Either their tree resolves the width
differently (a minimum device width applied to hairlines would do it) or the marks are not
those strokes.

That width is not otherwise clean, and it is worth recording: the processor lane's centroid
for a 0.1586-wide band is out by up to **+0.0412** device pixels, purely from rounding a
coverage of 0.08 to a byte. Small, real, and the same on both settings.

### 4.3 What their own table says, which settles the shape of it

Phase 4 is arithmetic on their published §31.2 numbers, run on every invocation of the
instrument so that it is a measurement rather than a paragraph. Two results.

**Their sampled column is their default column put through this lane's grid.** Applying the
lattice mean of §3.1 — the only centroid the sampled lane can produce for a one-pixel band —
to each of their default-lane numbers:

```
 rule      oracle    default    sampled     lattice  residual
    0      33.000     33.122     33.000      33.000   +0.0000
    1      49.500     49.602     49.500      49.500   +0.0000
    2      66.000     66.083     66.000      66.000   +0.0000
    3      82.500     82.567     82.500      82.500   +0.0000
    4      99.000     99.047     99.000      99.000   +0.0000
    5     115.500    115.531    115.500     115.500   +0.0000
```

Six for six, to zero. **So both settings were handed the same geometry**, and their
sampled-lane column being "identical to the oracle" is the quantiser landing back on the
oracle's values because those values are all on the lattice's own mean set. The same thing
happens in our phase 3 at §4.1, where the sampled lane snaps 16.5022 to 16.5000.

**Their default column is their oracle column under one affine.** Least squares over the six
pairs:

```
scale 0.998899, offset +0.1571 px, worst residual 0.0014 px
```

Two free values fit six commands to a byte. **A per-command quantiser cannot do that** — it
has one free value per command and no reason for them to lie on a line. A difference in the
*device transform* the two backends were handed has exactly these two, and 0.998899 is about
1 − 1/908 — the ratio two raster extents one pixel apart would have.

Their own §31.2 says "which is what makes it look like a scale error when the commands are a
regular grid". Read the other way round, it **is** a scale error, and the regular grid is what
made it visible.

### 4.4 What would settle it, and it is one thing only they have

Everything above is inference from six published numbers. What ends the question in a minute
is **the device-space geometry of one of `bug1743245.pdf`'s rules as `lane_diff.rs` hands it
to each backend**:

1. the rule's two endpoints in device pixels, as the display list states them;
2. the resolved `Stroke::device_width` for it (§4.5 settles that upstream, and §4.2 above says
   the answer is not 0.1586);
3. whether it arrives as a stroke or as a filled rectangle;
4. **the device transform each backend is given** — the `quorra_scene::Affine` handed to
   `render-quorra` and the `tiny_skia::Transform` handed to `render-cpu`, printed side by
   side. §4.3 predicts they differ by a scale of about 0.9989 and an offset of about 0.157 px
   in that axis; if they are equal to the bit, §4.3's conclusion is wrong and this tree has a
   defect it has not found.

The fourth is the one that decides it, and it is a two-line `dbg!` in their instrument.

## 5. What changed in the tree

- **`examples/lane_placement.rs` → `examples/lane_placement/`**, four modules, four phases.
  `--check` is intact and the ADR 0073 bound it has always asserted is intact, on the
  processor column where the atlas quantum lives and where it can still fail.
- **`fixture::ALONG` = 512.** The one number that reaches the sampled lane at all (§1).
- **`fixture::device`'s doc comment**, which carried the claim §1 corrects — it said the small
  atlas "is what reaches the sampled lane at all". It now states the fill/stroke split, names
  `push_coverage_styled` as the reason, and points at the triangle floor. The half of the
  original sentence the caller asked to keep is kept: on a page whose marks are cached glyph
  fills the two settings are one rasteriser, so a *page-wide* comparison of them averages
  marks the setting moved with marks it could not.
- **`CHECK_STEPS` = 7.** It was 4, and 0, ¼, ½, ¾ are all multiples of the sampled grid's own
  pitch — every position stood in the same relation to the lattice and phase 2's ladder came
  back with one rung. Third sighting of that trap; see §6.
- **`tests/thin_marks.rs`** gains `OFF_LATTICE_WIDTHS` and
  `the_device_lanes_ink_is_quantised_to_one_sample_row`. Every width the file previously swept
  at or above the sample spacing — 0.75, 0.5, 0.25 — is a multiple of the pitch, so its
  assertion of §10.7.4's area floor on the device lane has never been able to fail.
- **`Coverage::Gpu`'s rustdoc** carries the bound and names the sentence of §10.7.4 the lane
  does not meet, because that enum is where a caller chooses it.
- **`winding.wgsl`'s `fs_resolve` comment** corrected (§3.6).
- **ADR 0076.**

### One thing about verifying it, which cost twenty minutes and is not about this round

`cargo clippy --workspace --all-targets` in this worktree failed with

```
error[E0063]: missing field `alpha_is_shape` in initializer of `quorra_scene::GroupSpec`
  --> crates/quorra-pages/src/build.rs:158:20
```

and **no source in this worktree or in the shared checkout contains that identifier at all.**
A concurrent agent worktree does — it is adding the field — and every worktree here shares
one `CARGO_TARGET_DIR`. My `quorra-pages` was compiled against *their* `quorra-scene`. The
error is reproducible, names a file in this worktree, and is entirely false.

`CARGO_TARGET_DIR=…/quorra-<worktree>` makes it go away, and sccache still serves the
compilations, so the cost is a fingerprint pass rather than a rebuild. Worth knowing before
the next round spends a session bisecting an error in a file it did not touch: **a build
failure naming a symbol that `grep` cannot find in the tree is a shared target directory, not
a defect.**

### The corpus, which this round did not run

Predicted to move **nothing**: no lane condition, no shader arithmetic and no encoder path
changed. A `Coverage::Gpu` column should be byte-identical either side. The prediction is
written down so that it can be wrong.

## 6. The trap, for the third time

- **Round of 2026-08-22**: a 16-step sweep against a 1/16 glyph quantum reported zero error at
  all sixteen positions. `STEPS` became 37.
- **This round, `--check`**: four steps of ¼ against a ¼ sample pitch reported one rung where
  there are two. `CHECK_STEPS` became 7.
- **This round, `tests/thin_marks.rs`**: three widths of ¼, ½ and ¾ against a ¼ sample pitch
  have been asserting §10.7.4's area floor on the device lane, at the only widths where it
  holds, since the file was written.

The third is the one worth carrying, because the first two were sweeps of *position* and this
one is a sweep of *width* — the same trap in a dimension nobody was watching. The general form:
**a fixture whose parameter is a multiple of the grid under test measures the grid's fixed
points, and a fixed point looks exactly like conformance.**
