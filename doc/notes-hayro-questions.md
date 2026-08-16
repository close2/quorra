# Four questions from the caller's hayro reading, answered here

The caller wrote `/home/cl/projects/pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` on
2026-08-16, after reading all 167 issues on `LaurenzV/hayro`. It says of itself that it is
a reading list and **not** a defect list — "Nothing here is a claim that quorra has any of
these problems" — and that the reusable part of any entry is *why* something is right.

This file is that half, for the four entries that are concretely checkable against this
tree. Each question ends in a gate, including the two whose answer is "settled upstream",
because a gate on an assumption we depend on is worth more than a gate on a behaviour we
control: it fails when the assumption stops being true, which is the only way either side
learns.

**No behaviour changed in this round.** Four test files are new and no source file is
edited. Two of the four answers name a divergence that is deliberate and inherited, and
both are now written down on this side of the boundary as well as on theirs.

| question | answer | gate |
|---|---|---|
| 1. a leading degenerate `MoveTo` — do we deposit a cap? | **No**, under any cap style, and §8.5.3.2 says that is right | `crates/quorra-gpu/tests/degenerate_subpaths.rs` |
| 2. is a "no ink" paint a no-op in the compositor? | **Yes where its shape is zero**, in all four group kinds, byte for byte — and correctly **not** where only its opacity is zero and it is inside a knockout group | `crates/quorra-gpu/tests/no_ink.rs` |
| 3. a stencil mask on a grid other than its image's | **Cannot reach us**; resolved in `pdf-model`. The mismatch we *do* have is a soft mask, and it is sampled on the device grid | `crates/quorra-gpu/tests/mask_grid.rs` |
| 4. `mul_add` on a hot CPU path | **Not on one.** Two `src/` sites, both once-a-frame; the target has no FMA | `crates/quorra-gpu/tests/mul_add_hazard.rs` |

**Both adapters.** The three gates that open a device were run on llvmpipe — the suite's
pinned default, and `tests/common/headless.rs` says why it is pinned — and on RADV, by
temporarily letting that fixture read `QUORRA_ADAPTER`. All fifteen pass on both, and the
override was reverted; the fourth file opens no device.

---

## 1. A leading degenerate `MoveTo` (their §5, hayro #296)

### The clause answers it in one sentence, and it is not the sentence the question expects

Their note reasons from §8.4.3.3 — caps are applied "at both ends of open subpaths", and a
subpath of one point is an open subpath — and concludes that a round or square cap "can
deposit a dot at the origin". §8.4.3.3 does say that, but §8.5.3.2's last paragraph is
more specific and it overrides:

> If a subpath is degenerate (consists of a single-point closed path or of two or more
> points at the same coordinates), the S operator shall paint it only if round line caps
> have been specified, producing a filled circle centred at the single point. If butt or
> projecting square line caps have been specified, S shall produce no output, because the
> orientation of the caps would be indeterminate. This rule shall apply only to
> zero-length subpaths of the path being stroked, and not to zero-length dashes in a dash
> pattern of a non-degenerate subpath. In the latter case, the line caps shall always be
> painted, since their orientation is determined by the direction of the underlying path
> except in the case of a degenerate subpath. **A single-point open subpath (specified by
> a trailing m operator) shall produce no output.**

Three distinct shapes, three answers:

| shape | §8.5.3.2 |
|---|---|
| a bare `m` — a single-point **open** subpath | no output, **under every cap** |
| a single-point **closed** path (`m h`) | a filled circle under round caps; nothing under butt or square |
| two or more points at the same coordinates | the same as the closed case |

hayro #296's spurious leading `MoveTo` is the first row. So the clause's answer to the
question is unconditional: **no dot, whatever the cap.** The parenthetical "specified by a
trailing m operator" describes how such a subpath usually arises; the normative subject is
the single-point open subpath, and a spurious leading one is that.

### What this tree does

Nothing, which is right. `raster::flatten` (`crates/quorra-gpu/src/raster.rs`) keeps a
subpath only when it has more than one point, so a lone `MoveTo` is dropped before caps are
considered; `raster::stroke_polylines` declines a polyline that dedupes to one point, which
catches the coincident-points shape. Both lanes share that one expansion — the inline path
and `encode/parallel.rs`'s threaded one — so there is one answer, not two.

### The part that is a divergence, and it is §4.5's

The second and third rows above are a **disc under round caps**, and this tree paints
nothing there either. That is not §8.5.3.2; it is `RENDER_LIBRARY.md` §4.5 in force:

> degenerate subpaths | §8.5.3.2 — a zero-length subpath is a dot under round caps and
> *nothing* under butt or square | we pre-split them; draw what you are given

