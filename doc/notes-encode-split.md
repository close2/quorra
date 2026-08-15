# Splitting `encode.rs`: the responsibilities it held, and where each one went

Written 2026-08-15, against CLAUDE.md principle 1's file-scale rule and `doc/adr/0051`,
and in the shape of `doc/notes-device-split.md`, which is the worked example this round
follows without amendment. `encode.rs` was **2 421 lines** — the last file in the tree
far past the ~500-line smell, and the second of the two `doc/HANDOVER.md` names.

No ADR accompanies this round either. ADR 0051 decided where the parts of a split module
go, and `encode` is *private to the crate* (`mod encode;` in `lib.rs`), so the only
question it raises is `pub(crate)` versus `pub(super)` — answered by "as private as it
was". The judgement worth recording is §4: what was left alone.

## 1. What the file held, written down before anything moved

It had already lost the seams that were easy to see: seven modules existed under
`src/encode/` before this round. What was left, read whole and in order, was eleven more.

| # | Responsibility | Lines |
|---|---|---|
| 1 | The walk: `Encoder`'s state, `encode`, the dispatch, `resolve_clip`, the budget | 291 |
| 2 | The frame the walk hands back: `Encoded`, `retained_bytes`, `finish` | 192 |
| 3 | The layer tree's nodes and appends: `LayerPlan`, `Op`, `plan_mut`, `push_op`, `append_op` | 84 |
| 4 | Child layers: `ChildOp`, `MaskPlan`, `blend_word`, the group arm, `use_mask`, `push_child` | 290 |
| 5 | The instance streams: two strides, `Batch`, `DrawStyle`, three writers | 158 |
| 6 | Coverage: the two lanes and the four conditions that choose between them | 322 |
| 7 | Device space: `compose`, `apply`, `tile_side`, `CULL_MARGIN`, `target_rect`, `culled` | 147 |
| 8 | The analytic rectangle lane: `clipped_device_rect`, `encode_rect` | 80 |
| 9 | The fill arm: `SolidFill`, `linear_bits`, `encode_fill`, `fill_solid` | 307 |
| 10 | The stroke arm | 97 |
| 11 | Tests | 324 |

Line counts are the items themselves, so they exclude the blank lines between them and
the 82-line module comment and import block at the top. The count is the point, as it
was for `device.rs`: this is not one responsibility at 2 421 lines, and it is not two.

## 2. The seams, one commit each

Twelve commits. Eleven are moves of text, verbatim, plus the imports the new file needs
and a module comment saying what its one thing is; the twelfth is the module map.

| Module | Lines | Its one thing |
|---|---|---|
| `encode.rs` | 435 | the walk: the encoder's state, the one pass, the dispatch |
| `encode/encoded.rs` | 216 | what one walk hands back, and what the walk cost |
| `encode/plan.rs` | 104 | the ops one layer draws, in order, and the box that holds them |
| `encode/layer.rs` | 380 | a child layer: what becomes one, and the composite that puts it back |
| `encode/instance.rs` | 185 | one mark's instance bytes, and the run it joins |
| `encode/coverage.rs` | 358 | where a mark's coverage comes from, and the one place that decides |
| `encode/device_space.rs` | 138 | where a coordinate lands, and whether it lands on the target |
| `encode/rect.rs` | 110 | ADR 0007's analytic lane, and the box both its commands are held to |
| `encode/fill.rs` | 340 | which of three lanes draws a fill, and what all three are handed |
| `encode/stroke.rs` | 121 | a resolved width expanded into a fill, and the reach that adds |
| `encode/tests.rs` | 335 | which lane a command takes, asked through the entry point |

One function was split, as its own commit and for the same reason `device.rs`'s three
were: **the `Command::Group` arm** was forty-five lines inside the dispatch, all of them
ISO 32000-2 §11.4.5, while the dispatch's other four arms are one line each naming one
method. It became `Encoder::encode_group` in `layer.rs`, beside the implicit
one-element group and the soft mask's group — the two other places that build a
`ChildOp` from the same clause. Its text is unchanged apart from indentation.

### Visibility: nothing widened past the `encode` subtree

A child module sees its parent's private items, so **`Encoder`'s forty-two fields did not
widen at all**, and neither did the four methods that stayed in `encode.rs`
(`command`, `resolve_clip`, `charge`, `charge_tile`) — `clips`, `rare` and `parallel`
already reached those as private items and still do. What had to widen is every method
that *moved* and is called from outside its new home. All of them became `pub(super)`,
which here means the `encode` subtree and nothing else, which is exactly the reach they
had as private items of `encode.rs`:

- `plan.rs`: `LayerPlan::mark`, `plan_mut`, `push_op`, `append_op`;
- `layer.rs`: `ChildOp::implicit_blend_group`, `plan_child`, `use_mask`,
  `plan_group_residue`, `push_child`, `encode_group`;
