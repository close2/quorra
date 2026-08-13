# ADR 0038 — A plan accumulates in one texture, and the composite copies what it covers

Status: accepted, 2026-08-13, and landed in two commits — the blit's source origin at zero
(verified to change nothing), then the accumulator that uses it.

## Context

ADR 0036 sized every layer to its plan and ADR 0037 every mask to its own, and what was
left was one number: on `issue16287.pdf` at 4× the frame needed 203 188 174 bytes and
**186 126 336 of them — 91.6 % — were the root plan's ping-pong pair**, two textures at the
target's size because the root *is* the target. Every page with a group pays the same ratio
on every frame, refused or not.

`HANDOVER.md` asked whether the root could ping-pong against the target the frame draws
into. It cannot, and the reasons are three separate contracts: a `Target::Texture` is the
caller's and is validated for `RENDER_ATTACHMENT` alone, so it usually cannot be sampled; a
surface texture is a swapchain image in the surface's own format; and a damage patch may
not touch a pixel outside its rectangles, which a ping-pong partner is written all over.

## The observation the pair rested on, and did not need to

A composite writes its **whole** attachment. It has to, in a ping-pong: the pixels the
child does not cover must reach the other half of the pair, or the next pass reads stale
ones. But look at what it writes there. Outside the child's rectangle `s` is zero, and
every branch of `composite.wgsl` collapses:

| | with `s = 0` | |
|---|---|---|
| §11.3.6, isolated | `co = ab·cb`, `ao = ab` | `= b` |
| §11.4.6 stage 1, the erase | `b × (1 − 0)` | `= b` |
| §11.4.6 stage 2, the deposit | `b + 0` | `= b` |
| §11.4.4, non-isolated | `mix(b, s, w)` | *n/a — see below* |

So over most of its attachment the composite is an expensive copy of the pixels it just
read. The non-isolated row is not an exception: §11.4.4's seed is copied texel for texel,
so a seeded plan takes its parent's region (ADR 0036) and its rectangle *is* the parent's —
`s = 0` never happens.

## Decision

**A plan keeps one accumulator, and a composite writes only `child ∩ parent`.**

Before each composite, the pixels that composite covers are copied out of the accumulator
into a texture **the size of the child's rectangle**, because a pass cannot read the
attachment it writes. The composite then writes back into the accumulator, scissored to the
same rectangle, with `LoadOp::Load` — everything outside the scissor is already exactly
what the pass would have written there.

`child ∩ parent` and not the child's own rectangle: a plan's bounds grow by each child's
bounds **intersected with the clip the composite will apply** (`encode.rs`), so a child
clipped down to a corner has a region larger than the part of its parent it can reach. A
child that meets its parent nowhere composites to nothing, and is skipped: the clip that
shrank the parent's bounds is the same clip whose coverage the pass would multiply by, and
it is zero everywhere the child could have contributed.

`blit.wgsl` gains a source origin to make the copy, which is also the constraint that had
forced a non-isolated group to take its parent's region. Removing that constraint is *not*
part of this ADR — §11.4.4's interpolation is stated over the whole of the group's own
buffer, and shrinking it is a clause question rather than a plumbing one.

## Why it lands in two commits

The offset first, at zero, verified by equality — ADR 0028's rule, restated in
`HANDOVER.md` because a pane that taught three places to subtract an origin and shipped
with one missing drew nothing at all for every band after the first. It found something
smaller this time but of the same kind: the GPU coverage lane's resolve pass had been
borrowing the *blit's* bind-group layout for its own single sampled texture — the same
shape, never the same responsibility — and an origin the resolve knows nothing about is
what made that visible.

## What it bought

`issue16287.pdf` at 4×, a 2 448 × 9 504 page, in the bytes the frame prices before it
allocates:

| | frame bytes |
|---|---:|
| before ADR 0037 | 291 199 104 |
| after ADR 0037 | 203 188 174 |
| **after this** | **104 120 206** |

93 063 168 of what is left is the root's one texture, at the target's size because the root
is the target. **There is no second full-target texture in a frame any more.**

The count follows the same halving. A chain `n` plans deep held `2n` textures and now holds
`n + 1` — the extra one being the transient copy, which is one whatever the depth because
composites finish innermost first and each releases its copy before the next acquires one.
The artwork archetype reads 3 where it read 4; sixteen sibling groups read 4 where they
read 6.

**No pixel moves.** The caller's corpus is unchanged at both scales, per page and to the
last digit of every mean, worst tile and SSIM: 919 agree / 37 differ / 1 refused at scale
1, and 932 / 16 / 4 at scale 4.

**Time does not regress and probably improves.** The artwork archetype — eight groups at
1191 × 1684, llvmpipe, cold frame, ten runs alternating between the two builds on a quiet
machine — reads minima of **138.5 ms before and 108.9 ms after**, with spreads that
overlap. A composite scissored to a group plus a copy of that group's rectangle is less
work than one target-sized composite, and on a software rasteriser that shows. Through RADV
on the 957-page corpus it does not surface at all, because most pages have no group; the
totals move less than the run-to-run spread, so no number is claimed there.

## What it cost

**Two passes per composite where there was one.** The copy is a real pass with a real
bind group, and on a page whose groups each cover the whole target it is pure addition —
the same bytes moved, in two passes instead of one. That page exists (a full-page
transparency group is ordinary), and it is the case this trades against the common one.

**The pool is asked for a second size mid-frame.** A composite's copy is the child's size,
not the plan's, so `LayerPool` sees two sizes per level rather than one. It matched on
extent already (ADR 0036, and twelve corpus pages made it do so); this makes that matching
load-bearing rather than incidental, and `layers.rs`'s own test says so.

**`Counters::layer_textures` changes meaning slightly**, and it is a public counter: it
counted pairs doubled and now counts textures, one of which is not a plan's. Callers
comparing it against a recorded number will see it fall. Its doc comment says what it is
now and that the textures differ in size, so it is a count and never a size.

## What is left

The root's one texture is 89 % of that page's frame and it is not compressible: the frame
has to accumulate somewhere the compositor can read, and the target is not readable. What
*is* open is whether a frame with **no** internal layers at all still needs it — the flat
fast path already draws straight into the target, so this is only about a root that has
children. Sizing the root to the union of what the page marks, rather than to the target,
is the same trick ADR 0036 played on every other plan and is untried on the one plan that
was exempt from it.
