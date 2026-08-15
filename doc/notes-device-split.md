# Splitting `device.rs`: the responsibilities it held, and where each one went

Written 2026-08-15, against CLAUDE.md principle 1's file-scale rule and
`doc/adr/0051`, which had already decided *where* the parts of a split public module go
and is followed here without amendment. `device.rs` was **2 283 lines** — four and a
half times the ~500-line smell — and one of the two files `doc/HANDOVER.md` names as
left in that state.

No ADR accompanies this round. ADR 0051 decided the only question a split of a public
module raises (private submodules, re-exported from the parent, no new public path),
and nothing here needed a decision that is not simply the application of it. The one
judgement worth recording — what was left alone, and why — is §4 below.

## 1. What the file held, written down before anything moved

Reading it whole, in order, it held eleven things. The count is the point: this is not
one responsibility at 2 283 lines, and it is not two.

| # | Responsibility | Lines |
|---|---|---|
| 1 | The type itself: handles, `Limits`, the accessors that hand them out | 137 |
| 2 | Construction: adapter selection, device creation, the warm-up, `Drop` | 360 |
| 3 | The upload/release surface, and the paint textures it implies | 185 |
| 4 | Ramp sampling — `sample_ramp`, `ramp_color_at`, `RAMP_RESOLUTION` | 64 |
| 5 | A frame's orchestration: `render`, `render_retained`, `draw_encoded`, … | 403 |
| 6 | Phase 2: the buffers, sheets and atlas tiles a frame stages | 204 |
| 7 | Phase 3: `run_frame`, the routes, the submission | 106 |
| 8 | The damage plan (ADR 0012) | 77 |
| 9 | The target a frame is bound to, and its contract | 128 |
| 10 | The bind groups and uniform bytes of the compositor's passes | 237 |
| 11 | The same for the image and shading quads, plus the textures a device makes | 282 |

