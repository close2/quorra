# Is a soft mask a knockout element's shape or its opacity?

2026-08-18. The round `doc/PLAN.md` had open as "unresolved, and the tree and ADR 0025
disagree about it", flagged twice by reading and never settled: `doc/notes-hayro-questions.md`
§2 for all five lanes, `doc/notes-function-wiring.md` §4.5 for one. The decision is
**ADR 0066**; this note is how it was reached, what else the reading turned up, and what a
next round should do with the two things it deliberately did not fix.

Base: `a4f10f5`, confirmed against `main` before anything was written.

---

## 1. The question, and why the clause settles it without anybody being asked

§11.6.4.3 states the ambiguity and hands it off:

> The mask may serve as a source of either shape ( fm ) or opacity ( qm ) values, depending
> on the setting of the alpha source parameter in the graphics state

The previous rounds stopped there, because "nothing in this tree carries `AIS`" reads like
a question for the caller. It is not, for two reasons that are both in the specification.

**Table 57 gives the parameter an initial value**, so "carries no flag" is not "undefined",
it is `false`:

> alpha source … A flag specifying whether the current soft mask and alpha constant
> parameters shall be interpreted as shape values ( true ) or opacity values ( false ).
> This flag also governs the interpretation of the SMask entry, if any, in an image
> dictionary (see 8.9.5, "Image dictionaries"). Initial value: false .

**And it governs two parameters, not one.** §11.6.4.4 says the same from the other end:

> As described previously for the soft mask, the AIS ('alpha is shape') entry in a graphics
> state parameter dictionary shall determine whether the alpha constants are interpreted as
> shape values ( true ) or opacity values ( false ).

That second fact is what makes the tree's state decidable as *wrong* rather than as one of
two defensible readings. `fs_shape` multiplied in the soft mask and left out the paint's
constant alpha. Under `true` both would be shape; under `false` neither. **No value of the
flag produces one of each**, so the tree was not implementing `AIS = true` — it was
implementing a state the clause does not describe, and ADR 0025's prose described the other
one.

Sources read: `ISO_32000-2_sponsored_EC3.md` in the caller's tree — §11.3.7.2, §11.4.6,
§11.6.4.1–4, §11.5.1, §8.5.4, Table 57, Table 58's `AIS` row, Table 89's image `SMask` row.

## 2. Can a scene reach it? Yes, by four routes, and `SceneBuilder` refuses none

Enumerated from the builder and the encoder rather than from what the viewer emits, because
`quorra-scene` is a public API and the viewer is one caller.

| route | where the style comes from | mask parameter |
|---|---|---|
| any command inside `GroupSpec { knockout: true }` | `encode_group` sets `DrawStyle::Knockout` | `rect`, `fill`, `stroke`, `image` all take one |
| `Command::Fill { compose: Compose::Src }` under `BlendMode::Normal`, anywhere | `encode_fill`'s match, for that mark alone | `fill`'s `mask` |
| `Command::Fill { compose: Compose::DestOut }` | ADR 0025's staged erase, which *is* `fs_shape` | `fill`'s `mask` |
| a non-`Normal` blend **inside** a knockout group | `encode_fill` deliberately skips §11.3.5's implicit group there | `fill`'s `mask` |

The last row is not redundant with the first, and the difference is the reason it is listed:
*outside* a knockout group a blended mark is re-encoded inside the implicit one-element
group **without its mask** (`fill_through_blend_group` passes `None`, and
`ChildOp::implicit_blend_group` carries the mask to the composite instead), so it reaches
`fs_shape` unmasked. Inside one, the wrapper is skipped and the mask stays on the mark.

`scene/validate.rs` has no check that pairs a mask with a knockout position;
`check_staged_compose` looks only at the blend mode, and `check_isolation` only at
isolation. So the answer to the round's first question is **yes**, and the second question —
what the clause requires — is the one that had to be answered.

A nested `Command::Group` is *not* one of the routes: a group is composited by
`composite.wgsl`, which ADR 0033 governs. That turned out to matter for a different reason;
§8 below.

## 3. What the clause requires, and what the pixels were

For an element of an isolated knockout group, §11.4.6 composites with the transparent
initial backdrop and takes a weighted average with the immediate backdrop, weighted by the
source shape — the line `common::clause` has written once, `P' = (1 − f) × P + S`. With
Table 57's initial flag, `f` is §11.6.4.2's geometry met with §8.5.4's clip, and the mask
lives entirely inside `S`.

Measured on a wedge with a diagonal edge under a three-band alpha mask (0.4 / 0.55 / 0.7),
over an opaque cover, worst premultiplied deviation over 64 × 64:

| | llvmpipe | RADV |
|---|---|---|
| fill lane, mask as opacity (the clause) | **0.69** | 0.69 |
| fill lane, mask as shape (what was drawn) | 138.00 | 137.40 |
| image lane, mask as opacity | **0.69** | 0.57 |
| image lane, mask as shape | 138.00 | 137.40 |

The pre-round shaders, restored exactly, fail the new gate at **138.2** (fill) and **137.8**
(image). This was never a rounding argument.

## 4. What changed

- **Five shaders.** `rect.wgsl` and `coverage.wgsl` grew a `shape_at` with `coverage_at`
  becoming `shape_at × soft_mask_at`; `image.wgsl`'s `shape_at` lost its mask factor and
  `fs_main` gained it; `shading.wgsl` and `function_lane.wgsl` renamed `base_weight` to
  `shape_at` and did the same. The rename is not cosmetic — the function no longer computes
  "the geometric part of the weight", it computes `f`, and five lanes computing one quantity
  should call it one thing.
- **`tests/no_ink.rs`.** A fully transparent soft mask moved from the file's *no shape* half
  to its *no opacity* half: shape 1 over the mark's geometry, opacity 0, so inside a
  knockout group it erases exactly as a zero paint alpha does. §2 of
  `notes-hayro-questions.md` said "under either reading a fully transparent mask is a
  no-op", which is true at the root, in an isolated group and in a non-isolated group, and
  false in the one place this round is about — and that same note's own paragraph on
  opacity-zero marks says why.
- **Two clause citations**, in code and tests; §7.

Nothing outside a knockout group moves. `fs_main` computes the same product it always did.

## 5. The gates, and what each is for

**`src/shaders/shape_inputs.rs`** — a text gate, and the only thing here that speaks about
all five lanes at once. It walks each shader's module-scope call graph and requires that no
path from `fs_shape` reaches `soft_mask_at`, that `fs_main` still does, and that exactly
five shaders define an `fs_shape`. It shares its extractor with the `copies` gate through a
new `src/shaders/wgsl.rs`, because a second brace matcher is the drift `copies` exists to
refuse.

Why a text gate at all: the property is one statement about five lanes and it is an
*absence*, which a frame can only witness one fixture at a time. Two of the five lanes want
an expensive fixture (a compiled function program, a mesh raster) to be reached on a device
at all. The gate is exact where five fixtures would be five separate arguments.

**`tests/mask_shape_or_opacity.rs`** — five tests, on llvmpipe and on RADV:

| test | what it holds |
|---|---|
| `a_masked_fill_in_a_knockout_group_is_erased_by_its_geometry_alone` | the clause's line, coverage lane, measured against **both** readings |
| `a_masked_image_in_a_knockout_group_is_erased_by_its_rectangle` | the same, image lane, where the element has three sources of opacity and one of shape |
| `the_staged_erase_does_not_read_the_marks_mask` | `Compose::DestOut` with and without the mask is the **same frame, byte for byte**; the same two marks under `SrcOver` must differ |
| `the_shape_reading_is_expressible_with_the_group_stages` | `/AIS true` is a construction, not a refusal — §6 |
| `the_mask_this_file_draws_is_not_trivial` | the fixture's mask takes ≥ 3 distinct values, none 0 or 1, read from the device |

The last one is the round's own answer to two traps at once. "A gate whose assertion is an
absence needs a control" is answered by measuring the *other* reading in every test rather
than only bounding this one — a mask of 1 would satisfy "the tree takes the opacity
reading" while proving nothing. "Instrument the count of distinct keys, not the hit rate" is
why the fixture test counts distinct mask values rather than masked pixels: a mask that is
one number everywhere is still a mask, and any count of pixels would call it non-trivial.

**Verified able to fail**, each in the direction it claims:

1. The mask put back into `shading.wgsl`'s `shape_at` fails `a_shape_pass_cannot_reach_the_soft_mask`
   with the path `fs_shape -> shape_at -> soft_mask_at`, and leaves the control passing.
2. The mask removed from `image.wgsl`'s `fs_main` fails the control alone.
3. The pre-round shaders restored **exactly** — mask inside the shape and applied once, not
   twice — fail three of the five device tests at 138.2, 137.8 and a byte inequality, and
   leave `the_mask_this_file_draws_is_not_trivial` passing, which is right: that test is
   about the fixture and not about the shader.
4. The same pre-round shaders fail `no_ink.rs`'s
   `a_mark_with_no_opacity_knocks_out_inside_a_knockout_group` on `TransparentSoftMask` and
   nothing else in that file.

