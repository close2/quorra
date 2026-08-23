# The hairline that was a pixel low — round notes, 2026-08-22

Opened to answer the caller's `QUORRA_FEEDBACK.md` §31 (our two coverage lanes place an
axis-aligned rule differently, by up to an eighth of a device pixel, on four of their corpus
pages). It did not answer that. It found something else, four steps in, and ADR 0073 is the
decision. This file is what was measured, **including the three measurements that were wrong
before they were right**, because each was wrong in a way worth not repeating.

## 1. The instrument

`crates/quorra-gpu/examples/lane_placement.rs`: one hairline, `WIDTH` device pixels thick,
swept through a whole pixel of sub-pixel position; per position and per coverage setting, the
**centroid** of its coverage along the swept axis and the **total ink**, both against the
exact values, which for a band of known width and position are arithmetic rather than another
rasterisation.

Four rows, in the order that isolates one thing at a time, and the order is the finding's
own history:

| row | what it reaches |
|---|---|
| a **stroke**, drawn once | the path lane, under both settings |
| the same rule as a **fill** | the **glyph lane** — the atlas, and therefore the quantum |
| the same fill, atlas too small | the sampled lane is *reachable* here and nowhere else |
| that again, in x | the other axis of the same |

## 2. Three wrong measurements, and what each cost

**A window that truncated its own subject.** To keep a decoy placement out of the profile the
first version summed only half the target — and the measured rule sat on the boundary, so
half of it was cut off and the instrument reported a smooth half-pixel "error" that was
entirely its own. It looked exactly like a finding. The fix is that the window discards whole
*lanes* of the profile after the sum, never pixels from inside it.

**A band built in the wrong space.** The fill row put a user-space half-width on a
device-space centre and reported a 6.3-pixel error, growing with the offset. A number that
large is a bug in the fixture; the instinct to check the fixture before the renderer is the
one that pays here.

**A sweep aliased with the quantum.** With 16 steps of 1/16, *every* sample lands on a bucket
boundary and the quantiser has nothing to do: the run reported zero error at all sixteen
positions and the defect below sat inside the buckets, untouched. `STEPS` is 37 now, and
`offsets` always appends `1 − 1/4q` as well. **This is the one that matters**, because it is
the shape a whole class of gates has: a sweep whose step divides the thing it is sweeping
measures the thing's fixed points and calls them the thing.

## 3. What the lanes actually do — the answers §31 asked for

Measured on llvmpipe, 37 positions per row, `WIDTH` = 1 device pixel, the caller's own
`0.317180616` CTM carrying the position the way a PDF's `q … cm … S … Q` does.

