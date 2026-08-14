# 0047 — The rectangle lane belongs to the shape, not to the command

Date: 2026-08-14. Status: accepted. Answers the caller's `QUORRA_FEEDBACK.md` §19 from
our side of the boundary, and closes the second small debt in `doc/HANDOVER.md`.

## The situation

ADR 0007 built an analytic lane for axis-aligned rectangles: no tiling, no edge list, no
coverage byte — one instance, and `rect.wgsl` computes the exact area of the overlap
between the rectangle and each pixel cell. `RENDER_LIBRARY.md` §6.4 asks for it by name,
and the brief's §0 premise is that a page is glyphs and rectangles.

**No document reaches it.** The caller measured their 995-page corpus and not one page
emits a `Command::Rect`: every rectangle a document draws comes through
`pdf_render::Command::Fill` carrying an outline that happens to be four axis-aligned
edges, because §8.5.2.1's `re` operator appends a subpath to a path like any other
operator and their translation hands the path over without asking what shape it is.

The recogniser for that shape was already here, and already ran on every outline:
`ResourceStore::upload_outline` stores `rect_hint = axis_aligned_rect(path)` for each
uploaded path (`resources.rs`). Two readers used it — a clip link (`encode/clips.rs`) and
the shaded fill arm (`encode.rs`) — and nothing else. **A solid fill of a rectangular
outline fell into `fill_solid` before the check was reached** and took the GPU triangle
lane, the glyph atlas or the scratch coverage sheet like any other shape.

So the lane was reachable by a command nobody sends, and unreachable by the command
everybody sends.

## The decision

**A solid fill takes the analytic rectangle lane under exactly the conditions the shaded
fill arm already takes it under**, and the device rectangle both arms compute is computed
by one function.

```rust
resolved.residues.is_none() && transform_preserves_axes(&to_device) && stored.rect_hint
```

`Encoder::clipped_device_rect` is that one function: it maps the two corners, orders
them, intersects with the clip rectangle and with the target, and answers `None` when
nothing is left. `encode_rect` now calls it too, so the `Rect` command and the `Fill`
command reach the lane through the same arithmetic rather than through two copies of it.

### Why each condition, and why the fill rule is not one

- **No residue clip.** A non-rectangular clip link is kept as a residue and has to
  *multiply* into a coverage tile (ADR 0030). The analytic lane has no tile and nowhere
  to put it. The rectangular part of the chain is not a condition at all — it is
  intersected into the geometry, which is ADR 0007's whole point.
- **An axis-preserving transform.** Four axis-aligned edges under a shear or a rotation
  are a parallelogram, and `rect.wgsl` evaluates a box. `transform_preserves_axes` is the
  same exact-zero test the clip resolver and the shaded arm use.
- **`rect_hint`.** `quorra_scene::axis_aligned_rect` accepts one closed subpath of four
  corners with alternating vertical and horizontal edges, compared exactly — a
  nearly-closed path is not a rectangle, and a second subpath is not one either.
- **The fill rule is deliberately absent.** For a *simple closed* curve §8.5.3.3.3's
  even-odd rule and §8.5.3.3.2's non-zero rule bound the same region: a ray from an
  interior point crosses the boundary an odd number of times, and those crossings sum to
  a winding of ±1 whichever direction the corners were given in. Since the recogniser
  admits nothing but a simple closed quadrilateral, the rule cannot change the mark, and
  asking about it would be a condition that narrows the lane for no reason. This is
  asserted rather than argued — `a_solid_fill_of_a_rectangle_takes_the_rectangle_lane`
  encodes the same fill under both rules and compares the instance bytes.

Two things that look like conditions and are not, because they are settled before the
lane is chosen:

- **The blend mode.** A non-Normal blend has already been routed into §11.3.5's implicit
  one-element group by `encode_fill`, which recurses with `BlendMode::Normal`; the
  recursion reaches this lane, and the lane draws into the child layer. Nothing here
  needs to know.
- **The compose mode.** `Compose::Src`, `DestOut` and `Plus` become a `DrawStyle`, and
  the compositor already has `RectErase`, `RectAdd` and `RectOver` pipelines and ADR
  0010's strict per-instance interleaving for knockout. The only change needed was to
  pass the style into `push_rect_instance` instead of having it read `self.style` — a
  `Rect` command carries no compose mode, so it passes exactly what the method used to
  read.

## What changes in a pixel, and why it is the better answer in both places

The lane is not merely cheaper; it is a different computation, and it differs from the
coverage lanes in exactly two respects. Both were already the treatment a `Command::Rect`
and a *shaded* rectangular fill received, so this ADR spreads an existing answer rather
than inventing one.

