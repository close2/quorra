# Reading `raster.rs` and `pipeline.rs` against the file-scale rule

Written 2026-08-16, rebased and re-verified 2026-08-17, in the shape of
`doc/notes-device-split.md` and `doc/notes-encode-split.md`. **Two files, two independent
decisions**, and the round was set up to allow either to be *leave it and say why* —
which is the outcome the `error.rs` round earned its value from and the one this round
expected for `raster.rs`.

Both decisions came out the same way in the end, and the reasoning is different in each
case. What follows is the reasoning, including the case against.

**One ADR accompanies the round**, and it is not about where the parts went — ADR 0051
settles that, and both modules are private to the crate, so the only question it leaves is
`pub(crate)` versus `pub(super)`, answered by "as private as it was". It is
`doc/adr/0061-a-split-modules-tests-keep-the-parents-path.md`: both files' tests stayed at
`<module>::tests`, the decision was taken silently once before (the `encode.rs` round) and
twice more here, and it has a cost worth writing down — a 704-line test file left past the
smell so that "the 554 test names are identical" stays available as the round's evidence.

## 0. The measurement the round starts from, and one correction to it

`doc/HANDOVER.md`'s debt list has been wrong about which files are past the ~500-line
smell twice. At the base of this round (`1a642ac`) the whole-file counts were:

| File | Whole | To `#[cfg(test)]` |
|---|---|---|
| `quorra-gpu/src/raster.rs` | 1 563 | 864 |
| `quorra-gpu/src/pipeline.rs` | 805 | 573 |
| `quorra-gpu/src/encode/parallel.rs` | 532 | 417 |
| `quorra-gpu/src/resources.rs` | 606 | — |
| `quorra-gpu/src/outline.rs` | 566 | — |
| `quorra-gpu/src/atlas.rs` | 537 | — |
| `quorra-scene/src/geom.rs` | 634 | — |

**The whole file is the number the rule is about**, and this round says so explicitly
because both previous corrections argued about which of the two counts to quote. A
reviewer holds the test module too — `raster.rs`'s tests were 694 of its 1 563 lines and
they are where three of its clause arguments are actually written down. The
`#[cfg(test)]` count is worth carrying only because it is what says whether the *code* is
one thing.

## 1. `raster.rs` — what it holds

The claim under review, from a recent round, was that `raster.rs` is "one stated
responsibility, and its module comment says so". Read whole and in order, that is not
what is in the file:

| # | Responsibility | Clause | Lines | Raised from |
|---|---|---|---|---|
| 1 | Flattening: segments + transform → device-space polylines, and how finely | §10.7.2, ADR 0044 | 170 | `encode/fill.rs`, `encode/stroke.rs`, `encode/clips.rs`, `encode/parallel.rs` |
| 2 | Filling: polylines → coverage bytes over a region | §8.5.3.3, ADR 0049 | 342 | `encode/coverage.rs`, `encode/clips.rs`, `encode/parallel.rs`, `atlas.rs` |
| 3 | Stroking: a stroke → closed polygons, to be filled non-zero | §8.4.3 | 290 | `encode/stroke.rs`, `encode/parallel.rs` |
| 4 | Tests | — | 694 | — |

Line counts are the items themselves and exclude the 29-line module comment, the single
`use`, and the blank lines between them.

**Three things follow, and each was checked rather than assumed.**

- **The three are raised separately.** `encode/stroke.rs` calls flatten *then* stroke and
  hands the polygons to the coverage lane; `encode/clips.rs` calls flatten then fill and
  never strokes; `encode/parallel.rs` calls all three. The seam is where things are used,
  and these are used at different call sites in different orders.
- **They are three clauses, not one.** §10.7.2 is a tolerance, §8.5.3.3 is a rule for
  insideness, §8.4.3 is the geometry of caps and joins. Nothing in one constrains the
  other two.
- **The module comment's claim is false for a third of the file.** "The one producer of
  coverage bytes" is a statement about what leaves the module; `stroke_polylines`
  produces no coverage byte at all — it takes `Polyline`s and hands back `Polyline`s. By
  the same logic `encode.rs` at 2 421 lines was "the one producer of an encoded frame".
  **A comment that claims one responsibility over a file holding three is worse than no
  comment**, which is exactly the shape the round was told to look for.

### The case against splitting, and why it lost

It is a real case and it is the reason this file was left alone twice:

1. **This is the hottest path in the tree and the most safety-critical file in it.**
   Three arithmetic defects have been found in it — `deposit_slab`'s smeared border
   (ADR 0049, 185 of 255), `direction`'s length leaving `f32` at both ends, and
   `accumulate_edge`'s non-finite slope. A careless move here costs more than it does
   anywhere else, and §6 of this note is what that risk actually cost this round.
2. **The three parts are one pipeline.** A caller never wants a polyline; it wants bytes.
   Reading the file top to bottom is reading a mark's whole journey.
3. **The tests do not divide.** Almost every test goes through all three parts — a cap's
   area is measured by *filling* the polygons the stroke expands.

What answers (1) is that a split moves no line: the round's instrument is that every
statement is the same text, the 554 test names are identical and the seven archetype
counter rows are identical. What answers (2) is that the *order* is what the parent's
module comment now states, and stating it costs three lines where the file cost a reader
864. (3) is true, and it is why the tests are **one file** and not three — see §2.

### The decision, and what each part's one thing is

Split, along the three clauses, with the tests kept whole:

| Module | Lines | Its one thing |
|---|---|---|
| `raster.rs` | 61 | the definition of coverage, why it is ours (ADR 0008), and the map |
| `raster/flatten.rs` | 214 | an outline's segments, under a transform, as device-space polylines — and how finely |
| `raster/stroke.rs` | 310 | §8.4.3's stroke expanded into closed polygons: polylines in, polylines out, no coverage anywhere |
| `raster/fill.rs` | 359 | polylines into coverage bytes over a region: the grid, the two rules, and the cut at the border |
| `raster/tests.rs` | 704 | what the three produce, on shapes whose answer is derivable by hand or by a clause |

`raster` is **private to the crate** (`mod raster;` in `lib.rs`), so ADR 0051's public-path
question does not arise; the only visibility question it leaves is `pub(crate)` versus
`pub(super)`, answered by "as private as it was".

**Where each type went, and why there.** `CoverageMask` is in `fill.rs` because its
`crop` is a lookup *only* because `fill_mask` cuts at its region's border — the invariant
and the type that depends on it are now in one file. `Rule` is in `fill.rs` because it is
read in exactly one place, the prefix sum. `DeviceTransform` and `Polyline` are in
`flatten.rs` because flattening is the only thing that consumes a transform and the only
thing that manufactures a polyline. `polyline_bounds` went with `Polyline` rather than
with the fill that uses it to pick a region.

**Nothing became more visible.** Eight items keep `pub(crate)` through a re-export from
`raster` — `CoverageMask`, `Rule`, `fill_mask`, `DeviceTransform`, `Polyline`, `flatten`,
`polyline_bounds`, `stroke_polylines` — so `crate::raster::fill_mask` and its seven
siblings resolve exactly as before and **no path outside the module changed**.
`FLATTEN_TOLERANCE` and `RELATIVE_FLATTEN_TOLERANCE` are *not* re-exported: nothing
outside `raster` ever asked for either, and `raster/tests.rs` imports them from the module
that owns them. `cubic_tolerance` widened from private-to-`raster.rs` to `pub(super)`,
which is the same reach it had when `raster.rs` *was* the module.

### The dependency the split makes visible

`stroke` and `fill` each depend on `flatten` and **neither knows about the other**. That
is a fact about the code that was true before and unreadable before; it is now the
parent's module comment and the two `use super::flatten::Polyline;` lines. The
consequence worth having: **a defect in one of the three is a defect in one clause**, and
the three this code has had are one function of one part each — `stroke::direction`,
`fill::accumulate_edge`, `fill::deposit_slab` — with none of them reachable by reading
another.

## 2. What the `raster.rs` round judged irreducible, and left

*(The `pipeline.rs` half of the round is §4 to §6; the rebase is §7.)*

- **`raster/tests.rs` at 704 lines.** It is past the smell and it is left whole, as a
  decision rather than an oversight — ADR 0061. Two reasons. The tests do not divide
  along the source's seams — `each_cap_deposits_the_area_table_53_gives_it` is a statement
  about §8.4.3 *read out of* §8.5.3.3's bytes, and
  `a_circle_deposits_its_own_area_at_every_size` is a statement about §10.7.2 read the
  same way — so a file per source module would put most tests in the wrong one. And
  splitting them **renames every test**: `--list` reports a test by its module path, so
  `raster::tests::aligned_rectangle_is_exact` would become
  `raster::fill::tests::aligned_rectangle_is_exact`, and this round's evidence that no
  behaviour moved is that the 554 names are identical. Keeping the tests at
  `raster::tests` (the file is `raster/tests.rs`, the path is unchanged) is what makes
  that evidence available at all.
