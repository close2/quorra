# 0045 — What an unchanged frame need not pay again

Date: 2026-08-14. Status: **proposed for (A), (B) and (C) — accepted for (D)**.

(D) is behind the existing API and is landed with this ADR. (A), (B) and (C) change what
the caller sees, and CLAUDE.md's rule is that a decision either side can make alone is a
decision neither side has made: they are written here so the sync round has something to
answer, and they are not built. Their §3 asked for exactly this
(`pdf-viewer/doc/todo/44-a-draft-that-takes-ten-seconds.md`), and the reply is in
`doc/feedback-answers-draft.md`.

## Context — two measurements, one conclusion

**Theirs.** A 49.7 MB one-page document, 58 009 display commands, 28 traced frames on an
890M. Median frame 393.1 ms, of which `encode` 233.8 and `execute` 0.5. **The display
list never changed after the first frame.** Even a frame that culls everything — 58 029
of 58 029 commands — pays 112–190 ms for `encode` to walk the list and drop it. Their
conclusion: the item is an upstream ask first.

**Ours.** `doc/PLAN.md`'s 2026-08-14 entry: recording is 78.3 % of a steady dense-text
encode by instruction count, and over 40 % of recording is a pure function of
`(scene, viewport)` — the outline bounds, the phase quantisation, the key construction,
the lane choice, the instance bytes.

Two documents, two machines, two instruments, one sentence: **an unchanged page re-pays
its whole encode on every frame.**

## The four candidates

| | what is retained | who must agree | answers their obstacle |
|---|---|---|---|
| **A** | the frame's whole `Encoded`, keyed by `(scene identity, viewport)` | a `Scene` identity accessor; a device-side cache and its invalidation | (b) partly — see the survival table |
| **B** | a sub-scene's `Encoded`, composed into a frame by placement | `quorra_scene` gains fragment composition | (a) — the overlays rebuilt every frame |
| **C** | nothing; the page scene is built in page space under a root affine | **nobody — this is already the contract** | (b), as far as it can be answered |
| **D** | one control-hull box per `(outline, linear part)`, within one encode | nobody; no public surface changes | neither, and it needs no conversation |

## What each is measured at

Both instruments are the dense-text archetype at 1191×1684 — §6.2's page shape, the same
4 320 commands and 818 outlines `tests/archetypes.rs` pins, counters checked against that
file's row before any number below was believed.

