# ADR 0041 — A child with nothing to give is not emitted

Status: accepted, 2026-08-13. The item `HANDOVER.md` recorded under ADR 0038 as not taken:
*a child whose region misses its parent's is still rendered before the composite discovers
there is nothing to write; culling it belongs in the encoder, which is where the clip that
emptied it is known.*

## Context

ADR 0038 made a composite write `child ∩ parent` and skip a child that meets its parent
nowhere. By then the child has been rendered: a layer texture acquired, a pass run for
every plan beneath it, its own subtree composited. The discovery is correct and it is late.

It is also incomplete, and that is the sharper half. The parent's bounds grow by
`child.bounds ∩ child.clip_rect` and `LayerPlan::mark` ignores a rectangle with no area
(`encode.rs`), so a child its clip removes entirely leaves its parent's bounds *untouched*
while keeping bounds of its own — which is exactly how the two regions come to miss each
other. And §11.4.4's non-isolated group takes its parent's whole region (ADR 0038), so for
that kind the compositor's test can never fire at all, however far outside its clip the
group draws.

The clip that empties a child is known in the encoder, at the moment the `Op::Child` would
be appended. Nothing downstream can know it as well: by then it is two rectangles that
have both been rounded out to whole pixels and clamped to the target.

## Decision

**A child whose bounds, held to the clip its composite will apply, hold no device pixel is
not appended to its parent's plan at all.** One method, `Encoder::push_child`, decides;
`push_op` routes every `Op::Child` through it so no call site can reach the plain append
and skip the decision.

### Why the frame is the same frame

Write `b` for the backdrop the composite reads, `s` for the child's pixel and `w` for the
group's constant alpha times its soft mask, its clip coverage and its clip residue. The
test establishes that at every pixel of the parent either `s = 0` — outside the child's own
marks, and a plan marks nothing outside its bounds (ADR 0036) — or `w = 0`, outside its
clip rectangle. `composite.wgsl` is one of four things, and each lands on `b`:

| | with `s = 0`, or `w = 0` | |
|---|---|---|
| §11.3.6, the ordinary composite | `co = ab·(1−as)·Cb`, `ao = as + ab·(1−as)`, `as = 0` | `= b` |
| §11.4.6 stage 1, the erase (`compose == 1`) | `b × (1 − s.a·w)` | `= b` |
| §11.4.6 stage 2, the deposit (`compose == 2`) | `b + s·w` | `= b` |
| §11.4.4, the non-isolated group | `mix(b, s, w)`, and `s = b` | `= b` |

The erase is the row worth reading twice, because it is the one composite in clause 11 that
*removes* what is already on the page: a wrong cull there shows as a hole rather than as a
missing mark. **An erase weighted by a shape that is zero everywhere erases nothing** —
`P' = (1 − f) × P` with `f = 0` is `P` — and the shape it is weighted by is the group's own
alpha (ADR 0033), which is zero wherever the group marked nothing and multiplied by zero
wherever the clip admits nothing.

The last row needs its own sentence, because `s = 0` is not why it holds. §11.4.4's group
is seeded with a texel-for-texel copy of its parent's accumulator, and nothing writes that
accumulator between the seed and the composite's backdrop copy, so wherever the group
marked nothing `s` *is* `b` and `mix(b, b, w)` is `b` for any weight. The two cases cannot
combine into a third: `SceneBuilder::check_group_compose` refuses `DestOut` and `Plus` on a
group that is not isolated, precisely because the seed would put the backdrop's alpha into
the shape those stages read.

### Emptiness is decided at pixel granularity, not on area

The composite works a whole pixel at a time: `clip_coverage` is the overlap of the pixel
cell with the clip rectangle, and `s` is one colour for the whole pixel however little of
it the child marked. So a pixel `p` can carry a contribution only if `[p, p+1)²` overlaps
the bounds *and* the clip with positive area, and the integers that do are
`[floor(min), ceil(max))` for each. The test is therefore that the intersection **rounded
out** is empty, `floor(x0) ≥ ceil(x1)` in either axis.

Both other readings are wrong, in opposite directions. Rounding in would drop a real
half-covered edge pixel. Testing positive area — which is what `mark` does — would cull a
bounds and a clip that abut at a fractional coordinate while still sharing the pixel
between them, and the composite would have written a contribution there. The rounding-out
rule makes *culled implies unmarked* true: every child this drops is one the parent's
bounds had already ignored.

### The subtree is still encoded

The test needs the child's bounds, and the bounds are the walk's output, so the cull comes
after the walk and saves the *rendering* rather than the encoding. That ordering is not
only forced, it is wanted: ADR 0015's rule is that visibility does not decide validity, and
skipping a group's body would mean a scene naming an unknown ramp inside a clipped-away
group no longer refused. `tests/cull.rs` has held that property since M2 and would have
caught it.

## What it costs

**A culled child's plan stays in `Encoded::layers`, unreferenced.** That list is indexed by
`ChildOp::layer` *and* by `MaskPlan::root`; removing an entry would shift every index above
it, and an index that silently resolves to the wrong plan is a plausible-looking wrong page.
The orphan costs nothing that can be measured: `peak_layer_bytes` descends through
`Op::Child` from the root and the mask roots, so nothing reaches it, its `chain` entry is
computed and never read, and no texture is ever acquired for it.

**But it keeps the frame off the flat fast path**, because `Executor::is_flat` asks whether
`layers` is empty rather than whether any plan is reachable. A page whose only group is
clipped away now renders through a root accumulator and a blit where it could have drawn
straight into the target. That is the one real cost here, it is small (the flat path's own
saving is one texture and one pass), and it is left rather than fixed because "no plan is
reachable" is a different question from "no plan exists" and deserves to be asked
deliberately. `tests/cull.rs` pins the current answer at `layer_textures == 1` so that
changing it is a decision rather than a drift.

**A culled group still pays for its clip residue.** `plan_group_residue` runs before the op
is built, so a group with a non-rectangular clip rasterises and charges its residue tile
even when the cull then drops it. Moving the test earlier would save that, at the price of
a second place computing the same rectangle as `push_child` — and the two disagreeing is a
worse failure than the tile is a cost. Left, and worth revisiting only if a page is ever
seen to refuse for it.

**A culled group's soft mask is still realised.** Masks realise for the frame in id order
(§11.5) from `mask_plans`, which the cull does not touch. Skipping the mask of a group
nobody composites is a reachability question about masks, not about children, and it is the
same shape of question as `is_flat`'s.

## What it buys, and what is not claimed

The saving is a layer texture, a pass for each plan beneath the child, the copy of the
backdrop the composite would have read, and the composite itself — per culled child, per
frame. `Counters::layers_culled` reports the count, so the saving is measured rather than
assumed; it is deliberately *not* `commands_culled`, whose per-item cost is a geometry
build that never happened, while this one's is a render of a subtree that was fully
encoded. Adding the two would produce a number about nothing.

**No corpus figure is quoted, on purpose.** The caller's corpus was not run for this
change: it is one of several landing in parallel and the owner runs the gate once over all
of them. What the corpus would measure — how many real pages hold a group their clip
empties — is unknown here, and the honest statement is that the change is a strict
reduction in work per occurrence rather than a claim about how often it occurs. `cull.rs`
carries the whole correctness argument in the meantime, one test per composite the clause
defines.

The pixels do not move, and that is asserted rather than reasoned: each test renders the
scene with the clipped-away group and the same scene without it and compares the rasters
byte for byte, including the case where the two take different paths through the compositor
(the group-less scene is flat; the culled one is not).