and the caller's implementation of it is `pdf-render/src/degenerate.rs`, which is worth
reading because it settles the question with measurements rather than by preference. Its
`split_degenerate` separates a stroked path into `stroked` and `dots`, and `dots` is
documented as "the circles §8.5.3.2 asks for, to be **filled** with the stroking paint".
Its module comment carries the measurement that made it necessary — at width 10 on a
100-unit page, `m h` under round caps is 77.5 units of ink in `tiny-skia`, 0.0 in Vello and
78.5 in the clause; under square caps it is 100.0, 0.0 and *nothing*. So the disc reaches
us as a `Command::Fill` of its own geometry, and a `Command::Stroke` carrying a degenerate
subpath is a thing the contract says cannot arrive.

**Neither side may quietly change its mind about that**, which is why the gate states our
half rather than leaving it to be inferred: if the caller stopped splitting, or if this
side grew its own disc, a round-cap dot would be lost or doubled and no test either side
had would see it. Their mirror test is
`render-cpu/tests/degenerate_subpath.rs::a_single_point_closed_path_is_a_disc_under_round_caps`;
ours is
`degenerate_subpaths.rs::a_single_point_closed_path_draws_nothing_here_because_4_5_places_the_disc_upstream`,
and the two are consistent because they are about different sides of one split.

### The gate

`crates/quorra-gpu/tests/degenerate_subpaths.rs`, six tests, all through `SceneBuilder` with
the outline built by hand:

- a spurious `MoveTo` **before** the real subpath is byte-identical to the same stroke
  without it, under butt, round and square;
- the same for a spurious `MoveTo` **after** it;
- a path that is *only* a `MoveTo` draws nothing at all;
- `m h` draws nothing (the §4.5 divergence, stated as one);
- `m` + a `LineTo` back to the same point draws nothing (the same, by a second route: this
  one survives `flatten` and is stopped by `stroke_polylines`);
- and a control that the caps this file asserts are absent are ones the target can see —
  the two round caps of an ordinary line are a disc of 113 pixels and the two square caps
  are 144, both checked against §8.4.3.3's Table 53 arithmetic.

**Verified able to fail.** With `raster::flatten`'s one-point guard relaxed and
`stroke_polylines` made to deposit a pair of caps at a lone point — hayro #296's exact
hazard — five of the six fail and the control passes.

---

## 2. A "no ink" paint (their §6, hayro #4)

### What is ours and what is not

`/None` is a colourant name, and §8.6.6.4 is unusually direct about it:

> The special colourant name None shall not produce any visible output. Painting
> operations in a Separation space with this colourant name shall have no effect on the
> current page.

Colour is not ours (`PLAN.md` integration note 6: `ColourSpace::to_rgb` upstream is the
only place a colour becomes RGB, and a second one is forbidden), so `/None` never reaches
this library as a colourant. What reaches us is the caller's decision not to issue the
command. The property their note calls "the cheapest possible test of whether a 'no ink'
path in a compositor is really a no-op" *is* ours, and it is the question this gate asks.

### The clause splits "nothing" in two, and the two answers differ

§11.6.4.2 keeps an object's **shape** apart from its **opacity**, and §11.4.6 says why in a
sentence ADR 0025 already quotes: "The existence of the knockout feature is the main reason
for maintaining a separate shape value rather than only a single alpha that combines shape
and opacity."

**No shape** — an empty clip, a zero-area outline, a transparent soft mask read as shape.
§11.4.6's NOTE 5 is unconditional:

> The extreme values of the source shape produce the straightforward knockout effect. That
> is, a shape value of 1.0 (inside) yields the colour and opacity that result from
> compositing the object with the initial backdrop. A shape value of 0.0 (outside) leaves
> the previous group results unchanged.

"Unchanged" is byte identity, and it holds in every kind of group.