**(A), the crudest possible experiment**: a `git worktree` in which `Device::render`
holds the previous frame's `Encoded` and replays it when `(scene pointer, width, height,
the six transform coefficients)` match. Headless, RADV, into a retained `Texture`, 30–60
frames a run after two warm-ups, eight runs round-robin between the two variants so drift
falls on both, load average 3.1–3.8. **Minima**, because the medians on this machine
carry 8 ms outliers on *both* variants:

| `Device::render` | wall (min of 8 runs) | encode | upload | execute |
|---|---:|---:|---:|---:|
| re-encoded every frame | **1.538 ms** | 1.323–1.670 | 0.011–0.019 | 0.065–0.076 |
| `Encoded` replayed | **0.154 ms** | 0.000 | 0.010–0.016 | 0.062–0.074 |

**A tenfold reduction, and what is left is 0.15 ms.** The frame still uploads its instance
bytes (287 616 of them), still records and submits its pass, still executes. Nothing else
of substance remains.

The number that matters for §6.2 is arithmetic on top of that, not a measurement, and is
labelled as such: the owner's presenting dense-text frame is 2.84–3.38 ms of which
recording is 1.90–2.32. Removing the encode leaves **0.9–1.2 ms**, against §6.2's success
bar of 2.0 and clear-win bar of 0.6. A presenting run is the owner's to take.

**(D)**: callgrind, `--collect-atstart=no --toggle-collect='*steady_run*'`, on a device-free
harness (the recipe is `HANDOVER.md`'s), two warm-up encodes then one steady one:

| | Ir a steady encode |
|---|---:|
| before | 18 434 963 |
| after | **14 524 976** |

**−3 909 987 — 21.2 % of the encode, 27.6 % of recording** — with the counter row
identical to the digit. The direct computation was 5 171 040 Ir (28.1 % of the encode) for
4 320 placements of 818 outlines under one linear part; the memo costs 1 261 053 for the
same 4 320 calls. `crates/quorra-gpu/src/encode/hull.rs` carries the argument that the
memoised box is the direct box **bit for bit**, which is why no counter and no lane choice
moves.

The same instrument on the **artwork** archetype — 900 commands, 300 outlines of 24
cubics, 405 strokes, 600 commands under a curve clip — reads 621 599 548 → 620 321 847,
**−0.21 %**. That page's encode is 34× more instructions a command than dense text's and
almost all of it is 600 residue tiles being rasterised, so bounding is a fifth of a per
cent of it either way. Two page shapes, one direction: a fifth of the encode where the
brief's §0 premise holds, a wash where it does not, and no shape that loses.

## What survives which viewport change — the precise form of their obstacle (b)

Their §3 hopes that a reuse "that survives a transform change" makes a zoom step cost the
same ~60 ms. It does not, and the reason is not a limitation of any design here: the
device transform is *inside* the things a frame's encode is made of.

| the viewport changes by | what of the encode survives |
|---|---|
| nothing | **all of it** |
| its damage list only | **all of it** — `encode` never reads `Viewport::damage`; damage is planned target-side (ADR 0012), so a damaged frame reuses a full frame's encode |
| the target it draws into | **all of it** — phase 1 runs before any allocation and knows no target; a refusal is identical across targets |
| a translation of a whole number of device pixels | the atlas keys and every rasterised tile: the phase is the *fractional* part of the device translation and an integer shift does not move it. **Not** the device bounds, the cull, the clip rectangles, the scratch placements or the instance bytes — every one of those is an absolute device position |
| a translation of a fraction of a pixel | **nothing in the glyph lane.** The quantised phase changes, so every key changes and every tile is a different rasterisation (ADR 0009) |
| a scale — a zoom step | **nothing per command.** The linear part is in every atlas key, in the flattening, and in the lane choice, where a glyph past the atlas bound leaves the cache. Only the scene-level work is scale-free: the clip chain's topology and the census |
| the target's size, same transform | nothing safely: the cull and every `shape ∩ clip ∩ target` are against the target rectangle |

So (C) — building the page scene in page space under a root affine — is **already the
contract** (§2.3, and `Viewport::transform` is the affine), it needs nothing from us, and
what it buys their tree is the *`scene`* phase across a zoom step: median 50.2 ms of their
trace, 2.4 s of 17.1. It does not buy the `encode` phase, and no design can, because a
zoom step is a different rasterisation of every glyph on the page.

The reuse that *is* available is the one their trace is actually full of: **28 frames of
one document at one view**, and a frame that culls everything is the same frame twice.

## Invalidation — the whole list, because a stale encode is a plausible-looking wrong page

Principle 6 says the worst outcome either project has a name for is a wrong page that
looks right. A retained `Encoded` is a machine for producing exactly that, so the list is
enumerated rather than discovered:

- **the scene** — key on `Scene` identity, and hold a clone so the identity cannot be
  recycled. `Scene` is `Arc<SceneData>` and cloning is a refcount bump, so this is sound
  by construction: a freed scene whose allocation is reused cannot be mistaken for the
  retained one while we hold it. A caller that rebuilds an identical scene gets a *miss*,
  which costs the encode we pay today and is never wrong;
- **the viewport** — width, height and all six affine coefficients by bits. **Not** the
  damage list (see the table above);
- **`Device::set_coverage`** — it chooses which lane makes coverage bytes;
- **`Device::release`** — the retained encode names outline, image, ramp and mesh ids, and
  a released id must not be drawn from a stale instance stream. Every release clears;
  `upload_*` need not, since it only mints ids nothing retained refers to;
- **`AtlasStore::reset`** — the post-frame repack (ADR 0024) moves every tile, and the
  retained quad instances carry absolute texel origins into the atlas sheet. A reset
  clears. So does any atlas insertion that could evict, which today means any *other*
  scene encoded between two frames of this one;
- **the atlas texture's extent**, if a growth ever repacks rather than appends;
- **the scratch sheet**, which `Device::upload` today *takes* out of the `Encoded` — a
  retained encode must borrow it instead, or retain the uploaded texture.

What does not invalidate: `warm_for`, `invalidate_surface`, a surface format
renegotiation, and the target.

## Memory, priced against §11.5's dozen resident pages

A retained `Encoded` is not small and is not bounded by the page's size in the way a
`Scene` is. For the dense-text archetype it is 276 480 bytes of quad instances plus a
40-tile scratch sheet — call it a third of a megabyte. But `HANDOVER.md`'s item 5 has the
number that decides the shape of the design: three corpus pages place **194, 240 and
253 MB of coverage tiles** at 4×. **A cache of a dozen retained encodes is a quarter of a
gigabyte per page in the worst case**, which is not a cache, it is a leak with a key.

So if (A) is built it retains **one** encode — the last frame's — priced against
`max_frame_bytes` like everything else, and a page too expensive to retain simply is not
retained. The dozen-resident-pages posture stays where §11.5 put it: on `Scene`, which is
cheap, viewport-free and already what the caller holds.

## What each candidate answers, and the recommendation

**(A) is the one worth having, and it is small.** It is one field on `Device`, one key, the
invalidation list above, and a `Scene` identity accessor in `quorra_scene` — which is not a
viewport, does not make a scene know a target, and does not touch §2.3's property. It is
worth 1.4 ms on our own dense page and, by their fit, ~230 ms a frame on their document.

**(B) is the one their obstacle (a) actually needs, and it is not small.** Their frame's
scene carries a background and overlays rebuilt with fresh `Arc`s every frame, so under
(A) alone their whole frame scene is a miss every time and they get nothing. Fragment
composition — a frame as a list of `(fragment, placement)` — is new vocabulary in
`quorra_scene`, a walk that descends into fragments, and instance batches rebased by a
per-fragment offset. It should be designed with them rather than for them, and the
question to put first is whether the *host* can instead hold one `Scene` for the page and
compose the overlays as a second `render` call into the same target — which needs nothing
new at all and is worth asking before building a vocabulary.

**(C) needs nothing from us** and is theirs to take today; the survival table is what it
buys and what it does not.

**(D) is landed**, because it needed no conversation: no public item changed, the counters
are identical, the allocation is bounded by the count `encode` already checks, and the
benchmark is beside the code.

## Consequences

- A dense-text encode is 21.2 % cheaper, measured; §6.2's gap narrows by that much of
  1.90–2.32 ms and the rest of the gap is still recording.
- Nothing about (A)'s pricing is a hypothesis any more: **0.154 ms against 1.538**, and
  the argument for it in the sync round is a number rather than a plausibility.
- The claim "reuse survives a zoom step" is refuted here rather than left to be discovered
  after an API is built for it.
- `encode.rs` gains a fourth submodule and loses `outline_device_bounds`; the direct
  computation survives inside `hull.rs`'s tests as the thing the memo is compared against,
  which is the only place it should exist.

## What would overturn this

- **(D)**: a page whose outlines are each placed once — the corpus median is 1.33
  placements per distinct outline — pays the memo's hash and probe for one box each. The
  artwork run above is that case within a factor of three and still came out ahead, but a
  corpus page of thousands of distinct single-placement outlines is the shape that could
  turn the sign, and no such page has been measured through this seam.
- **(A)**: a caller who rebuilds the page scene every frame gets nothing from it. That is
  their §3's obstacle (a) exactly, and it is why (B) is on this list at all.
