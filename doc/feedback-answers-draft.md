# Draft answers for the caller's `QUORRA_FEEDBACK.md`

**This is a draft for the owner, not a document either project publishes.** It is written
so that it can be carried, section by section, into the conversation in
`/home/cl/projects/pdf-viewer/doc/QUORRA_FEEDBACK.md` — a tree this side never edits. The
three sections it answers are the three that document lists as waiting on us: **§15**,
**§19** and **§22.5**. A fourth answer follows them, to the upstream ask their
`doc/todo/44` §3 raises — **the encode cache** — which is not in `QUORRA_FEEDBACK.md` yet
and is the largest thing either side is holding. It ends with what the pending push
delivers, so it doubles as the release note for the sync round.

Everything below was produced on 2026-08-14, and the line numbers and figures were brought
up to `7896874` — the end of that day's ten rounds — at the close of it.

---

## §15 — the coverage lane already bounds a clipped fill by its clip

**Yes, and the section can close as *already handled*.** It has been true since the lane
was written, which is why nothing in the release notes ever announced it.

Every coverage tile this device places is `shape ∩ clip ∩ target`, and the intersection
happens *before* anything is rasterised rather than after. The evidence, in the order a
reader should take it:

- `crates/quorra-gpu/src/encode.rs:1470` — `visible_tile`, whose doc comment is the claim
  verbatim: *"The tile a shape with these device bounds occupies: shape ∩ clip ∩ target,
  rounded out to whole pixels."* It intersects the shape's device bounds with
  `resolved.rect` and with the viewport, and returns `None` — draws nothing, legitimately
  — when the result is empty.
- `crates/quorra-gpu/src/encode.rs:1415` — `coverage_tile`, the rasterising sibling, which
  does that same arithmetic and then rasterises **only the surviving rectangle**. A page
  rectangle admitted by a 24-pixel clip rasterises 24 pixels here, not a page.
- `crates/quorra-gpu/src/encode/clips.rs:146` — `residue_intersection`, which handles the
  part a rectangle cannot express: when the clip chain has a non-rectangular link, its
  coverage is rasterised over *the tile that survived the rectangular intersection* and
  multiplied in. So a residue clip narrows the tile further; it never widens it.
- `crates/quorra-gpu/src/encode/rare.rs:42` — the same rule stated for the image lane
  (`ImageOp::dest`: *"The quad drawn: footprint ∩ clip ∩ target, at pixel bounds"*), and at
  `:87` for the shading and mesh lane. It is the whole device's rule, not the path lane's.

So for `bug1721218_reduced.pdf`'s 3 490 page-sized `sh` rectangles under 24-pixel clips:
each one costs us a clipped tile, not a page. **Calling `pdf_render::cropped_rectangle`
before handing the scene over would change nothing on this side** — the geometry it would
shrink is geometry we already intersect.

### The secondary ask — "can the scene say *this fill is bounded by this rectangle*" —
### is answered by the same fact

It can, and it already does: **the clip is the bound.** A `ClipId` whose outline is an
axis-aligned rectangle collapses, at
`crates/quorra-gpu/src/encode/clips.rs:96`, to a device-space rectangle and nothing else —
`StoredOutline::rect_hint` is computed once at upload (`resources.rs:160`,
`quorra_scene::axis_aligned_rect`) and a rectangular clip under an axis-preserving
transform never becomes a mask, a texture or a residue. It intersects into
`ResolvedClip::rect` and is carried by every lane. That is exactly the "state the bound
instead of shrinking the geometry" shape §15 says is better, and it needs no new
vocabulary — it is what a rectangular clip *is* here.

The one thing worth saying plainly: this is true because the clip is rectangular, not
because it is a clip. A non-rectangular clip becomes a residue, and a residue is
rasterised.

### The cross-reference §15 should carry: the tile is bounded, but the *bound* can still be a page

