# The shared probes, and dividing `raster/tests.rs`

Two rounds in one document, both about the test suite holding the bar it holds the source
to. Neither moves behaviour, and each says how that is proved rather than asserting it.

Base: `6ee3072`, current `main`. **The worktree this round was handed started at
`eada81e`, sixty commits behind** — the stale-base trap, caught by `git worktree list`
before a line was written. Everything below is on `6ee3072`.

---

## 1. The probes had no home, and the recorded obstacle was two different mistakes

`doc/HANDOVER.md` had carried this bullet through two rounds:

> The two-argument `render`, `alpha`, `pixel` and `deviation_from_the_clause` each index a
> raster through their own file's `SIZE`, so one home for them is one home for `SIZE` — and
> `SIZE` means 64 in six files and something else in four others, which is a decision about
> what those probes are rather than a refactor. **`alpha` is the reason to be careful**: its
> text is identical in `coverage_lanes.rs` and `mask_regions.rs` and the `SIZE` it reads is
> *not*.

`deviation_from_the_clause` was unified first, and the reason it could be is in
`tests/common/clause.rs`: the clause's line runs over every pixel of the rasters it is
handed, so it needs no dimension at all. That was read as the exception. It is not — it is
one of two answers, and the bullet's single sentence covered two different facts:

| probe | what it needs | so |
|---|---|---|
| `deviation_from_the_clause` | **nothing** — it runs over a whole raster | no dimension to share |
| `max_byte_diff` | **nothing** — same shape | no dimension to share |
| `pixel`, `alpha` | the raster's **stride**, to name a pixel | a stride is an *argument* |
| the two-argument `render` | the target's **width and height** | likewise |

**A stride is not a `SIZE`.** That is the whole finding. The bullet reasoned that a probe
needing a dimension must share the dimension, and the step does not follow: a probe that
*takes* the dimension shares no dimension at all. And the answer was already in the tree in
three places — `function_lane.rs` and `m7.rs` had `fn pixel(pixels, width, x, y)` and
`thin_marks.rs` had `fn alpha(pixels, side, x, y)`, because those three draw at more than
one size and so could not close over a constant. Ten files had copied the *other* shape.

### What moved, and where

`tests/common/probe.rs` — a fifth part of `tests/common/`, one responsibility stated as the
mirror of `headless.rs`'s: **`headless` asks a device for a frame and hands back its bytes;
`probe` turns those bytes into the number an assertion is written on.** Nothing in it draws
and nothing in it asserts.

- `pixel(raster, width, x, y) -> [u8; 4]` — 11 files, 94 call sites (63 of them given a
  stride here, 31 already passing one)
- `alpha(raster, width, x, y) -> u8` — 7 files, 37 call sites (34 given a stride here),
  written as `pixel(..)[3]` so the two cannot come to disagree about which byte belongs to
  which pixel
- `max_byte_diff(actual, expected) -> i32` — 2 files, 3 call sites, no dimension

The eight private two-argument `render`s point at the existing `common::headless::render`,
which already took `width` and `height`; each call site names the size its deleted body had
written into its own `Viewport::full`.

`cull.rs` and `mask_grid.rs` called theirs `alpha_at`; they now call `alpha`. That renames
no test — a helper's name is not a gate.

**The height is deliberately not a parameter of `pixel`.** An index into row-major RGBA is a
function of the stride alone, and asking for a number the arithmetic never uses is a number
that can be wrong in silence.

### The proof that each caller still reads the pixel it meant

Two instruments, because one of them is not enough.

**Mechanical, and it is a proof rather than evidence.** Every deleted body computed
`((y * SIZE + x) * 4)` for its file's `SIZE`; every call site now reads
`pixel(raster, SIZE, x, y)`, which computes `((y * SIZE + x) * 4)` for the *same identifier
resolved in the same file*. The rewrite was done by a paren-aware pass that inserted the
stride as the second argument of exactly the three-argument calls and left every other arity
alone — it reported `0 occurrence(s) left alone` in all fourteen files whose calls were all
three-argument, and in `function_lane.rs`, `m7.rs` and `thin_marks.rs` it left every call
alone, because those three were already passing a stride and only the definition went.

