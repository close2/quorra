# Four of the caller's hayro rows, answered and gated

Four rows of `doc/notes-hayro-coverage-map.md`'s "What is open", named rather than
numbered because that list is renumbered as rows close: **`/Interpolate` honoured, never
overridden**; **no CMS is reachable**; **banding under one 8-bit level**; and
**`encode_threads` nested, with `Scene: Send + Sync` still asserted**. One section each:
the answer, the clause where one is load-bearing, the gate, and the forced defect that
proved the gate can fail.

Every gate below was run on **llvmpipe** (the suite's pin) and the two pixel files were
re-run on **RADV** by name with the same result, because a claim about UNORM rounding and
a claim about sampler behaviour are both claims about a rasteriser.

---

## `/Interpolate` is honoured, never overridden (their #1310)

**The answer: yes, and the decision is per *command*.** `ImageFilter` arrives on
`Command::Image` — `doc/PLAN.md` integration note 1 — because upstream `is_smoothed` is a
method of the *placement*, not a flag of the image. Nothing in this library re-takes it.

### A citation correction for the owner to carry back

Their §4 quotes Table 87 as *"( Optional ) A flag indicating whether image interpolation
**shall** be performed by a **conforming reader**"*. The sponsored EC3 text of
ISO 32000-2 reads:

> (Optional) A flag indicating whether image interpolation **should** be performed by a
> **PDF processor** (see 8.9.5.3, "Image interpolation"). Default value: false.

("shall … conforming reader" is ISO 32000-**1**'s wording.) The clause number, §8.9.5.1
Table 87, is right.

**Their conclusion stands and gets stronger.** §8.9.5.3 settles it outright:

> Image interpolation is an attempt to produce a smooth transition between adjacent sample
> values when rendering an image whose resolution is significantly lower than that of the
> output device. Setting the value of the Interpolate entry in an image dictionary to true,
> is a way for a PDF to declare to a PDF processor that a specific image might render
> better if interpolation is used for this particular image. However, this is only a hint,
> and a PDF processor may ignore it.

So overriding `/Interpolate` is not merely "a viewer preference and not a correctness fix"
— the clause *hands the choice to the processor by name*. Which puts the whole of the
obligation on the boundary rather than on the flag: **the processor is the caller, and the
one thing that must not happen is the renderer taking the decision behind the viewer's
back.** That is exactly what integration note 1's shape makes impossible, and what the gate
holds.

### The gate — `crates/quorra-gpu/tests/interpolate_filter.rs` (new)

An 8×8 black-and-white checkerboard drawn into a **5×5** target — a minification, which is
#1310's own case, and a fixture whose every 2×2 neighbourhood averages to 127.5, so "every
pixel is one of the two samples" decides whether anything filtered.

| test | claim |
|---|---|
| `the_two_decisions_at_one_placement_draw_different_pixels` | one placement, two filters, two different rasters |
| `a_minified_nearest_placement_invents_no_value_between_its_samples` | `/Interpolate` false: no value that is not a sample, and both samples present so the loop is not vacuous |
| `a_minified_linear_placement_smooths_as_the_document_asked` | `/Interpolate` true: at least one pixel strictly between — the control for the row above |
| `each_command_carries_its_own_filter_decision` | one upload, one scene, two placements, one filter each: the decision is not cached with the image or taken for the frame |

`tests/m7.rs` already held each filter's arithmetic on its own; none of its tests can see
one placement under both.

**Forced defects** (`encode/rare.rs`, `linear:`): forcing `true` — literally #1310's
workaround, applied one layer too low — fails three of the four; forcing `false` fails the
other three. Between them every test failed.

**Carried into the file's header, from their §4:** *a quality complaint about small marks is
very often a scan-conversion defect wearing a filtering costume.* Nothing here is evidence
about scan conversion.

---

## No colour-management engine is reachable (their #205, #235, #355, #390)

**The answer: none, and the enforcement now has two independent halves.** Colour is settled
upstream (integration note 6), so an `ICCBased` profile reaches no parser here because
there is no parser here to reach.

### The hole in `deny.toml`, which is a hole of *shape* rather than of contents

`deny.toml` names `qcms`, `moxcms`, `lcms2` and `lcms2-sys` — the four that matter today,
including the two hayro's fuzzers landed in. What it cannot name is the CMS crate published
next year: **a blocklist is prose with a parser**, and CLAUDE.md's stated reason for that
file is that a non-goal written only in prose arrives as a transitive dependency.

Two concrete findings beside it, both from walking the graph rather than reading the list:

1. **`ab_glyph`, `owned_ttf_parser`, `ttf-parser`, `tiny-skia` and `tiny-skia-path` are in
   `Cargo.lock` today.** They arrive through `winit → sctk-adwaita` (Wayland client-side
   decorations) for `examples/window_smoke.rs`, so they are **dev-only** and reachable from
   no published crate. `deny.toml` bans `swash`, `rustybuzz` and `harfbuzz-sys` but not the
   `ttf-parser` family, and bans `vello` but not `tiny-skia` — which is the caller's own
   oracle. Adding them outright would fail CI; the reviewable fix is cargo-deny's
   `wrappers`, recommended verbatim below.
2. `quorra-scene` has an empty `[dependencies]`, which is ADR 0001 as a fact about the
   graph rather than as a promise. It had no gate.

### The gate — `crates/quorra-gpu/tests/no_colour_management.rs` (new)

Reads the three published manifests and `Cargo.lock`, walks the **shipping** graph
(members' `[dependencies]` only — the lock does not separate a member's dev-dependencies,
which is precisely the distinction the file exists to make), and asserts:

| test | claim |
|---|---|
| `a_published_crate_depends_on_four_names_and_each_has_a_reason` | an **allowlist** over direct dependencies: `quorra-scene`, `quorra-gpu`, `thiserror`, `wgpu`, `pollster`, each with its reason. This is what closes the blocklist's shape |
| `the_scene_crate_has_no_dependencies_at_all` | ADR 0001 / §2.3: building a scene requires no device |
| `the_shipping_graph_reaches_no_non_goal` | no reachable crate name matches a colour-management, font-loading, shaping or second-2D-renderer pattern — a pattern, so the unpublished crate matches too |
| `the_font_and_raster_crates_in_the_lock_are_dev_only` | `ab_glyph`, `tiny-skia`, `owned_ttf_parser` are in the lock **and** not in the shipping graph; also the control that the walk is not returning everything |
| `deny_toml_still_bans_every_engine_this_test_knows_by_name` | the CI policy and this gate cannot drift apart |
| `no_source_file_parses_a_colour_profile` | we did not *write* one either: no `acsp` (ICC.1's signature at offset 36), `IccProfile`, `cmsOpenProfile`, `qcms_`, `moxcms` in any `src/` |

**Forced defects**, each verified: `winit` moved into `quorra-gpu`'s `[dependencies]` fails
three of the six (allowlist, non-goal walk, dev-only); a dependency added to `quorra-scene`
fails the ADR 0001 gate; `qcms` deleted from `deny.toml` fails the drift gate; `acsp`
planted in `quorra-scene/src/paint.rs` fails the source gate.

---

## Banding under one 8-bit level (their #60)

**ADR 0010 settled rgba8 layers deliberately and this does not re-open it.** What was
missing is the number.

### What the specification says, which is less than one expects

Clause 11 computes in real numbers and states no storage precision. It says twice, in
**NOTEs** — non-normative both times — that committing to a raster loses information.
§11.2:

> The order in which objects are specified determines the stacking order but not
> necessarily the order in which the objects are actually painted onto the page. In
> particular, the transparency model does not require a PDF processor to rasterize objects
> immediately or to commit to a raster representation at any time before rendering the
> entire stack onto the page. This is important, since rasterization often causes
> significant loss of information and precision that is best avoided during intermediate
> stages of the transparency computation.

and §11.7.2:

> To minimise the accumulation of round off errors and avoid additional errors arising from
> the use of linear group colour spaces, more precision is needed for intermediate results
> than is typically used to represent either the original source data or the final
> rasterized results.

So this is a place the clause **advises** and does not require; we took the other branch
with a reason, and principle 5's rule for that case is to say so plainly and record the
choice as a choice.

### The arithmetic, and the number

One mark of shape × opacity `a` and colour `s` over destination `d`, `SrcOver` with a
Normal blend, stored to a UNORM8 attachment:

```
v    = d·(1 − a) + a·s
byte = round(255 · v)
```

Black on white (`s = 0`, `d = 1`) gives `byte = 255 − round(255·a)`, so the stored byte
moves **iff `a ≥ 1/510`**. Two consequences:

1. **The floor is half a level, not one.** Between half a level and a whole one the mark is
   rounded *up* to a whole level, so it is drawn heavier than its ink. "Under one 255th" is
   the wrong phrase for what disappears; under one 510th is right.
2. **The quantisation is per composite, not per frame.** Every mark reads and writes an
   8-bit destination, so a mark under the floor leaves the byte where it was and the next
   one starts from the same place. **Two hundred of them are lost one at a time and never
   add up** — measured, not reasoned: fifty levels of ink go in and the page reads 255. A
   transparency group does not rescue them, because its own layer is `Rgba8Unorm` too.

That second point is the whole difference between this design and one that composites in a
wider buffer, and it is what §11.2's NOTE is warning about.

### The gate — `crates/quorra-gpu/tests/eight_bit_floor.rs` (new)

Six tests: a quarter of a level leaves the page at 255; three quarters is drawn as one
whole level (254); the floor from both sides a quarter of a level either way (the widest
margin the claim admits — closer to the tie and the assertion would be about a driver's
rounding mode); two hundred sub-floor marks never accumulate; **the same stack above the
floor darkens the page**, which is the control the absence needs; and the group case.

Every expectation is the arithmetic above, and every one was met to the level on both
adapters.

**Forced defects**, in `encode/instance.rs`'s premultiply: clamping alpha up to `2/255`
("we fixed banding") fails five of six; **truncating** alpha to a whole level instead of
letting the store round — the classic banding bug — fails the two boundary tests; dropping
sub-unit alpha to zero fails the control.

---

## The thread knob nested, and a scene shared across threads (their #1316, #1343)

### Does ADR 0054's fixture genuinely contend? **Yes — checked, not assumed.**

`tests/encode_threads.rs` **fails** when `plan_child`'s inner drain is removed
(`a_busy_page_is_the_same_bytes_at_every_thread_count` and
`a_budget_refusal_names_the_same_numbers_at_every_thread_count`, both). The handover's trap
is closed for that fixture.

**But one drain site of the five is not gated by it, and was not gated anywhere.**
`Encoder::push_op`'s drain can be deleted and all four tests of `encode_threads.rs` go on
passing — because every op `busy_page` pushes follows a `plan_child` that drained already.
Reaching it needs a **rare-lane** command in the middle of a run of queued fills, and
`busy_page` has none: its module comment claimed "an image-free rare paint" sat between
runs of fills, and there is no rare paint in the file at all. The comment is corrected in
place (`crates/quorra-gpu/tests/encode_threads.rs`, header only — no fixture touched) and
the site is now gated by the new file's image.

### What a *nested* use does here, which is the opposite of hayro's

`Options::encode_threads` is clamped once at `Device` construction (against
`std::thread::available_parallelism`) and read from there for the frame. A child plan, a
soft mask and a nested body all encode against the same `Encoder` with the same count —
**nothing re-forces it**, which is precisely #1316's complaint about `hayro::render`.

What nesting *does* change is batch size, not permission: `plan_child` drains at both ends
of a body, so a group boundary caps the run the fan-out divides, and a page of many tiny
nested groups will sit under the 4 096-segment floor at every one of them. That is the
ordering requirement doing its job, and it is not a defect.

### What cannot be observed, and why that is deliberate

There is no public counter for how a frame's geometry was divided, **and there must not
be**: `tests/encode_threads.rs` asserts `Counters` equality across thread counts, so a
counter recording the division would contradict the design it would be measuring. So the
"nothing re-forces it" claim is gated at the source (the shape `mul_add_hazard.rs` already
uses in this tree), and the queue's presence inside the nesting is shown by the forced
defect rather than by a number.

### The gates

**`crates/quorra-gpu/tests/encode_threads_nested.rs` (new)** — four group levels deep, a
curve-clipped mark per level (the residue path, which the fan-out declines, so serial work
sits between the runs it takes), a translucent image over the deepest level's queued marks,
and twelve 402-segment marks at twelve distinct scales in the deepest plan (4 824 segments,
above the fan-out's 4 096 floor; one scale would have been one atlas key and eleven
weightless residents).

| test | claim |
|---|---|
| `the_nested_fixture_is_order_sensitive` | the same marks in the opposite order draw a **different** page — proved, not argued, which is the property a disjoint fixture cannot have |
| `a_nested_page_is_the_same_bytes_at_every_thread_count` | §4.6 at 1, 2, 3, 7, 64 |
| `a_nested_refusal_names_the_same_numbers_at_every_thread_count` | a refusal from *inside* a child plan does not move |
| `the_thread_count_is_never_reassigned_after_the_frame_has_it` | no `self.threads =` anywhere in `quorra-gpu/src` |

**Forced defects**: spreading the marks apart fails the order-sensitivity control;
`push_op`'s drain removed fails the byte equality (**and nothing else in the tree**);
`plan_child`'s inner drain removed fails the byte equality and the refusal; `self.threads =
1;` planted in `plan_child` fails the source gate.

**`crates/quorra-gpu/tests/scene_across_threads.rs` (new)** — the assertion
`assert_send_sync::<Scene>()` still exists in `tests/retained_handle.rs` and still means
what it says, but it is a claim about *types* and #1343 is a bug about *use*. So:

| test | claim |
|---|---|
| `a_scene_is_send_sync_and_one_pointer_wide` | the bounds, plus **cheap to clone** stated as the fact that makes it true: `size_of::<Scene>() == size_of::<usize>()`. A scene that grew an inline field would still be `Send + Sync` and no longer cheap to clone, and nothing else in the tree would say so |
| `one_scene_renders_concurrently_on_several_devices` | four threads share one scene by `&` (which needs `Sync`, where a clone needs only `Send`), each on its own device at four encode threads, and every one draws the reference page |
| `a_cloned_scene_drawn_elsewhere_is_the_same_page` | a clone made on a third thread, drawn on a second device |

**Forced defects**: a `u64` added to `Scene` fails the width gate (16 against 8); a shifted
viewport in one reader fails the concurrency comparison; a blank scene in place of the
clone fails the clone gate.

**What could not be forced, stated rather than skipped**: a genuine data race cannot be
planted in `Scene` without `unsafe` (principle 3) — it holds an `Arc` around immutable data
with no interior mutability and no cache under it, so there is nothing to make
unlinearisable. The concurrency test is therefore an *exercise* whose comparison is
verified live, not a race detector; run it under a thread sanitiser if that is ever wanted.

---

## Recommended `deny.toml` addition, for the owner to take or refuse

The two families in the lock today are dev-only and would fail an outright ban, so the
reviewable form is `wrappers` — the crate is admitted **only** when its named parent pulls
it, and a route from the library itself still fails:

```toml
    # Present in Cargo.lock through `winit → sctk-adwaita`, which draws Wayland
    # client-side decorations for `examples/window_smoke.rs`. Admitted only there:
    # a route to either of these from a published crate is §9's font-loading and
    # second-2D-renderer non-goals arriving in the caller's process.
    { crate = "ttf-parser", wrappers = ["owned_ttf_parser"], reason = "no font loading (§9); dev-only via winit's decorations" },
    { crate = "owned_ttf_parser", wrappers = ["ab_glyph"], reason = "as ttf-parser" },
    { crate = "ab_glyph", wrappers = ["sctk-adwaita"], reason = "as ttf-parser" },
    { crate = "tiny-skia", wrappers = ["sctk-adwaita"], reason = "a second 2D renderer, and the caller's own oracle (principle 5)" },
    { crate = "tiny-skia-path", wrappers = ["tiny-skia"], reason = "as tiny-skia" },
```

`cargo-deny` is not installed for this user, so **this block has not been run** —
`tests/no_colour_management.rs` holds the same policy from the graph side and does not
depend on it.
