# Round notes — three files split along their seams

2026-08-15. Three source files stood at two to two-and-a-half times CLAUDE.md's
~500-line smell: `quorra-scene/src/scene.rs` (1 216), `quorra-gpu/src/compose.rs`
(1 014) and `quorra-gpu/src/winding.rs` (844). This round split all three. Nothing else
was touched: no public API changed, no behaviour changed, and the four sibling files
under other agents' hands (`encode.rs`, `raster.rs`, `device.rs`, `atlas.rs`,
`retained.rs`) were not opened.

**None of the three was irreducible.** That was the outcome to be open to and it did not
arise: each of them announced its own seams, twice in the module comment's own headings.

## `scene.rs` — four responsibilities, and the file said so itself

The old module comment had a heading, *"Validation is here, and it is loud"*, sitting in
the middle of a comment whose other headings were about what a scene *is*. That is a
file telling you where to cut.

| module | its one thing | lines |
|---|---|---|
| `scene.rs` | the finished `Scene`, and the list of the parts | 159 |
| `scene/command.rs` | the vocabulary: `Command` and the four definitions it points at | 252 |
| `scene/builder.rs` | one method per command, each the same three steps | 378 |
| `scene/frames.rs` | the open-frame stack | 183 |
| `scene/validate.rs` | §4.7's refusals, in boundary order | 342 |
| `scene/cost.rs` | `Cost` and the walk `finish` runs | 57 |
| `scene/fixtures.rs` | the three values the module's tests are written in terms of | 31 |

Two seams are worth their reasoning:

- **`frames` is separate from `builder`** because it is a state machine, which is one of
  the three things CLAUDE.md names as deserving its own module. It is popped on both
  paths (so an errored group is discarded whole), it *is* the depth bound, and it
  carries the knockout question a command's position answers. Three invariants that
  hold together and nowhere else.
- **`validate` holds every refusal, including three that used to be inlined** in
  `rect`, `stroke` and `group`/`image`. They are now `check_rect`, `check_stroke` and
  `check_alpha`. The order of checks inside each builder method is unchanged, so the
  error a given bad input produces is the same error. `check_alpha` was written twice
  in the old file (§11.4.5's group alpha, and the image lane's constant alpha) and is
  now written once — see finding 2 for what that made visible.

`MAX_COORDINATE` moved with the refusals it bounds, and is re-exported, so
`crate::scene::MAX_COORDINATE` (which `paint.rs` imports) still resolves. ADR 0051
records why the parts are private submodules rather than public ones, and what that
costs.

## `compose.rs` — one walk, five kinds of pass

The seam here is not "state versus passes"; it is that **five different questions each
had exactly one answer in this file**, and a reader after any one of them had to hold
the other four. Where a plan's pixels are; how a run of marks becomes a pass; what
§11.3.6 does to a finished child; how a rectangle of texels is moved unchanged; how
§11.5's masks are realised.

| module | its one thing | lines |
|---|---|---|
| `compose.rs` | the `Executor`'s state, `render_plan`'s walk, the scissor rule, the two timestamps | 329 |
| `compose/region.rs` | the frame's rectangle arithmetic — and no device in it | 133 |
| `compose/draw.rs` | the content pass and the preparation that must precede it | 262 |
| `compose/child.rs` | §11.3.6: a finished child composited onto its parent | 120 |
| `compose/blit.rs` | the three passes that run `blit.wgsl` | 179 |
| `compose/masks.rs` | §11.5's masks realised, and the lookup everything binds by | 106 |