This is the honest other half, and it is our finding rather than a hedge. §15 asks whether
a clipped fill is bounded by its clip. It is. But **the coverage tile of a page-sized shape
under a page-sized clip is still page-sized**, and when the clip is a residue that tile is
re-rasterised every frame — a clipped tile cannot enter the glyph atlas (the clip
multiplies into it, so caching it would poison every other placement of the shape) and the
scratch sheet is per-frame.

Measured from our side on 2026-08-14, on the corpus's p99 clip shape (`doc/PLAN.md`'s entry
for the date, `examples/surface_measure.rs`, RADV presenting to a real surface, minima over
80 frames):

- **artwork archetype: 43.3–61.4 ms steady per frame**, of which encode is 39.6–57.3 and
  **geometry alone is 35.4–47.4 ms** — 600 residue-clipped commands re-rasterising their
  coverage, every frame;
- against the dense text page's 1.8–3.4 ms total on the same instrument. Twenty times the
  cost, on a shape about 5 % of documents have — and the ratio grew rather than shrank
  over the day, because everything that made the text page faster was in recording and
  this page's cost is not.

It is the same seam as the three pages this device still refuses with `ScratchExhausted`
(`bug1703683_page2_reduced`, `issue1905`, `bug1721218_reduced` — 194 to 253 MB of coverage
placed against a 268 MiB budget before they run out of sheet height), and it is
`doc/HANDOVER.md`'s item 5. So: **§15's question is answered yes, and the thing behind it
that costs real milliseconds is on our list with a number beside it.** If the viewer wants
one thing from us on this seam, it is that number moving, not a bound we already apply.

---

## §19 — what a `Command::Rect` costs against a `Fill` of the same four-edge outline

The instrument is `crates/quorra-gpu/examples/rect_lane.rs`, added this round. It builds
three scenes that draw the identical set of device rectangles on the caller's 1 191 × 1 684
window and times `Timings::encode` **round-robin**, one frame of each per round, reporting
minima — because this machine is somebody's desktop and ADR 0040 is the price of believing
a wall clock that was not measured that way. Contiguous per-scene blocks put a factor of
three between two runs of this example at load 85; round-robin holds the ratios to a few
per cent.

The three scenes:

- **`rect`** — `SceneBuilder::rect`, the analytic lane of ADR 0007
  (`encode.rs:938`, `encode_rect`): intersect with the clip, intersect with the target,
  push one instance. Nothing is rasterised and no tile is placed.
- **`fill (shared)`** — one uploaded unit-square outline placed by scale-and-translate.
  The friendliest case the fill lanes have: one shape at many placements is exactly what
  the glyph atlas and the census exist for.
- **`fill (distinct)`** — one uploaded outline per rectangle, geometry baked, placed by a
  translation. What a page of varied rules, underlines and table cells actually is.

### The numbers

Load average **2.8–4.5** across the five runs quoted, checked before and after each. (The
same binary was also run at load 40–95 earlier; those runs are excluded and are on record
only as the reason the round-robin exists — they disagreed with each other by a factor of
three, and with these by rather less once the drift was spread evenly.) Minima in
milliseconds, ratio to the `rect` lane in the same run.

**RADV (AMD Radeon 890M, `RADV STRIX1`), headless:**

| commands | `rect` | `fill (shared)` | `fill (distinct)` |
|---:|---:|---:|---:|
| 12 (median page) | 0.0006–0.0012 | 0.0023–0.0049 (**3.3–4.1×**) | 0.0026–0.0053 (**3.7–4.4×**) |
| 4 320 (p99 dense) | 0.083–0.103 | 0.679–0.804 (**7.7–8.4×**) | 0.991–1.438 (**11.4–14.0×**) |

**llvmpipe (LLVM 22.1.8), headless:**

