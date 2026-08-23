# The clip a group met by multiplying — round notes, 2026-08-23

Opened to answer the caller's `QUORRA_FEEDBACK.md` §36: `composite.wgsl` multiplied a
group's clip into the same scalar as its constant alpha and its soft mask, so a group whose
`/BBox` stands on its content's own edge painted that edge at the **square** of its coverage.
ADR 0074 is the decision. This file is what was measured, in the order it was measured,
including the two things the round found that the report did not ask about and the one thing
it asked for that this round declined to build.

## 1. What the four factors were, and which of them is not a factor at all

```wgsl
let w = params.alpha * soft_mask_at(p) * clip_coverage(p) * residue_value(p);
```

- `params.alpha` — §11.6.4.4's constant, an **opacity** input (ADR 0066: a `Scene` has no
  `/AIS`, and Table 57's initial value is `false`).
- `soft_mask_at(p)` — §11.6.4.3's mask, an **opacity** input by the same reading.
- `clip_coverage(p)` — the analytic overlap of the pixel cell with the chain's resolved
  **rectangle** (ADR 0007).
- `residue_value(p)` — the chain's **non-rectangular** links, rasterised on the CPU and
  intersected with each other by `min` (ADR 0030), read out of the scratch image.

§11.3.7.2 multiplies three shape inputs and three opacity inputs, and the clip is in none of
the six; §8.5.4 applies it to the object's own shape instead. So the first two multiply and
the last two do not — and **the last two are one clip**, split in half by our own encoding
rather than by the model.

**Where the group's own shape lives** was the question to answer before touching anything: it
is not a factor of `w` at all. It is the alpha of the group's raster, `s.a`, which is the
union of each element's shape *times its opacity* — §11.4.4's Table 140 group alpha is not
carried (ADR 0019) and there is no shape channel.

## 2. The fixture, and why 0.6 and 0.2

One black rectangle whose right edge falls inside device column 2, under a clip whose own
edge falls in the same column, read at one pixel. Both coverages are exactly representable in
eight bits — `0.6 = 153/255`, `0.2 = 51/255` — so no assertion in the file depends on a
rounding rule, and the child layer (`Rgba8Unorm`) stores the group's alpha exactly rather
than near-exactly.

Measured on llvmpipe, `crates/quorra-gpu/tests/group_clip_is_a_set.rs`:

| what is drawn | shape `S` | clip `C` | drawn | the product would be |
|---|---:|---:|---:|---:|
| the mark alone, no group, no clip | 0.6 | — | **153** | — |
| group, `/BBox` on its own edge, opacity proved | 0.6 | 0.6 | **153** | 92 |
| the same, opacity not proved | 0.6 | 0.6 | **92** | 92 |
| group inside a containing clip, opacity proved | 0.2 | 0.6 | **51** | 31 |
| group under an integral clip edge (x = 8) | 0.6 | 1 | **153** | 153 |

The second row is the caller's 0.5059 → 0.2549 in our own numbers: 0.6 → 0.36. The fourth is
the row that decides the argument rather than illustrating it — the two boundaries are
*different*, the group's half-plane lies inside the clip's, and the exact area of the
intersection inside the pixel is the group's own 0.2. `min` is exact there; the product is
not off by rounding but by 0.08 of a pixel of ink.

## 3. The number that says why a flag — of some kind — is needed at all

Take `min` against a raster whose alpha is *opacity* and the error runs the other way. A
group covering column 2 completely at half opacity, under the same 0.6 clip:

| | alpha of the boundary pixel |
|---|---:|
| §11.3.7.1's answer, `min(1, 0.6) × 0.5` | **77** |
| the product, which is what it gets | **77** |
| `min` against its alpha, `min(0.5, 0.6)` | 128 |

So the pass cannot be made to take `min` unconditionally, and the 51-byte gap is what a wrong
answer to "is this alpha a shape?" costs.

## 4. What the caller asked for, and what was built instead

§36.4 asks for a boolean on `GroupSpec` — their `Command::Group::alpha_is_shape` — on the
grounds that "only the interpreter knows the `/AIS` reading; a scene vocabulary cannot derive
it from the commands."