`region` is the cut with the most in it: ADR 0036 gave this frame two coordinate spaces
(device space, and an attachment's own), and every conversion between them was scattered
among the passes that needed one. It is `Copy`, it takes no device, and every answer it
gives is checkable by hand — which is the argument for it being apart from the passes
rather than among them.

`blit` and `child` are separate on purpose although both draw one triangle: a blit moves
pixels and a composite changes them, and the clause citation is on exactly one of the
two.

## `winding.rs` — the lane, and three things the lane uses

| module | its one thing | lines |
|---|---|---|
| `winding.rs` | the sample grid, `SheetUse`, `render_into`'s loop, the end-to-end coverage tests | 384 |
| `winding/sheet.rs` | what the encoder built, and what it will cost before anything is allocated | 151 |
| `winding/buffers.rs` | everything the two passes read, built once per frame | 192 |
| `winding/passes.rs` | the winding target, and the two passes that write and read it | 192 |

The one judgement call: **`WindingTexture` is in `passes.rs` rather than in a file of its
own.** It could stand alone at ~70 lines, and it should not, because the invariant that
makes it correct is not stated by the texture or by either pass alone — *the pane is the
top-left of a texture kept from a larger frame*, and it holds only because
`accumulate`'s viewport puts the pixels where `resolve` reads them. That is the defect
the caller reported as `QUORRA_FEEDBACK.md` §11 (a glyph's coverage under another
glyph's quad at 1000% zoom and back), and it is a defect that a reviewer can only catch
by reading the texture and the two passes together. So they are together.

`sheet.rs` is the opposite argument: it is separate precisely because it has *no*
device in it. Its `device_bytes` must price exactly the condition under which
`upload_scratch` allocates, and five real corpus pages were refused when the two
disagreed. A file with no `wgpu::Device` in it is a file where that can be checked.

## Findings — noticed, deliberately not acted on

1. **`compose.rs`'s module comment is stale, and I moved it verbatim anyway.** It says a
   child is composited "through `composite.wgsl` — a ping-pong between the parent's two
   textures", which ADR 0038 replaced with one texture per plan; `render_plan`'s own
   comment says so twelve lines below ("One texture, not a ping-pong pair (ADR 0038)").
   Correcting a claim is a content change and this round was behaviour- and
   text-preserving, so it is written here instead. It wants one sentence changed by
   whoever next owns that file.
2. **An image's alpha is refused as `SceneError::InvalidGroupAlpha`.** The check is
   identical to §11.4.5's group alpha and is now `check_alpha`, called from both
   `group` and `image` — which is what made the shared variant obvious. A caller that
   passes `alpha: 1.5` to `image()` is told about a *group*. Changing the variant is a
   public API change and was out of scope; it is worth a variant of its own, or a
   rename, next time `SceneError` is opened.
3. **`cargo doc --no-deps` is not warning-free in this tree, and was not before this
   round.** Seven warnings, all "public documentation for X links to private item Y":
   `mask.rs` (2), `pipeline.rs` (4), `retained.rs` (1). None of them is in a file this
   round touched, and the six files I created add none. The gate as stated is currently
   met by my crates and not by the tree.
4. **The perf gate flapped on the *baseline*, before any of my changes.**
   `perf_gate.rs::a_readback_frame_does_not_pay_for_its_pixels_twice` read 34.7 ms
   against a gate derived from 1.32 ms, at load average 80 on a machine that is somebody's
   desktop. It is the known wall-clock flap the handover names. It failed identically
   before and after; see the verification below for the run at a lower load.
5. **`nested_body`'s `unwrap_or` would swallow a lost frame.** If the stack were ever
   popped by something other than the matching `nested_body`, the body's commands would
   silently become an empty `Vec` rather than a refusal. It cannot happen today — push
   and pop are in one function and `finish` `debug_assert!`s the stack is empty — and it
   is the one place in the builder where an impossible state has a quiet answer rather
   than a loud one.
6. **`compose::RunOp` is no longer re-exported.** Nothing in the crate ever named it
   (the device writes `compose::run_ops(...)` and lets the item type follow), and an
   unused `pub(crate) use` is a rustc warning. Internal only — no public path changed —
   and the reason is written at the re-export.
7. **Two textual changes inside moved code, both mechanical.** `composite_child` and
   `realise_masks` reach a `Rendered`'s region through its existing `region()` accessor
   rather than the field, because they now sit beside the type rather than inside it.
   Everything else moved character for character.

## Verification

**Same test counts, before and after.** Baseline was taken on the unmodified tree in
this same worktree and target directory, on the same day, per the handover's rule.

| | before | after |
|---|---|---|
| test binaries | 33 | 33 |
| passed | 257 | 257 |
| failed | 1 (`perf_gate`, finding 4) | 1 (`perf_gate`, finding 4) |
| ignored | 2 | 2 |

The counts agree **per binary**, not only in total: the two runs' `(binary, passed,
failed, ignored)` rows are identical line for line. The one failure is the same test in
both, and it **passes 5/5 when `perf_gate` is run on its own** on the split tree, at
load average 43 — so what fails it is the rest of the suite running beside it, in both
runs, and not this round.

`cargo clippy --release --all-targets -- -D warnings` is clean, `cargo fmt --check`
passes, and `cargo doc --no-deps` adds no warning to the seven of finding 3.

**Byte equality, beyond the suite.** The suite's own gates compare rasters, but they
compare small ones. A page exercising every lane at once — 39 commands: rectangles, 24
glyph-shaped fills at fresh sub-pixel phases, a stroke, an axial shading, two images
(linear and nearest), a rectangular clip, a non-rectangular clip chain, an alpha mask, a
luminosity mask with a transfer, an isolated group holding a knockout group with a
staged `DestOut` element, and a non-isolated group — was rendered to `Target::Readback`
at scale 1 and scale 3.7, on both coverage lanes, before and after the whole split. The
harness lives outside this tree (it is an instrument for one round, not a test); the
FNV-1a hashes of the straight-alpha RGBA it returns:

| | before | after |
|---|---|---|
| CPU lane, 200×200 | `682d1f9d2dc75f71` | `682d1f9d2dc75f71` |
| CPU lane, 740×740 | `8c5f7fb955a6aeb6` | `8c5f7fb955a6aeb6` |
| GPU lane, 200×200 | `682d1f9d2dc75f71` | `682d1f9d2dc75f71` |
| GPU lane, 740×740 | `5b03e5e1dacc3015` | `5b03e5e1dacc3015` |

`Scene::cost` for that page is identical too — 39 commands, 2 clips, 2 masks, depth 2,
5 480 retained bytes — which is the cost walk's own answer, moved to `cost.rs`. Neither
run produced a single `Report`, which is the other half of "unchanged": a lane that had
quietly stopped drawing would have said so.

**Every published path, checked by the compiler.** The same harness carries a module
that does nothing but `use quorra::scene::scene::{ClipDef, Command, Cost, GroupSpec,
ImageFilter, MAX_COORDINATE, MAX_GROUP_DEPTH, MaskDef, Scene, SceneBuilder}` — the whole
of what `scene.rs` published before the split, named through the path a caller would
write. It compiles, which is the claim "no public path moved" made by something other
than my reading of it.