- **`fill.rs` at 359 lines.** One subject: the accumulation grid, at three scales. The
  three deposit functions are one rule — cut at the border, then split at each cell
  boundary, then deposit one trapezoid — and ADR 0049's argument for the cut is only
  checkable next to the arithmetic it replaced.
- **`stroke_polylines`'s 73 lines.** Four phases with a comment each (dedupe, quads,
  joins, caps), under clippy's threshold, and on the hottest walk in the tree. Naming the
  phases as functions is a defensible next edit; it is not a move, so it is not this
  round's.
- **`accumulate_edge`'s `#[allow(clippy::too_many_arguments)]`** is the only `#[allow]` in
  the subtree that is not a cast or an arithmetic bound, and its reason is stated: "the
  coordinate bundle is the function's whole input, and a struct would only rename the
  eight numbers". There is **no `too_many_lines` and no `cognitive_complexity` allow
  anywhere in `quorra-gpu`**, in these two files or outside them.

## 3. Behaviour and API: what the `raster.rs` round checked, and how

- **`tests/archetypes.rs`'s seven counter rows** (now built from `quorra-pages`,
  ADR 0060), recorded on the base and again after the split, identical — every field is an
  exact function of the scene and the viewport, so this is the instrument a rasteriser
  that changed would fail:

  | archetype | commands, culled, outlines, atlas keys, clip regions, tiles, layer textures, residue regions, residue tiles, coverage texels |
  |---|---|
  | median page | `12, 0, 9, 12, 0, 0, 0, 0, 0, 0` |
  | dense text | `4320, 0, 818, 2164, 1, 40, 0, 0, 40, 8956` |
  | artwork | `684, 0, 300, 300, 1, 600, 3, 66, 384, 3542360` |
  | image page | `232, 0, 60, 158, 4, 0, 0, 0, 0, 0` |
  | clip mountain | `1200, 0, 200, 800, 1200, 0, 0, 0, 0, 0` |
  | giant | `1500, 0, 1500, 1500, 0, 0, 0, 0, 0, 0` |
  | drawing | `1200, 0, 1200, 1194, 0, 6, 0, 0, 0, 245` |

- **Tests**: `RUSTFLAGS="-D warnings" cargo test --workspace` — **551 passed, 0 failed,
  3 ignored, 77 suites** before and after, cargo's own exit status read from a file rather
  than through a pipe. `-- --list` gives **554 names, sorted, identical** before and
  after, and they reconcile: 550 `#[test]` functions in `crates` (the `grep` extraction of
  555 includes five helper `fn`s that sit within two lines of a `#[test]`) plus 4
  doctests; the three `#[ignore]`d tests are listed.
- **Clippy**: `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` clean, with
  `Checking quorra-gpu` printed rather than only `Finished`. No `#[allow]` was added.