**Most of it can be derived, and the derivation is a proof.** §11.3.7.1 defines `α = f × q`;
§11.3.7.3's union and §11.4.6's stages apply the same recurrence to `f` and `α`, differing
only in the opacity inputs; and §11.6.4.2 gives every elementary object an opacity of 1.0. So
if no opacity input below 1 exists anywhere in a group's subtree, `α = f` at every step of it.
That is `encode::opacity::every_opacity_is_one`, and it is what the compositor is told.

**One hole in that argument was found by writing it down**, which is the reason to write it
down: a nested *non-isolated* group has every opacity input at 1.0 and still breaks
`α = f`, because its own raster is `E(B)` and its clip therefore reaches it as a weight —
`f × C` contributed where its shape is `min(f, C)`. The walk refuses it explicitly, with the
isolated twin beside it in the unit test so that the refusal is not "all nesting is refused".

What the walk cannot see is the case their sentence is actually about: content painted under
`/AIS true`, where a mask *is* shape. In our vocabulary that group carries a mask, a mask is
opacity (ADR 0066), and the walk answers "not proved". Cost, measured with a mask worth 1.0 at
every pixel — which cannot change what any pixel should be, and changes the route, which is
their own §36.3 probe design:

| | alpha of the boundary pixel |
|---|---:|
| the clause's answer | 153 |
| proved-opaque route | **153** |
| the same fixture with a mask of ones | **92** |

That is a hole in the improvement, not a defect the improvement introduced — 92 is what every
group got before this round — and it is asserted in a test rather than described.

Three things decided against adding the field: a proof cannot be wrong where an assertion can
(principle 3), it is a breaking change to the type every caller builds by struct literal, and
it would read as a reversal of ADR 0066's "a `Scene` carries no such flag" taken as a side
effect of a clip fix. ADR 0074's "Revisit when" names what would reopen it.

## 5. The second product, found by reading rather than by report

`clip_coverage(p) * residue_value(p)` is a chain's rectangular half times its curved half —
two links of one clipping path, composed by the rule ADR 0030 exists to forbid. That rule was
taken in `Encoder::intersect_links` (link against link) and had never reached this blit.

Fixture: a rectangular clip at x ≤ 2.6 chained with a **pentagon** whose only near edge is
the same vertical line, over a group whose content fills column 2 completely — so the group's
own shape is 1 there and what is left is the chain alone.

| | alpha of the boundary pixel |
|---|---:|
| the links intersected (this decision) | **153** |
| the links multiplied (what the pass did) | 92 |

A pentagon and not a quadrilateral on purpose: `axis_aligned_rect` recognises the four-sided
form and resolves it into the chain's *rectangle*, and the test would then have composed one
link with nothing and passed. `Counters::clip_residue_regions` is asserted to be 1 in that
test for exactly that reason — the assertion about the pixel is worth nothing without it.

## 6. Forcing each defect back, and what went red

Each was restored on its own, because "a test failed" is not the same claim as "these tests
failed and no others".

**Forcing the group product** (`meet_clip` returning `s * c` unconditionally) — two tests:

```
a_group_whose_clip_stands_on_its_own_edge_paints_that_edge_once   left 92, right 153
a_clip_that_contains_the_group_takes_nothing_from_it              left 31, right  51
```

**Forcing the chain product** (`clip_at` returning `clip_coverage(p) * residue_value(p)`) —
one test:

```
the_links_of_one_chain_intersect_at_a_groups_blit                 left 92, right 153
```

**Forcing the proof to answer `true` for everything** — the unsafe direction, which is the
one worth a gate. Five of the six unit tests in `encode::opacity::tests` and two device
tests:

```
a_group_whose_opacity_is_below_one_is_not_intersected_with_its_clip   read 128, wanted 77
a_group_whose_opacity_cannot_be_valued_keeps_the_product              left 153, right 92
```

The whole workspace was run under the first two forcings, and **no existing test moved under
either**: nothing in the suite drew a group whose clip edge shared a pixel with anything. The
tests in this round are the instrument that did not exist.