**Verified able to fail, per caller.** Each file in turn had its `use common::probe::…`
replaced by a local shim of the same name passing `width + 1`, and that test binary alone
was run:

| result | files |
|---|---|
| red on a corrupted stride | 18 of 20 |
| green | `non_isolated_groups.rs`, `thin_marks.rs` |

The two greens are the interesting half, and neither is a hole in the merge:

- **`non_isolated_groups.rs` marks its whole target with every command** (`full_rect()`), so
  the page is one colour and *every* stride reads the same byte. Where no stride can read
  the wrong pixel, corrupting the stride proves nothing.
- **`thin_marks.rs`'s two `alpha` calls are the absence half of a claim** (`== 0`, the
  columns either side of a sub-pixel mark), and a shifted read lands on empty page, which is
  also 0. The file's own comment already names its control — the `row_ink` assertion above
  it — which is why this reads as a property of the fixture rather than as a defect.

Both were then forced in a **second direction**, on the value rather than the address —
`pixel` returning a channel plus 40, `alpha` returning `255 − a` — and both went red, 4
tests each. So all 20 callers are proved to depend on what the shared probe returns, and 18
of them on the stride they pass it specifically.

The unified `render` got the same treatment: a shim drawing one column wider. Six of eight
red; `non_isolated_groups.rs` green for the reason above, and **`knockout_blend.rs` green
for a better one** — every raster it compares goes through the same `render`, and
`clause.rs`'s arithmetic is size-free by construction, so enlarging all four together
changes nothing it asserts. Halving the target instead takes it red on 2 tests, which is
the size being load-bearing after all, in the direction that can clip its wedge.

### What was left apart, and why

**`UNORM_TOLERANCE` stays two constants.** `HANDOVER.md` said the reason was that `m1.rs`
and `m3.rs` "each state their own derivation". Read rather than quoted forward, that is not
what is there: `m3.rs`'s comment said *"ADR 0006's cross-implementation bound, as in
m1.rs"* — it derives nothing and points at a derivation belonging to another fixture.

And that derivation does not reach it. `m1.rs` gets ±2 from ADR 0006's ±1 unorm step per
blend stage in premultiplied space, amplified by `255/α`, **on a golden whose minimum alpha
is 128**. `m3.rs` draws 4 000 rects of `α = 0.9` on a quarter-pixel grid; measured on
2026-08-17, the alphas its page produces are `{0, 29, 57, 86, 115, 172, 230}`, so the same
premise gives `255/29 ≈ 9` there, not 2.

So the constants stay apart — one name over two fixtures would be one number claiming two
derivations — and the right conclusion is the opposite of the recorded one: *they were
never two derivations, and that is why they must not be merged.* Both comments now say
which fixture their number is a property of.

**One finding falls out of measuring it.** `m3.rs`'s page and its CPU reference agree to
**0 unorm steps** — the tolerance has never spent any of its slack. It is left at 2, because
a threshold is behaviour and this round moves none. Tightening it to 0 would turn a bound
that cannot fail into a gate; that is a round with a corpus question attached (does every
adapter agree exactly on a rectangle-only page, or only llvmpipe, which is the adapter this
file pins?), and it is recommended rather than taken.

**`max_byte_diff`'s doc is corrected on the way.** It claimed the function panics "with PNG
artefacts if the shapes differ"; the artefacts are written by `m1.rs`'s caller, and the
function only asserts that two lengths match. The shared version says what it does and says
explicitly that the bound and the artefacts belong to the test — which is the same seam
`clause.rs` already draws when it makes each caller state its own partial-pixel floor.

### The suite, before and after

`cargo test --workspace`, own `CARGO_TARGET_DIR`, cargo's own exit status: **551 passing, 3
ignored** before and after, and the reconciliation holds in both — 550 `#[test]` attributes
plus 4 doctests, minus 2 ignored tests and 1 ignored doctest, is 551. `RUSTFLAGS="-D
warnings" cargo clippy --workspace --all-targets` clean, with `Checking quorra-gpu` printed
rather than only `Finished`.


