# The present pass, measured as a count — round notes, 2026-08-17

`HANDOVER.md` listed one small debt against ADR 0056: *"the present pass draws a
full-screen triangle per layer rather than a transformed bounding quad. Nothing has
measured it; on a page-sized layer they are the same, and on a small chrome layer they
are not."* This is the measurement, and the decision it produced is ADR 0058.

**The seam this round was run along is ADR 0052's**: a claim about *how many* is a count
and is exact; a claim about *how fast* is a duration and this machine cannot measure one.
So the instrument that decides is a fragment count computed from the placements, and the
device timings below are context printed beside it — never the argument.

## 1. The window, and where its numbers come from

Every number is for **2048 × 2560 = 5 242 880 pixels**, which is the caller's own window:
`pdf-viewer/doc/QUORRA_NONBLOCKING_RENDER.md` §2 measures `tmp/Entwurf.pdf` "into a
1280×1600 window at a device scale of 1.6". Nothing here is a round number chosen for
being one.

The four layers of `PLAN.md`'s M9 entry — the page, the selection, the sidebar and the
modal card — are sized from the caller's own tree, and **the sizes are the finding that
shapes everything else:**

| layer | extent | where it comes from |
|---|---|---|
| page | 1568 × 2217 at (480, 171) | A4 fitted to the window less the sidebar's inset — `chrome.rs`'s `PANEL_WIDTH = 300` logical × 1.6 |
| selection | *see below* | `overlays.rs`'s `highlight_list` |
| sidebar | 480 × 2560 at the origin | the same `PANEL_WIDTH`, full height |
| modal card | 2048 × 2560 | `chrome.rs`'s `Notice::draw`, which dims **the whole window** at α 0.45 and insets a card inside it |

**The selection has no natural size, and neither does anything else the host draws in
window pixels.** `viewer-ui`'s three highlight overlays and the modal each build a
`DisplayList::new(Size::new(width, height))` — the *window's* size — because in the
arrangement they have today all of them are composed into one scene through one
`Device::render`. If those lists become layer textures unchanged, every overlay is a
window-sized texture; if a host sizes each texture to the marks it holds, they are not.
That is a decision on the host's side of the boundary, and this round measures **both
ends of it** rather than guessing which one the caller will take.

## 2. The counts. Exact — arithmetic over the placements, no adapter in them

`covered` is the number of the target's pixel centres whose inverse-mapped point lands
inside `[0, source)` — the fragments the shader's own bounds test accepts, counted the way
the shader decides it. `rectangle` is what the shipped implementation draws: the four
device corners grown outward to whole pixels and clamped to the target.