## 7. The uniform grew, and the gate for that worked

`Params` went from 128 to 144 bytes (WGSL §14.4.6 rounds a uniform struct up to 16).
`min_binding_size` in `pipeline/layouts.rs` still said 128, and the effect was immediate and
loud: `the_warm_set_includes_the_present_format_when_given_one`,
`the_warm_set_compiles_one_format_when_no_second_is_needed` and
`the_presenting_pass_is_warmed_for_the_surfaces_format_whatever_it_is` failed at pipeline
creation. `shaders::layout` then checked the new word's *offset* against the WGSL
declaration, which is the half no toolchain does — `wgpu` validates the buffer's size and
nothing validates where a field sits inside it.

## 8. What this round did not do

- **The mark's clip still multiplies** — `coverage.wgsl`'s `shape_at` and
  `Encoder::residue_product`. That is the caller's §24, ADR 0030's deliberate half, and it is
  a different piece of arithmetic on a different side of the CPU/GPU boundary. Taking it in
  the same round would have left neither half measured.
- **The rectangular link still multiplies into a mark's tile**, one level below the blit this
  round fixed, and by ADR 0030's own rule that is a defect. Same reason for leaving it: it
  moves ink on every clipped mark rather than on a group's boundary pixel, and it owes its
  own corpus run.
- **No corpus run.** Owed, and the main session runs them in one copy of the caller's tree.
  Both halves of this change are reachable from their corpus as it stands — no adapter change
  is needed, which is the difference the declined boolean would have made.

---

## The corpus run the round owed, and the trigger it fired

Taken by the integrating session on 2026-08-23, one copy of the caller's tree rsynced that
day, both columns in the same copy, `[patch]` flipped between `97ad95ac` and the merged tree
(ADR 0074 + ADR 0075 + ADR 0077), page one at scale 1, their gate's own configuration — which
since their §37.2 is **the shipping quantum**, not the quantum off.

| | agree | differ | refused | not comparable |
|---|---:|---:|---:|---:|
| `97ad95ac` | 933 | 22 | 2 | 17 |
| merged | 933 | 22 | 2 | 17 |

**Exactly one page line moves out of 957**, and per-page comparison is the only reason it was
seen at all — every total is identical:

```
- differs: 22060_A1_01_Plans.pdf: mean 0.7838 worst tile 5.69 at (576, 768) … ssim 0.98626
+ differs: 22060_A1_01_Plans.pdf: mean 0.8248 worst tile 5.69 at (576, 768) … ssim 0.98558
```

**It moves away from their oracle**, and that is the finding rather than a disappointment,
because their oracle has *already taken this same clause reading*: their §36.5 records the
group-level intersection landing on their CPU backend with their cross-backend gate unmoved at
933 / 22. Two implementations of one clause should have converged. On this page they parted.

**The trigger this ADR named has therefore fired on the first corpus run.** ADR 0074 §"What
this does not decide" says the caller's boolean is the answer "if a corpus page turns up in
it". One has. The difference between the two sides is exactly the one the ADR isolates: they
*state* `alpha_is_shape` from the interpreter, because only the interpreter has read `/AIS`;
we *prove* it from the command list, and the proof cannot see a mask that `/AIS true` made a
shape. Such a group keeps the product here and takes the `min` there.

Two candidate readings of the direction, and they are distinguishable:

1. **Our proof is too strict** — the page has a masked group under `/AIS true`, we keep the
   product, their oracle takes `min`, and we part by the shortfall this round already measured
   at 92 of 255 against 153.
2. **Our proof is too permissive** — it fires on a group their flag leaves unset, and we take
   a `min` they do not.

**What settles it is one line on their side**: whether `alpha_is_shape` is set on any group of
`22060_A1_01_Plans.pdf`'s first page. It is their flag, from their interpreter, on their page;
we cannot read it from here and should not guess at it. Either way the remedy is the same and
it is the one they asked for — the boolean — which is why this note ends by handing the page
back rather than by proposing arithmetic.

**Nothing else moved.** ADR 0075 and ADR 0077 predicted no movement and moved nothing: the
other 956 page lines are identical to the character.
