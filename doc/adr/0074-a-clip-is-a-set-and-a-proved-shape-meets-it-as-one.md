# 0074 — A clip is a set, and a group whose alpha is provably its shape meets it as one

Date: 2026-08-23. Status: **accepted, and built**. **It moves pixels**, in two places and
for two different populations: a group whose elements are provably opaque now meets its
clip by `min` rather than by a product, and the two halves of one clip chain now intersect
at a group's blit instead of multiplying.

The measurements are `doc/notes-group-clip-as-a-set.md`. The code is
`crates/quorra-gpu/src/shaders/composite.wgsl` (`clip_at`, `meet_clip`, `fs_main`) and
`crates/quorra-gpu/src/encode/opacity.rs`, which is new. It answers the caller's
`QUORRA_FEEDBACK.md` §36 — their §24 one level up — and **declines the API change they
asked for**, for the reason §"What this deliberately does not add" gives.

## Context

`composite.wgsl` weighted the whole of a group's premultiplied raster by one scalar:

```wgsl
let w = params.alpha * soft_mask_at(p) * clip_coverage(p) * residue_value(p);
```

Three of those four factors are multiplications ISO 32000-2 states in as many words.
§11.3.7.2:

> The three opacity inputs shall be multiplied together, producing an intermediate value
> called the source opacity.

with §11.6.4.4's constant and §11.6.4.3's mask as two of the three, and §11.3.7.1 making
`αs = fs × qs`. **The clip is in neither of that subclause's two products.** It enters one
step earlier, on the object's own shape, and §8.5.4 says it of a group in its own sentence:

> In the context of the transparent imaging model (PDF 1.4), the current clipping path
> constrains an object’s shape (see 11.2, "Overview of transparency"). The effective shape
> is the intersection of the object’s intrinsic shape with the clipping path; the source
> shape value shall be 0.0 outside this intersection. Similarly, the shape of a transparency
> group (defined as the union of the shapes of its constituent objects) shall be influenced
> both by the clipping path in effect when each of the objects is painted and by the one in
> effect at the time the group’s results are painted onto its backdrop.

§10.7.4 makes "influenced by" a set operation and not an arithmetic one:

> For clipping, the clipping region consists of the set of pixels that would be included by
> a fill operation. Subsequent painting operations shall affect a region that is the
> intersection of the set of pixels defined by the clipping region with the set of pixels
> for the region to be painted.

The consequence is the one the caller names. Where a group's `/BBox` is exactly the
rectangle its content fills — the common form-XObject shape — the group's own edge and its
clip's edge are the same line, and the boundary pixel was painted at the **square** of its
coverage. On this tree's own fixture: **92 of 255 where the clause asks for 153**, an edge
covering 0.6 of a device column drawn at 0.36.

**Why the compositor could not simply take `min` and be done.** A finished group reaches
this pass as one premultiplied raster whose alpha is the union of each element's shape
*times its opacity* — §11.4.4's Table 140 group alpha is not carried (ADR 0019), and neither
is a shape channel. Intersecting a clip with an *opacity* is wrong in the other direction,
and measurably: a group at half opacity covering a whole pixel under a clip admitting 0.6 of
it must be painted at 0.3 — **77** of 255 — and `min` against its alpha paints it at
**128**. So the question the pass cannot answer from the raster is *which of the two a
fractional alpha is*.

**And a second product at the same site**, found by reading rather than reported: the clip
reached the pass as `clip_coverage(p) * residue_value(p)` — the chain's rectangular links,
resolved into one rectangle at encode time (ADR 0007), times its curved links, rasterised
into the residue. Those are two links of **one** clipping path, and ADR 0030 already decided
what composes links: `min`, because §8.5.4 sets the graphics state's clipping path "to the
intersection of the current clipping path and the newly constructed path", so a chain is one
region rather than a stack of boundaries. The rule was taken in `Encoder::intersect_links`
and had never reached this blit.

## Decision

**Two changes, and the first is what makes the second sayable.**

**1. The clip in force at a group's blit is one region.**

```wgsl
fn clip_at(p: vec2f) -> f32 {
    return min(clip_coverage(p), residue_value(p));
}
```