Two of those (4 and 11's texture half) were **not device work at all**: ramp sampling is
colour arithmetic on the CPU that touches no `wgpu` handle, and it is what
`doc/HANDOVER.md` has carried as a named debt for several rounds. It is the first
commit of this round.

## 2. The seams, one commit each

Fourteen commits, then one for the module map. Every one is a move of text, verbatim,
plus the imports the new file needs and a module comment saying what its one thing is.

| Module | Lines | Its one thing |
|---|---|---|
| `device.rs` | 235 | the device itself: its handles, its `Limits`, its accessors |
| `device/construct.rs` | 429 | what a device costs to exist and when it is ready |
| `device/resident.rs` | 215 | a resource resident until released, in both its forms |
| `device/ramp.rs` | 79 | a colour ramp sampled to texels (ADR 0011) |
| `device/render.rs` | 467 | one frame, from the call to the `Frame` |
| `device/damage.rs` | 101 | ADR 0012's reading of a damage list |
| `device/bound.rs` | 156 | what a frame draws into, and each target's contract |
| `device/staging.rs` | 230 | phase 2: what a frame stages before anything is recorded |
| `device/record.rs` | 201 | phase 3: the route the content takes, recorded and submitted |
| `device/binds.rs` | 262 | the compositor passes' bind groups and uniform bytes |
| `device/rare.rs` | 199 | the same for the image and shading quads |
| `device/textures.rs` | 133 | the textures a device makes, and the usages each asks for |

Three functions were then split into named phases, each as its own commit, because the
file they were in was the file being split:

- `draw_encoded` (133 lines) → `price_internal_textures` and `counters`, ~95 left. Both
  extracted parts are things that are *not* ordering, which is what `draw_encoded` is
  for.
- `run_frame` (102) → `Route`, `Route::of`, `record_content`, 55 left. The four routes
  were an `Option` of a bounding box, a second `matches!` on the same damage plan, and
  a bare `else`; the case that draws nothing *on purpose* looked exactly like the case
  nobody had thought about. It is now a variant with a name and a doc comment.
- `build` (115) → `request_device` returning a `Requested`, ~90 left.

### Visibility: nothing widened past the `device` subtree

A child module sees its parent's private items, so **`Device`'s fields did not have to
widen at all** — every `impl Device` block in `device/*.rs` reaches `self.gpu` exactly
as the one block used to. What did have to widen is what one *sibling* reaches in
another: `Bound`, `DamagePlan`, `Upload` and `FramePhases`, and the methods called
across a seam (`bind_target`, `abandon_frame`, `run_frame`, `upload`, `plan_damage`,
`ensure_paint_textures`, `ensure_dummy`, `rgba_texture`, `quad_uniform`, `sample_ramp`).
All of those became `pub(super)`, which here means the `device` subtree and nothing
else — ADR 0051 §3's cost, paid at the same bound.

`Upload`'s five fields are the one place where a struct's *fields* widened, because
phase 3 reads what phase 2 staged. `RAMP_RESOLUTION` went the other way: it was written
`pub(crate)` and nothing outside `device.rs` ever used it, so it is `pub(super)` now.
Nothing became more visible than it was.

### Four doc links were rewritten with their paths

Links to `Frame`, `RetainedScene`, `Target::Texture` and `Limits::max_resource_bytes`
resolved through imports that the moves removed. Each is now written as a link with an
explicit `crate::` path — same rendered text, and no import that exists only for
rustdoc's benefit (ADR 0051 §2). **Three of the four are on private items, where rustdoc says
nothing**: they were found with `cargo doc --document-private-items`, which is the only
way to see that class of breakage, and the same run confirms no other link in the
`device` subtree is unresolved.

## 3. Behaviour and API: what was checked, and how

- **Tests**: `cargo test --release --no-fail-fast` — **385 passed, 0 failed, 2 ignored,
  45 suites** before, and the same 385/0/2/45 after. Test count floor met exactly, with
  no test added or removed by this round.
- **Clippy**: `cargo clippy --all-targets -- -D warnings` clean before and after, with
  no new `#[allow]` anywhere in the split.
- **Format**: `cargo fmt --check` clean.
- **Docs**: `cargo doc --no-deps` gives **7 warnings for `quorra-gpu` before and after**
  — the same seven, all "public documentation links to private item" in `mask.rs`,
  `pipeline.rs` and `retained.rs`. None is in `device`.
- **Public API**: compared item by item from rustdoc JSON (`cargo +nightly rustdoc
  --output-format json`) on the base commit and on the split, with ids and spans
  stripped so an item that moved files hashes the same. **1 572 items before, 1 576
  after; every name, kind, signature and doc string identical** except the two doc
  comments whose links gained a path (`Device::wgpu`, `Device::resource_bytes_in_use`)
  and `device`'s own module comment. The four extra entries are `impl` blocks, which is
  the one visible consequence below.

### The one visible consequence: `Device` documents in five impl blocks

rustdoc renders each `impl` block separately, so the published page for `Device` now
shows its 26 public methods under five `impl Device` headings (5 accessors here, 11 in
`construct`, 7 in `resident`, 2 in `render`, 1 in `bound`) instead of one. The method
list, the sidebar and every signature and doc comment are unchanged — checked by name
against the JSON, all 26 in both. This is the file-scale analogue of ADR 0051's cost 1:
the responsibility seam is legible in the source tree and in `device.rs`'s module
comment, and the docs show the seam only as five headings with no names on them.

## 4. What was judged irreducible, and left

- **`render.rs` at 467 lines.** It is one thing — a frame from the call to the `Frame` —
  and its module comment says so. `render` and `render_retained` cannot be separated
  from `draw_encoded`, which exists precisely because they share everything but phase 1
  (ADR 0048); and the refusal *order* they establish is the subject, so scattering the
  steps would hide the one property the file is written to make checkable.
- **`construct.rs` at 429 lines.** Four entry points that differ only in who made the
  instance, one `build`, the warm-up questions and `Drop` — one lifetime, from the
  adapter to the thread that has to be joined before the driver goes away (ADR 0018).
  Splitting the warm-up out would put `Drop`'s reason in a different file from the
  thread it joins.
- **`composite_bind` (80 lines), `image_bind` (77), `shaded_bind` (86).** Straight-line
  byte writing at literal offsets that mirror a WGSL `Params` struct. There is no seam
  inside one: every line is an offset paired with the field it writes, and splitting it
  would separate the two halves of the only thing that makes it reviewable. They keep
  their existing `#[allow]`s and their existing comments naming the array size.
- **`StartupSteps` stays in `device.rs`**, beside the field it types, although only
  `construct.rs` reads it. Moving it would require widening its four fields to
  `pub(super)` for no gain: as it stands they are private to `device.rs` and visible to
  the child that writes and reads them.
- **`upload_scratch` at 72 lines** is one texture with two producers and a comment that
  says why (ADR 0016); under the bar and left alone.

## 5. Findings noticed and deliberately not acted on

1. **`take_pass_query`'s doc comment has a stale opening line** — "The query set and
   buffers for one frame's timestamps, when the adapter has them." — left from when
   `PassQuery::new` lived beside it, so the item now reads as two openings for one
   function. Preserved verbatim by the rule that a move changes no text; it is a
   one-line deletion for whoever next owns the instrument.
2. **`doc/HANDOVER.md`'s "Small debts" is now stale in two ways**: it says `device.rs`
   hosts ramp sampling (it no longer does) and that it is 2 215 lines (it is 235, and it
   was 2 283 when this round opened). Not edited, because this round was told not to
   write `HANDOVER.md`; it is the first thing to fix in the round that may.
3. **`Device::pipelines()` and `Device::pipeline_store()` are two accessors for one
   field**, and the second carries `#[allow(dead_code)]` because only `winding.rs`'s
   tests reach it. Merging them is a two-line change that touches `winding.rs` and
   `compose.rs`, which siblings hold this round.
4. **`Device::gpu()`, `Device::queue()` and the public `Device::wgpu()`** are three
   accessors over the same two handles, one of them public API. Not touched: the public
   one is a contract, and the private pair is what the crate actually uses.
5. **Nothing checks a uniform's byte layout against the WGSL struct it mirrors.** wgpu
   refuses a buffer of the wrong *size*; a field at the wrong offset produces a picture.
   A test comparing each `Params` struct's declared size against the array length in
   `binds.rs` and `rare.rs` would be cheap, and `tests/` belongs to a sibling this
   round.
6. **`sample_ramp` and `ramp_color_at` have no unit test of their own.** ISO 32000-2
   §7.10.4's half-open stitching boundary — at coincident offsets the later stop wins —
   is currently checked only through a rendered frame. Now that they are a module of
   their own with no device in them, they are directly testable.
7. **`device.rs` carried two separate `use quorra_scene::…` statements** (one for
   `Scene`, one braced list) — an artefact of an earlier edit. It resolved itself: both
   moved out with the code that used them.
8. **`encode.rs` is the other file past the smell**, 2 406 lines, and `HANDOVER.md`
   names it beside `device.rs`. A sibling is in that file this round, so nothing here
   went near it.
9. **`tests/retained_frame.rs` at 1 139 lines** is the third item on that debt list and
   is untouched for the same reason.
