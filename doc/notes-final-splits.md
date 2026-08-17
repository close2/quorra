# The last four files on the size list: two split, two declined

Written 2026-08-17, against CLAUDE.md principle 1's file-scale rule and `doc/adr/0051`,
and in the shape of `doc/notes-device-split.md` and `doc/notes-encode-split.md`, which
are the worked examples this round follows without amendment. Base: `eada81e`, confirmed
before a line was written.

Four files were handed to this round in size order — `geom.rs` (634),
`resources.rs` (606), `outline.rs` (566), `encode/parallel.rs` (532), with `atlas.rs`
excluded because a sibling was in it. **All four were reached, and two of them are not
split.** The decline is the interesting half and it is argued first in each section,
because a round told to split five files that splits five files has not made four
decisions.

## 0. Three corrections to the brief this round was given, made before anything else

Recorded first because each of them would otherwise become a false citation in the
record, and one of them is the size list itself for the fourth time.

1. **ADR 0061 and 0062 do not exist in this tree.** The highest ADR is 0056, and the
   next free number is **0057**, not 0065. The method those two were described as
   carrying — *ask what a test is a statement about, not what its call graph touches* —
   is genuinely the right question and is used throughout below, but it is used **on its
   merits and cited to nothing**, because a citation to a document that is not here is
   worse than no citation. Two commit messages of this round were rewritten before they
   were final for exactly this reason.
2. **`doc/notes-error-split.md` and `doc/notes-raster-pipeline-read.md` do not exist
   either.** `doc/notes-device-split.md` and `doc/notes-encode-split.md` do, and they
   were read in their place.
3. **The size list is still wrong, and this is its fourth statement.** Measured on the
   whole file with `wc -l` over `crates/**/src`, the ranking on `eada81e` was:

   | file | lines |
   |---|---|
   | `quorra-gpu/src/raster.rs` | **1 400** |
   | `quorra-gpu/src/pipeline.rs` | **805** |
   | `quorra-scene/src/geom.rs` | 634 |
   | `quorra-gpu/src/resources.rs` | 606 |
   | `quorra-gpu/src/outline.rs` | 566 |
   | `quorra-gpu/src/error.rs` | 558 |
   | `quorra-gpu/src/encode/parallel.rs` | 532 |
   | `quorra-gpu/src/atlas.rs` | 509 |

   **The two largest source files in the tree were not on the list this round was
   given**, and `error.rs` — which `doc/HANDOVER.md`'s own debt bullet names as "the next
   split candidate" — was not either. `atlas.rs` was listed at 537 and is 509. Nothing
   here touched `raster.rs`, `pipeline.rs` or `error.rs`; they are named in §6 so that
   the fifth statement of that bullet can be right.

## 1. `geom.rs` — split, three ways

### The decline, argued first

`geom` is the coordinate vocabulary of the crate the caller links directly (ADR 0001),
its five types are the contract, and its first bullet says the boundary mirrors the
caller's own `pdf_render::geom` — which is itself one file, of 997 lines. Splitting a
*public* module carries ADR 0051's cost 1: rustdoc inlines a re-export from a private
module, so the seam becomes invisible in the published documentation and the parent's
comment is the only place it survives. And the types are mutually referential: `Rect` is
two `Point`s, `Affine::apply` maps a `Point`, `axis_aligned_rect` maps `&[Segment]` to a
`Rect`.

### Why it lost

- **The file's own module comment names four subjects joined by "and".** The exemption
  CLAUDE.md offers a long file is available "when the module comment says what that one
  thing is", and no honest one-thing sentence was available here: the comment was already
  a list.
- **The tests divide with nothing left over.** Asked what each is a *statement about*
  rather than what it calls, the ten split six / two / two: six are matrix algebra that
  never mentions a rectangle, two are `Rect`'s two distinct questions, two are the
  recogniser's accept and reject sets. Nothing straddles.
- **The subjects are not the same kind of thing.** Measured in non-comment, non-blank
  lines: `Affine` 120, the recogniser 48, `Rect` 37, `Point`+`Size` 20, `Segment` 11,
  tests 186 — 422 in all. The vocabulary is a fifth of the code; the rest is arithmetic
  with a clause behind it (§8.3.3) and a lane decision with a brief section behind it
  (§6.4, ADR 0007).

### Where the parts went