ADR 0030's rule at the site it had not reached. `residue_value` is exactly 1 where a chain
has no residue, so this is the identity wherever the question does not arise.

**2. The encoder *proves* when a group's alpha is its shape, and then the group meets that
region by intersection.**

The proof is `encode::opacity::every_opacity_is_one`, a walk of the group's own commands
that answers whether any opacity input below 1.0 exists inside it. It is a proof and not a
heuristic because the clause keeps the two quantities in step by construction: §11.3.7.1
defines `α = f × q`, and §11.3.7.3's union and §11.4.6's knockout stages apply the *same*
recurrence to `f` and to `α`, differing only in the opacity inputs they carry. §11.6.4.2
supplies the base case —

> All elementary objects shall have an intrinsic opacity qj of 1.0 everywhere.

— which leaves exactly three doors for an opacity below 1: §11.6.4.4's constant (a paint's
alpha, an image's alpha, a group's alpha), §11.6.4.3's soft mask (opacity by ADR 0066), and a
nested group carrying either. Close all three and `α = f` at every step, so the group's
accumulated alpha *is* §11.6.4.2's group shape.

The compositor then takes:

```wgsl
fn meet_clip(s: vec4f, c: f32) -> vec4f {
    if params.alpha_is_shape == 0u { return s * c; }
    if s.a <= c { return s; }
    return vec4f(s.rgb * (c / s.a), c);
}
```

A clip cuts a shape and never touches a colour, so the straight colour is unchanged and only
the alpha moves; the premultiplied vector is rescaled by the ratio of the two alphas, and
`s.a > c` implies `s.a > 0`, so the division cannot be by zero. The soft mask and the
constant still multiply, after the meeting rather than into it — which is §11.3.7.2's own
order, and the caller's §24a arrives at the same place from the other end:
`min(f·S, C·S) = min(f·S, P)`.

**Every unknown answers "not proved".** An image is declined whatever its constant alpha
says, because its samples carry an alpha the walk never sees; a shading, a mesh and a
function paint are declined because their colours live on the device rather than in the
scene. A wrong `false` costs only the improvement; a wrong `true` paints a half-transparent
group at more than it asked for.

**A non-isolated group is never proved, and never *counts* either.** §11.4.4 seeds its buffer
with its own backdrop, so the raster is `E(B)` and its alpha carries the backdrop's — nobody's
shape, however opaque the elements are. It is refused twice for that: as the group being
composited (`spec.isolated` is anded into the answer) and as an *element* of one, because a
nested non-isolated group's own clip reaches it as a weight, so what it contributes to the
enclosing alpha is `f × C` where its shape is `min(f, C)`. The isolated case is admitted, and
the recursion is what makes that sound rather than hopeful: an admitted nested group is one
whose body passed this same test, which is the condition its own `encode_group` evaluates, so
its clip *was* intersected and its contributed alpha *is* its shape.

## What this deliberately does not add

**The caller asked for a boolean on `GroupSpec` (`alpha_is_shape`, their §36.4) and this
decision does not add one.** Three reasons, in the order they decide it:

1. **We can prove what they would assert**, for the population their own witness is in — a
   form XObject full of opaque marks under a `/BBox` clip. A proof from the command list
   cannot be wrong; an assertion can, and a wrong one has no failure mode we could detect.
   Principle 3's "data is not trusted just because a friend produced it" points the same way.
2. **It would be a breaking public API change** on the type every caller constructs by
   struct literal, taken as a side effect of a clip fix.
3. **It would read as a reversal of ADR 0066**, which decided from Table 57 that one flag
   governs the soft mask and the alpha constant together, that its initial value is `false`,
   and that **a `Scene` carries no such flag**. Nothing in this decision changes that: our
   masks and constants are still opacity, still multiply, and still after the clip.

**What the proof cannot see, and what that costs.** A caller who painted a group's content
under `/AIS true` — where §11.6.4.3's mask *is* shape — has a group whose alpha is its shape
and whose commands, in our vocabulary, carry a mask that is opacity. We answer "not proved"
and draw the product. Measured on the fixture: a mask worth 1.0 at every pixel changes no
pixel's correct value and moves ours from 153 to **92**. That is the hole, it is asserted in
`a_group_whose_opacity_cannot_be_valued_keeps_the_product` rather than described, and if a
corpus page turns up in it, the boolean is the answer — taken with the caller, as its own
decision, with the API bump written down.

