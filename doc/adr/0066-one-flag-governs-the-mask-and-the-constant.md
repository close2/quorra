# ADR 0066 — One flag governs the mask and the constant, and we carry neither

Status: accepted, 2026-08-18. Settles the disagreement `doc/PLAN.md` recorded as open
between ADR 0025's prose and every lane's `fs_shape`, found by reading in
`doc/notes-hayro-questions.md` §2 and flagged for one lane in
`doc/notes-function-wiring.md` §4.5. **It moves pixels.**

## Context

ISO 32000-2 §11.6.4.3's first sentence does not settle what a soft mask is:

> At most one mask input -called a soft mask , or alpha mask -shall be provided to any PDF
> compositing operation. The mask may serve as a source of either shape ( fm ) or opacity
> ( qm ) values, depending on the setting of the alpha source parameter in the graphics
> state (see 8.4, "Graphics state").

Two other places do, and both name **two** parameters where the question had been asked
about one. Table 57, the graphics state:

> alpha source … ( PDF 1.4 ) A flag specifying whether the current soft mask and alpha
> constant parameters shall be interpreted as shape values ( true ) or opacity values
> ( false ). This flag also governs the interpretation of the SMask entry, if any, in an
> image dictionary (see 8.9.5, "Image dictionaries"). Initial value: false .

And §11.6.4.4's last sentence, from the constant's end:

> As described previously for the soft mask, the AIS ('alpha is shape') entry in a graphics
> state parameter dictionary shall determine whether the alpha constants are interpreted as
> shape values ( true ) or opacity values ( false ).

**One flag, two parameters, and its initial value is `false`.** That is what makes this
decidable without asking anybody: a `Scene` carries no alpha source flag and no way to set
one, so both parameters take the initial value and both are opacity.

### What the tree did, and why it was neither reading

`fs_shape` — the knockout erase pass, in all five lanes — returned the mark's coverage
**times its soft mask**, and ignored the paint's alpha. There is no value of the flag that
produces that pair. Under `true` both would be shape and the constant would weight the
erase; under `false` neither would. The tree applied `true` to the mask and `false` to the
constant, which is a third state the clause does not describe, and ADR 0025's own Context
paragraph states the `false` reading as though it were what the tree did.

The difference is invisible wherever only the product `α = f × q` is used, which is
everywhere §11.3.6 composites. §11.4.6 is the one place that reads them apart, and says so:

> The existence of the knockout feature is the main reason for maintaining a separate shape
> value rather than only a single alpha that combines shape and opacity.

### It is reachable, by four routes, none of them refused

The question is about our contract rather than about one caller's usage, so the paths were
enumerated from `SceneBuilder` and the encoder rather than from what the viewer emits. A
soft-masked element reaches `fs_shape` with a mask that is not 1:

1. **Inside `GroupSpec { knockout: true }.`** `encode_group` sets `DrawStyle::Knockout` for
   every command in the body, and `rect`, `fill`, `stroke` and `image` all take a
   `mask: Option<MaskId>`. Nothing in `scene/validate.rs` refuses the combination.
2. **`Compose::Src` on a fill under `BlendMode::Normal`, anywhere.** `check_staged_compose`
   returns early for it, and `encode_fill` maps it to `DrawStyle::Knockout` for that mark
   alone.
3. **`Compose::DestOut` on a fill** — ADR 0025's staged erase, which *is* `fs_shape` on its
   own. Accepted with a mask everywhere except under a non-`Normal` blend.
4. **A blended mark inside a knockout group**, which is deliberately *not* wrapped in
   §11.3.5's implicit one-element group (`encode_fill`'s `self.style == DrawStyle::Over`
   guard) and so draws the erase/add pair directly, **keeping its mask**. Outside a
   knockout group the wrapper takes the mask over and the mark is re-encoded without one,
   so that position never asked the question.

A nested `Command::Group` is not among them: it is composited by `composite.wgsl` rather
than by a lane, and ADR 0033 governs it. See "What this does not touch" below, which is
where that turns out to matter for a different reason.

## Decision

**A soft mask is opacity. `fs_shape` computes §11.6.4.2's object shape met with §8.5.4's
clip, and nothing else.**

§8.5.4 is the authority for the clip half, and it is worth quoting because the shaders had
been citing a clause that does not exist:

> In the context of the transparent imaging model ( PDF 1.4 ), the current clipping path
> constrains an object's shape ( see 11.2, "Overview of transparency"). The effective shape
> is the intersection of the object's intrinsic shape with the clipping path; the source
> shape value shall be 0.0 outside this intersection.

Concretely, in all five lanes:

- `rect.wgsl` and `coverage.wgsl` grow a `shape_at`, and `coverage_at` becomes
  `shape_at × soft_mask_at` — the source alpha, which is what `fs_main` wants.
- `image.wgsl`'s `shape_at` loses its mask factor, and `fs_main` gains it.
- `shading.wgsl` and `function_lane.wgsl` rename `base_weight` to `shape_at`, for the
  reason the rename exists: it is no longer "the geometric part of the weight", it is `f`,
  and five lanes computing one quantity should call it one thing.

**`quorra-scene` does not grow an `/AIS`.** The caller resolves the flag — `pdf-model`
reads it, and `PLAN.md` integration note 6's rule is that a decision either side can make
alone is a decision neither side has made. Adding a scene-level flag would be an API change
that duplicates a graphics-state parameter they have already reduced.

**And the other reading is not refused, it is spelled.** A caller holding content painted
under `/AIS true` can state `coverage × mask` as a shape with ADR 0033's group stages: a
`Compose::DestOut` group's erase weight is its own alpha times its soft mask, so the shape
half is the object drawn opaque and unmasked *inside* a group carrying the mask, and the
deposit half is the object inside a second such group under `Compose::Plus`. Measured at
**1.15 of 255** against §11.4.6's line with `f = coverage × mask`, while missing the
default reading's line by 138.20 — so the construction is the other clause, not a near miss
of this one. That is the whole of §5's requirement here: the library draws what is asked,
refuses what it cannot, and approximates nothing.

