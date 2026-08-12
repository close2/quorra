# ADR 0036 — A layer is as big as its plan

Status: **accepted and landing in two commits**, 2026-08-13. The first is the origin, at
zero everywhere and verified to change nothing; the second is the sizing that makes it
non-zero. This file records the decision and says which half is in the tree, because a
half-finished cross-cutting change that nobody wrote down is how a renderer acquires a
defect nobody can attribute.

**In the tree now:** the origin, plumbed through every stage that renders into a layer,
with the value zero. **Not yet:** the plan bounds and the allocation that uses them.

## Context

The compositor allocates a layer as a **ping-pong pair of full-target `Rgba8Unorm`
textures** and a soft mask as a full-target R8, whatever the group or mask actually
covers. On the three pages of the caller's corpus that refuse for bytes at 4×, the plans
that own those textures cover:

| page | target | the plans cover | refused at |
|---|---|---|---|
| `issue269_2` | 3 628 × 5 104 | **0.0 %** — a 4 × 208 sliver, a 134 × 4 sliver, six more | 296 MB, all pairs |
| `issue14297` | 4 763 × 3 368 | **0.1 – 0.4 %** | 321 MB, 257 of it pairs |
| `issue16287` | 2 448 × 9 504 | **4 – 6.5 %** | 279 MB: 186 pairs + 93 masks |

against a 268 MiB budget. So a page is refused for a hundred times the memory its groups
need, and every page with a group pays the same ratio in allocation and clears on every
frame.

## Decision

**A layer is allocated at its plan's device bounds** — the union of what the plan draws,
intersected with its clip and the target, rounded out to whole pixels — and every stage
that renders into it learns where that rectangle sits.

The mechanism is ADR 0028's, and so is the hazard. A pane taught three places to subtract
its origin and shipped with one of them missing, which drew nothing at all for every band
after the first. The same discipline applies here: the origin is one number, stated in one
place, and each stage that needs it says in its own comment what it is for.

- `vs_*` subtracts the origin before dividing by the attachment's size.
- `fs_*` adds it back to recover the device pixel it is shading — clip rectangles, tile
  lookups, residues and masks are all stated in device space.
- The composite reads its child at the child's origin and writes at its own.

## Why it lands in two commits

The first commit adds the origin to every one of those stages **and leaves it zero**. That
is a change whose correctness is checkable exactly: the frame must be what it was. It is —
207 tests, and the caller's 957-page corpus at 915 agree / 37 differ / 5 refused, the same
verdicts as the commit before it.

It also caught the one thing a compiler cannot: the globals uniform was declared visible to
the **vertex stage only**, and reading it from a fragment stage is a validation error that
refuses every pipeline. Better found by a zero-valued no-op than by a page drawn wrongly.

The second commit computes the bounds and allocates from them, and its check is the corpus
again — plus the three pages above, which should stop refusing.

## What it will cost

**Pages whose groups cover the whole target gain nothing**, and pay one subtraction per
vertex and one addition per fragment for the privilege. That is the price of a uniform
mechanism, and it is small.

**The pool is keyed by size**, so a frame whose plans differ in size cannot reuse one
plan's pair for another's. Pairs are per frame (ADR 0012, re-examined and upheld in
ADR 0035), so this costs a little more allocation on a page of differently-sized groups
than the current one-size-fits-all does — measured against a page that no longer allocates
a hundred times what it needs.

**And `Device::warm_for` becomes a poorer predictor** (ADR 0035's own "revisit when"): the
size worth warming stops being the target's and becomes the largest plan's, which a host
cannot know. What it warms will still be right for the frame's *root*, which is the one
plan that is always target-sized.