| commands | `rect` | `fill (shared)` | `fill (distinct)` |
|---:|---:|---:|---:|
| 12 (median page) | 0.0010–0.0013 | 0.0030–0.0041 (**2.7–3.5×**) | 0.0033–0.0047 (**3.3–3.9×**) |
| 4 320 (p99 dense) | 0.100–0.110 | 0.791–0.916 (**7.7–8.6×**) | 1.855–2.203 (**18.0–21.0×**) |

The ratios are the stable part and they agree across the two adapters everywhere except
the last row. That exception is worth naming rather than smoothing over: `encode` is host
work running byte-identical code on both, so an adapter has no business changing it — what
changes it is that llvmpipe is rasterising the previous frame on the same cores and caches
between our measurements. Which is `HANDOVER.md`'s oldest rule arriving in a new place:
**never publish a crossover as a constant.** Read the ratio, not the millisecond, and
re-measure on your own machine before betting a morning on it.

### §19's second condition, checked rather than assumed

The example reads all three scenes back through `Target::Readback` and compares bytes.

> **0 bytes differ, out of 8 022 576, in every cell of both tables.**

So the `Rect` lane and a `Fill` of the same four axis-aligned edges are *exactly* the same
mark — measured, at both sizes, on both adapters, for both fill variants. That is the
guarantee §19's condition 2 asks for, and it is now a property an example prints rather
than a claim either side is making.

(The condition it does **not** cover is the one §19 names itself: a fill of an `re` path
under a transform that is not axis-aligned is not a rectangle, and any recogniser has to
say so. Our own lanes ask `transform_preserves_axes` before taking any analytic path —
`encode.rs:949` for the rectangle, `encode/clips.rs:96` for a rectangular clip — and fall through to the
path lane when it is false. A recogniser on your side needs the same question, on the
composed device transform rather than on the outline.)

### What the number means, stated as a per-rectangle cost

The ratio is the wrong quantity to carry across the boundary, because it is a ratio of two
things that both scale with the command count. The transferable number is the difference
divided by the count:

> **A rectangle costs roughly 0.13–0.19 µs more as a `Fill` than as a `Rect` when it is one
> shape at many placements, and 0.21–0.49 µs more when each is its own outline.**

Multiply that by the number of rectangles on the page. Which gives the answer §19 wanted,
and it is two answers, not one:

- **For the median page — 12 commands — it is nothing.** Two to four *microseconds* over
  the whole page, even if every one of the twelve were a rectangle. No recognition pass can
  be worth writing for that, and the recognition itself would eat it.
- **For a page whose commands really are thousands of rectangles, it is 0.6 to 2.1 ms
  per frame.** For scale: `doc/PLAN.md`'s closing 2026-08-14 entry measures a whole steady
  presenting dense-text frame at **1.816 ms**, of which per-command recording is 1.130. So
  on such a page the `Rect` lane is worth between a third of the frame and more than the
  whole of one — the day's encode work made this saving a *larger* share, not a smaller.

**The honest reading is that neither side yet knows whether such a page is in the corpus.**
Your profile counts commands per page; what decides this is *how many of them are
rectangular fills*, and that row does not exist. Before writing a recogniser, count that —
it is a one-line addition to the walk that produced `doc/corpus-profile.md`, and it turns
this from a decision into an arithmetic. If the p99 page has a dozen rectangles, §19 closes
as *already handled* like §15. If it has four thousand, it is worth a morning on your side.

### And one thing that is ours, which this measurement found

**The recogniser exists here already, and it is wired to the wrong half of the fill path.**
`StoredOutline::rect_hint` is computed for every outline at upload
(`resources.rs:160`), and `encode.rs:1086` uses it to send a *shaded* fill of a rectangular
outline down the analytic lane with no scratch tile at all. But a **solid** fill returns at
`encode.rs:1071` into `fill_solid` (`:1104`) before that check is ever reached, and takes
the GPU triangle lane, the glyph atlas or the scratch coverage path like any other shape.
The counters in the table above show it: the `fill` rows report 12, 280 and 4 320 distinct
atlas keys where the `rect` rows report none.