- **The clip is intersected, not multiplied.** The coverage lanes compute
  `coverage(shape) × coverage(clip)` per pixel (`coverage.wgsl`); the analytic lane
  intersects the two rectangles first and computes the coverage of the result. For a
  pixel that a shape covers on the left and a clip admits on the right, the product is
  0.25 and the true area of the intersection is 0. ISO 32000-2 §8.5.4 states a clip as
  *the intersection of two regions*, so the exact area of the intersection is the value
  the clause describes and the product is the approximation.
- **The sub-pixel phase is not quantised.** The glyph atlas keys a placement by a phase
  rounded to `1/q` of a pixel (§4.5's fifth decision, ADR 0009), which moves a cached
  rectangle by up to half a quantum. The analytic lane has no cache to key and draws the
  edge where it is. This one is invisible to the corpus gate below, which runs with the
  quantum **off** by design — it is a fidelity gain in the product rather than in the
  instrument.

What does *not* change: the coverage arithmetic itself. `rect.wgsl` and `coverage.wgsl`
compute the same overlap formula from the same ADR 0005, which is why
`rect_lane_and_glyph_lane_agree_on_a_rectangle` holds the two within one premultiplied
step, and why a fill straddling the target's edge reads the same 128 down either lane
(`cull.rs`).

## The measurement

`examples/rect_lane.rs`, three scenes drawing the identical set of device rectangles at
the two sizes the caller's profile says a page contains, timed **round-robin** and
reported as minima. Base commit `87898c6` and this change, built into their own target
directories and run alternately in the same hour.

Three rounds of each binary, alternating, at load average 32–39 — a busy desktop, which
is why the **ratio to the `rect` row** is the column that travels and the milliseconds
are context. (The absolute numbers below are not comparable with `PLAN.md`'s earlier
table, measured at load 2.8–4.5; the ratios are.)

| commands | lane | RADV before | RADV after | llvmpipe before | llvmpipe after |
|---:|---|---:|---:|---:|---:|
| 12 | `rect` | 0.0004 ms | 0.0004 ms | 0.0006 ms | 0.0005 ms |
| 12 | `fill`, one outline many placements | 4.38–4.60× | **2.54–2.60×** | 4.13–4.39× | **1.85–2.38×** |
| 12 | `fill`, an outline each | 5.13–5.26× | **2.94–3.21×** | 4.57–5.02× | **2.70–3.22×** |
| 4 320 | `rect` | 0.0784 ms | 0.0788 ms | 0.0845 ms | 0.0856 ms |
| 4 320 | `fill`, one outline many placements | 5.94–6.12× | **2.66–3.49×** | 6.12–6.21× | **2.63–2.70×** |
| 4 320 | `fill`, an outline each | 18.10–20.70× | **6.07–12.67×** | 19.31–20.01× | **11.12–11.65×** |

Read as milliseconds off the p99 page's encode, from the minima: the reused outline goes
**0.466 → 0.210 ms** on RADV and **0.521 → 0.231** on llvmpipe; the placed-once one
**1.507 → 0.528–0.998** and **1.631 → 0.997**. Per rectangle that is **0.06 µs saved when
the outline repeats and 0.12–0.23 µs when it does not.**

Stated as the gap that remains over a `Rect` command, which is what §19 is really asking:
**0.090–0.101 → 0.030–0.034 µs a rectangle for a reused outline** (a third of what it
was), and **0.33–0.36 → 0.10–0.21 µs for a placed-once one** (between a third and three
fifths). `PLAN.md`'s earlier figure for that gap — 0.13–0.19 and 0.21–0.49 µs — was taken
on a quiet machine and this one on a loud one, so the two are not the same measurement and
only their direction is comparable.

**The counters are the part that cannot be a wall clock.** Before, the two fill scenes
reported 12, 280 and 4 320 distinct atlas keys and wrote 276 480 bytes of quad instances;
after, they report **0 tiles, 0 atlas keys and the same 138 240 bytes of rectangle
instances the `Rect` scene writes** — 32 bytes an instance, and the same 32 bytes.
`bytes differing from rect` stays at **0 of 8 022 576** for every row, which is what says
the lane change moved a cost and not a mark.

**What is left in the ratio is not the lane.** A `Fill` still pays a resource lookup, a
control-hull box (ADR 0045's memo, which a `Rect` needs no entry in), a distinct-outline
probe and a clip resolution — and, in the `distinct` column, a census entry and a hull
memo that misses. That residue is the honest answer to §19's question now: **recognising
the shape on the caller's side would still save something, and it is a third of what it
was.**

### The caller's corpus

One copy of their tree, both runs of each pair inside it, flipping only the `[patch]`
between the base commit `87898c6` and this change.

| | verdicts | pages whose numbers moved |
|---|---|---|
| scale 1 | 934 agree / 20 differ / 2 refused / 18 not comparable, **before and after** | 4 |
| scale 4 | 936 agree / 10 differ / 5 refused / 23 not comparable, **before and after** | 1 |

No page changed its verdict at either scale, and no page changed its refusal. What moved:

| page | scale | mean before | mean after | worst tile | differing | SSIM |
|---|---:|---:|---:|---|---|---|
| `22060_A1_01_Plans.pdf` | 1 | 0.7905 | **0.7903** | unchanged | unchanged | unchanged |
| `pr12564.pdf` | 1 | 1.3860 | **1.3859** | unchanged | unchanged | unchanged |
| `standard_fonts.pdf` | 1 | 1.7329 | **1.7327** | unchanged | unchanged | unchanged |
| `issue4402_reduced.pdf` | 1 | 4.0985 | **4.0986** | unchanged | unchanged | unchanged |
| `issue19971.pdf` | 4 | 0.1218 | **0.1239** | unchanged | 0.0052 → 0.0053 | 0.99901 → 0.99893 |

Three of the five moved toward the oracle and two away, by between 0.0001 and 0.0021 of a
mean stated in unorm steps — under a hundredth of one coverage step, on pages that already
differ. **The direction is not the point and neither side of it is evidence of a defect**:
the two computations differ where a clip has a fractional edge (the intersection against
the product, above) and where a coverage byte was rounded, and `tiny-skia` multiplies its
clip in exactly the way the coverage lane used to. A page whose rectangles sit under a
fractional clip is therefore *expected* to move away from the oracle here, and §8.5.4 is
why that is the right direction to move. The magnitudes are what make it a footnote rather
than a question: nothing here is visible, and no verdict moved.

The scale-1 pair ran 42 s and 231 s of wall clock for identical work, which is the
machine and not the change; timings from these runs are not quoted anywhere.

## What it costs

- **Three tests had been using a rectangle as a stand-in glyph**, and this change would
  have made them compare a computation with itself while still passing —
  `atlas_and_scratch_fallback_are_byte_identical` and
  `rect_lane_and_glyph_lane_agree_on_a_rectangle` would have gone quietly vacuous, and
  `the_quantum_is_settable_and_off_is_exact` failed loudly, which is the only reason the
  other two were looked at. They now use a rectangle carrying one redundant vertex on its
  top edge: the identical region, and not a shape the recogniser folds. **The lesson is
  the file's, not this ADR's**: a fixture that names a lane should say which lane it
  means.
- **A page's rectangles no longer populate the atlas.** That is a saving on the page that
  draws them and a change of working set for the page that draws rectangles *and* text —
  the text now has the atlas to itself. Nothing in the corpus moved on it.
- **Two arms now share `clipped_device_rect`.** A reader of `encode_rect` has one more
  hop to make. The alternative was two copies of the intersection, which is how two doors
  to one lane come to draw two different marks.
- **The fill arm still measures a control hull it does not use.** `encode_fill` bounds the
  outline before it knows which lane the paint will take, because the cull has to run
  before the implicit blend group is built; a rect-hinted fill then computes its device
  box a second time, exactly. That is what the residue in the `distinct` column mostly is
  — **0.21 µs a rectangle**, against 0.03 µs when the outline repeats and ADR 0045's memo
  answers. Moving the lane decision above the bound would take it and is *not* taken here:
  it reorders the cull, it is an optimisation rather than the symmetry this ADR is, and it
  is worth its own measurement. The number is written down so the next round need not
  rediscover it.

## Alternatives not taken

- **A recogniser on the caller's side**, which is what their §19 proposes. It is theirs
  to write and it would still be worth what the residue above measures — but it costs
  them a pass over every outline and a change to a display list that is *the contract*,
  to buy a lane we can reach from a hint we already compute. The ordering follows
  CLAUDE.md: a decision either side can make alone is one neither has made, and this half
  needed no conversation.
- **Widening the recogniser** — accepting a rectangle with a redundant collinear vertex,
  or two subpaths that happen to be one rectangle. Every widening is a new claim about
  what a document means, and none of them is measured. `axis_aligned_rect` accepts what
  `re` emits, which is the population the caller's corpus has.
- **Recognising the rectangle at encode time rather than at upload.** It is already at
  upload, it is already paid for, and moving it would put a scan of the segment list on
  the hottest walk there is.
