# ADR 0039 — The root is as big as what the page marks

Status: accepted, 2026-08-13. One commit, because the mechanism it needs was already in
the tree: ADR 0038's blit carries a source origin, and this gives it a negative one.

## Context

ADR 0036 made every plan as big as its own bounds and wrote down one exception — "the root
*is* the target" — which sounded like a definition rather than a choice. It is a choice.
The root renders into a texture the compositor can read and the frame then hands that
texture to whatever the caller gave it; nothing requires the two to be the same size.

After ADR 0038 there was exactly one full-target texture left in a frame, and on
`issue16287.pdf` at 4× it was 93 063 168 of the page's 104 120 206 bytes — 89 %.

`HANDOVER.md` said to measure the corpus's *distribution* before building this, and warned
that most pages mark most of their area and would gain nothing. **The warning was wrong**,
and the measurement is why this ADR exists rather than a note saying it was not worth it.

## The measurement

A temporary probe printing `Region::of(root.bounds)` against the viewport, over the
caller's 974-page corpus. Only frames that allocate a root texture at all count — a flat
frame draws straight into the target and has none — and at scale 4 that is **77 frames of
970, 7.9 %**. Of those, the fraction of the target the root marks:

| | scale 1 (90 frames) | scale 4 (77 frames) |
|---|---:|---:|
| p10 | 0.080 | 0.080 |
| p25 | 0.228 | 0.146 |
| **p50** | **0.674** | **0.641** |
| p75 | 0.985 | 1.000 |
| p90 | 1.000 | 1.000 |
| mark the whole target | 26 % | 26 % |

So a quarter of them gain nothing and the median gains a third — and the tail is where the
bytes are. `issue16287.pdf` at 4×, the heaviest layered frame in the whole corpus, marks
**2 224 × 875 of a 2 448 × 9 504 target: 8.4 %**.

## Decision

**The root is sized like every other plan**, to `Region::of(root.bounds)`. The exception
that remains is the one ADR 0038 also left: a seeded non-isolated group takes its parent's
region, because §11.4.4's interpolation is stated over the whole of the group's buffer.

The hand-off is the part that had to be got right, and it is the one copy in the frame
whose **destination is larger than its source**. `blit.wgsl` reads at `p + origin` with
`origin = −root.origin`, and outside the root's extent it writes **`vec4(0)`** — not stale
bytes, not the nearest edge texel:

- a page is rendered onto transparency (§3), so transparency is what a full redraw leaves
  where the page marked nothing;
- and under a damage patch the load op is `Load`, so writing nothing there would keep the
  **previous frame's** pixels inside a rectangle the contract says must equal a full
  redraw. That is the stale-page failure §5 is named after, and it would have appeared
  only on a patched frame over a page that shrank.

## What it bought

`issue16287.pdf` at 4×, priced before anything is allocated:

| | frame bytes |
|---|---:|
| ADR 0036's "what is left" | 291 199 104 |
| after ADR 0037, masks sized | 203 188 174 |
| after ADR 0038, one texture per plan | 104 120 206 |
| **after this** | **6 158 496** |

Across every layered frame of the corpus at scale 4, the priced total falls from
**2 259.2 MB to 1 325.5 MB, 41.3 %**. The heaviest frame is no longer that page but a
2 448 × 3 168 one at 93.0 MB whose root marks the whole target — where there is nothing
left to take.

The corpus is unchanged at both scales, per page and to the last digit of every mean,
worst tile and SSIM: 919 agree / 37 differ / 1 refused at scale 1, 932 / 16 / 4 at scale 4.

## What it cost

**`Device::warm_for` warms a size the root usually is not.** ADR 0035 measured it taking a
first frame from 24.7 ms to 10.3 by making a target-sized layer texture ahead of the frame;
ADR 0036 made that the right size only for the root, and this makes it the right size for
the root only when the page marks its whole area — a quarter of layered frames. It is a
hint and nothing depends on it (its shipped test asserts that a device warmed for this
size, another, or none draws the same bytes), and the 91 % of frames that are flat never
wanted it. But the number in ADR 0035 is a number about a page whose root filled the
target, and it should not be quoted for one whose root does not. *(It should not be quoted
at all: ADR 0040 re-measured it, including on a page whose root does fill the target, and
priced the allocation at 0.06 ms.)*

**The hand-off gained a branch per pixel of the target**, and the target is the biggest
thing a frame touches. It replaces nothing — the old blit was an unconditional
`textureLoad` — so this is real per-frame work added to every layered frame. Against it:
the root's clear and every pass into it now cover what the page marks rather than the page.
On the corpus at scale 1 through RADV nothing surfaced above the run-to-run spread, which
is the honest statement rather than a claim either way.

**A test's arithmetic got much smaller, and that is a signal worth keeping.** `m1`'s budget
refusal now prices a group in the corner of a 64 × 64 target at **768 bytes** where it once
priced 65 536: three 8 × 8 textures. A frame that is refused is refused for what it needs,
and what it needs is now what it marks.