| module | lines | its one thing |
|---|---|---|
| `geom.rs` | 57 | the contract, and the map — nothing else |
| `geom/shape.rs` | 143 | where a mark is and how big: `Point`, `Size`, `Rect` |
| `geom/affine.rs` | 309 | §8.3.3's matrix, and the four questions other subsystems ask of one |
| `geom/segment.rs` | 187 | the one step an outline is made of, and the one shape a run of them is *recognised* as |

`axis_aligned_rect` is filed with `Segment` rather than with the `Rect` it returns
because it is a decision about which lane a mark takes, and its refusals are all
statements about segment shapes — a curve, an extra corner, a second `MoveTo`.

**Public paths are unchanged.** The parts are private `mod`s re-exported from the parent
(ADR 0051), so `quorra_scene::geom::Affine` and `quorra_scene::Affine` both resolve as
before, `crate::geom::{Affine, Point, Rect}` — which `scene/builder.rs`, `error.rs`,
`paint.rs`, `scene/validate.rs` and five others write — resolves as before, and
`quorra_scene::geom::affine::Affine` is not a path that exists, so it is not a path we
have to keep. The caller writes `quorra_scene::Affine`, `quorra_scene::Point` and
`quorra_scene::Rect` at the crate root (`render-quorra/src/present.rs`, `lib.rs`) and
never `geom::` at all — checked read-only in their tree; **nothing there needs to
change.**

## 2. `outline.rs` — split, two ways

### The decline, argued first

Both halves are ADR 0016, both are named in the same module comment, and the triangle
builder consumes exactly what the converter produces. `QuadOutline` would have methods in
two files.

### Why it lost: the seam is *when*, and the callers prove it

The two halves have **no caller in common**:

- the conversion (`from_segments` and everything it reaches) has exactly one caller,
  `resources.rs`, and runs once per outline **at upload**. That is the whole reason a
  frame at 100× costs what a frame at 1× costs;
- the triangles (`append_triangles`, `append_polyline_triangles`, `WindingVertex`) have
  four callers, all on the frame path — `encode/coverage.rs`, `pane.rs`,
  `winding/sheet.rs`, `pipeline/spec.rs` — and run once **per placement per frame**.

The six tests divide four / two the same way. The one that looks mixed is not:
`a_degree_elevated_quadratic_converts_exactly` calls `append_triangles`, but only as the
instrument that reads a control point back out — it is a statement about the conversion.

| module | lines | its one thing |
|---|---|---|
| `outline.rs` | 423 | the conversion: cubics to quadratics, subpaths to closed contours, once at upload |
| `outline/triangles.rs` | 190 | the triangles a filled outline becomes, and the vertex the winding pass reads |

**Nothing widened.** A child module sees its parent's private items, so `QuadOutline`'s
two private fields are reached from `triangles.rs` exactly as they were from the one
file, and `WindingVertex::solid`/`curve` became *more* private — they were private to a
566-line file and are now private to a 190-line one. `WindingVertex` and
`append_polyline_triangles` keep `pub(crate)` through a re-export, so
`crate::outline::WindingVertex` and `crate::outline::append_polyline_triangles` resolve
exactly as before and no path outside the module changed.

## 3. `resources.rs` — **not split**

The candidate seam was the obvious one: the five `upload_*` in a module, the registry
(five id spaces, one budget, one generation, five lookups) left behind. It is refused on
a fact rather than on taste.

- **`charge` and `allocate_id` have no caller anywhere but those five methods** —
  verified by grep across the crate; the other `charge(` hits are `Encoder::charge`,
  a different function in a different type. The cut would separate two private helpers
  from every call site they have, which is a cut through the middle of one operation.
- **The order is the subject.** Allocate, then charge, then insert, so that a refusal at
  any step has stored nothing and charged nothing. Each of the five uploads promises that
  in its own doc comment; the two helpers keep it; and the only thing that checks five
  promises against two implementations is a reader with both in front of them. Same
  argument as `notes-encode-split.md` §4 for `coverage.rs` and `notes-device-split.md` §4
  for `render.rs`.
- **The five uploads are one shape written five times** — validate against the clause the
  resource has, convert, price, allocate, charge, insert — and reviewing the fifth *is*
  comparing it against the first four.

So the deliverable is the module comment that earns the exemption, and it states the
evidence and not only the claim, because `doc/HANDOVER.md`'s trap is exactly that a
comment asserting one responsibility is evidence about the comment. It names the one
thing, the two facts that make the candidate seam wrong, and what the length buys: of 635
lines, **417 carry code and 150 of those are the tests**, longest item forty lines.

