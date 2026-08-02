# Plan

The brief is `RENDER_LIBRARY.md`; this file is the order of work and the state of it.
Section numbers below are that document's.

## Where we are

**M0, the skeleton, is done.** Workspace, tooling, lints, licence, supply-chain policy,
the module map of all three crates, and the two decisions worth an ADR so far. No
rendering is implemented, and nothing in the tree pretends to be: a module that will
hold code today holds the contract that code has to satisfy, and `doc/PLAN.md` is where
the order lives.

Nothing here has been measured by us yet. Every number quoted in the brief was measured
in the caller's tree, on this machine's GPU, and is repeated in the milestones below as
a target to beat rather than as a fact about our code.

## How the order was chosen

Not by intuition, and not from the top of the brief. §6.1 measured the current
Vello-based backend and the result reversed the obvious plan: **between 55% and 92% of
an offscreen frame is paid before any of the page is drawn**, scene encoding is flat in
resolution at 1.1–1.6 ms, and 5 933 glyph fills cost about what one rectangle costs.
The brief's own ranking follows from those numbers, and this plan follows the ranking:

> the surface and texture target paths first, because they delete the largest single
> item; then whatever makes the per-pixel floor cheaper for a target that is mostly
> untouched; then the atlas and the rectangle path; then the retained scene; then
> damage.

Two consequences worth stating, because they are easy to get backwards:

- **The first milestone is a measurement, not a feature.** §11's first question — how
  much of that fixed cost is the readback — cannot be answered with a wall clock, and
  the answer decides whether the atlas is a headline or a second-order effect. It needs
  timestamp queries, which means it needs a device, which is why M1 is a device and a
  rectangle and nothing else.
- **Correctness work is not deferred to the end.** §4 is the reason this library exists
  at all; what is deferred is *breadth*. The knockout group's diagonal edge, the sixteen
  blend modes and a full page at a real window size are the three scenes that will find
  bugs on day one (§10), so each lands with the milestone that first makes it possible
  rather than in a conformance push afterwards.

## Milestones

### M1 — A device, a rectangle, and the measurement that settles §11.1

Deliverable: `Device::headless`, `Device::for_surface`, all three targets of §2.4, one
analytically-covered axis-aligned rectangle, `Timings` with real timestamp queries, and
a `Counters`. Nothing else.

- Answers §11.1: the split between execute and readback, per target kind, per
  resolution. Until this number exists the rest of the design is guesswork.
- Establishes the per-pixel floor for a target that is almost entirely untouched —
  which is what the last column of §6.1's table measures, and what M3 has to improve.
- Establishes the startup number of §7, split into adapter enumeration, device creation
  and pipeline compilation, and gates it in CI from the first commit that has one.
- Establishes that a `Frame` tells the truth about itself (§8, principle 6).

Not in M1: any path, any glyph, any group. A rectangle is the primitive that needs no
tiling, no binning and no edge list, so it isolates the floor from everything else.

### M2 — The scene, retained and viewport-free

Deliverable: `quorra-scene`'s real types, `SceneBuilder`, `Scene: Send + Sync`, the
resource ids and the upload/release path of §2.2, `Scene::cost()`.

- §2.3's property is the one the crate split already enforces structurally (ADR 0001);
  M2 is where it becomes true of the *data* as well: no viewport, no resolution, no
  device transform, no target size anywhere in a `Scene`.
- 107 outlines uploaded once and referenced 5 933 times, keyed by the caller's
  `Arc::as_ptr` identity. A zoom re-uploads nothing.
- Byte equality across repeated encodes of one scene (§4.6) starts being gated here.

### M3 — Rectangles and rectangular clips, analytically

Deliverable: `SceneBuilder::rect`, clip chains, and the rectangle fast path in the
fragment stage. Empty clip admits nothing; a chain is an intersection; the caller's
worst page holds 3 608 chains and its page 6 collapses 303 identical clip states into
one identifier.