| arrangement | layers | full-screen triangle | rectangle | share |
|---|---:|---:|---:|---:|
| window-sized overlays (the caller's shape today) | 4 | 20 971 520 | 19 210 251 | **91.6 %** |
| content-sized overlays | 4 | 20 971 520 | 10 270 775 | **49.0 %** |
| content-sized overlays, no modal | 3 | 15 728 640 | 5 027 895 | **32.0 %** |
| one page layer, reprojected at 1.25× | 1 | 5 242 880 | 5 022 720 | **95.8 %** |

And the same fact from the other side — what the pass shades **to no effect** today, i.e.
fragments whose only outcome is the bounds test returning transparency and a blend that
changes nothing:

| arrangement | shaded to no effect | of the pass |
|---|---:|---:|
| window-sized overlays | 1 766 624 | 8.4 % |
| content-sized overlays | 10 711 584 | **51.1 %** |
| content-sized overlays, no modal | 10 711 584 | **68.1 %** |
| one page layer at 1.25× | 225 280 | 4.3 % |

Three things worth reading out of that table:

- **The page layer alone is 8.4 % of a four-layer present**, because a page centred in a
  window with a sidebar reaches 66.3 % of it. That is the part of the win that does not
  depend on any host decision.
- **The modal card is why the content-sized row is 49 % and not 32 %**: the caller's modal
  is a full-window dim, so it is exactly as expensive either way. A window with no modal
  up — which is nearly every window — is the third row.
- **The reprojection case, which is the one ADR 0056 exists for, wins almost nothing.** A
  page zoomed to 1.25× covers 95.7 % of the window; there is no chrome layer in it and no
  saving to have. This is the honest half, and it is why the decision needed the count of
  a whole window rather than of the motivating frame.

**What the outward pixel costs**, stated because it is the price of the construction being
a bound rather than a decision: the rectangle exceeds `covered` by 5 355 fragments on the
first arrangement and 10 839 on the second — **0.03 % and 0.11 %** of what it draws.

**A rotated placement pays for its box.** The rectangle is the axis-aligned bounding box
of the placement's parallelogram, so a layer turned 45° draws up to twice its own area —
still never more than the window, which is the bound that matters, and no measured
arrangement rotates anything. ADR 0058 records that as a cost rather than closing it.

## 3. The durations. Indicative, and here is exactly how far they can be trusted

The pass timed by the **device's own timestamp queries** (not a host wall clock), both
arrangements round-robin inside one process so drift falls on both, minima of 20 rounds
with round 0 discarded, drawing into an offscreen `Rgba8Unorm` 2048 × 2560 target.

RADV (Radeon 890M), three runs at load averages 3.0, 3.3 and 10.6:

| arrangement | triangle | rectangle | saved |
|---|---:|---:|---:|
| window-sized overlays | 1.49 – 1.58 ms | 1.44 – 1.49 ms | 0.02 – 0.09 ms |
| content-sized overlays | 0.95 – 1.63 ms | 0.74 – 0.89 ms | 0.21 – 0.74 ms |
| content-sized, no modal | 0.53 – 1.38 ms | 0.34 – 0.56 ms | 0.19 – 0.82 ms |
| one page layer at 1.25× | 0.24 – 0.54 ms | 0.24 – 0.52 ms | nothing |

llvmpipe, one run: 19.7 → 18.3, 19.3 → 14.0, 10.1 → 6.3, and **7.5 → 8.6 on the page-only
row, where the rectangle is 4 % smaller and came out 14 % slower.** That last cell is the
whole argument for not deciding on these numbers: the same configuration read 6.774 →
6.771 an hour earlier. It is noise, and a round that had run llvmpipe once and stopped
would have published it.

Three limits on all of the above, stated rather than left to be found:

- **llvmpipe is a software rasteriser**, so a per-fragment cost measured on it is not
  RADV's and must never be quoted as one. It is here because the *sign* agreeing across
  two independent implementations is worth something and the magnitude is not.
- **The target is an offscreen texture, not a swapchain image.** A surface's image may
  differ in tiling or compression, so these are the pass's shape rather than the window's
  cost.
- **Nothing here says anything about frame *rate*.** `Xvfb` reports a refresh of 0.00 on
  this machine, so 60 or 120 Hz cannot be observed here at all — ADR 0056 said the same and
  it has not changed.

## 4. The equality, which is the other half of the deliverable

Both arrangements were drawn into the same target and read back: **0 differing pixels of
5 242 880**, in all four arrangements, on **both** adapters. That is §1.7's determinism
answered by the bytes rather than by an argument about the construction — and it is the
reason the change was allowed to be an implementation detail rather than an API question.

## 5. What was built, and how each part was verified able to fail

ADR 0058 has the decision. The change is:

- `present.wgsl` — `Params` gains `bounds: vec4f` (48 → 64 bytes) and the vertex stage
  picks a corner of it per index instead of spanning clip space.
- `present/layer.rs` — `Layer::device_bounds`, in `f64`, and `Placed` so that the bound
  cannot be computed without the contract having passed first.
- `present/pass.rs` — the clip-space conversion, which is the only place in the present
  path that knows how big the target is.
- `pipeline/spec.rs` — `strip: true`; `pipeline/layouts.rs` — the uniform is visible to
  **both** stages now, which is what the warm-set tests caught within a minute of the
  first build.

| gate | forced defect | what happened |
|---|---|---|
| `examples/present_thread` (Xvfb, llvmpipe) — the page's own first row and column | the bound **eroded** by a pixel instead of dilated | `the page's left edge: the window shows [0, 0, 0], the scene says [51, 102, 204]` — and every pre-existing assertion in that example still passed, which is why the two new points were needed |
| the five `device_bounds` unit tests | the same erosion | 5 of 16 present tests failed, naming the rectangles |
| the `Params` layout gate | `bounds` and `inv1` exchanged (two fields of one width) | ``Params.inv1` is at byte 16, and the host wrote something else there` |

## 6. The instrument itself

A throwaway example — built into its own `CARGO_TARGET_DIR`, run, read, **deleted with the
round**, exactly as the corpus-ramp probe of ADR 0055 was. It held the retired full-screen
triangle inline as arrangement A and `include_str!`ed the shipped `present.wgsl` as
arrangement B, so the two could be round-robined in one process rather than across a
`git checkout`. `Cargo.toml`'s standing note that a benchmark harness does not live in
this tree is why it is not still here.

**What could not be measured from this account, and what the owner would have to run.**
Everything above is the *pass*, on a texture, off the display the caller uses. What is
still open is what it is worth inside a real frame loop, and it is one number: with the
presenter running at the display's refresh while a page renders on another thread, the
share of each refresh the present pass takes — on the display that states its own refresh,
behind the caller's ADR 0383 trace lines, at their window and their layer sizes. The
question to put to it is not "is the rectangle faster" (this round answers that in
fragments) but **"does the present pass matter at all against a refresh"** — because if it
is 0.3 ms of 8.33, then this change bought the caller a fifth of a percent and its real
value is the one in ADR 0058's decision section: that a host sizing its layers now gets
something for it.
