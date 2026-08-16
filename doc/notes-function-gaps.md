# The three gaps in the function lane's coverage, closed — and one null result worth having

Written 2026-08-16, closing the three bullets `doc/notes-function-wiring.md` §4.5 has
carried since the lane was wired, and the second bullet of `doc/notes-function-tests.md` §5.
All three were found **by reading rather than by a failure**, which is exactly why they were
worth closing: nothing in the tree observed them, so a defect in any of them would have drawn
a plausible wrong page and no gate would have seen it.

**Ten tests in three new files, no change to `src/`, and no defect found.** That is the
honest headline and it is a measurement rather than an absence: every gate below was broken
in a working copy, watched to fail in the direction it claims, and restored (§5). A gate that
has only ever passed proves that a gate exists.

| file | tests | what it is about |
|---|---:|---|
| `crates/quorra-gpu/tests/function_weights.rs` | 5 | the clip and soft-mask factors of `base_weight` |
| `crates/quorra-gpu/tests/function_coverage.rs` | 2 | this paint on a device set to `Coverage::Gpu` |
| `crates/quorra-gpu/tests/function_staged.rs` | 3 | ADR 0025's `DestOut`/`Plus` over this paint |

One test-suite debt closed on the way: `doc/HANDOVER.md`'s "`deviation_from_the_clause` is
§11.4.6's arithmetic written out three times" (§4).

---

## 1. The clip and the soft mask — `tests/function_weights.rs`

`function_lane.wgsl`'s `base_weight` is `coverage × clip × soft mask`, and only the first
factor had ever been anything but 1 anywhere in the tree. The line is textually the shading
lane's, which is an argument that it works and is not evidence that it does.

| test | asserts | from |
|---|---|---|
| `a_rectangular_clip_weights_the_paint_by_the_area_it_admits` | a clip at `x < 40.25` admits column 20 whole, column 40 by **a quarter** (alpha 64 of 255), and column 41 not at all | §8.5.4 — a chain is one region arrived at by intersection, so a mark is painted on `shape ∩ clip`; the fraction at a clip edge inside a pixel is the area of the pixel inside the region, the quantity `coverage` means everywhere in this tree |
| `a_residue_clip_weights_the_paint_by_the_region_the_clause_intersects` | under a diamond clip a **rect-hinted** function fill becomes one rasterised coverage tile (`Counters::tiles == 1`), and its alpha equals, to 1 of 255, the region the same outline under the same clip admits when painted opaque white | §8.5.4 again, read off the device by geometry alone rather than assumed |
| `an_alpha_soft_mask_weights_the_paint_by_11_5_2s_mask_value` | 0.4 where the mask's group marks at that alpha (byte 102 exactly), **nothing** where it marks nothing | §11.5.2 (the mask value is derived from the group's alpha) and ADR 0037 for the transparent reduction outside the group |
| `a_luminosity_soft_mask_weights_the_paint_by_11_5_3s_luminosity` | `(0.30·51 + 0.59·102 + 0.11·153)/255` inside the group, and **1** outside it | §11.5.3 — the group composited with a fully opaque backdrop of a specified colour, then the luminosity of the result; the backdrop is white, so outside the group the mask admits everything |
| `a_clip_and_a_soft_mask_multiply` | 0.25 × 0.4 = 0.1 at the clipped column under the mask | the two clauses above, and that `base_weight` is a **product** |

Three things about the fixtures are load-bearing:

- **The clip's edge is at 40.25, not at 40.** A clip that cut on a pixel boundary would be
  honoured by the quad's own rectangle — `rect_placement` rounds `dest` out to whole pixels
  — so a shader that ignored the clip factor entirely would still draw the right picture. The
  quarter cannot come from anywhere but the weight.
- **The luminosity backdrop is white and its group's mark is not grey.** A grey mark would
  give the same answer under any three coefficients summing to one, and a black backdrop —
  the caller's default — hides ADR 0037's constant behind a zero.
- **Every colour assertion is made in premultiplied space.** The weight scales the
  premultiplied colour and the alpha together, so it cancels out of a straight-alpha channel
  and is invisible there. `assert_weighted` checks both halves, so a lane weighting only one
  of them fails.

**No defect.** The lane draws all four factors correctly on both adapters.

## 2. `Coverage::Gpu` — `tests/function_coverage.rs`, and the bound is not a bound

§4.5's bullet said the risk was "small and stated rather than measured". Reading the encoder
says something stronger, and the file asserts *that* rather than a tolerance:

**`Encoder::take_gpu_lane` is consulted in exactly two places** — `encode/fill.rs`'s
`fill_solid` and `encode/coverage.rs`'s `push_coverage_styled` — **and both are the solid
arm.** A rare paint reaches the sheet through `encode/rare.rs`'s `push_rare_coverage`, which
calls `coverage_tile` directly. So a function fill's coverage is the CPU rasteriser's under
either setting, and a page of function paint is the **same bytes** on the two devices.

