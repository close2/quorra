# Every ceiling in this tree, and what crossing it does

Round of 2026-08-17. The subject is CLAUDE.md principle 6 — *a frame is drawn, or it is
refused; there is no third state* — asked from the outside, by the caller's
`pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` §1:

> If quorra has an equivalent ceiling anywhere in strip generation, the thing to check is
> not whether it can be raised but whether crossing it returns rather than aborts.

That document is explicitly **not** a defect list against us — "nothing here is a claim
that quorra has any of these problems" — so every item below was treated as a question to
answer with evidence. Three of the five answers were clean. Two were not, and the two
defects found are recorded here with their fixes and their gates.

**Every answer ends in a test**, including the clean ones, because a written argument
decays and a gate does not. Nineteen tests were added; the two places where a gate was not
possible are named as such with the reason.

---

## The two defects, first

Both are in `raster.rs`, both are reachable from an outline a document may state and the
scene boundary already admits, and both are §5's third state rather than a refusal.

### 1. A stroke at the top of the coordinate range was drawn as nothing

`direction` computed a segment's length as `(dx*dx + dy*dy).sqrt()`. **`dx * dx` overflows
to infinity above `1.9e19`**, which is eight orders of magnitude below what the scene
contract admits: `MAX_COORDINATE` is `1e9` on an outline point *and* on a command
transform coefficient *and* (now) on the viewport transform's, so a device delta reaches
`1e27`. The length was then infinite, the normal `(0, 0)`, and every quad the stroke
expanded to had no width — a mark asked for and silently not drawn.

### 2. A mark thinner than the float grid painted a solid row

Two ends of the same arithmetic:

- `dx * dx` **underflows to zero** below `1.1e-22`, and `stroke_polylines`' dedupe drops
  coincident neighbours by comparing coordinates for *equality* — which two points `1e-30`
  apart pass. The length was zero, the direction `(NaN, ±inf)`, and the expansion was NaN
  geometry. CLAUDE.md's rule is that a document's numbers must "never produce NaN
  geometry"; this was the one place in the tree that did.
- `accumulate_edge`'s slope `(bot_x - top_x) / (bot_y - top_y)` **overflows** for an edge
  that is wide and vertically thin. A triangle `1e9` wide and `1e-30` of a pixel tall —
  both numbers inside `MAX_COORDINATE` — has slope `1e39`, which is infinite in `f32`.

Either way a NaN entered the accumulation grid, and **a NaN does not stop a frame**: it
survives the prefix sum, and the non-zero rule's `running.abs().min(1.0)` returns **1.0**
for a NaN, because `f32::min` returns the non-NaN operand. One invisible sliver therefore
painted the rest of its row solid, and the frame reported itself drawn.

Measured before the fix, on an 8 × 8 probe:

```
PROBE inf coverage row2 = [255, 255, 255, 255, 255, 255, 255, 255]
PROBE tiny stroke polylines = [Polyline { points: [Point { x: NaN, y: inf }, …] }]
PROBE tiny stroke coverage row1 = [255, 255, 255, 255, 255, 255, 255, 255]
```

### The three changes