- `instance.rs`: `instance_reserve`, `push_rect_instance`, `push_quad_instance`;
- `coverage.rs`: all eight — `push_coverage`, `push_coverage_styled`, `coverage_tile`,
  `visible_tile`, `take_gpu_lane`, `push_gpu_tile`, `pack_scratch`, `push_scratch_quad`;
- `device_space.rs`: `compose`, `apply`, `transform_preserves_axes`, `tile_side`,
  `CULL_MARGIN`, `target_rect`, `culled`, `note_culled`;
- `rect.rs`: `clipped_device_rect`, `encode_rect`;
- `fill.rs`: `encode_fill`, `fill_through_blend_group`;
- `stroke.rs`: `encode_stroke`;
- `encoded.rs`: `finish`.

**Five things went the other way**, because the move put them in the same file as their
only callers: `distinct_clip_regions`, `note_batch`, `fill_solid`, `linear_bits` and
`SolidFill` are private to one file where they were private to a 2 421-line one.
`blend_word` narrowed from `pub(crate)` to `pub(super)`: nothing outside the encode
subtree has ever called it. **Nothing became more visible than it was.**

Ten items keep `pub(crate)` through a re-export from `encode` — `Encoded`, `LayerPlan`,
`Op`, `ChildOp`, `MaskPlan`, `Batch`, `BatchKind`, `DrawStyle` and the two instance
strides — so `crate::encode::Batch` and its nine siblings resolve exactly as before and
no path outside the module changed.

### Seven doc links were rewritten with their paths

`target_rect`, `CULL_MARGIN`, `HullMemo::bounds`, `encode`, `Command::Rect`, `Compose`
and `Compose::DestOut`/`Plus` resolved through imports that the moves removed. Each is
now an explicit path — same rendered text, and no import that exists only for rustdoc's
benefit (ADR 0051 §2). `hull.rs`'s doc link to `super::apply` and its test's
`use crate::encode::apply` gained the same segment.

**All of them are on private items, where rustdoc says nothing by default.** They were
found with `cargo doc --document-private-items`, which is the only way to see that class
of breakage, and the same run is the check in §3.

## 3. Behaviour and API: what was checked, and how

- **`tests/archetypes.rs`, the instrument this round was given.** Its seven counter rows
  are exact functions of the scene and the viewport. Recorded before the first commit and
  again after the last, byte-identical:

  | archetype | commands, culled, outlines, atlas keys, clip regions, tiles, layer textures, residue regions, residue tiles |
  |---|---|
  | median page | `[12, 0, 9, 12, 0, 0, 0, 0, 0]` |
  | dense text | `[4320, 0, 818, 2164, 1, 40, 0, 2, 0]` |
  | artwork | `[684, 0, 300, 300, 1, 600, 3, 185, 0]` |
  | image page | `[232, 0, 60, 158, 4, 0, 0, 0, 0]` |
  | clip mountain | `[1200, 0, 200, 800, 1200, 0, 0, 0, 0]` |
  | giant | `[1500, 0, 1500, 1500, 0, 0, 0, 0, 0]` |
  | drawing | `[1200, 0, 1200, 1194, 0, 6, 0, 0, 0]` |

- **`tests/encode_threads.rs`**: equal bytes and equal `Counters` at 1, 2, 3, 7 and 64
  threads, before and after. This is the gate a disturbed walk order shows up in and
  nowhere else, and it was also run at the halfway point rather than only at the end.
- **Tests**: `RUSTFLAGS="-D warnings" cargo test --workspace` — **404 passed, 0 failed,
  2 ignored, 49 suites** before and the same 404/0/2/49 after, on RADV and again under
  `QUORRA_ADAPTER=llvmpipe`. No test added or removed by this round.
- **Clippy**: `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` clean, and
  **no new `#[allow]` anywhere in the split** — every one in the new files came with the
  function it is attached to.
- **Format**: `cargo fmt --all --check` clean.
- **Docs**: `cargo doc --document-private-items` gives **37 warnings for `quorra-gpu`
  before and after — the same 37, name for name.** `RUSTDOCFLAGS="-D warnings" cargo doc
  --workspace --no-deps` fails on both, on three links in
  `quorra-function-conformance` and seven "public documentation links to private item"
  in `mask.rs`, `pipeline.rs` and `retained.rs`; none is in `encode`, and none moved.
- **No line of code was lost or invented.** Every non-comment, non-blank line of the
  base `encode` subtree was compared as a multiset against the split one: the 47 lines
  that appear only in the base are 35 signatures whose visibility changed, ten import
  lines, `mod tests {` and the group arm's `=> {`; the lines that appear only after are
  those same signatures, the new imports, the `mod` declarations and the eight
  `impl Encoder<'_> {` wrappers. Not one statement differs.

### Why this could not be a performance change