**No opacity** — a fill whose paint alpha is 0 has shape 1 inside its path (§11.6.4.2: "the
shape shall always be 1.0 inside and 0.0 outside the path") and constant opacity 0
(§11.6.4.4). Outside a knockout group §11.3.6's formula leaves the backdrop. **Inside one it
must not**: §11.4.6 composites the element with the group's *initial* backdrop — transparent,
for an isolated knockout group — and then replaces a shape-fraction of the accumulated
group with the result, so a transparent element with shape 1 clears what it covers. A
compositor that made that a no-op would be wrong.

### What this tree does

Both, correctly.

- Shape-zero: byte-identical in all four contexts (root, isolated group, knockout group,
  non-isolated group), for four independent routes into the target — an empty clip chain
  (culled at encode), a zero-area rectangle (the analytic rect lane), a zero-area outline
  (the coverage lane), a transparent soft mask (sampled in the fragment shader).
- Opacity-zero: byte-identical at the root, in an isolated group and in a non-isolated
  group; and inside a knockout group it clears the group's own content and leaves the page
  showing through, which is §11.4.6.

The mechanism is `fs_shape` in each lane returning coverage and ignoring the paint (ADR
0010's erase/add pair, ADR 0025's reasoning) — so the erase pass scales the backdrop by
`1 − shape`, which is exactly 1 when the shape is 0, and the add pass adds a premultiplied
zero.

### The thing worth handing back to the caller

**`/None` cannot be modelled as a zero-alpha paint.** The two are the same everywhere
except inside a knockout group, where one is required to change nothing and the other is
required to erase. So a painting operation in a `/None` Separation space has to be a
command that is never issued — which is what `pdf-model` already does by deciding the
colourant before the alternate space and tint transform are read, and their §6 records that
getting it wrong once made a page come out red. This note is only to say that the choice is
load-bearing for a reason beyond the tint transform, and that quorra would faithfully erase
if a `/None` mark ever arrived as `Color { a: 0.0 }` inside a knockout group.

### The gate

`crates/quorra-gpu/tests/no_ink.rs`, five tests over a page with three kinds of backdrop
pixel on purpose — opaque, half-transparent premultiplied, and an antialiased diagonal edge
— because byte identity over a flat opaque region is a weak claim.

- `a_mark_with_no_shape_leaves_the_target_byte_identical` — 4 kinds × 4 contexts, all 16
  reported rather than the first failure, because the four kinds are answered at four
  different depths and which one broke is the finding;
- `a_mark_with_no_opacity_leaves_the_target_byte_identical_outside_a_knockout_group` —
  2 kinds × 3 contexts;
- `a_mark_with_no_opacity_knocks_out_inside_a_knockout_group` — the discriminating half:
  the knockout page differs from the plain one at the covered pixel, and what is left there
  equals the page without the group at all;
- `the_marks_this_file_asserts_are_invisible_are_visible_when_they_are_given_ink` — the
  control, and the reason the rest are measurements: each of the six kinds with its one
  nothing-making property restored and nothing else changed must reach the target;
- `a_group_that_marks_nothing_leaves_the_target_byte_identical` — the same question one
  level up, where §11.4.4's Result step could turn a nothing into a rounding.

**Verified able to fail**, twice, and the second attempt is why the control test exists:

1. An ink floor of 0.02 planted in `coverage_at` in both `coverage.wgsl` and `rect.wgsl`
   failed `TransparentSoftMask` in all four contexts — and left `EmptyClip`,
   `ZeroAreaRect` and `ZeroAreaFill` passing, because those three never reach a fragment
   shader at all. That is a true statement about the tree and a warning about the gate,
   which is what the control test now answers permanently.
2. `encode/layer.rs`'s `spec.knockout` forced to `false` failed
   `a_mark_with_no_opacity_knocks_out_inside_a_knockout_group` and nothing else.

### One thing found by reading, not by a failure, and deliberately not changed

`fs_shape` in `rect.wgsl`, `coverage.wgsl` and `image.wgsl` multiplies the mark's soft mask
into the **shape** it returns. ADR 0025's own text says the opposite reading — "§11.6.4.2
gives an object's shape from its geometry alone; §11.6.4.3's soft mask and §11.6.4.4's
constant alpha are *opacity*" — and `doc/notes-function-tests.md` §4.5 already flags the
same thing for the function lane ("`fs_shape` weights by `base_weight`, which includes the
mask"). It is true of all five lanes, not only that one.

§11.6.4.3 is where the answer is, and it does not settle it by itself:

> The mask may serve as a source of either shape ( fm ) or opacity ( qm ) values, depending
> on the setting of the alpha source parameter in the graphics state

with NOTE 1 naming that parameter as the `AIS` entry, "true if the soft mask contains shape
values, false for opacity". Nothing in this tree carries `AIS`, and nothing in the caller's
display list does either. So the current behaviour is §11.6.4.3's `AIS = true` reading
applied unconditionally, and ADR 0025's prose is its `AIS = false` reading applied
unconditionally, and the two disagree only for a masked element inside a knockout group.

**Not resolved here, on purpose.** It is a clause decision that changes pixels, it needs
the caller's answer about whether `AIS` survives their interpreter, and it needs a corpus
run — which makes it an ADR and a round of its own rather than an edit inside a round about
something else. It does not affect this question's answer: under either reading a *fully
transparent* mask is a no-op, because either the shape or the opacity is zero and both
routes leave the backdrop.

---

## 3. A stencil mask whose grid is not the image's (their §4, hayro #1315 / #1319 / #2)

### A correction to the citation, and it does not change their point

Their note cites §8.9.6.4 for the permission. §8.9.6.4 is **colour key masking** — a range
of colours to be masked out — and says nothing about resolution. The permission is
**§8.9.6.3 Explicit masking**:

> The base image and the image mask need not have the same resolution ( Width and Height
> values), but since all images shall be defined on the unit square in user space, their
> boundaries on the page will coincide; that is, they will overlay each other.

which leans on §8.9.5.1:

> The correspondence between image space and user space is constant: the unit square of
> user space, bounded by user coordinates (0, 0) and (1, 1), corresponds to the boundary of
> the image in image space

Their substantive claim — that a mismatched grid is the ordinary case and that scanners
produce it constantly — stands unchanged; only the subclause number moves.

### It cannot reach us

`ImageSpec` is one straight-alpha RGBA8 buffer on one grid, three fields and no fourth.
`Command::Image` names one `ImageId`; its only other attenuation is a `MaskId`, and a
`MaskId` names a **list of drawing commands** (§11.5's transparency group), not a raster
with a grid. There is no stencil concept anywhere in this workspace — `grep -rni stencil`
over `crates/` finds only wgpu's `depth_stencil: None`.

So a `/Mask` at a different resolution is resolved before it reaches us, and it is worth
recording exactly where, because that is the answer:

- `pdf-model/src/image.rs::apply_explicit_mask` → `combine_on_the_finer_grid`, which builds
  the output on `max(image.width, mask.width) × max(image.height, mask.height)`,
  nearest-neighbour resamples both sources onto it and **multiplies** the alphas rather than
  replacing (its comment reads that as §11.3.7's `α = shape × opacity` through §11.6.4.1).
  An explicit `/Mask` is always folded eagerly; there is no deferred path for it.
- A `/SMask` whose refinement is unbuildable is deferred instead:
  `pdf-render::ImageSource::AtDeviceScale` keeps the mask in the file's own packed bits and
  rasterises it at `Grid::for_placement`'s device resolution when the placement is known.
  Their witness is `issue16263.pdf` — a **2 × 2 image with a 34 862 × 4 332 mask**, 604 MB
  of RGBA on the finer of the two grids.

What arrives here is one already-composited raster plus a `Nearest`/`Linear` flag
(integration note 1). The decoding cost hayro #1319 measures at 30× is entirely theirs; the
drawing cost #1315 measures is ours only in the form below.

### The mismatch we do have

A **soft mask** is a second grid: §11.5 renders it at device resolution while the image
keeps its own. An image at a coarse grid under a soft mask is therefore "drawing through a
mask whose grid does not match", in the one form that reaches this library — and the answer
must be that the mask's edge lands where the device says, because §11.5.1 makes a soft mask
a thing that "defines values that may vary across different points on the page", not across
points of some image's grid.

### The gate

`crates/quorra-gpu/tests/mask_grid.rs`, four tests at two depths:

- `an_uploaded_image_is_one_raster_on_one_grid` and
  `an_image_command_carries_no_second_raster` destructure `ImageSpec` and `Command::Image`
  **exhaustively**, with no `..`. Neither type is `#[non_exhaustive]`, so a field added to
  either stops this file compiling and whoever adds it reads the comment. That is the gate
  the assumption actually needs: the assumption is not "we sample two grids correctly", it
  is "there is only one grid", and only the type can hold that.
- `a_soft_mask_edge_lands_on_the_device_grid_not_the_images` — a 4 × 4 image over a 48-pixel
  square, so each texel is 12 device pixels, under a soft mask whose rectangle ends at
  device x = 30, which is *inside* a texel. The transition is asserted at 30, and asserted
  not to be at 20 or 32, which is where the two neighbouring texel boundaries are.
- `an_images_own_texel_boundary_lands_where_the_unit_square_puts_it` — the other side of
  §8.9.5.1's mapping: a 2 × 2 image over the same square puts its texel boundary at device
  32, four distinct colours, each constant across the 24 device pixels it covers.

**Verified able to fail**, four ways:

1. `soft_mask_at(p)` in `image.wgsl` replaced by `soft_mask_at(floor(p / 12.0) * 12.0)` —
   the mask sampled on the image's grid — fails the soft-mask test and nothing else.
2. `tex_uv` shifted by a quarter of the unit square fails the texel-boundary test and
   nothing else.
3. A `stencil` field added to `ImageSpec` fails `mask_grid.rs:140` with E0027, "pattern does
   not mention field `stencil`".
4. A `stencil` field added to `Command::Image` fails the build at `quorra-gpu`'s own
   exhaustive match first — which is `command.rs`'s stated promise doing its job — and the
   test's destructuring behind it.

