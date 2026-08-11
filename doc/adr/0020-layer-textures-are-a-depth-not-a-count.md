# ADR 0020 — A frame's layer textures are a depth, not a count

Status: accepted, 2026-08-11. Found while answering a question about ADR 0019's seed:
whether bounding the seeded region would stop documents exceeding limits. It would not,
and this is what does.

## Context

Every plan renders into a ping-pong **pair** of full-target RGBA textures, because a
pass cannot read its own attachment (ADR 0010). The compositor created one pair per
plan, all of them at once, before the first pass — and `internal_texture_bytes` priced
exactly that: `(plans + 1) × 2` full-target textures.

A plan is created per group **and** per element with a non-Normal blend mode, since
§11.3.5 for a single element is an implicit one-element group. At 1191×1684 a pair is
16.05 MB, so the default 256 MiB frame budget held **sixteen plans**. A page with
seventeen — eight groups each holding one blended rectangle, which is ordinary
Illustrator artwork — was refused with `FrameBudgetExceeded { needed: 272767584 }`.

The refusal was honest and the arithmetic was right. What was wrong was the model
behind it: it priced every plan as if all of them existed at the same moment.

## Decision

**Pairs are borrowed from a pool and given back.** The compositor walks the plan tree
depth-first; a child's pair is needed while it renders and while the parent's composite
pass reads it, and never again. Siblings therefore never need pairs simultaneously, so
the peak is the **depth** of the tree.

`internal_texture_bytes` prices `peak_pairs(encoded)` — the depth, computed once from
the plan tree, with masks folded in as the deepest single mask group rather than the sum
(they realise one at a time and release to the same pool). A mask's own R8 output is
still priced per mask: unlike a layer, it is read by draws all over the frame.

**Why handing a texture back with someone else's pixels in it is safe.** Every acquired
pair is fully written before it is read: the first draw pass clears it, a seeded
non-isolated group blits its backdrop over it (ADR 0019), a composite writes its whole
attachment, and a plan with no ops at all clears once. Under a damage scissor the region
written and the region read are the same region. Passes recorded into one encoder
execute in order, so a texture reused by a later sibling is written strictly after the
earlier sibling's composite read it, and `wgpu` inserts the usage transitions.

**`Counters::layer_textures` reports the peak**, because the instrumentation rule here
is the count, not a ratio: a frame that says 6 allocated 6, whatever its group count is.

## What it buys, measured

1191×1684 on RADV, N sibling groups each holding one blended rectangle — three plans
deep, so six textures whatever N is:

| N groups | plans | before | after |
|---|---|---|---|
| 1 | 3 | 6 textures, 48 MB | 6 textures, 48 MB |
| 4 | 9 | 18 textures, 144 MB | 6 textures, 48 MB |
| 8 | 17 | **refused** — 273 MB over a 256 MiB budget | 6 textures, 48 MB |
| 64 | 129 | refused, 2.1 GB | 6 textures, 48 MB |

The ceiling stops being a count. What still costs is nesting, which the scene builder
already bounds at 16 — and a maximally nested page is 17 pairs, 273 MB, still refused
at page size by the same arithmetic. That is the case bbox-bounded layers would answer,
and it is left open below.

No pixel changes: `tests/layer_reuse.rs` holds the counts, the refusal, and that each
group's patch among fifteen siblings is byte-identical to the same group drawn alone.

## What is deliberately not done

**Bounding each layer to the group's device bbox.** It is the other half — it would cut
the *size* of each texture rather than their number, help the deep case this leaves
open, and save the full-target clear and composite a small group pays today. It is also
invasive in a way this is not: every pass writing into a layer would need its geometry
offset and every read would need the origin, across eight shaders, and a wrong offset is
a picture that looks plausible. It wants its own ADR, its own measurement, and the
corpus gate — and it is worth doing *after* this, because this one made the common case
stop being a refusal.

**A pool that outlives the frame.** Textures are created per frame and dropped with the
executor. Keeping them between frames is ADR 0012's deferred-pool question, whose own
measurement (the winding texture) said keeping is worth it; the same may hold here, and
it is a separate change with a separate number.

## Revisit when

A page is refused for internal textures again. With the count gone, that means depth,
and the answer then is the bbox above rather than a bigger budget.