## 6. `/AIS true` is expressible today, which is why nothing is refused

The round's caution was that concluding "we need the flag" would be an API change needing
the caller. It does not need one. ADR 0033 already gives a group's erase weight as
`s.a × alpha × mask × clip`, so a caller who has `/AIS true` writes the element as two
groups carrying the mask, whose bodies draw the object *unmasked*:

- `GroupSpec { compose: DestOut, mask: Some(m) }` over the object drawn opaque — the erase
  weighs `coverage × mask`, which is `fm` under the flag;
- `GroupSpec { compose: Plus, mask: Some(m) }` over the object at its own opacity.

Measured: **1.15 of 255** against §11.4.6's line with `f = coverage × mask`, and 138.20 away
from the default reading's line. So the two readings are both available, each by name, and
the difference between them is a thing a caller states rather than a thing they hope for.
This is the answer to "what does a caller with the flag do", and it belongs in the adoption
round's material.

## 7. §11.4.7.2 does not exist

`rect.wgsl`'s `fs_shape` cited **§11.4.7.2** for "object shape ∧ clip ∧ mask shape", and
ADR 0025 quotes that citation approvingly. ISO 32000-2 §11.4.7 is **"Page group"** and has
no subclauses at all. The clause the tree means is **§11.3.7.2, "Source shape and opacity"**,
whose six inputs are object shape, mask shape, constant shape, object opacity, mask opacity
and constant opacity — the distinction every one of those comments was reaching for. The
clip half is **§8.5.4**:

> The effective shape is the intersection of the object's intrinsic shape with the clipping
> path; the source shape value shall be 0.0 outside this intersection.

Corrected in **code and tests**: `rect.wgsl`, `shading.wgsl`, `function_lane.wgsl`,
`image.wgsl`, `tests/function_staged.rs`, `tests/function_knockout.rs`. Four sites also
cited §11.6.4.3 for a *constant* alpha, which is §11.6.4.4 — `src/device/rare.rs`,
`src/encode/rare.rs`, `src/shaders/image.wgsl`, `tests/m7.rs`.

**Left uncorrected, on purpose**: `doc/adr/0010`, `doc/adr/0011`, `doc/notes-function-tests.md`,
`doc/notes-function-gaps.md`, `doc/notes-function-wiring.md`. A note is a record of the round
that wrote it, and ADR 0025's amendment carries the correction where a reader following the
citation will meet it. `notes-function-tests.md` §4.5 asked for "a round that can move all
five" and named §11.4.7.2 as where it should start; this is that round, and the answer is
that it should have started at §11.3.7.2.

## 8. Found while enumerating, not fixed: a nested group in a knockout group is not knocked out

`encode_group` pushes an `Op::Child` without consulting `self.style`, so a plain
`Command::Group` inside a knockout group is composited by §11.3.6 over the accumulated group
content instead of replacing a shape-fraction of it. ADR 0025 and ADR 0033 both assume the
caller writes such an element as the staged pair; nothing refuses them if they do not.

Measured, 16 × 16, an opaque cover then a half-opaque isolated group, both inside a knockout
group:

```
knockout group  : [128, 76, 128, 255]
ordinary group  : [128, 76, 128, 255]   <- byte-identical
clause requires : [26, 102, 229, 128]
```

§11.6.4.2 makes the group's shape the union of its elements' shapes, which is 1 over the
cover here, so §11.4.6's NOTE 5 requires the group composited with the transparent initial
backdrop — the third row. The alpha is 255 where the clause requires 128. **Nothing is
refused and nothing is reported**, which is principle 6's forbidden third state, and it is a
plausible-looking wrong page rather than a hole.