The file grew by 28 lines of comment in the course of not being split. That is the honest
price of an exemption that has to be earned in writing.

## 4. `encode/parallel.rs` — **not split**

It had already given up the seam it had (`commit` is step 3, in `parallel/commit.rs`,
256 lines). What was proposed for the rest — the `Job` record apart from the fan-out — is
not there:

- **`rasterise` is the join, not a member of either half**: the pure function of a `Job`
  that the whole byte-equality claim rests on;
- **`partition` balances by `Job::weight` and the commit bounds the queue by
  `Job::held`**, so a fan-out module without the record is arithmetic over fields it
  cannot see, and the two constants are in the same position;
- **the four tests do not divide**: three are statements about the partition and the
  floor, the fourth about the queue's bound, and **none is a statement about a `Job` on
  its own** — which is the tell that the record is not a subject.

Of 559 lines, 313 carry code and 96 of those are the tests; sixty are the module comment,
which is the design argument the caller's `doc/QUORRA_ENCODE_THREADS.md` asked for and
the reason the determinism claim is checkable at all.

This one also retires a stale reason: `doc/HANDOVER.md` says the file "was left because
ADR 0054 had landed in it two commits earlier", which was true in the round that wrote it
and says nothing now. A debt whose only stated reason has expired is a debt nobody can
act on, so the argument above is now in the file itself.

## 5. Evidence that no behaviour moved

Every run below used a **private `CARGO_TARGET_DIR`** (`/home/AI/cargo-target/quorra-finalsplits`),
after `find crates -name '*.rs' -o -name '*.wgsl' | xargs touch`, and cargo's own exit
status was read from a redirected file rather than through a pipe.

- **`tests/archetypes.rs`'s seven counter rows are byte-identical**, base against the
  finished round — the same rows `notes-encode-split.md` recorded:

  | archetype | commands, culled, outlines, atlas keys, clip regions, tiles, layer textures, residue regions, residue tiles |
  |---|---|
  | median page | `[12, 0, 9, 12, 0, 0, 0, 0, 0]` |
  | dense text | `[4320, 0, 818, 2164, 1, 40, 0, 2, 0]` |
  | artwork | `[684, 0, 300, 300, 1, 600, 3, 185, 0]` |
  | image page | `[232, 0, 60, 158, 4, 0, 0, 0, 0]` |
  | clip mountain | `[1200, 0, 200, 800, 1200, 0, 0, 0, 0]` |
  | giant | `[1500, 0, 1500, 1500, 0, 0, 0, 0, 0]` |
  | drawing | `[1200, 0, 1200, 1194, 0, 6, 0, 0, 0]` |

- **Tests**: `cargo test --workspace`, exit status **0**, **445 passed, 0 failed, 2
  ignored across 55 result lines**, before and after, identically. The arithmetic
  reconciles exactly: 444 `#[test]` attributes in `crates`, two of them `#[ignore]`d,
  gives 442 run, plus 3 doctests = 445. `-- --list` reads **447 both times** (444
  attributes + 3 doctests). *(`doc/HANDOVER.md`'s successor and this round's brief both
  quote "564 passing" — that is not this tree at `eada81e`, and the reconciliation above
  is what says so.)*
- **The sorted test-name lists differ in exactly twelve lines, all of them paths, none of
  them names.** The mapping is §5.1.
- **Clippy**: `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`, exit
  status 0, with `Checking` printed for all four crates — so the gate looked at the
  edited files rather than reporting a cached `Finished`.
- **Docs**: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`, exit status 0.
  `cargo doc --document-private-items` gives **33 warnings for `quorra-gpu` before and
  after** — none in `outline`, `resources` or `encode/parallel` — and **one for
  `quorra-scene`**, in `scene/cost.rs`, which no file this round touched.
- **Format**: `cargo fmt --all --check` clean.
- **No line of code was lost or invented**, checked as a multiset of every non-comment,
  non-blank line, base against result:
  - `geom`: 422 → 438. Only in base: one `use super::{…}` from the old test module. Only
    after: three `mod`s, three `pub use`s, four imports, and the two extra
    `#[cfg(test)] mod tests { … }` wrappers. Not one statement differs.
  - `outline`: 372 → 384. **Nothing at all is only in the base** — the difference is
    empty in that direction. Only after: one `mod`, one `pub(crate) use`, four imports,
    an `impl QuadOutline {` wrapper and one extra test-module wrapper.
  - `resources`: 417 → 417, **identical in both directions**.
  - `encode/parallel`: 313 → 313, **identical in both directions**.
