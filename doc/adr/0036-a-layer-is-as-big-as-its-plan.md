# ADR 0036 — A layer is as big as its plan

Status: accepted, 2026-08-13, and landed in two commits — the origin at zero everywhere
(verified to change nothing), then the sizing that makes it non-zero.

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

The second commit computes the bounds and allocates from them, and the corpus was again the
check — and it earned its place twice, because the sizing shipped with two defects that no
unit test in this tree would have caught.

**A mask realised at its plan's size and reduced as though it were whole.** `realise_masks`
renders the mask's group through the same `render_plan` and then reduces it into an R8
mask the whole frame samples in device space; the reduce reads at the fragment's own
position and has no origin to subtract. Thirty-one pages left *agree*. Mask plans now
render at the target's size, which is what they did before this ADR — sizing them is the
next thing to take off the number, and it needs the reduce and every sampler of a mask to
learn an origin too.

**A pair reused at somebody else's size.** `LayerPool::acquire` popped any free pair, which
was the same as popping a matching one while every pair was the target's. Twelve pages,
with a highlight sitting above the line it belongs to. The pool now matches on extent, and
`layers.rs`'s own test states the property.

Both were found by running the caller's corpus and comparing verdicts — 884, then 903,
then 915 — which is the argument for the corpus being part of the change rather than a
check after it.

## What it bought

The caller's corpus at scale 4, CPU lane, against the commit before:

| | agree | differ | refused |
|---|---:|---:|---:|
| before | 925 | 16 | 11 |
| after | **927** | 16 | **9** |

`issue269_2.pdf` and `issue14297.pdf` refused for 296 and 321 MB of layer pairs and now
draw — and agree with the oracle rather than merely drawing. At scale 1 nothing moves:
915 / 37 / 5, the same pages for the same reasons.

`issue16287.pdf` still refuses, at 291 MB, and its arithmetic says what is left: 186 MB is
the **root's** pair, which is the target's size because the root *is* the target, and 93 MB
is four full-target soft masks. Neither is a plan this ADR can shrink.

## What it cost

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