### What §1 recommends and does not take

- **`m3.rs`'s `UNORM_TOLERANCE`**: 2 where the page agrees to 0. Tightening it is a
  behaviour change with a real question attached — this file pins llvmpipe, and whether
  every Vulkan adapter agrees exactly on a rectangle-only page is `m1.rs`'s question, not
  one this gate has ever asked. Worth one round with `vulkan_adapters()` borrowed.
- **`mask_grid.rs`'s `rgb_at`** was left where it is. It is one file's probe with one
  caller's shape (`[u8; 3]`), and `common::probe` holds what was duplicated, not what could
  be. It is `pixel(..)[..3]` if it ever gains a second copy.
- **A copy the bullet did not list, found by checking a claim before writing it down.**
  `sample_tail.rs::at_unit` was about to be recorded here as *unrelated* to
  `coverage_lanes.rs::at_unit`. Reading the two settled it the other way: same `UNITS = 48`,
  same `MAGNIFY = 16`, same `SIZE`, same meaning, and `sample_tail`'s body was `alpha`
  written inline. It is folded in, and what stays in each file is the unit-to-device map,
  which is that fixture's arithmetic rather than a raster's. The lesson is the tree's own —
  *cost to disprove: one `grep` and one reading* — and it is why the HANDOVER bullet's list
  of four should not have been trusted as complete.
- **Five inline `alpha` indexings are left, deliberately, and here they are** so the next
  round does not have to find them: `coverage_lanes.rs:634` and `:853`, `zero_extents.rs:241`,
  `m3.rs:523`, `scale_invariance.rs:92`. The line drawn is *`common::probe` holds what was
  duplicated as a named helper*; an index written inside one test function is that test's own
  arithmetic and sweeping it is a different round with a different justification. Stated as a
  choice rather than an oversight, which is the difference this list exists to make.


---

## 2. `raster/tests.rs` divides, and the seam is the parent's after all

ADR 0061 put all eighteen cases in one file when `raster.rs` was split, so the split could be
proved by an identical list of test names, and wrote the cost down: 704 lines, past the
smell, dividing them a round of its own. This is that round. **Its whole visible effect is
that every one of those eighteen tests gains one path segment**, and the mapping is below.

### Whether the source's seam fits the tests

ADR 0061 said it does not, and gave the reason: almost every case goes through all three
parts, so a file per source module would put most of them in the wrong one. That is true of
the *call graphs* and it does not decide anything, because `fill_mask` is the **instrument**
almost all of them observe through — `stroke` takes polylines and returns polylines, and
`flatten` returns polylines too, so filling is the only way to see either. An instrument is
not a subject.

Asked instead *which clause is this test a statement about*, the eighteen divide with
nothing left over: 3 / 9 / 6. And the parent's own module comment is the check, because it
already attributes each of this code's three arithmetic defects to one part — so each
defect's regression test lands in its defect's file without anyone choosing that. ADR 0062
records the rule and the four agreements.

### The mapping — every name that changed, and to what

Eighteen renames, each exactly one inserted path segment, nothing else:

| was | is |
|---|---|
| `raster::tests::collinear_cubic_equals_the_line` | `raster::tests::flatten::collinear_cubic_equals_the_line` |
| `raster::tests::a_circle_deposits_its_own_area_at_every_size` | `raster::tests::flatten::a_circle_deposits_its_own_area_at_every_size` |
| `raster::tests::a_large_curve_keeps_the_segment_count_it_had` | `raster::tests::flatten::a_large_curve_keeps_the_segment_count_it_had` |
| `raster::tests::aligned_rectangle_is_exact` | `raster::tests::fill::aligned_rectangle_is_exact` |
| `raster::tests::fractional_rectangle_covers_halves` | `raster::tests::fill::fractional_rectangle_covers_halves` |
| `raster::tests::diagonal_covers_by_area` | `raster::tests::fill::diagonal_covers_by_area` |
| `raster::tests::nested_same_winding_separates_the_rules` | `raster::tests::fill::nested_same_winding_separates_the_rules` |
| `raster::tests::geometry_outside_the_region_still_winds` | `raster::tests::fill::geometry_outside_the_region_still_winds` |
| `raster::tests::a_tile_is_the_crop_of_the_region_that_contains_it` | `raster::tests::fill::a_tile_is_the_crop_of_the_region_that_contains_it` |
| `raster::tests::a_tile_whose_geometry_enters_from_outside_is_exact` | `raster::tests::fill::a_tile_whose_geometry_enters_from_outside_is_exact` |
| `raster::tests::the_region_and_the_tile_agree_on_a_pixel_whose_area_is_known` | `raster::tests::fill::the_region_and_the_tile_agree_on_a_pixel_whose_area_is_known` |
| `raster::tests::an_edge_whose_slope_leaves_f32_deposits_nothing` | `raster::tests::fill::an_edge_whose_slope_leaves_f32_deposits_nothing` |
| `raster::tests::butt_stroke_of_a_horizontal_line_is_a_rectangle` | `raster::tests::stroke::butt_stroke_of_a_horizontal_line_is_a_rectangle` |
| `raster::tests::square_caps_extend_by_half_the_width` | `raster::tests::stroke::square_caps_extend_by_half_the_width` |
| `raster::tests::miter_join_fills_the_corner` | `raster::tests::stroke::miter_join_fills_the_corner` |
| `raster::tests::each_cap_deposits_the_area_table_53_gives_it` | `raster::tests::stroke::each_cap_deposits_the_area_table_53_gives_it` |
| `raster::tests::a_stroke_spanning_the_coordinate_range_is_not_drawn_as_nothing` | `raster::tests::stroke::a_stroke_spanning_the_coordinate_range_is_not_drawn_as_nothing` |
| `raster::tests::a_segment_below_the_float_grid_produces_finite_geometry` | `raster::tests::stroke::a_segment_below_the_float_grid_produces_finite_geometry` |

**The three defect tests, by name, because they cover arithmetic that drew wrong pages:**
`a_stroke_spanning_the_coordinate_range_is_not_drawn_as_nothing` and
`a_segment_below_the_float_grid_produces_finite_geometry` are in `raster/tests/stroke.rs`
(`stroke::direction` and `stroke_polylines`' float-equality dedupe);
`an_edge_whose_slope_leaves_f32_deposits_nothing` is in `raster/tests/fill.rs`
(`fill::accumulate_edge`). None lost an assertion, a comment or a line — see below.

### That nothing else moved

Three instruments, and each is exact rather than a reading:

1. **The whole workspace list, stripped.** `cargo test --workspace -- --list` gives 554
   names before and after. Removing the one inserted segment from each of the after names
   reproduces the before list **byte for byte** — `diff` is empty. So no name changed in any
   way other than gaining a segment, and none appeared or disappeared.
2. **The multiset of every non-comment, non-blank line**, the instrument the `raster` split
   trap names, run over the old file against the four new ones. It differs in exactly: three
   `mod` declarations; the import blocks (one `use` became six, spread over four files); and
   one call that lost its `super::` prefix, `super::polyline_bounds(&lines)` →
   `polyline_bounds(&lines)`, because the name is now imported. **Every other line is
   identical.** That is ADR 0061 cost 3's prediction — *the imports change even though the
   tests do not* — holding one level down.
3. **The counts.** 551 passing and 3 ignored before and after; 550 `#[test]` attributes
   before and after.

### Sizes

| file | lines | holds |
|---|---|---|
| `raster/tests.rs` | 80 | the map, and the three fixtures more than one file builds |
| `raster/tests/flatten.rs` | 184 | 3 cases, plus `circle_path` and its two derived constants |
| `raster/tests/fill.rs` | 266 | 9 cases |
| `raster/tests/stroke.rs` | 261 | 6 cases, plus `hairline` and `LARGEST_DEVICE_COORDINATE` |

Everything used by exactly one file moved into that file, where its reasons are; `IDENTITY`,
`cov` and `rect_path` stayed in the parent, which is the only thing three of the four share.