That is a defect of symmetry rather than of correctness — the output is byte-identical, as
the readback check says — but it means **most of the saving §19 is asking you to buy with a
translation-side recogniser, we can deliver with four lines on ours**, and then no scene
vocabulary changes, no `Command::Rect` is ever emitted, and the corpus keeps handing us
outlines. It has its own entry on our list now. The remaining difference after that would
be the outline upload and the scene's own bytes, which is smaller than either column above.

**So the recommendation is: do nothing on your side yet.** Count the rectangular fills
first; we will take the solid-fill rect lane regardless, because it is ours and it is
cheap, and then the number this section is about will have moved and want re-running.

---

## §22.5 — a counter whose meaning changed while its name did not

Taking the process note seriously, because it is the right note.

**The recommendation is *document*, not rename — but with a rule attached, because
"document" on its own is the answer that lets this happen again.**

The reasoning in one sentence: `layer_textures` has always meant *layer textures alive at
one instant* and still does, so the name was never the thing that was wrong; what was wrong
was a **derived claim** our own rustdoc made about it — that it is "the number
`max_frame_bytes` is spent on" — which was true by accident while every layer texture was
target-sized, and which your `QUORRA_NON_ISOLATED_GROUPS.md` inherited from us in good
faith.

That sentence is gone as of this round, and the field's rustdoc
(`crates/quorra-gpu/src/frame.rs`) now states the unit first, says what stopped being
derivable from it and when, and cross-references `layers_culled` (ADR 0041), which moves in
the opposite direction on the same page. Your two sentences of prose were downstream of our
one sentence of rustdoc; correcting ours is the fix that reaches the cause.

The rule we will hold ourselves to, so this is a policy rather than one correction:

- **A rename is owed when the *unit* changes** — when a caller's existing arithmetic on the
  field becomes wrong rather than merely unhelpful. A compile error is the only instrument
  that reliably reaches a reader, and one line of migration is a fair price for it.
- **A dated correction is owed when a *derived claim* changes** and the field still means
  what its name says. Renaming a correct name to force attention is theatre, and the next
  change to the same subsystem would owe another one.
- **An ADR that moves a counter's value names the counter**, in as many words, so the
  meaning-change is visible in the diff of decisions rather than only in the diff of code.
  ADRs 0036 and 0038 changed what `layer_textures` reports and neither says the word; that
  is the actual gap, and it is the one worth closing.

**And if you would rather have the rename anyway, say so and we will take it** — it is one
line here and one line there. This is a judgement about which instrument reaches a reader,
not a principle, and you are the reader.

---

## `doc/todo/44` §3 — the encode cache: priced from this side, and one of your two obstacles is already answered

**Yes, and here is what it is worth measured rather than fitted.** Our own profiling had
arrived at your sentence from the other end a day earlier (`doc/PLAN.md`, 2026-08-14):
recording is **78 %** of a steady dense-text encode by instruction count, and **over 40 %
of recording is a pure function of `(scene, viewport)`**. Your trace says the same thing
about a document 13× larger. The whole design, its pricing, its invalidation list and what
it costs in memory are `doc/adr/0045`; this is the part you need to decide something.

### What a reused frame costs, measured

A `git worktree` in which `Device::render` holds the previous frame's `Encoded` and
replays it when the scene pointer and all eight viewport numbers match. Our dense-text
archetype at 1191×1684 — 4 320 commands, 818 outlines — headless on RADV into a retained
`Texture`, eight runs round-robin between the variants, **minima** (the medians on this
machine carry 8 ms outliers on both variants and are not evidence of anything):

| `Device::render` | wall | encode | upload | execute |
|---|---:|---:|---:|---:|
| re-encoded every frame | 1.538 ms | 1.32–1.67 | 0.011–0.019 | 0.065–0.076 |
| `Encoded` replayed | **0.154 ms** | 0.000 | 0.010–0.016 | 0.062–0.074 |