## What it costs

**Pixels move, and only inside a knockout group.** A masked element of a knockout group is
now erased by its geometry rather than by geometry × mask. Measured on a wedge with a
diagonal edge under a three-valued alpha mask, over an opaque cover, worst premultiplied
deviation from §11.4.6's line:

| | deviation |
|---|---|
| the mask as opacity (this decision) | **0.69 of 255** (unorm rounding) |
| the mask as shape (what the tree drew) | **138.00 of 255** |

The same fixture on the image lane reads 0.69 against 138.00, and the pre-round shaders
fail the new gate by 138.2 and 137.8. On RADV the four numbers are 0.69 / 137.40 / 0.57 /
137.40 — the same statement, so this is the design and not an adapter.

**A test moved from one half of the clause to the other.** `tests/no_ink.rs` classified a
fully transparent soft mask as *no shape*, and asserted it left the target byte-identical
in all four group kinds. Under Table 57 it is *no opacity*: shape 1 over the mark's own
geometry, opacity 0. So inside a knockout group it now **erases**, exactly as a zero paint
alpha does, and the file asserts that instead. `doc/notes-hayro-questions.md` §2's
"under either reading a fully transparent mask is a no-op" was true at the root and in an
ordinary group and false in the one place this ADR is about.

**A corpus run is owed.** No page of the caller's corpus reaches this today by the route
that matters — they build a knockout element's shape themselves, by removing the mask and
the constant, and refuse a knockout group where an element may have been painted under
`/AIS true` (their ADRs 0234 and 0327). So the expected corpus movement is none, and
"expected none" is exactly the claim a run is for.

**Two citations were wrong in sixteen places.** `fs_shape`'s comment cited **§11.4.7.2**
for the shape/opacity distinction, and ISO 32000-2 has no such subclause: §11.4.7 is "Page
group" and has none. The clause meant is **§11.3.7.2**, "Source shape and opacity", which
lists mask shape, mask opacity, constant shape and constant opacity as four of its six
inputs — the very distinction the citation was reaching for. Four sites also cited
§11.6.4.3 for a *constant* alpha, which is §11.6.4.4. Corrected in code and tests;
`doc/notes-mask-shape-or-opacity.md` §7 lists the sites left in historical notes, which are
records of the rounds that wrote them.

## What this does not touch, deliberately

**`composite.wgsl`'s staged group weight still carries the group's mask and alpha.** ADR
0033 made a `Compose::DestOut` group's erase weight `s.a × alpha × mask × clip × residue`
and said why: a group's shape is the union of its elements' shapes, which no raster
carries, so the caller states it by drawing the shape half and everything they attach to
that half is part of the statement. That is a different quantity from an element's `f`,
where the clause's own parameters are in play, and this ADR makes the element case
*consistent* rather than making the two the same: for an element neither §11.6.4.3's mask
nor §11.6.4.4's constant weights the erase, and for a staged group both do, because there
they are the caller's own words for the shape.

**A plain nested group inside a knockout group is still composited rather than knocked
out, and that is a separate defect.** Measured while enumerating the routes above, at
16 × 16 over an opaque cover: a half-opaque isolated group inside a knockout group draws
`[128, 76, 128, 255]`, **byte-identical to the same group inside an ordinary group**, where
§11.4.6 at shape 1 requires the group composited with the transparent initial backdrop,
`[26, 102, 229, 128]` — the alpha is 255 where the clause requires 128. Nothing refuses it
and nothing reports it, which is §5's forbidden third state. It is not this ADR's subject
and it is not fixed here; it wants its own round, and `doc/notes-mask-shape-or-opacity.md`
§8 carries the reproduction.

## The gates, and that each was verified able to fail

- **`crates/quorra-gpu/src/shaders/shape_inputs.rs`** — a text gate over all five lanes at
  once: no call path from `fs_shape` reaches `soft_mask_at`, and — the control an absence
  needs — `fs_main` still does in every one of them. A count of five shape passes catches a
  lane renamed out of the gate. Forced: the mask put back into `shading.wgsl`'s `shape_at`
  fails the first with the path `fs_shape -> shape_at -> soft_mask_at`; the mask removed
  from `image.wgsl`'s `fs_main` fails the second and leaves the first passing.
- **`crates/quorra-gpu/tests/mask_shape_or_opacity.rs`** — five tests. The clause's line on
  the fill and image lanes, each measured against **both** readings so the fixture is shown
  to tell them apart; the staged erase held to byte equality with and without the mask, with
  an ordinary composite of the same two marks as its control; the group-stage construction
  above; and the fixture's own mask held to at least three distinct values, none of them 0
  or 1, read from the device. Forced: the pre-round shaders restored exactly fail three of
  the five at 138.2, 137.8 and a byte inequality, and leave the fixture test passing —
  which is the right split, since that test is about the fixture and not the shader.
- **`crates/quorra-gpu/tests/no_ink.rs`** — now a gate for this too: the pre-round shaders
  fail `a_mark_with_no_opacity_knocks_out_inside_a_knockout_group` on `TransparentSoftMask`
  alone.

Both device files pass on llvmpipe and on RADV.

## Revisit when

A caller needs the flag *per mark* rather than per page region — that is, when a document
turns up that sets `/AIS true` for one element of a knockout group and `false` for the next,
often enough that expanding it into ADR 0033's group stages costs more than a scene-level
parameter would. Then this is reopened with the caller in the loop, because a flag in
`quorra-scene` is a decision neither side can take alone.