`encode` is 1.126 ms of a 1.816 ms presenting frame and this machine cannot measure a
millisecond (HANDOVER's "wall clocks lie under load"), so the question was answered by
construction rather than by a stopwatch: **the release profile is `lto = "fat"` with
`codegen-units = 1`**, so module boundaries are not codegen-unit boundaries and no
inlining decision can turn on which file a function is written in. Nothing was moved
between crates, nothing changed a signature's types, and nothing changed an order.

## 4. What was judged irreducible, and left

- **`coverage.rs` at 358 lines.** One subject with two branches, and the branches must
  not be separated: `take_gpu_lane` is the choice, and a lane chosen on one reading of
  the cache and entered on another is a tile rasterised twice or not at all. Its 69 lines
  are 53 of measurement table, which is exactly what CLAUDE.md asks an optimisation to
  carry, and is most of why the file is that long.
- **`layer.rs` at 380 lines.** §11.4.5 read three ways — a `Command::Group`, §11.3.5's
  implicit one-element group, and a soft mask's group — plus the composite's own refusal.
  The fields of a `ChildOp` are where the three would drift apart, and `push_child`'s
  four-bullet argument for why dropping a child draws the same frame is only checkable
  next to the thing it drops.
- **`fill.rs` at 340 lines.** One decision, taken in one order. Splitting `fill_solid`
  out would put the three lanes in a different file from the two conditions
  (`encode_fill`'s rect-hint test) that decide a fill never reaches them.
- **`encode.rs`'s 86-line module comment**, a fifth of what is left of the file. It is
  the map, and ADR 0051 §1's cost is that it is the *only* place the structure survives
  into the documentation.
- **`parallel.rs` at 532 lines** is now the only file in the subtree past the smell. It
  was not opened: ADR 0054 landed in it two commits before this round, its own
  `parallel/` directory shows the seam it has already taken, and a sibling agent was
  reading its neighbours.
- **`Encoder`'s forty-two fields.** They are one frame's working state and every one is read
  by a different part of the subtree; grouping them into sub-structs would be a change to
  every line that touches them, which is not a move.

## 5. Findings noticed and deliberately not acted on

1. **`push_op`'s doc comment is two openings for one function** — exactly the shape
   `doc/HANDOVER.md` records from `take_pass_query` in the `device.rs` round. Its first
   paragraph ("Append an op to the current plan, and grow the plan's bounds to hold it…
   A `Draw` is the exception…") describes `append_op`, which sits directly below it with
   no doc comment at all; the second ("[`Encoder::append_op`] with the queue drained
   first…") is `push_op`'s own, and the two run together with no blank line between them.
   Moving the first three sentences down one function is a two-line edit for whoever next
   owns `plan.rs`.
2. **`CULL_MARGIN`'s doc cites `Encoder::push_glyph`, which no longer exists.** ADR 0054
   replaced that method with `parallel`'s `Job::glyph`. It is one of the 37 private-doc
   warnings and has been for at least a round; the sentence is still true, only the name
   is gone.
3. **`culled`'s doc has a link definition in the middle of its prose.** The line
   `/// [`Counters::commands_culled`]: crate::frame::Counters::commands_culled` sits
   between two paragraphs, so "**What it costs when it wins nothing**" reads in the
   source as a continuation of a reference definition. Rustdoc renders it correctly; a
   reader of the file does not.
4. **`fill_solid` looks the outline up a second time.** `encode_fill` has already done
   `self.resources.outline(outline)` and taken the `UnknownOutline` refusal when
   `fill_solid` does exactly the same lookup with the same id, because `SolidFill` carries
   the `OutlineId` rather than the borrow. `ResourceStore::outlines` is a `HashMap`, so
   that is a second hash lookup per solid fill on the hottest walk in the tree — 4 320 of
   them on the dense-text archetype. It is not a defect and it is not free; carrying the
   borrow needs a lifetime on `SolidFill` that fights `&mut self`, which is a design
   question and not a move.
5. **`command`'s `#[allow(clippy::only_used_in_recursion)]` no longer fires.** Removing
   it after the group arm moved out leaves clippy clean at `-D warnings`. Verified and
   then reverted, because an allow is not a comment made wrong by the move — but it is a
   one-line deletion, and its comment ("with only M7's images left to refuse…") is about
   a state of the walk that has moved on.
6. **`visible_tile` and `coverage_tile` share ten lines of identical arithmetic**, and
   `visible_tile`'s comment says so. Before this round they were 60 lines apart in a
   2 421-line file; they are now adjacent, which is the first time the duplication is
   visible in one screen. Factoring it is behaviour-preserving but it is not a move.
7. **`encode/rare.rs`'s module comment says "What the ops mean to the device that draws
   them is `device.rs`'s half".** Since the `device.rs` split that half is
   `device/rare.rs`, and `device/rare.rs`'s own comment says the same thing about
   `device.rs`. Two stale sentences, one on each side of the same seam.
8. **`Encoded::retained_bytes`'s `[`RetainedScene`]` link has never resolved** —
   `encode.rs` never imported the type. It moved to `encoded.rs` unchanged and is still
   one of the 37.
9. **The dispatch's five arms are now five modules and one line each**, which was not
   the goal — the seams were taken by responsibility, and the shape fell out. Worth
   noticing because it is the shape the *next* command lane should be added in.