- **Why this could not be a performance change**, for the two files that did move text:
  the release profile is `lto = "fat"` with `codegen-units = 1`, so a module boundary is
  not a codegen-unit boundary and no inlining decision turns on which file a function is
  written in. Nothing moved between crates, no signature's types changed, no order
  changed.

### 5.1 The full old-name → new-name mapping

A renamed test is a gate whose identity changed, and anyone bisecting needs the map. No
test's own name changed; twelve module paths did.

| before | after |
|---|---|
| `geom::tests::composition_is_application_order` | `geom::affine::tests::composition_is_application_order` |
| `geom::tests::composition_agrees_with_sequential_application` | `geom::affine::tests::composition_agrees_with_sequential_application` |
| `geom::tests::preserves_axes_covers_quarter_turns_and_rejects_shears` | `geom::affine::tests::preserves_axes_covers_quarter_turns_and_rejects_shears` |
| `geom::tests::max_stretch_of_scale_and_rotation` | `geom::affine::tests::max_stretch_of_scale_and_rotation` |
| `geom::tests::invert_round_trips` | `geom::affine::tests::invert_round_trips` |
| `geom::tests::invert_refuses_degenerate_and_non_finite` | `geom::affine::tests::invert_refuses_degenerate_and_non_finite` |
| `geom::tests::rect_ordered_and_empty_are_distinct` | `geom::shape::tests::rect_ordered_and_empty_are_distinct` |
| `geom::tests::intersection_overlap_and_disjoint` | `geom::shape::tests::intersection_overlap_and_disjoint` |
| `geom::tests::axis_aligned_rect_recognises_rectangles` | `geom::segment::tests::axis_aligned_rect_recognises_rectangles` |
| `geom::tests::axis_aligned_rect_rejects_non_rectangles` | `geom::segment::tests::axis_aligned_rect_rejects_non_rectangles` |
| `outline::tests::a_chord_triangle_is_never_discarded` | `outline::triangles::tests::a_chord_triangle_is_never_discarded` |
| `outline::tests::the_vertex_stride_matches_its_floats` | `outline::triangles::tests::the_vertex_stride_matches_its_floats` |

`outline::conversion_error::the_conversion_stays_inside_its_stated_bound` and
`outline::tests::a_degree_elevated_quadratic_converts_exactly`,
`…::an_open_contour_is_closed_by_a_chord` and `…::a_degenerate_contour_emits_no_triangles`
kept their paths — they stayed in the parent with the conversion.

## 6. Findings noticed and deliberately not acted on

1. **`raster.rs` at 1 400 lines and `pipeline.rs` at 805 are the two largest source files
   in the tree and were on no list this round was given.** `raster.rs` is more than twice
   the file this round's own brief describes it as ("864 lines"), which means it grew
   after that reading and nobody re-measured. They are the next two candidates and they
   are not close.
2. **`error.rs` at 558 lines is the file `doc/HANDOVER.md` itself calls "the next split
   candidate"** and it was not on the list either. It is above `parallel.rs`, which was.
3. **`geom.rs`'s `Size` has one user in the workspace** — `examples/rect_lane.rs` — plus
   the re-export in `quorra`'s `lib.rs`. It is public API of the caller-facing crate, so
   it stays; recorded because a reader of `shape.rs` will wonder.
4. **The `#[allow(clippy::expect_used)]` on `outline.rs`'s remaining `tests` module no
   longer has an `expect` under it.** Preserved verbatim by the rule that a move changes
   no text, and it is a policy statement rather than a claim about the file's contents;
   deleting it is a one-line edit for whoever next owns that module.
5. **`quorra-scene` has one private-doc warning**, `scene/cost.rs:32`, "redundant explicit
   link target" on `[`Paint::Function`](crate::paint::Paint::Function)`. Pre-existing,
   one line, and outside everything this round touched.
6. **A public `Device` documents in five `impl` blocks and `QuadOutline` now documents in
   two**, which is the file-scale analogue of ADR 0051's cost 1 recorded again rather
   than newly. `QuadOutline` is `pub(crate)`, so no published page changes.