- §6.4: a rectangular clip becomes four floats and a comparison, never an R8 mask.
- Instrument the count of distinct clip regions, not the hit rate (§6.3's lesson).

### M4 — The glyph atlas

Deliverable: an R8 coverage atlas with eviction and a settable budget, keyed on
`(outline, scale bucket, sub-pixel phase)` with the **quantum settable and documented**
— default 1/16 of a pixel, off if asked. 1/16 reused 5.0× on a dense page and left the
oracle's verdicts unmoved; 1/8 contradicted pages.

- Answers §11.3: what the atlas costs on a page it cannot help. `tracemonkey.pdf`
  reuses 1.3×, and a cache that is 5× on one page and a net loss on another is a
  decision, not a feature.
- Reports `atlas_distinct_keys`, not a hit rate.
- Open design question, to be decided with a measurement and an ADR: whether the atlas
  is rasterised on the GPU or on the CPU by `tiny-skia` — the latter makes the glyphs
  come from the same code that is the caller's correctness oracle, which is a
  correctness argument no other arrangement gets for free, and a dependency and a
  transfer this library would otherwise not have.

### M5 — General path coverage

Deliverable: fills and strokes of arbitrary cubic outlines, both fill rules, nested
subpaths wound the same way. The caller pre-splits degenerate subpaths and does its own
dashing, including zero-length dashes whose caps face along the path; our job is not to
undo that.

- Answers §11.2: does a document renderer want tiles at all for the glyph path, or is
  the general binned path reached by a small minority of commands? Measure the minority
  on the caller's corpus *before* designing for it.
- No quadratics anywhere: PDF has no quadratic operator and TrueType outlines are
  elevated upstream, so one curve type reaches us.

### M6 — Clause 11, natively

Deliverable: the four things a general vector API lacks, and the reason this library
exists.

1. **Coverage-modulated Porter-Duff Source** (§4.1) — shape is not opacity. The scene
   that tests it has a diagonal edge on purpose, because axis-aligned rectangles would
   agree while being wrong.
2. **Soft masks reduced on the device** (§4.2) — `Alpha`, `Luminosity { backdrop }`,
   and an optional 256-entry `/TR` table, with the group built through the same
   `SceneBuilder`. No readback, no round trip. Our shader becomes a second
   implementation of the caller's `SoftMask::value` and must agree with it to the byte;
   that conformance test is part of the deliverable, not a follow-up.
3. **Sixteen blend modes** (§4.3), the four non-separable ones written from the clause's
   `Lum`, `ClipColor`, `SetLum` and `SetSat` — our own implementation, deliberately not
   shared with the caller's, because a shared one would make the cross-backend
   comparison compare an implementation with itself.
4. **Groups that composite onto transparency** (§4.4), isolated, painted once under the
   group's own alpha and blend mode. Depth bounded at 16.

### M7 — Shadings, meshes, images

Axial, radial and function-based shadings from a `RampId`; `MeshRaster` as a
pre-rasterised triangle mesh the caller hands us; decoded RGBA8 images with the filter
decision *already made upstream* (see the integration note below). Straight alpha at
the boundary, premultiplied internally, rendered onto transparency, always.

### M8 — Damage, and the rest of the performance contract

Per-tile dirty tracking against a retained scene and a cheap viewport, so a caret blink
redraws tiles rather than a page. Persisted pipeline cache. Answers §11.5: what a
`Scene` costs to hold, against the target of a dozen resident pages.

### M9 — The swap

Replace the caller's `render-gpu`. Held to §10: the cross-backend scene suite, the
1 794-page oracle, byte equality where we claim it, and a window driven by real key
presses under `Xvfb`. Not before: this is the last milestone, and the two trees stay
independent until it.

## What we must build ourselves, and when

§10 tells us what we will be judged by, which means the harness is ours to write rather
than to wait for:

| | lands with |
|---|---|
| Headless golden renders, PNG artefacts, byte-equality gate | M1 |
| A scene suite that starts small and includes **one full page at a real window size** — a suite of small scenes tests small scenes, and the first real page at a real size came back blank | M3 |
| The knockout group with its diagonal edge; the sixteen-mode scene; the soft-mask agreement test | M6 |
| Adapter-to-adapter byte equality, RADV against lavapipe (§11.4) | M1, re-checked every milestone |
| Fuzzing the scene boundary (principle 3) | M2 |
| Perf gates: device creation, encode, execute, readback, one real page at a window size | M1 |

## Integration notes against the caller — settle these before M2 freezes the API

These are places where the brief and `pdf-render` as it stands today do not line up
exactly. None is a problem; each is a question whose answer belongs in the API rather
than in a translation layer's judgement (§4.5: a decision either side can make alone is
a decision neither side has made).

1. **The image filter flags are not flags.** §4.5 names `Image::is_smoothed` and
   `Image::area_averaged` as flags to honour. In the tree they are *methods taking the
   placement transform* — `is_smoothed(placement)`, `area_averaged(placement) ->
   Option<Image>`. So what must reach us is the **resolved** decision for the placement
   the command carries, not the flag it was derived from. Our `ImageSpec` therefore
   states the filter, and the caller answers the question; if we took the flag we would
   be re-deciding it, on data we do not have.
2. **`Viewport` versus `TargetSpec`.** §2.4's viewport carries a full affine — scale,
   y-flip and tile offset. The trait in the tree carries a scale and a pixel budget
   (`TargetSpec::for_page(list, scale, max_pixels)`). The affine is the more general
   of the two and is what tiled output and damage need; the bridging belongs in the
   caller's adapter, and the rounding rule for a fractional page stays theirs.
3. **The soft-mask transfer function.** The brief says a 256-entry lookup table because
   a mask value is one byte. The tree has a `Transfer` type; we take `[u8; 256]` and
   the caller samples. Confirm the sampling convention — inclusive of both endpoints —
   in the conformance test, not in prose.
4. **Which `raw-window-handle`.** §2.1 asks for `raw-window-handle` and nothing more
   specific, but the version has to be the one `wgpu` was built against or a surface
   cannot be created. Reach it through a `wgpu` re-export if one exists in 30.0.0;
   otherwise pin the same version here and say so.
5. **`MeshRaster` is shared on purpose.** Both of the caller's backends share the
   pre-rasterised mesh because neither rasteriser has the primitive and a second copy
   would drift. We inherit that: we consume the mesh, we do not re-triangulate it.
6. **Colour is not ours.** Device RGB arrives; `ColourSpace::to_rgb` upstream is the
   only place a colour becomes RGB, and adding a second one is forbidden. If we offer
   colour management the caller cannot use us.

## The five questions of §11, and where each is answered

| | |
|---|---|
| 1. How much of the fixed cost is the readback? | M1, with timestamp queries |
| 2. Does the glyph path want tiles at all? | M5, measured on the corpus before designing |
| 3. What does the atlas cost on a page it cannot help? | M4 |
| 4. Is byte-identical output across adapters achievable? | M1, and re-checked every milestone — the answer changes how CI works, so it is not allowed to arrive late |
| 5. What does a `Scene` cost to hold? | M2 for the number, M8 for the dozen-page target |

## Not planned, ever

§9's non-goals: colour management, font loading, shaping, hinting, text layout, bidi,
filters and effects beyond §4, any document format, a scene graph or animation, hit
testing. `deny.toml` enforces as much of the list as a dependency policy can express.