Not fixed here because it is a different clause question with two possible answers — refuse
the construction at the builder (cheap, honest, and it moves the obligation to the caller
who already has ADR 0033's stages), or composite a group by §11.4.6 when its parent knocks
out (a `compose` word the composite shader does not have, and a shape a raster does not
carry, which is the whole reason ADR 0033 exists). It wants the caller in the loop, and it
is the natural successor to this round.

## 8a. The same sentence has one more consequence, for images, and it is not ours yet

Table 57's alpha source entry ends with a clause nobody has had to read here:

> This flag also governs the interpretation of the SMask entry, if any, in an image
> dictionary (see 8.9.5, "Image dictionaries").

So an image's *own* soft mask is opacity under the default too, which is what
`image.wgsl` does with `sample.a` — it is out of `shape_at` and in `fs_main`. But
§11.6.4.2 gives the other case a different answer in the same breath:

> For images (8.9, "Images"), the shape shall be 1.0 inside the image rectangle and 0.0
> outside it. This may be further modified by an explicit or colour key mask (8.9.6.3,
> "Explicit masking" and 8.9.6.4, "Colour key masking").

An explicit or colour-key mask is **shape**, unconditionally — the alpha source flag is
about `/SMask`, not about `/Mask`. `ImageSpec` carries one RGBA buffer and cannot tell the
two apart, so if a stencil-masked or colour-key-masked image ever arrives with its mask
folded into the alpha channel, its shape will be wrong inside a knockout group in the
opposite direction from the defect this round fixed. It cannot happen today —
`doc/notes-hayro-questions.md` §3 records that stencil masking does not reach this library
— and it is worth one line in the adoption round's material so that it is decided when the
route opens rather than discovered.

## 9. The corpus

**Expected to move nothing**, and that is a claim a run is for rather than a reason not to
run one. The caller builds a knockout element's shape themselves — `stated_shape` removes
the mask and the constant, which their own comment calls "the clause under `/AIS false`" —
and refuses a knockout group where an element may have been painted under `/AIS true`
(their ADRs 0234 and 0327; nine corpus documents state the entry). So no page of theirs
reaches `fs_shape` with a mask by the route this round changed. The run is owed with the
next bump, against a baseline taken in the same copy on the same day.

## 10. Recommended edits to files this round may not touch

`doc/PLAN.md`, replacing the open bullet at lines 267–276:

> - **A soft mask is a knockout element's opacity, not its shape — settled, ADR 0066**
>   (2026-08-18). ISO 32000-2 Table 57 gives one flag for two parameters — "whether the
>   current soft mask and alpha constant parameters shall be interpreted as shape values
>   ( true ) or opacity values ( false ) … Initial value: false " — and §11.6.4.4 says the
>   same from the constant's end. `fs_shape` had the mask in and the constant out, which is
>   neither reading; all five lanes now compute §11.6.4.2's geometry met with §8.5.4's clip
>   and nothing else. **Pixels moved**, by 138 of 255 on the round's fixture, inside a
>   knockout group and nowhere else; a corpus run is owed with the next bump and is expected
>   to move nothing, because the caller builds a knockout element's shape themselves and
>   refuses `/AIS true`. `/AIS` was *not* added to `quorra-scene`: the flagged reading is
>   already expressible with ADR 0033's group stages, measured at 1.15 of 255, so nothing is
>   refused and nothing is approximated. `doc/notes-mask-shape-or-opacity.md` is the round.
> - **A nested group inside a knockout group is composited, not knocked out.** Found while
>   enumerating the routes above: `encode_group` pushes an `Op::Child` without consulting
>   `self.style`, so a half-opaque isolated group inside a knockout group draws
>   byte-identically to the same group inside an ordinary one — `[128, 76, 128, 255]` where
>   §11.4.6 at shape 1 requires `[26, 102, 229, 128]`. ADR 0025 and ADR 0033 assume the
>   caller writes such an element as the staged pair, and nothing refuses them if they do
>   not, which is principle 6's third state. Two candidate answers, one of them a refusal;
>   it needs the caller. `doc/notes-mask-shape-or-opacity.md` §8 has the reproduction.

`doc/HANDOVER.md`, a new entry in **Traps**:

> **A flag with an initial value is not an open question.** Two rounds recorded "whether a
> soft mask is shape or opacity" as needing the caller's answer, because §11.6.4.3 defers to
> a graphics-state parameter and nothing in this tree carries one. Table 57 gives that
> parameter an initial value — `false` — so "carries no flag" *is* an answer, and the same
> sentence governs the alpha constant, which the tree already treated as opacity. The tree
> was therefore in a state no value of the flag produces, and the disagreement was decidable
> from the clause alone for two rounds while being filed as a question for somebody else.
> **When a clause hands a decision to a parameter, read the parameter's table before
> recording a silence** — including its default, which is where the answer usually is.

And, in the same section, an amendment to the existing `no_ink.rs` control entry is not
needed: that trap is about the control, and the control is what made this change safe to
make. What is worth adding beside it:

> **A fixture whose subject is a *reading* must measure both readings, not bound one.** The
> mask-as-shape round's first instinct was to assert that the frame matches §11.4.6's line
> with `f = coverage`. That assertion passes on a mask of 1, on a mask of 0, and on an
> element with no partially covered pixel — three fixtures that hold nothing. Every test in
> `tests/mask_shape_or_opacity.rs` measures the frame against the rejected reading as well
> and requires it to *miss*, which is the same shape as `knockout_blend.rs`'s "and the
> ordinary group must not be" and is the only thing that makes the bound a measurement.