**Tenfold, and 0.15 ms is what is left**: the instance upload, the pass, the submit. On
your document, at your own fit of 3.86 µs a command, that is the 233.8 ms median `encode`
going to approximately nothing, and your fully-culled frames' 112–190 ms with it.

### Your obstacle (b) is already the contract, and it does not buy what you hope

**Build the page scene in page space and put the scale in `Viewport::transform`** — you
need nothing from us to do that, and it is what §2.3 of the brief asks for in the first
place: *"a `Scene` must contain no reference to a viewport, a resolution, a device
transform, or a target size."* A `Viewport` already takes a full affine, not a scale.

What it buys is your **`scene` phase** — median 50.2 ms, 2.4 s of your trace's 17.1 —
across zoom steps and window resizes. What it does **not** buy is the `encode` phase, and
this is the part of your §3 we have to correct rather than confirm:

| your viewport changes by | what of our encode survives |
|---|---|
| nothing | **all of it** |
| the damage list only | **all of it** — `encode` never reads it; damage is planned target-side |
| the target it draws into | **all of it** — phase 1 runs before any allocation and knows no target |
| a whole number of device pixels of scroll | the atlas keys and the rasterised tiles; **not** the bounds, the culls, the clip rectangles or the instance bytes, all of which are absolute device positions |
| a *fraction* of a pixel of scroll | **nothing in the glyph lane** — the quantised sub-pixel phase changes, so every key changes |
| a scale, i.e. a zoom step | **nothing per command** — the linear part is inside every atlas key, the flattening and the lane choice |

So "under reuse that survives a transform change it is the same ~60 ms" is not available
at any price: a zoom step is a genuinely different rasterisation of every glyph on the
page. Scroll by whole pixels and re-encode; zoom and re-encode. What reuse takes is the
case your trace is actually full of — **28 frames of one document at one view**.

### Your obstacle (a) is the real one, and it decides what we build

Under a device-side cache keyed on scene identity, a host that rebuilds the frame's scene
every frame with fresh `Arc`s for its background and overlays **misses every time and gets
nothing**. So before either side builds scene-fragment composition — new vocabulary in
`quorra_scene`, a walk that descends into fragments, batches rebased per fragment — the
cheaper question, and the one we would like answered in this round:

**Can the host draw the page and the overlays as two `render` calls into the same target?**
The page's `Scene` would then be stable across frames and hit the cache; the overlays are a
handful of commands and would cost their own encode, which is microseconds at that size.
Nothing new is needed on either side. If there is a reason that does not work — a blend
that must see the page beneath it inside one transparency group, an overlay that must be
clipped by page geometry — that reason is the specification for fragment composition, and
we would rather design it from that than from the general shape.

### What is already landed, and needs nothing from you

The half of this that is behind our API is taken: the device box of a glyph placement is
now its neighbour's box translated, memoised per `(outline, linear part)` within one
encode. **−21.2 % of a dense-text encode by instruction count** (callgrind: 18 434 963 →
14 524 976), the counter row identical to the digit, and a proof beside the code that the
memoised box is the direct box bit for bit. It is in the push below.

---

## What the pending push delivers

Twenty-four commits past the `a7babab` your `Cargo.lock` pins, and forty-one past
`a35dc70`, which is the last revision the owner pushed. The four that change something you
can see, and one thing to re-run:

- **`d594566` — the §21.1 round cap.** Your prediction in §22.7 was right, and the effect
  is worse than the reading: the far cap is a correct outward semicircle and the near cap
  is the **inward** one wound *against* the body it lies inside, so under §8.5.3.3.2's
  non-zero rule the two cancel and a **hole** is punched where a cap belongs. Both ends
  wrong, equal and opposite, invisible to any instrument that sums ink — which is why your
  measurement saw a round cap depositing exactly what a butt cap does. Against Table 53's
  own arithmetic: **−8.9 % becomes −0.1 %** on a 40 × 5 rule and **−74 % becomes −1.7 %**
  on a 0.15 × 0.5 one. On your corpus at scale 1, **919/37 becomes 921/35**:
  `extgstate.pdf` and `inks_basic.pdf` join the oracle, and `bug1743245.pdf`'s mean
  deviation falls. **§21.3's held row for `render-quorra/tests/sub_pixel_coverage.rs` can
  be written after the bump** — that is the thing to re-run first.
- **ADR 0043 — the warm set learns the surface's format.** Every presenting first frame was
  paying one `pipeline compile (first use)` of 0.3–1.0 ms, because the warm set compiled
  for `Rgba8Unorm` and a surface negotiates `Bgra8Unorm`. The presenting lanes are now
  warmed keyed by the negotiated format, held by two headless unit tests, and the owner's
  re-run on the real display reads **compiles: none, on eight presenting first frames of
  eight.** `Composite` is deliberately excluded — since ADR 0038's hand-off its target is
  always an internal accumulator, never the surface.
- **§21.2 — a tiny outline no longer flattens to its inscribed polygon** (ADR 0044). Your
  report was right and the mechanism is exactly the one you named: `FLATTEN_TOLERANCE` is a
  quarter of a device pixel, and a curve whose whole extent is a pixel meets that bound with
  four chords — the inscribed square. A cubic's bound is now the tighter of that tolerance
  and 1/32 of the cubic's own device extent, which floors it at 16 chords a full turn. On
  your corpus, one copy, the same hour, flipping only the `[patch]`: **at scale 1, 919/35
  becomes 934/20; at scale 4, 935/11 becomes 936/10. Sixteen pages moved onto your oracle
  and none moved off.** Most of them are prose rather than the sub-pixel dots the report is
  about — a bowl at body size is a cubic two to five device pixels across — so **§21.3's
  deferred gate is worth writing after the bump**. One correction to carry with it: **the
  clause you cite is not the one that governs.** §10.7.3 is *smoothness*, a shading's colour
  error; flatness is §10.7.2, and it is the stronger citation for you — it licenses a device
  tolerance outright ("PDF processors may choose to ignore any flatness tolerance specified
  within a PDF file") and its own NOTE 2 says where that licence stops: "the purpose of the
  flatness tolerance is to control the precision of curve rendering, **not to draw inscribed
  polygons**".
- **§6.2 measured in its own terms at last, and met** (`doc/PLAN.md`, 2026-08-14). The
  brief's success criterion had never been measured *presenting to a surface*; every number
  this tree tracked was offscreen with readback. The first run read **2.84–3.38 ms** for
  dense text against your CPU backend's 5.9 and the 2.0 ms bar; **the closing run the same
  day reads 1.816 ms**, under the bar, on nothing but encode work. **The composition was the
  finding and still is: execute is 0.071–0.13 ms — the GPU is about four per cent of the
  frame** — and the rest is per-command CPU recording (1.130 ms for 4 320 commands, from
  1.90–2.32) plus 0.380 of submit and device wait. Read it as a minimum from one run on a
  desktop rather than as a margin. The artwork archetype's 43.3–61.4 ms is in §15 above, and
  it is the one that did not move.
- Also in the range, and none of them should be visible to you: ADR 0040 (which
  **retracts the 24.7 → 10.3 ms first-frame figure your §9.2 quotes** — it could not be
  reproduced in five configurations, and the allocation it was credited to costs 0.06 ms;
  the answer to `warm_for` is unchanged and is still *no*), ADR 0041 (a child the encoder
  drops when its clip leaves it nothing to contribute), ADR 0042 (a WGSL compile failure is
  now a refusal that names its span rather than a silent test-suite hang), and this round's
  `examples/rect_lane.rs` with the §19 numbers above.