1. **`raster::direction` falls back to `hypot`.** `hypot` is the same length without the
   square and is exact at both ends. It is the *second* path rather than the only one for
   two reasons, both stated beside it: it is a libm call on the hottest stroke loop there
   is (the caller's reading of hayro #630 is the standing warning about exactly that), and
   every segment the fast path already handles must keep the arithmetic it had **to the
   bit**, so that no page of the corpus moves. When neither computation gives a finite
   positive length the direction is `(0, 0)`, which makes every piece built from it
   degenerate — §8.5.3.2's degenerate subpath, depositing nothing — rather than a NaN.
2. **`raster::accumulate_edge` returns on a non-finite slope.** Zero is not a compromise
   here but the exact answer to eleven decimal places: the numerator is bounded by twice
   the largest device coordinate the contract admits (`4e27`), so a non-finite ratio means
   the slab is under `2.4e-11` of a pixel tall, where one coverage step is `1/255`. The
   same test catches a NaN from any source, which is what makes it defence in depth as
   well as a fix.
3. **`Device::render` bounds the viewport transform** by `MAX_COORDINATE`, as
   `RenderError::ViewportTransformTooLarge { coefficient, limit }`. This is not cosmetic:
   it is the third factor of every device coordinate and was checked for finiteness alone,
   so `point × command × viewport` could reach infinity while every input was legal. With
   all three bounded, "no device coordinate is infinite" becomes a statement one can check
   rather than one that happens to hold. Nothing real is refused by it — a coefficient of
   `1e9` on a page is `1e9` device pixels, sixty thousand times `max_target_size`.

**No corpus run was taken, and the reason is checkable rather than an assurance**: each
change is guarded by a condition that is *false* for every input the current code handles
at all. A finite positive length takes the same branch it always did; a finite slope takes
the same path it always did; a viewport coefficient under `1e9` is not refused. There is
no input that both reaches the new code and reached the old code successfully.

---

## 1. Every ceiling, and what crossing it does

The enumeration below is of every limit, assertion, `unwrap`, `expect`, `panic!`,
`debug_assert` and overflowing arithmetic **outside `#[cfg(test)]`** on a path a scene can
reach. It was taken by stripping each source file at its first `#[cfg(test)]` and matching
the whole set of constructs, then reading each hit.

### The refusals, and where each is crossed

| Ceiling | Refusal | Crossed by |
|---|---|---|
| the frame's byte budget | `FrameBudgetExceeded { needed, budget }` | `m1.rs`, `encode/tests.rs`, `layer_reuse.rs`, `scratch_sheet.rs`, `mask_regions.rs`, `coverage_lanes.rs`, `tiling_ceiling.rs` |
| the coverage sheet's height | `ScratchExhausted { limit }` | `tiling_ceiling.rs` |
| the adapter's target size | `TargetTooLarge { width, height, limit }` | `m1.rs` |
| a zero-size `Texture` / `Surface` | `ZeroSizeTarget { target }` | `m1.rs` (Texture); **`zero_extents.rs` (Surface, new)** |
| a non-finite viewport transform | `NonFiniteViewportTransform` | `m1.rs`, `retained_refusals.rs` |
| **a viewport transform above `MAX_COORDINATE`** | **`ViewportTransformTooLarge` (new)** | **`ceilings.rs` (new)** |
| a malformed damage rectangle | `InvalidDamage { index }` | `m8.rs` |
| the resource budget | `DeviceError::ResourceBudgetExceeded` | `m2.rs`, `resources.rs` |
| an outline coordinate above `MAX_COORDINATE` | `ResourceProblem::OutlineCoordinateTooLarge` | `resources.rs` |
| the outline identifier space | `DeviceError::…` naming it | `resources.rs` |
| every scene-boundary condition (§4.7) | `SceneError`, one variant each | `quorra-scene`'s `validate.rs` |
| the group-depth bound | `SceneError` | `quorra-scene`'s `frames.rs` |

Every one of them is a value. **There is no `panic!`, `todo!`, `assert!` or
`debug_assert` on any path a scene reaches that a document-derived value can cross.**

### The panicking constructs that do exist outside tests, and why each is sound

Seven, and no more:

- `compose/draw.rs:72`, `encode/rare.rs:281`, `winding/passes.rs:75` — `unreachable!`, each
  with the argument beside it and each true by the immediately preceding match or
  assignment in the same function. `winding/passes.rs` is the model: it matches rather than
  `expect`s, precisely so the invariant is in the type.
- `device/textures.rs:131` — `expect("created above")` under an `#[allow]`, on a value the
  five lines above it assign unconditionally.
- `frame.rs:39`, `surface.rs:87`, `scene/builder.rs:303` — three `debug_assert`s, each
  stating an invariant a *caller inside this crate* holds. None takes a
  document-derived value: `Raster::new`'s length comes from the readback path's own
  arithmetic, `SurfaceSlot::attach` follows an `is_detached` the same function asked, and
  `SceneBuilder::finish`'s open-frame count is the builder's own stack, which `group` and
  `mask` close on both paths.

`shaders/copies.rs`, `shaders/layout.rs` and `encode/tests.rs` are `#[cfg(test)]` modules
and their assertions are gates, not frame code.

### Three things the enumeration turned up that are not defects

**The frame budget's pre-check counts top-level commands only.** `encode` charges
`scene.commands().len() × 96` before allocating, where `Scene::cost().commands` counts
*through* group nesting — so a page whose marks are inside one group is pre-charged for
one command. It is not a bomb and the reason is arithmetic: the instance streams are 32 and
64 bytes per command against a `Command` the caller already holds, so the amplification is
below one, and every *tile* is still charged individually by `charge_tile`. What it does
mean is that `Cost::commands × 96` and the number `FrameBudgetExceeded` names are different
quantities for a nested scene. Left as it is: making the check recursive would refuse pages
the tree draws today, and principle 6 ranks that above a tidier number.

**`fill_mask` allocates about five times what the tile is charged.** `charge_tile` charges
`width × height` bytes; `fill_mask` holds an `f32` accumulator of `(width + 1) × height`
*and* the coverage bytes, so the peak is `5 × width × height`. On a full-page tile at the
default 268 MiB budget that is transient host memory of over a gigabyte. It is not
unbounded — it is a fixed multiple of a number the budget already bounds — but
`max_frame_bytes` is not the ceiling on host memory that its name suggests. Recorded rather
than changed: moving the constant would move which pages refuse, which is a corpus round.

**`coverage_tile`'s `width == 0 || height == 0` test is unreachable.** The `vx0 >= vx1 ||
vy0 >= vy1` test three lines above it already returns, and `ceil − floor` over a non-empty
interval is at least 1. Discovered by forcing it and finding no test could tell: it is
correct defensive code, and it is now named as such here rather than mistaken for the guard
that does the work. The guard that does the work is in two places — `coverage_tile` and
`encode/parallel.rs`'s `rasterise` — and removing **both** is what
`a_mark_with_a_zero_extent_axis_draws_nothing_and_stops_no_frame` needs before it fails.

### Gates added

`ceilings.rs`, six tests: the new viewport bound refused **and** the same bound *not*
refused one step below it (a ceiling that refuses at the limit is a different defect from
one that does not refuse above it); the stroke across the coordinate range; the mark
thinner than the float grid; the below-grid stroke segment; and the first mark's batch.

---

## 2. Release-mode wrapping, and every `#[allow(clippy::arithmetic_side_effects)]`

### What release actually does

**`Cargo.toml` sets no `overflow-checks` in `[profile.release]`, so a release build of
this library wraps.** The profile sets `lto = "fat"`, `codegen-units = 1` and a note about
`panic`, and nothing else; cargo's default for `release` is `overflow-checks = false`. So
the caller's #646 note applies here exactly as written — a debug panic, a release wrap —
and every `#[allow]` below is a claim that the wrap cannot happen rather than a claim that
it would be caught.

### The audit

Thirty-one `#[allow(clippy::arithmetic_side_effects)]` sites exist; **eighteen are in
`#[cfg(test)]` modules or test files** and carry the standing test-file lint policy. The
thirteen in frame code, each with the argument that has to hold and the verdict:

| Site | Argument | Verdict |
|---|---|---|
| `raster.rs` `crop` | corners in `i64`, offsets are differences inside the overlap | **sound**, written down |
| `raster.rs` `fill_mask` / `accumulate_edge` / `deposit_slab` / `deposit_inside` | "coordinates clamped into the region, whose dimensions were checked against the frame budget" | **the argument was about the region and not about the geometry** — this is defect 2; the slope guard is the fix |
| `raster.rs` `stroke_polylines` / `join_at` / `cap_at` / `cap_fan` / `arc_fan` | f32 geometry on bounded coordinates | **sound for the sums**; the *length* underneath them was not — defect 1 |
| `raster.rs` `push_cubic` | bounded by `MAX_COORDINATE` and `MAX_SPLIT_DEPTH` | sound |
| `outline.rs` `triangle_count` | a sum of two lengths of the same `Vec` | sound |
| `atlas.rs` `GlyphKey::of` | `q` is non-zero, `Options` validates it | sound |
| `atlas.rs` `AtlasStore::new` | width ≥ 1 by `max(1)` above | sound |
| `atlas.rs` `allocate` | bounded by the texture dims; `width == 0` and `> self.width` refused first | sound, **and now gated** |
| `encode/scratch.rs` `reserve` / `pack` | "bounded by width/max_height checks" | sound — a tile's height is bounded by the viewport, which `validate_viewport` bounds by `max_target_size`, which is what `max_height` is; `width` is refused above `self.width` on entry. **Now gated** for the zero end |
| `encode/coverage.rs` `coverage_tile` etc. | tiles clamped to the viewport; the residue product is `255·255 + 127 < 2¹⁶` | sound |
| `encode/instance.rs` `note_batch` | **none written** | the argument (`- 1` cannot wrap because both callers write a whole instance first) is correct but was *unstated*; **written down, and gated** |
| `encode/parallel.rs` `rasterise` / `partition` | the same corner arithmetic moved unchanged; `taken + len` bounded by the loop condition | sound |
| `device/rare.rs`, `device/binds.rs`, `readback.rs`, `winding.rs`, `winding/buffers.rs` | fixed-layout offsets into fixed-size arrays; a `const fn` whose bounds stop at 256 | sound — a `const fn` that overflowed would fail the build, which is the strongest form the check can take |

Two silences were wrong, one was missing, and the rest hold. `readback.rs` deserves a
separate mention in the other direction: its row arithmetic is `checked_mul` and
`checked_next_multiple_of` into `TargetTooLarge`, which is what the rest of the tree's
`saturating_*` should be read against — saturating is right where the saturated value is
then *refused*, and `charge`/`charge_tile` are exactly that shape.

### Gates added

The two arithmetic defects are gated at both levels — three unit tests in `raster.rs` on
the expansion and the accumulator, and three tests in `ceilings.rs` on the picture. Each
is fed **the largest coordinate the scene contract admits**, derived rather than picked:
`LARGEST_DEVICE_COORDINATE = 2e27` in `raster.rs` states the derivation beside itself.
`note_batch`'s invariant is gated by `no_batch_is_noted_before_its_instance_is_written`.

---

## 3. A lane-width-rounded tail

**This tree rounds no count up to a vector width, and has no SIMD, no `align_to`, no
`(n + k - 1) / k`, and no compute shader at all** — every WGSL file in
`crates/quorra-gpu/src/shaders/` is vertex/fragment, so there is no host-side dispatch
count to round up either. The search covered `chunks`, `chunks_exact`, `windows`,
`step_by`, `div_ceil`, `next_multiple_of`, `with_capacity`, `resize` and every
`@workgroup_size` in the tree.

Four round-ups exist, and each already handles its tail:

1. **The readback's 256-byte row alignment** (`readback.rs`) — `checked_next_multiple_of`
   into a refusal, and `demultiply` reads `width × 4` from each `bytes_per_row` row, so the
   padding is never read. Gated by `demultiply_skips_row_padding`.
2. **The scratch sheet's restride** (`encode/scratch.rs`) — rows are packed at the device
   width and restrided down at `finish`; the tail is cut with `truncate` *before* the
   `resize` grows the sheet, which is the defect that drew 136 410 texels of another
   shape's coverage. Gated by `a_shelf_the_cpu_lane_did_not_write_is_blank`.
3. **A pane's outward `ceil`** (`pane.rs`) — rounds outward only, so a pane can be too big
   and never too small, and a tile larger than the budget becomes its own pane. Gated by
   `a_tile_larger_than_the_budget_is_its_own_pane`.
4. **The coverage sample grid, chunked four wide** (`winding/buffers.rs`) — and this is the
   one that had no test at the interesting size.

The fourth is the closest thing in the tree to `#373`'s shape. `Options::coverage_samples`
is clamped to `4..=64` and rounded **down** to a perfect square, so the reachable set is
`{4, 9, 16, 25, 36, 49, 64}` — and three of the seven leave a last group of **one** sample
against a pass that writes four channels. Every test in the suite used sixteen, which is a
multiple of four.

The tail is answered on the device rather than padded on the host, in two halves that have
to be read together: the winding attachment is cleared every round, so an unwritten channel
holds winding 0 and `inside(0, rule)` is false under both of §8.5.3.3's rules; and the
divisor is the *total* sample count, not four times the group count.

**Both halves now have a gate** (`sample_tail.rs`), run at all seven reachable counts.
Verified able to fail by changing `chunks` to `chunks_exact`, which drops the tail sample:
a wholly covered pixel then reads 226 at nine samples.

**One measured cost of a short tail, which was not known before this round.** Each round
deposits its share into the frame's R8 sheet, so each share is rounded to a byte before the
next is added. A wholly covered pixel reads **255 at 4, 16, 25, 36, 49 and 64 samples and
254 at 9**: `4/9` rounds down twice and the tail's `1/9` rounds down again, where at 25 and
49 the full rounds round *up* and the sum saturates. One step of 255 is inside ADR 0006's
bound for the whole device path and the default is 16, where the answer is exact — but it
is a real cost and it is now written down. The gate asserts the bound the arithmetic gives
(`255 − ⌈g/2⌉` for `g = ⌈n/4⌉` rounds), not the value the lane happens to produce.

**One test deliberately not written**, with its reason: a *partly* covered pixel's value at
a short tail. Its exact answer is a function of where the grid puts its samples and is
supposed to differ between counts, so any bound wide enough to be true is wider than the
whole error a dropped tail causes — written and then removed after it passed with the tail
dropped. The wholly covered pixel is the sharp probe.

---

## 4. A zero-sized target is a legal thing to ask for

**Yes, this tree catches it at the top, for every kind — and now every kind has a test
saying which of the two answers it is.** `doc/PLAN.md`'s "a blank scene is a legitimate
scene, and so is a zero-length buffer slice that follows from one" holds everywhere it was
checked.

| Kind | Answer | Why that answer | Gate |
|---|---|---|---|
| viewport into `Readback` | **`Ok`**, an empty raster | a zero-size raster follows from a zero-size window | `m1.rs` |
| viewport into `Texture` | `Err(ZeroSizeTarget { target: "Texture" })` | a texture cannot exist at zero size | `m1.rs` |
| viewport into `Surface` | `Err(ZeroSizeTarget { target: "Surface" })` | likewise — and the check is **above** the target binding, so a headless device still says the zero rather than `NoSurface` | **`zero_extents.rs` (new)** |
| layer / plan | **`Ok`**, one texel | wgpu refuses a zero-sized texture and a composite still reads what the plan left; `Region::of(None, …)` is `1 × 1` | **`zero_extents.rs` (new)** |
| soft mask | **`Ok`**, masks everything | §11.6.5.2's alpha mask over a group that marks nothing is alpha 0 — a picture, not a failure | **`zero_extents.rs` (new)** |
| coverage tile | **`Ok`**, nothing drawn, frame continues | `coverage_tile` and `parallel::rasterise` both return before a tile is charged | **`zero_extents.rs` (new)** |
| atlas entry | `None`, and the lane rasterises into the sheet instead | a zero-width shelf entry would sit at its neighbour's cursor; a zero-height one would open a shelf every later tile is seated inside | **`atlas.rs` (new)** |
| scratch sheet reservation | `None`, and the sheet is unchanged | same argument, at the sheet's own door | **`encode/scratch.rs` (new)** |

The layer gate asserts the counters as well as the pixels, because the fixture's shape is
not obvious from its scene: an empty group is a real layer by §11.4.5, ADR 0041 **culls the
child**, and what is left holding no bounds is the frame's root accumulator. Without that
assertion a later change could leave the test passing while exercising something else.

---

## 5. A defect above 1× is invisible to a suite that renders at 1×

**The honest answer is that this suite rendered almost everything at 1×.** Counted on
2026-08-17, over every test function that hands a `Viewport` to `Device::render`,
`render_retained` or `encode`:

- **198 such tests; 187 at a viewport whose scale is exactly 1** — identity, a pure
  translate, or a y-flip whose diagonal is ±1.
- Both shared helpers are identity: `tests/common/headless.rs`'s `render` and
  `tests/common/retained.rs`'s `viewport`.
- The golden comparison against the independent CPU reference — the one test that checks
  this tree's pixels against a second implementation — is `m1.rs`'s y-flip, i.e. scale 1.
- Of the eleven that are not 1×: **seven** are `coverage_lanes.rs` at a single factor of
  16, and they compare the two coverage lanes *with each other*, so a defect shared by both
  passes; **two** are `perf_gate.rs` at 20× asserting only `commands_culled`; **one** is
  `retained_invalidation.rs` at 1.5× asserting only an `EncodeSource`; **one** is the
  fuzzer, which reaches a random affine about one seed in sixteen.
- **No test anywhere walked a range of scales**, no viewport scale below 1 is exercised at
  all, and `examples/zoom.rs` — the only thing in the tree that sweeps a scale range —
  asserts nothing and is never run by CI.

### The smallest change that fixes it

`tests/scale_invariance.rs`: one fixture rendered at **1×, 2× and 4×** into a target that
scales with it, asserting the same *property* at each rather than the same bytes — because
the same page at two magnifications is two different pictures, and what does not differ is
what the area *is*.

Two properties, both derivable from the coverage definition in `raster.rs` rather than from
any output:

1. **Ink is area.** A mark of scene area `A` at magnification `s` covers `A·s²` device
   pixels, so the page's alpha, summed in units of full coverage, is `A·s²`. Tolerance 1 %,
   and the fit gets *tighter* with scale: the rounding error grows with the boundary (as
   `s`) while the area grows as `s²`.
2. **The mark is where the transform puts it.** The bounding box of the inked pixels is the
   scene box times `s`, to within the pixel each edge is rounded out to.

Three marks, so that the three paths a zoom stresses are each walked at each scale: a fill
(the coverage lane), a stroke (whose expansion is `direction`'s arithmetic, and whose
device delta grows with `s`), and a fill under a **non-rectangular clip** — the residue
path, which is where the caller's corpus starts refusing at 2× and not at 1×
(`doc/notes-tiling-ceiling.md`).

Verified able to fail by capping a coverage tile at 64 pixels a side in both lanes — a
quantity computed in device space and assumed into a range only scale 1 guarantees, which
is precisely what hayro's `#40` and `#63` were. All three tests pass at 1× and fail at 2×
or 4× under it:

```
at 2× the page carries 1984.09 pixels of ink where its area is 2048.00
at 4× the page carries 4096.00 pixels of ink where its area is 8192.00
at 4× the band carries 1024.00 pixels of ink where it is 2048.00
```

This is a floor, not a ceiling. What it does not buy is a *reference* comparison above 1×:
the golden test against the independent CPU rasteriser is still scale 1 only, and pointing
it at 2× and 4× is the next thing worth doing in this direction.

---

## What was verified able to fail, and how

Every gate, with the defect forced:

| Gate | Forced defect |
|---|---|
| `a_stroke_spanning_the_coordinate_range_is_not_drawn_as_nothing`, `a_stroke_across_the_coordinate_range_still_draws_its_band`, `a_stroke_segment_below_the_float_grid_still_draws_its_cap`, `a_segment_below_the_float_grid_produces_finite_geometry` | `direction`'s `hypot` fallback made unreachable |
| `an_edge_whose_slope_leaves_f32_deposits_nothing`, `a_mark_thinner_than_the_float_grid_paints_no_row` | the slope guard in `accumulate_edge` deleted |
| `a_viewport_transform_above_the_coordinate_bound_is_refused_by_name` | the bound disabled |
| `a_viewport_transform_at_the_coordinate_bound_still_draws` | the bound tightened to `>=` |
| `no_batch_is_noted_before_its_instance_is_written` | `note_batch`'s `- 1` removed |
| `a_zero_size_surface_target_is_refused_by_name` | the `Surface` arm changed to `NoSurface` |
| `a_layer_whose_plan_marks_nothing_draws_a_blank_frame` | `encode_group` made to refuse an empty body |
| `a_soft_mask_whose_group_marks_nothing_masks_everything` | `use_mask` made to skip a mask with no commands |
| `a_mark_with_a_zero_extent_axis_draws_nothing_and_stops_no_frame` | both zero-extent guards removed, in `coverage_tile` and in `parallel::rasterise` |
| the three `scale_invariance.rs` tests | a coverage tile capped at 64 pixels a side, in both lanes |
| `a_sample_count_that_is_not_a_multiple_of_the_pass_width_still_covers` | `chunks` → `chunks_exact` in `winding/buffers.rs` |

**Two gates could not be made to fail on their own, and both are stated rather than
skipped.** `a_tile_with_a_zero_side_is_never_admitted` and
`a_tile_with_a_zero_side_takes_no_place_on_the_sheet` are unit tests on the packers' own
doors: removing the guard makes the *packer* wrong in ways nothing above it can observe,
because both lanes already return before offering a zero-extent tile. They are regression
guards on an invariant two callers currently maintain, which is exactly the reason to state
it at the door as well.

---

## One trap for `HANDOVER.md`

**A cross-worktree build can fail with another worktree's source in the error.** One
`cargo test` in this round failed with `E0027: missing field 'stencil' in Command::Image`
against `encode.rs:394` — a field that does not exist anywhere in this worktree, and does
not exist at `HEAD`. A sibling agent was mid-build in the shared
`/home/AI/cargo-target/quorra` at that moment; re-running the same command immediately
afterwards succeeded with no change to any file. The existing trap ("the shared cargo target
dir is not yours alone") covers a stale *binary*; this is the same hazard producing a stale
*compile error*, which reads like a defect in your own tree and is not. Re-run before
believing an error that names a symbol you cannot find.