That matters for what the gate is allowed to say. `tests/coverage_lanes.rs` bounds the two
*lanes* at an eighth of a pixel for the sample grid plus a quarter for the CPU lane's
flattening (96 of 255 on a curve), and ADR 0006 bounds two *adapters* at ±1 unorm. **Neither
is the right bound here**, and adopting either would have been a gate that could not fail for
its own regression — ADR 0052's shape. The equality is asserted, and the test's own doc
comment says what to do if it ever fails: not loosen it, but recognise that a rare paint has
learned the device lane and rewrite the file's subject.

The interaction the setting *does* create is that the frame's scratch sheet then has two
producers. `a_function_tile_and_a_device_drawn_tile_share_one_sheet` is that case: a solid
curve the atlas will not hold beside a function-painted triangle, on one sheet, drawn under
both settings. The blob's edge **must move** between the settings and the function
triangle's forty-by-forty region **must not** — the first is what proves the device lane was
actually taken, since a tile count cannot say (both lanes pack exactly one tile per mark).

**No defect.**

## 3. ADR 0025's stages — `tests/function_staged.rs`

`Style::of` maps `Compose::DestOut` to the erase pipeline and `Compose::Plus` to the add
one, `pipeline::function` compiles both for a program, and nothing drew either.
`tests/function_knockout.rs` draws the pair only as the two halves a *knockout group* runs
together, and the builder refuses a staged mark inside such a group — so the two
constructions are disjoint and neither is evidence about the other. §5's `Style::of` break
below demonstrates that: it fails all three of these tests while leaving all six of
`staged_compose.rs` green.

| test | asserts | from |
|---|---|---|
| `the_staged_pair_over_a_function_paint_is_the_clause` | the pair is `P' = (1 − f)P + S` to **0.57 of 255** on llvmpipe and 0.70 on RADV, where one source-over mark of the same element is **114.90** away from it | §11.4.6's `𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t`, §11.4.5 and §11.3.6 for why the initial backdrop leaves `co = as·Cs` |
| `dest_out_over_a_function_paint_erases_only_where_the_clause_paints` | with the domain a quarter of the shape and no `Background`, the page is erased to nothing on the left and left **byte for byte** on the right | §8.7.4.5.2 ("such points shall be left unpainted") ∧ §11.6.4.2 (shape is geometry): an unpainted point has no shape, and ADR 0025 weights `DestOut` by shape |
| `dest_out_over_a_function_paint_ignores_the_paints_own_opacity` | a `Background` of alpha ¼ and one of alpha 1 erase the **same frame**, and both erase all of it | §11.6.4.2 against §11.4.7.2 — the `Background`'s alpha is opacity, the mark's geometry is shape |

The comparison numbers are worth putting beside ADR 0025's own: it measured **0.77 of 255**
for the staged pair against **114.95** for source-over, on a solid wedge. This lane, on a
function paint, reads 0.57/0.70 against 114.90. Two lanes, one clause, the same separation.

**The fixture had to be a translucent one.** `doc/notes-function-tests.md` §1.4: for a source
of alpha 1 the replacement and an ordinary over-composite are the same arithmetic, and a
function paint is opaque wherever it marks *inside its domain*. The only way to state a
source of alpha below one at full shape on this paint is §8.7.4.5.2's `Background`, so that
is what the first test paints outside its domain — and §5 confirms the point by re-running it
with an opaque background, where the control collapses from 114.90 to 0.60.

**No defect.**

## 4. One debt closed: §11.4.6's line has one home

`doc/HANDOVER.md`'s small-debts list: "`deviation_from_the_clause` is §11.4.6's arithmetic
written out three times, two identical and the third in `function_knockout.rs`; giving it one
home is a round that must touch that file too." This round is that round — a fourth copy was
the alternative — and it is now `crates/quorra-gpu/tests/common/clause.rs`, used by
`knockout_blend.rs`, `staged_compose.rs`, `function_knockout.rs` and `function_staged.rs`.

**The obstacle HANDOVER named did not survive being read.** It was that each copy indexed its
raster through its own file's `SIZE`, so one home for the function meant one home for `SIZE`
— and `SIZE` means 64 in six files and something else in four others. But the clause's line
is stated over *every pixel of the rasters it is handed*, and a raster's own length says how
many that is. No dimension is shared, so no file's probes are tied to another's. All three
callers happened to use `SIZE = 64` anyway, which is why the merge moved no number: 3 tests
in `knockout_blend.rs`, 6 in `staged_compose.rs` and 3 in `function_knockout.rs` pass with the
same names and the same printed deviations before and after.