## Consequences

**The two readings are 61 bytes apart on the fixture that separates them**, and the fixture
is one rectangle and one clip rather than a document. Column 2 of an opaque black rectangle
whose right edge is at x = 2.6, under a rectangular clip with the same edge, read from
llvmpipe:

| | alpha of the boundary pixel |
|---|---:|
| the mark alone, no group and no clip | 153 |
| the group, opacity proved (this decision) | **153** |
| the group, opacity not proved (the product) | 92 |

and where the clip *contains* the group — the group's edge at 2.2, the clip's at 2.6, two
different boundaries in one pixel — 51 against the product's 31. That row is the one where
`min` is provably exact rather than merely no further from the truth: `S ∩ C = S` where
`S ⊆ C`, so the exact area of the intersection inside the pixel is the group's own 0.2.

**For two unrelated boundaries in one pixel, neither rule is exact**, and this decision does
not pretend otherwise. The true value is the area of the intersection of the two regions
inside the cell, which lies in `[max(0, a+b−1), min(a, b)]`; the product is inside that
interval and `min` is its upper end. ADR 0030's argument carries over unchanged — `min` never
moves *away* from the clause, and is exact wherever the two boundaries coincide or nest,
which is what a `/BBox` on its content's edge is. Only a conflation-free rasteriser answers
the general case, and neither backend is one.

**The chain change moves pixels for groups whose opacity is not proved as well**, and it is
the narrower of the two: it needs a group under a chain holding both a rectangular link with
a fractional device edge and a residue link that is also fractional **in the same pixel**.
Where either is 0 or 1 — every pixel of an integral clip, every pixel a curve does not cross
— `min` and the product agree exactly.

**What it costs to prove.** One pass over a group's own commands, and a nested group's body
is walked again by its own `encode_group`, so the repeat is bounded by `MAX_GROUP_DEPTH`
(16) and not by the page. The shader gains one `min` and one uniform-predicated branch per
fragment of a composite. Neither is benchmarked and no claim is made about either: the
suite's clocks cannot see a change this size, and inventing a number would be worse than
saying so.

**The composite uniform grew from 128 to 144 bytes**, and the gate that exists for it worked:
`min_binding_size` still said 128, and three `pipeline` unit tests went red on the spot
rather than a page being drawn wrong. `shaders::layout` then checked the new word's offset
against the WGSL declaration, which is the check nothing in the toolchain does.

**A corpus run is owed.** Predicted movement: pages with a group whose content is opaque and
whose clip edge shares a pixel with the group's own — upward, at boundary pixels — plus the
chain half above. Nothing in it needs a change on the caller's side to be reachable, which is
the difference the boolean would have made.

## What this does not decide

**It does not move the mark.** Where a clip meets a *mark's* coverage — `coverage.wgsl`'s
`shape_at`, and `Encoder::residue_product` on the CPU side — the product stays, which is
ADR 0030's deliberate half and the caller's §24 unanswered. The reason is not that the clause
differs: it is that the mark's site is a different piece of arithmetic, on the other side of
the CPU/GPU boundary, with its own cost and its own corpus movement, and taking two of them
in one round would leave neither measured. **The two sites now differ, and that is recorded
rather than tidied.**

**The rectangular link still multiplies into a mark's tile**, one level below the blit this
ADR fixed, and by ADR 0030's own rule that is a defect. Same reason for leaving it.

**It does not give us a shape channel.** A group whose elements are genuinely
half-transparent still cannot be intersected with anything, and the product it gets is the
right arithmetic there rather than a fallback.

**It does not touch the mask.** §11.6.4.3's soft mask remains opacity by ADR 0066, and it
multiplies the group after the clip has met it, which is where §11.3.7.2 puts it.

## Revisit when

A corpus page shows the hole above: a group whose alpha is its shape by a route our walk
cannot see — `/AIS true` inside it, or an opaque image, or an opaque ramp. The cheapest
answers in order are to read the resources the walk already has handles to (a ramp's stops, an
image's samples), and only then the caller's boolean, which is a decision neither side can
take alone and an API bump either way.