- **Docs**: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` clean.
  `cargo doc --document-private-items` gives **34 warnings for `quorra-gpu` before and
  34 after, none of them in `raster`** — the base was measured in this worktree by
  restoring the file, not quoted from an older round. Two doc links needed paths, which is
  ADR 0051 §2's cost paid again: `ARC_STEP`'s reference to `FLATTEN_TOLERANCE` is now
  `[`FLATTEN_TOLERANCE`](super::flatten::FLATTEN_TOLERANCE)`, and `flatten` is a module
  *and* a re-exported function, so the three links that mean the module say `mod@flatten`.
- **Format**: `cargo fmt --all --check` clean. One line was reflowed by rustfmt — a
  closure in a test that fits in 100 columns once the test module's four spaces of
  indentation are gone. It is the only line of the round whose text differs from its
  original for any reason other than a `mod` declaration, an import or a visibility.
- **No line of code was lost or invented.** Every non-comment, non-blank line of the base
  file was compared as a multiset against the split subtree. The differences are exactly:
  the `mod tests {` wrapper and its brace; the `#[allow]` on that module becoming an inner
  `#![allow]`; the reflowed closure; the import block, reflowed and split three ways; three
  `mod` lines, four `mod` declarations and three `pub(crate) use` lines; and
  `cubic_tolerance`'s `pub(super)`. **Not one statement differs**, and §7 is why that
  comparison is the load-bearing one rather than a formality.

## 4. `pipeline.rs` — what it holds

805 lines, 573 to the test module. Read whole:

| # | Responsibility | Lines | Raised from |
|---|---|---|---|
| 1 | The module comment: §7's startup rules, and the map | 56 | — |
| 2 | `captured`: how a `wgpu` creation failure becomes a value rather than a panic | 21 | here, `pipeline/function.rs`, `present.rs` |
| 3 | `Modules`: the nine WGSL modules, parsed all-or-nothing | 45 | `base`, and `spec.rs` reads them |
| 4 | The store's state, its lock, its laziness, `get`, `base`, `compile` | 189 | every frame, through `get` |
| 5 | The eleven layout accessors and their one helper | 67 | `device/binds.rs`, `device/rare.rs`, `winding/`, `present/`, `compose/` |
| 6 | The warm-up: the guard, the thread, the two sets, and three observers | 166 | `device/construct.rs` — construction, `is_warm`, `wait_until_warm`, `StartupTimings` |
| 7 | Tests | 231 | — |

**The file's own comment named the seam.** It read: "this file is the store — its one
lock, its laziness *and its warm-up*". Two things joined by "and" is the same tell
`doc/HANDOVER.md` records from `push_op`'s two-openings doc comment, and it had been
sitting in the module comment since the file was three files.

### The case against splitting, and why it lost

- **The warm-up is only a caller of `get`.** True, and it is the argument *for*: a
  module whose only coupling to its parent is one method call and one field is a module.
- **The `Condvar` field stays behind.** Also true, and it comes out better than it went
  in: after the move, `warmed` is declared in `pipeline.rs` and **every line that waits on
  it or notifies it is in `warm.rs`**, so "exactly one notifier, on every exit path" — the
  ADR 0042 invariant — becomes a property of one file. The field carries a comment saying
  so, which is the only new prose in the parent.
- **Nothing was measured to be wrong here.** Correct: this is a legibility change and it
  is stated as one. `pipeline.rs` is not on a hot path; `get` is a `HashMap` lookup under a
  lock, and `lto = "fat"` with `codegen-units = 1` means a module boundary is not a
  codegen-unit boundary, so no inlining decision can turn on which file a function is in.

### The decision

| Module | Lines | Its one thing |
|---|---|---|
| `pipeline.rs` | 428 | the store: one lock, one map, laziness, and the compile that fills it |
| `pipeline/warm.rs` | 198 | the warm-up: which pipelines are compiled before anyone asks, and the state machine that reports what became of that |
| `pipeline/tests.rs` | 243 | what the store and its warm-up promise, asked through them |
| `pipeline/layouts.rs` | 299 | (unchanged) the binding tables — the half that cannot be refused |
| `pipeline/spec.rs` | 343 | (unchanged) what each `Kind` *is* |
| `pipeline/function.rs` | 435 | (unchanged) ADR 0053's generated shaders, keyed by a content hash |

**Nothing became more visible.** `spawn_warm_up`, `warm_up`, `warm_duration` and
`wait_until_warm` keep `pub(crate)` and are still reached as methods on `PipelineStore`,
so `device/construct.rs`'s four call sites are unchanged to the character. Three items
widened from private-to-`pipeline.rs` to `pub(super)` — `WarmUpGuard`, its `new`, and
`warm_up_now` — which is the reach they already had, because `pipeline::tests` and
`pipeline::function` are descendants of the module they were private to.
`warm_presenting_lanes` and `WarmUpGuard::finish` went the other way: they are private to
one 198-line file where they were private to an 805-line one.

## 5. What the `pipeline.rs` round considered and left

- **The eleven layout accessors were not moved to `layouts.rs`**, though the helper's own
  doc points there ("Infallible, and that is the point of `layouts.rs` being its own
  module"). They stay because they are the *store's* laziness applied to layouts — each
  one takes `self.lock()` and fills the cell on first need — while `layouts.rs`'s one thing
  is the descriptions themselves, which no lock and no caching appear in. Moving them
  would have taken `pipeline.rs` to 361 lines and `layouts.rs` to 366; that is a
  line-count argument, and CLAUDE.md names that as the thing not to do.
- **`Modules` stayed in the store's file.** It is 45 lines, `base` pairs it with `Layouts`
  three lines below, and `compile` is the only thing that ever holds one. Its own module
  would be a file whose one thing is a nine-line constructor.
- **`captured` stayed in the parent**, where `Modules::new`, `compile`, `function.rs` and
  `present.rs` all reach it. It is the subtree's shared primitive, which is what a parent
  module is for.
- **The tests are one file at `pipeline::tests`**, for the same reason `raster/tests.rs`
  is (ADR 0061): five of the eight are the warm-up's and three are the store's, and
  dividing them renames all eight.
- **`pub mod pipeline;` in `lib.rs` exposes a module with no public items at all** —
  every item in the subtree is `pub(crate)` or narrower. It is why the four doc links this
  round first wrote into the module comment failed `RUSTDOCFLAGS="-D warnings"` as
  "public documentation links to private item", and why the map's names are code spans
  rather than links. Making it `mod pipeline;` would break no caller (there is nothing to
  import) and would delete a whole empty page from the published documentation, but it is a
  change to a public path, so it belongs to a bump rather than to a refactor.

## 6. Behaviour and API: what the `pipeline.rs` round checked, and how

Same instruments, run again after the second commit: **551 passed, 0 failed, 3 ignored,
77 suites**; the **554 test names sorted and identical**; the seven archetype counter rows
identical (the wall clocks beside them are not, and are not evidence of anything —
`giant` read 0.58 s on one run and 7.6 s on another under a neighbour's build);
`RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` clean with `Checking
quorra-gpu` printed; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` clean;
`cargo doc --document-private-items` **34 before, 34 after, and the only one in the
subtree is `spec.rs`'s pre-existing unresolved `PipelineStore::compile`**;
`cargo fmt --all --check` clean with **no line reflowed at all** — the warm-up's methods
kept their indentation because they moved into another `impl PipelineStore` block.

**No line of code was lost or invented.** The multiset comparison of every non-comment,
non-blank line gives exactly: the tests module's `#[allow]` becoming `#![allow]`; three
`pub(super)`s; one extra `impl PipelineStore {` and its brace; `mod tests {` becoming
`mod tests;` plus `mod warm;`; and `warm.rs`'s seven import lines. **Not one statement
differs.**

One stale reference was found and fixed by the move rather than by luck: `WarmUpGuard`'s
doc said "the error scopes **above** make the known panic a value", and `captured` is no
longer above it. It now says `[`captured`](super::captured)` — a path, not an import
(ADR 0051 §2).

## 7. The rebase, and the lesson it is worth more than the split

The round was first done on `eada81e` and offered for merge against a `main` that had
moved twenty commits, **two of them inside `raster.rs`**: `direction` gained a `hypot`
second path (a length that overflowed above `1.9e19` to an infinite length, a `(0, 0)`
normal and a stroke drawn as nothing, and underflowed below `1.1e-22` to NaN geometry),
and `accumulate_edge` gained a `!dxdy.is_finite()` early return (a NaN that survived the
prefix sum, where `abs().min(1.0)` returns **1.0** for a NaN, so an invisible sliver
painted the rest of its row solid).

**Merging the split as it stood would have deleted both fixes and passed every gate.**
Every test would still have been green, because the tests that cover those fixes were
*also* in the stale copy of the file — a split replaces a file wholesale, so a stale base
takes the fixes and their tests out together, and nothing is left to fail.

Three things follow, and they are the round's real finding:

1. **A split is a whole-file replacement, so its base is part of its correctness.** For an
   edit, a rebase conflict is the safety net; for a split there is nothing to conflict
   with, because the file the other change touched no longer exists on this side.
2. **The multiset comparison is what catches it**, and only if it is run against the base
   the branch will actually merge into. Run against `eada81e` it said "not one statement
   differs" — truthfully, and about the wrong file. Run against `1a642ac` it says the same
   sentence and it means something: `raster/stroke.rs` contains `dx.hypot(dy)` and
   `raster/fill.rs` contains `if !dxdy.is_finite()`, both checked by name.
3. **Redoing beat rebasing.** `git rebase` would have offered a conflict in a file this
   branch had reduced to sixty lines, with the incoming hunks belonging in three files
   that did not exist in the base. Re-running the extraction against the new `raster.rs`
   took one command per chunk and produced a comparison that proves itself.

The parent's module comment was updated as part of the redo: it named two arithmetic
defects and now names three, because `direction`'s was not "a normal at a degenerate
segment" but a length leaving `f32` at both ends, and the third — `deposit_slab`'s smeared
border — belongs beside them.