The rest of `HANDOVER`'s "deliberately not unified" list is untouched and still right: the
two-argument `render`, `alpha` and `pixel` each need a dimension to name a pixel, which is the
thing the clause's arithmetic does not need.

## 5. Every gate, broken and watched to fail

ADR 0052's rule, not by inspection. Each defect was forced in a working copy, run, and
restored.

| forced defect | result |
|---|---|
| `base_weight` returns `cov * soft_mask_at(p)` — the clip factor dropped | `a_rectangular_clip_…` and `a_clip_and_a_soft_mask_multiply` **fail**; the two mask tests and the residue test pass |
| `base_weight` returns `cov * extent.x * extent.y` — the mask factor dropped | both mask tests **and** `a_clip_and_a_soft_mask_multiply` **fail**; the rectangular-clip and residue tests pass |
| `base_weight`'s scratch branch set to `cov = 1.0` | `a_residue_clip_…` **fails** alone, in `function_weights.rs` |
| `coverage_placement` zeroes one coverage byte under `Coverage::Gpu` — the rare lane made setting-dependent | **both** `function_coverage.rs` tests fail, one on the frame's byte equality and one on the function region's |
| `take_gpu_lane` forced to `false` | `a_function_tile_and_a_device_drawn_tile_share_one_sheet` **fails on its own discriminating guard** — the blob's edge no longer moves, so the fixture would have been comparing the CPU lane with itself |
| `Style::of(DrawStyle::DestOut)` → `[Some(Style::Over), None]` | **all three** `function_staged.rs` tests fail, and **all six** of `staged_compose.rs` pass — which is the argument for the new file in one line, since `Style::of` is read by the function lane alone |
| `fs_shape`'s `if straight.a <= 0.0 { return vec4f(0.0); }` deleted | `dest_out_over_a_function_paint_erases_only_where_the_clause_paints` **fails**, and it fails as predicted: `[0, 0, 0, 0]` where the opaque page `[230, 51, 26, 255]` belongs — a transparent hole rather than a shade |
| the staged fixture's `Background` alpha changed from ½ to 1 | `the_staged_pair_…`'s **control** fires: the source-over deviation collapses from 114.90 to 0.60 and the `>= 16.0` assertion fails, which is what says the fixture is discriminating rather than lucky |

Two of the eight are in `src/encode/`, which this round otherwise did not touch; both were
made and reverted in this worktree only.

## 6. Which adapters, and the full suite

Both adapters of this machine, by name — `llvmpipe (LLVM 22.1.8, 256 bits)` and
`AMD Radeon 890M Graphics (RADV STRIX1)`, selected with `QUORRA_ADAPTER`, which every one of
the three new files honours because ADR 0053 promises no cross-adapter identity for this
paint. All ten new tests pass on both. The only number that moves between them is the staged
pair's deviation, 0.57 against 0.70 of 255, both far inside the 3.0 the clause is held to.

- `cargo test --workspace`: **455 passed, 0 failed**, over 58 test binaries — against 445
  over 55 before the round. `grep -rc '#\[test\]'` reads 454 against 444, so the ten new
  tests all ran and the one-test difference is the workspace's single doctest, before and
  after. (The count is checked because cargo can call a stale artefact fresh; `doc/HANDOVER.md`
  carries the round that lost 21 tests to it.)
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`: clean, with
  `Checking quorra-gpu` printed — and it was reached the hard way, since the first run failed
  on `too_many_lines` and `similar_names` in the new files.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean.
- `cargo fmt --check`: clean.

## 7. What this round deliberately did not do

- **A knockout group over a function paint under a soft mask.** `doc/notes-function-tests.md`
  §5's first bullet, and it is still open. `fs_shape` weights by `base_weight`, which includes
  the mask, so a knockout element's shape carries §11.6.4.3's opacity in it. That is not this
  lane's own reading: `rect.wgsl`'s `fs_shape` cites §11.4.7.2 for "object shape ∧ clip ∧ mask
  shape" and every lane follows it, so re-deciding it for the function lane alone would be a
  fifth reading of one clause. It is a clause question for a round that can move all five, and
  §11.4.7.2's own text is what that round has to start from.
- **A `Paint::Function` under a clip or a mask through a retained frame.**
  `doc/notes-function-tests.md` §5's third bullet: the retained page is flat. This round's
  pages are immediate.
- **The caller's corpus.** Nothing in it reaches this paint yet, so a run would measure that
  we changed nothing — which is still true and still worth doing when the bump lands rather
  than now (`notes-function-wiring.md` §4.5).
- **An ADR.** Nothing was decided. The one thing that looks like a decision — asserting
  equality rather than a bound between the coverage settings — is a *reading of the encoder*
  that the test states and that stops being true the moment the rare lanes gain a device path;
  §2 above is where it is written down, and the test's own doc comment says what to do when it
  fails.