- **A stroked hairline is exact in both settings**, to **0.0019 device pixels** — which is a
  byte of alpha, not a placement. Both settings took the **path lane** for it.
  **The reason given here was wrong, and the caller corrected it** (their §37.4, 2026-08-23):
  this bullet said the two settings are "the same lane for a mark this size" because
  `take_gpu_lane` declines the device lane for anything `worth_caching`. That holds for a
  **solid fill** and not for a **stroke** — `Encoder::push_coverage_styled` passes
  `CacheProspect::TooLarge` at the call site, its own comment saying why ("the atlas caches
  outlines by key, not polylines"), so `worth_caching()` is `false` by construction for every
  stroke and cannot decline anything. What kept *this* hairline off the sampled grid was the
  third bullet below — the triangle floor — a different condition with a different fix. Their
  pixels settle it: §31's four pages, default lane against sampled lane, read means of
  2.5978, 1.5174, 1.1683 and 0.2539, and two settings that were the same lane would differ by
  zero.
  **The half of the sentence that survives is its converse**, and it is worth keeping: on a
  page whose marks are *cached glyph fills*, the two settings do go through the same
  rasteriser, so a page-wide lane comparison mixes marks the setting moved with marks it could
  not.
- **A filled hairline goes to the glyph lane**, where the quantum applies, and there the
  placement is quantised — bounded, after ADR 0073, at **1/32 of a device pixel**.
- **The sampled lane needs an atlas that refuses the tile** to be reached at all. Even then,
  this round did not get a hairline onto it: `take_gpu_lane`'s last condition compares the
  tile's area against its triangles' bytes, and a six-triangle band of 528 texels fails it.
  **So §31's second question — is the gpu lane's y coverage quantised, and to what — is
  not answered here.** What can be said is where to look: the condition is in
  `encode/coverage.rs`, and a fixture that reaches the grid needs a mark with fewer triangles
  per texel than a band has.

## 4. The defect, and the arithmetic behind it

At offset 0.973 the fill row read a centroid **0.973 device pixels below** where the geometry
puts it — not a bounded quantisation, a whole pixel.

```
fx=0.96   round(fx*16)=15  %16=15  phase=0.9375  error=-0.0225
fx=0.969  round(fx*16)=16  %16=0   phase=0.0     error=-0.9690
fx=0.99   round(fx*16)=16  %16=0   phase=0.0     error=-0.9900
```

`(fx · q).round()` reaches `q` for `fx ≥ 1 − 1/2q`; `% q` sent that to bucket 0 of the *same*
pixel. ADR 0073 carries it into the origin instead. Three unit tests now hold the arithmetic —
the bound over 513 positions, the carry itself in both halves, and the exact-phase case — and
`GlyphPlacement::of` had **none** before this round.

## 5. The corpus, and the gate that could not see it

**The first pair of runs looked like a broken instrument and was not one** — corrected here
on the same day it was written, because the first draft of this section said they "measured
nothing" and that is false. The `[patch]` does print `warning: patch 'quorra' was not used in
the crate graph`, which is alarming and is about the **umbrella crate the caller does not
depend on**; `Cargo.lock`'s entries for `quorra-gpu` and `quorra-scene` carry no `source =`
line, which is how a patched path dependency appears, so both runs were real. What they
measured is a genuine and useful null: **at their gate's own configuration the fix moves
nothing**, 933/22/2/17 before and after. The instrumentation's silence is the same fact from
the other side, and it is what led to the obstacle:

**`render-quorra/tests/corpus.rs` sets `glyph_quantum: None`.** Their gate runs with the
quantum *off*, on purpose — their comment says it isolates the backend's fidelity from "the
deliberate sub-1/32-pixel trade" that `real_pages.rs` gates separately. So the 974-page
instrument both projects rely on **cannot see this code path at all**, and the separate gate
that can is an envelope over mean, worst tile and SSIM, which a 3 % population of whole-pixel
misplacements moves but does not break.

Re-run in the same copy with `glyph_quantum: Some(16)` — the setting `render_quorra::options()`
actually ships, and the one thing that had to change to see anything at all — base against the
working tree, 2026-08-22:

| | agree | differ | refused | not comparable |
|---|---:|---:|---:|---:|
| base | 800 | 155 | 2 | 17 |
| with the carry | **933** | **22** | 2 | 17 |
| (quantum off, either revision) | 933 | 22 | 2 | 17 |

Per-page rather than per-total, which is this project's own rule: **133 pages moved onto the
oracle, and the set of pages that differ afterwards is a strict subset of the set before —
nothing regressed.** The 22 that remain are, name for name, the 22 that differ with the
quantum off.

The third row is the sentence worth carrying: **with the carry fixed, the quantum costs no
page its verdict.** The 133 pages were not the trade. The trade is real and is what it always
said — sub-pixel, visible in the third decimal of a mean — but the page-level cost that has
been attributed to it in both trees since the quantum landed was this defect.

## 6. What is owed next

- **§31 is still open**, and this round narrows it rather than answering it: their
  measurement was taken with the quantum off, so whatever their per-command offset is, it is
  not the quantum and not this. Question 2 (the sampled lane's y quantisation) needs a
  fixture that reaches that lane at all — see §3.
- **The caller should run at least one corpus column at the shipping quantum.** 133 pages is
  what "the gate does not measure the setting the product ships" was worth this time.
- **Nothing here touched the sampled lane**, `take_gpu_lane`, or any shader.
