# A mark thinner than a device pixel on `Coverage::Gpu`: what each answer costs

Status: **a costing, for the owner and the caller to decide from.** No behaviour changed in this
round; one test was added, which pins a boundary that was unpinned. Written 2026-08-18 against
`main` at `fa2747c`.

The finding being costed is `doc/notes-hayro-paints.md` §2 and
`crates/quorra-gpu/tests/thin_marks.rs`: the device coverage lane samples an ordered 4 × 4 grid,
so its sample columns sit a quarter of a pixel apart, and a 0.1-device-pixel bar swept across ten
sub-pixel positions draws **nothing at six of them** and two and a half times its own ink at the
other four, where the processor lane draws its exact area at all ten. Byte-identical on llvmpipe
and RADV, so it is the design and not an adapter. It is the subject of the caller's standing ask
(`pdf-viewer/doc/QUORRA_HAIRLINE_MARKS.md`).

**The one-line answer.** It is a clause violation rather than a quality choice, its population on
the caller's own 954-page corpus is **35 marks at scale 1, 26 at 4× and 16 at 8×**, and the whole
of its *observable* cost — two pages of 956 that leave the oracle's agreement on the GPU lane — is
bought back by **one comparison in `take_gpu_lane`**, measured, for none of an area rule's cost.
An area rule is not an improvement to ADR 0016's lane; it is a different lane that gives up the
property ADR 0016 exists for.

---

## 1. What the clause requires, and what is ours

### 1.1 The three sentences, and which of them binds

ISO 32000-2 §10.7.4, verbatim:

> A shape shall be scan-converted by painting any pixel whose half-open square region intersects
> the shape, no matter how small the intersection is. This ensures that no shape ever disappears
> as a result of unfavourable placement relative to the device pixel grid, as might happen with
> other possible scan conversion rules. The area covered by painted pixels shall always be at
> least as large as the area of the original shape. This rule applies both to fill operations and
> to strokes with non-zero width. Zero-width strokes may be done in an implementation-defined
> manner that may include fewer pixels than the rule implies.

§10.7.1's NOTE, which is the licence for anti-aliasing at all:

> The specifics of the scan conversion algorithm are not defined as part of PDF. Different
> implementations can perform scan conversion in different ways; techniques that are appropriate
> for one device could be inappropriate for another.

and §11.3.7.2's NOTE 1, which is where a fractional answer is given a meaning:

> Mathematically, elementary objects have "hard" edges, with a shape value of either 0.0 or 1.0 at
> every point. However, when such objects are rasterized to device pixels, the shape values along
> the boundaries can be anti-aliased, taking on fractional values representing fractional coverage
> of those pixels. When such anti-aliasing is performed, it is important to treat the fractional
> coverage as shape rather than opacity.

### 1.2 The boundary, drawn precisely

| behaviour | verdict | why |
|---|---|---|
| Drawing fractional coverage at all, instead of painting whole pixels | **quality choice**, licensed | §10.7.1's NOTE leaves the algorithm open; §11.3.7.2's NOTE 1 gives the fraction a meaning as *shape*. Under that reading "painted" becomes "given non-zero shape" |
| Coverage proportional to the shape's exact area (ADR 0005) | **quality choice**, ours | §10.7.4 says nothing about proportion; ADR 0005 chose it and documents it as a choice |
| The device lane drawing 0.25 where the shape covers 0.10196 | **not a violation** | the clause's second requirement is a floor — "[t]he area covered by painted pixels shall always be at least as large as the area of the original shape". An overshoot satisfies it. It is a fidelity cost against ADR 0005's choice, nothing more |
| The device lane drawing **nothing** for a mark it intersects | **violation of §10.7.4** | the sentence is normative ("shall"), and the disappearance it produces is by "unfavourable placement relative to the device pixel grid" — the clause's own stated reason for the rule, word for word. The last sentence's exemption is for *zero-width strokes* and reaches nothing here |
| Both lanes losing a mark below 1/510 of a pixel (ADR 0006's 8-bit store) | **a cost, and the same violation in kind** | but it is the store's floor rather than the rule's, it is shared by any 8-bit device, and no mark observed on a real page lands there (`tests/thin_marks.rs` pins the constant) |

So the decision is **not** "fidelity versus cost" — one row of that table is a clause violation, and
principle 6's own vocabulary fits it exactly: a rule that is there in the document and absent on
the screen is a plausible-looking wrong page, not a refusal.

What the decision *is* about is how much a clause violation with a population of 35 marks is worth
spending on, and that is arithmetic once the population is measured.

The caller reached the same reading from the other side, and their words are worth carrying back
because they are sharper than ours (`pdf-render/src/sub_pixel.rs`, module comment):

> Coverage that rounds to nothing is not the anti-aliasing departure — it is the disappearance the
> clause's stated purpose forbids, arrived at by a different route.

---

## 2. The population, measured

### 2.1 The instrument

`doc/HANDOVER.md`'s "Counting a feature's population without the corpus test" is ADR 0067's
recipe and does not reach this question: thinness is a *device-space* property of a mark, so it can
only be counted where the device transform exists, which is our own encoder
(`doc/notes-census.md` §1 makes the same argument for lane choice). So this is the census's
instrument rather than ADR 0067's: a throwaway probe on `main`, run, read and **deleted with this
round**.

- **Probe**: three counting sites in the encoder — `encode/stroke.rs` (the stroke's resolved device
  width), and the two places `Encoder::take_gpu_lane` is consulted, `encode/coverage.rs`'s
  `push_coverage_styled` and `encode/fill.rs`'s `fill_solid`. Each mark contributes its **thin
  axis** (the stroke's own device width where it has one, otherwise the narrower side of its device
  bounding box) and whether the device lane drew it.
- **Corpus**: the caller's gate corpus, `doc/pdf.js/test/pdfs` in a scratch copy of
  `/home/cl/projects/pdf-viewer` taken **2026-08-18**, first pages, 1 434 files.
- **Configuration**: `Coverage::Gpu` — the only setting where the defect exists — `glyph_quantum`
  off as the gate runs it, RADV, scales 1, 4 and 8.
- **Threshold**: one sample-column spacing, `1/√coverage_samples` = 0.25 device pixels at the
  default sixteen. Below it a mark can fall between two columns; at or above it cannot, because
  the columns sit at `(k + ½)/√n` and so are `1/√n` apart, including across the pixel boundary.

Its limits, stated: a mark thin only in *part* (a glyph stem, a curve's tip) is not counted, so
these are counts of marks thin **everywhere**; and for a turned rule the bounding box overstates the
thin axis, so a diagonal hairline is counted as thick. §2.4 says what that residual does.

### 2.2 The counts

| `Coverage::Gpu`, RADV | scale 1 | scale 4 | scale 8 |
|---|---:|---:|---:|
| pages drawn | 954 | 948 | 929 |
| strokes | 57 437 | 22 117 | 21 940 |
| — narrower than one device pixel | 55 030 | 133 | 31 |
| — narrower than a quarter | 29 449 | 13 | 0 |
| solid fills and path-lane marks at the two lane sites | 329 608 | 287 798 | 286 862 |
| — whose thin axis is under a quarter pixel | 84 564 | 36 559 | 24 037 |
| — the pages they are on | 19 | 8 | 6 |
| marks the **device lane** drew | 988 | 3 456 | 7 554 |
| — the pages that used it | 176 | 388 | 557 |
| — with a thin axis under one device pixel | 209 | 121 | 46 |
| — **with a thin axis under a quarter pixel: the population that can vanish** | **35** | **26** | **16** |
| — the pages they are on | **7** | **2** | **1** |

**Each column is over that scale's own drawn population**, which is not the same set of pages: six
pages leave between 1× and 4× — five on the gate's 64 Mi pixel budget and `issue1905.pdf` refused at
render, which is `doc/notes-census.md` §1's own accounting — and one of them —
`issue12810.pdf`, 29 375 sub-quarter-pixel strokes — is the corpus's largest thin-stroke page. So
the stroke rows are checked on the common population before anything is concluded from them:
without that page, marks under a quarter pixel go **74 at scale 1 → 13 at 4×**, and the vanishing
population goes **34 → 26**. Both still fall, which is the finding; the page's departure inflates
the raw fall and does not cause it.

Four things fall out of that table and each of them moves the decision:

1. **The corpus is full of thin marks and the device lane draws almost none of them.** 84 564 marks
   at scale 1 have a thin axis under a quarter pixel; 35 of them take the device lane. The filter is
   ADR 0026's own cost comparison: the lane costs `3 × 32` bytes of vertex per triangle and buys
   `width × height` bytes of coverage, so a stroked rule — five points, five triangles, 480 bytes —
   needs a tile of 480 bytes before the device is cheaper. A thin mark's tile is one or two pixels
   across, so it has to be **hundreds of pixels long** to qualify. Short hairlines, which is most of
   them, never reach the lane that loses them — the defect is gated by a condition that was measured
   for a different reason entirely.
2. **The thin population is two pages.** `issue12295.pdf` (54 674) and `issue12810.pdf` (29 653) are
   **99.7 %** of the corpus's sub-quarter-pixel marks at scale 1, which is the same concentration
   `doc/notes-census.md` §5 found for the path lane as a whole.
3. **Magnification removes the population rather than adding it**, and that is the reverse of the
   intuition. A stroke's width arrives already resolved into device pixels (§4.5 of the brief, from
   §8.4.3.2 with §10.7.5), so a rule 0.1366 pixels wide at 1× is 0.55 at 4× and 1.09 at 8× and stops
   being thin. That width is the caller's own measurement of the corpus's extreme page, taken from
   the other side of the boundary by `pdf-model/examples/sub_pixel_width_census` — "all 65 859
   sub-pixel strokes are 0.1366 of a device pixel wide" on `issue12295.pdf` — which is the same page
   the counts above find at the top of every column. Strokes under a quarter pixel go **29 449 → 13 → 0**. Meanwhile the device lane is
   used *more* under magnification (988 → 3 456 → 7 554 marks, 176 → 388 → 557 pages), which is the
   lane working as designed.
4. **That matters because of when the setting is used.** The caller draws every page with
   `Coverage::Cpu` and switches past their `GPU_COVERAGE_MAGNIFICATION`, which is **ten**. So the
   scale-1 row is a population nobody's product reaches, and the row that describes the product is
   the 8× one: **16 marks, on one page of 929.**

### 2.3 What it looks like in the pixels

Counts are not sightings, so the same corpus was put beside the oracle. Full run, scale 1,
`Coverage::Gpu`, one copy of the caller's tree, the same hour:

| | agree | differ | refused | not comparable |
|---|---:|---:|---:|---:|
| `Coverage::Cpu` (`PLAN.md`'s matrix) | 931 | 23 | 2 | 18 |
| `Coverage::Gpu`, as built | **929** | **25** | 2 | 18 |
| `Coverage::Gpu`, thin marks kept on the processor lane (§5.4's experiment) | **931** | **23** | 2 | 18 |

The two pages are `bug1883609.pdf` (mean 0.4926, differing 3.11 %, SSIM 0.98615) and `vertical.pdf`
(mean 0.1526, differing 0.92 %, SSIM 0.98572) — 7 and 2 vanishing marks respectively. **Exactly
those two page lines change and every other line is identical to the character.** So the whole
observable cost of the defect, on the caller's whole corpus, is two pages; and the device lane's
fidelity gap against the processor lane at scale 1 is *entirely* this defect rather than the
sampling in general, which nobody had established before.

At 4× on the same eight-page subset, `issue12295.pdf` is the only page that differs on either lane,
and it differs for other reasons: mean **0.9810** on the processor lane, **0.9517** on the device
lane as built, **0.9201** with its 16 thin marks moved back. The defect is inside the noise of a
page that already disagrees.

The same copy's full run at 4× on the device lane reads **938 / 10 / 3 / 23**, which is
`PLAN.md`'s recorded row for that configuration to the page — so the scale-1 numbers above are a
measurement against a reproduced baseline rather than against a remembered one
(`HANDOVER.md`'s "always run the baseline in the same copy, on the same day").

### 2.4 The residual the counts do not cover

A hairline at 45° has a bounding box far wider than the mark, so it is counted as thick above and
would keep the device lane under any bounding-box rule. It does not *vanish* there — it passes
through many pixels and catches a sample column in some of them — it **dots**. The corpus says the
residual is small: `issue12810.pdf`'s 29 375 sub-quarter-pixel strokes are the corpus's largest such
population and exactly **one** of them takes the device lane, and after §5.4's experiment no page
outside the processor lane's own differing set remains.

---

## 3. What an area rule on the device lane would take

Sketched, not built. ADR 0016's lane is: one triangle per segment fanned from the contour's start
plus one Loop–Blinn control triangle per quadratic, `+1`/`−1` per fragment by facing, additively
blended into one channel of an `rgba16float` target whose pixel centres *are* the sample points;
then a resolve pass that applies §8.5.3.3's rule per channel and adds `covered/n` into the R8 sheet.
Sixteen samples is four rounds of that pair.

### 3.1 It is a different accumulation *and* a different primitive

Exact area cannot be reached by moving sample points, because the quantity changes: what has to
accumulate per pixel is the shape's signed **area** inside it, not a hit count at points. The
construction that computes that is the one `raster::fill_mask` already runs on the host (ADR 0008's
scanline): an `area` term for the pixel an edge crosses, plus a `cover` term carried along the row
by a prefix sum. A fragment pipeline has no prefix sum, so the carry has to be put into the
geometry, which is what makes it a different primitive:

- **per edge, two primitives** — (i) a quad over the edge's own device bounding box whose fragment
  shader computes the exact signed area the edge contributes inside that pixel (a trapezoid clipped
  to the unit square, the same closed form `accumulate_edge` uses), and (ii) a rectangle spanning
  from the edge to the **tile's right border** over the rows the edge crosses, carrying the edge's
  signed `dy` per row so that additive blending performs the prefix sum;
- **resolve**: one scalar per texel, `min(|a|, 1)` for §8.5.3.3.2 and a triangle wave for
  §8.5.3.3.3 — the same two lines `fill_mask` computes, so the two lanes' fill-rule arithmetic
  becomes one statement in two languages;
- **one round instead of `samples/4`**, and one scalar channel instead of four halves.

### 3.2 The consequence that decides it: the flattening comes back

Loop and Blinn's `u² < v` answers *inside or outside at a point*. It is a sample test by
construction, so an area rule cannot use it.

**And the curve itself is the obstacle, though not an absolute one** — `HANDOVER.md`'s rule that a
"cannot" is a claim that decays applies here and the honest form is: the signed area a segment
contributes to a pixel is `∫ x dy` along the part of it inside that pixel, which for a quadratic
means finding the parameters where the curve crosses the pixel's four boundaries — a root-find per
fragment per boundary, branchy, and unpriced by anyone in this tree. Every renderer whose source
argues about this flattens first instead. So the realistic shape of an area rule replaces
ADR 0016's "the quadratics the upload converted to, drawn directly" with
"line segments flattened to a device-space tolerance" — and ADR 0016's own sentence about why the
lane exists is:

> **Nothing in that depends on the device scale.** That is the whole point, and the difference from
> `raster.rs`, whose flattening tolerance is in device pixels.

Flattening per frame at a device tolerance is exactly the cost ADR 0015 measured at **6.8 ms a
frame at 20×** and exactly what a zoom gesture defeats. Either it happens on the host — and the lane
loses its reason to exist — or in a compute pre-pass, which is §1.6's candidate 1, retired by
`doc/notes-census.md` with a population rather than an argument.

And it costs fidelity in the direction the device lane is currently the *better* one. ADR 0016
measured it: on a curved edge the two lanes differ by up to **96 of 255 and the processor lane is
the one that is wrong**, because a chord cuts inside a convex curve at `FLATTEN_TOLERANCE`'s quarter
pixel. An area rule that flattens inherits that error, four times the 32-of-255 bound the sample
grid costs on a straight edge, unless the tolerance is tightened to the 0.004 that took the worst
difference to zero — which multiplies the segment count, and the segment count is what ADR 0026's
criterion is made of.

**So an area rule trades the one fidelity advantage the device lane has for the one it lacks.**

### 3.3 What survives of ADR 0016, and what does not

| survives | does not |
|---|---|
| the sheet integration — one R8 texture, two producers, `SheetUse`, `push_gpu_tile`'s reservation | the cubic → quadratic conversion as the upload's product, and the resident `QuadOutline` |
| ADR 0028's panes and the three places that must agree about `pane_origin` | the Loop–Blinn control triangle and its `u² < v` discard |
| the per-tile resolve quad, each tile carrying its own fill rule | the ordered sample grid: `sample_offsets`, `SAMPLES_PER_PASS`, and "sample count costs time, never memory" |
| `fs_winding`'s per-fragment tile-clip test | ADR 0026's criterion *as a number* — it prices quadratic fan triangles, and would have to be re-derived for flattened edges (ADR 0026's own "Revisit when" says so) |
| signed accumulation into a float target with additive blending, and one quantisation at the store | the `rgba16float` format, and with it the exactness argument: f16 is exact on the integers a winding number is, and an area sum is not an integer |

### 3.4 The counts, where counts exist

Durations are not available on this machine and would be worthless if they were
(`HANDOVER.md`'s trap), so what follows are counts:

- **Accumulate passes per frame**: `samples / 4` today — **four** at the default sixteen — against
  **one**. The resolve pass runs the same number of times.
- **Winding target bytes per texel**: **8** today (four f16 channels) against **4** for the
  `r32float` an area accumulation needs. ADR 0026's 82.8 MB winding texture on the corpus's largest
  page becomes 41.4 MB — which changes which pages refuse, so ADR 0026's arithmetic is re-taken
  rather than inherited.
- **Fragments per primitive**: unknown and **not** obviously better. A fan triangle today reaches
  from the contour's anchor to its segment, clipped to the tile; a cover rectangle reaches from the
  edge to the tile's right border. Both are O(tile) in the bad case, and nobody here has measured
  either.
- **Coverage levels**: 17 at sixteen samples, against 256 — the store's, which is where the
  processor lane already is.

### 3.5 What it would buy

- **The 35 / 26 / 16 marks stop vanishing**, and §10.7.4 is met by construction rather than by a
  condition.
- **The device lane's corpus verdict becomes the processor lane's.** Measured for the *condition*
  in §2.3 (929/25 → 931/23), and an area rule would reach the same place; it is worth knowing that
  the condition already reaches it, because that is the whole of the fidelity prize at scale 1.
- **ADR 0064's objection dissolves, and it is worth pricing exactly.** That ADR declined to send
  rare paints to the device largely because "the higher the zoom, the more of a page's
  shading-painted text it would send to the device", onto a grid that loses thin marks. With an
  area rule that reason is gone, and its own numbers say what is then available: **0.110 % of the
  coverage a frame rasterises at scale 1 (28 marks) and 0.629 % at 4× (88 marks)**, on 9 and 14
  pages of 954, the largest single page being 0.18 % of the corpus's coverage. ADR 0064 also says
  where the real prize is, and it is not here: **82 % of all rare-painted coverage is under a
  residue clip**, which needs the residue multiply on the device and nothing in an area rule
  touches it. So an area rule turns a 0.11 %–0.63 % opportunity from refused into available, and
  leaves the 82 % exactly where it is.
- **`tests/shader_copies.rs`'s kind of sameness gains a member**: the fill rule would be computed
  by one arithmetic in two languages rather than two arithmetics.

### 3.6 The unknowns, named rather than estimated

1. **Cross-adapter identity.** ADR 0006's promise — byte-identical coverage on RADV and llvmpipe,
   which is what lets CI run on a software rasteriser and is §11's question 4 — is currently
   protected by the accumulation being **integer**. An area accumulation is float, and while the
   blend order is defined by primitive order, whether two adapters round the same sum identically is
   unestablished. A round that builds this must measure it first, on the smallest possible fixture,
   because the answer changes how CI works.
2. **Whether a quadratic's exact area could be integrated per fragment** rather than flattened
   (§3.2). It is not ruled out; it is unpriced, and it is the one thing that would let an area rule
   keep ADR 0016's scale-independence. A round that wants the area rule should price *this* first,
   because everything else about the option follows from the answer.
3. **The fragment count of the cover-rectangle formulation** on a real page. Nobody has measured it
   here or stated a bound for it.
4. **Where the flattening runs**, and if it is a compute pre-pass, how its buffers are budgeted
   without the "hand-picked constants" failure principle 6 names.
5. **What `Options::coverage_samples` becomes** — §4 below.

### 3.7 The size of it

A new pipeline pair, a new shader with its own clause citations, a new vertex format, a
re-derivation of ADR 0026's criterion, gates on both adapters, and a corpus run. That is the shape
of ADR 0016 itself: a milestone, not a round.

---

## 4. What an area rule does to `Options::coverage_samples`

It makes it **meaningless**, and that is public API.

`Options::coverage_samples` is documented as "how many samples the GPU lane takes per pixel …
Coverage has `samples + 1` levels rather than 256, so this is the quality knob and it costs time,
not memory", with `DEFAULT_COVERAGE_SAMPLES` a public constant and `sample_column_spacing()` derived
from it in `tests/thin_marks.rs`. Under an area rule there are no samples and coverage is exact to
the store's eight bits, so the field has three possible fates and each is a decision:

- **Deprecate it.** A caller that sets it gets an option that does nothing, which principle 6 calls
  by name — so it would have to be *reported*, and a `Report` for a field that was legal in the
  previous release is a bump's business.
- **Keep both rules and let the field select.** Two device lanes to maintain, two sets of gates,
  and the sameness argument in §3.5 dies.
- **Refuse a non-default value.** Honest, and it turns a quality knob into a compatibility error.

None of them is expensive. It is listed because "an area rule" sounds like a shader change and is
in fact a public-API change as well.

---

## 5. The alternatives, each costed the same way

### 5.1 Leave it, and document it

**Cost**: zero work, and a clause violation left standing where the caller has already asked about
it twice (their #104 and #1023, and `QUORRA_HAIRLINE_MARKS.md`).

**What a caller would have to be told**, and it belongs on `Coverage::Gpu`'s own rustdoc beside the
two lane rules already there: *a mark narrower than `1/√coverage_samples` of a device pixel may draw
nothing at all on this lane, depending on where it lands; the processor lane draws its exact area.
The setting is for magnification, and magnification is what takes marks out of that band — measured
on the caller's corpus, 35 such marks at scale 1, 26 at 4× and 16 at 8×.*

**Against it**: `tests/thin_marks.rs`'s recorded gap becomes permanent, and the two corpus pages
stay off the oracle on the lane the caller switches to at zoom. It is the only option that leaves a
"shall" unmet with nothing written against it but a paragraph.

### 5.2 Refuse `Coverage::Gpu` for a mark below a stated width

**Cost**: a `RenderError` variant, its message, its gate; the caller must handle a refusal for a
scene it could draw.

**Against it, decisively**: principle 6 offers refusal as the alternative to *silently drawing
nothing*, not as the alternative to drawing the mark correctly by the other producer — which is one
`if` away in the same function. This option is dominated by §5.4 in every respect. It is recorded
so nobody re-proposes it.

### 5.3 Let the caller keep doing what `sub_pixel.rs` does

**This one needs a correction to its premise, and the correction is load-bearing.** The caller's
`sub_pixel_bands` / `substitute_width` / `enlarged_mark` machinery — a mark thinner than a device
pixel replaced by the whole pixel line it lies in at the coverage its own area implies — is applied
by **`render-cpu` only**. `render-quorra` calls `pdf_render::split_collapsed_fill` — the *zero*-area
case, snapped to whole device pixels on their side so that every backend inherits one answer
(`QUORRA_HAIRLINE_MARKS.md`, answered in their 368th session) — and **nothing else**, because our
processor lane needs no substitution. Their own module comment says why, quoting our numbers:

> its coverage is a multiple of a sixteenth of a pixel and its smallest non-zero answer is 1/16 …
> The graphics device has no such quantum and answers 0.0510, 0.1020 and 0.2000 for the same three.

So this option is not "let them keep doing it". It is "**ask them to start** doing it for us,
conditioned on which coverage producer we are running" — which makes their display list depend on
our internal lane setting, where today it is lane-independent. That is CLAUDE.md's third
consequence in its unhappy direction: a decision neither side can take alone, taken for a reason
that lives entirely inside our encoder, and undone again the day an area rule lands.

**Cost**: their machinery exists and is tested, so their side is small; the coupling is the price,
and it is paid in the contract rather than in code. **Buys**: what §5.4 buys, from further away.

### 5.4 A fifth condition on `take_gpu_lane` — measured

`Encoder::take_gpu_lane` is four conditions today, each "a measurement rather than a taste": the
caller asked, no residue clip, the tile is worth more than its triangles, and the cache is no use
for this placement. A fifth — **a mark thinner than one sample-column spacing keeps the processor
lane** — is one comparison, and both call sites already hold the float extents they would compare
(`bounds` in `push_coverage_styled`, `fill.bounds` in `fill_solid`).

**The threshold is derived, not tuned**: `1/√coverage_samples`, for the arithmetic in §2.1, and it
moves when the sample count moves. The thinness measure is the stroke's own resolved device width
where the mark is a stroke and the device bounding box otherwise, which is what the probe used.

**Measured, on the caller's whole corpus, scale 1, `Coverage::Gpu`** (§2.3): **929/25/2/18 →
931/23/2/18**, exactly two page lines change, every other line identical to the character, and the
result is the processor lane's own verdict for that scale.

**What it costs**: those marks pay the CPU rasteriser instead of the device. They are thinner than
a quarter pixel by construction, so their tiles are one or two pixels across and among the smallest
the sheet ever holds; the count is 35 at scale 1, 26 at 4× and 16 at 8×, and a count is the honest
statement because this machine cannot measure the duration (`HANDOVER.md`'s trap, and ADR 0052's
seam).

**What it does not fix**: the diagonal hairline of §2.4, which keeps the device lane and dots rather
than vanishing. That residual is real and should be written into whatever ADR takes this, because a
condition that reads as "thin marks are safe now" would be over-claiming.

**Why it is not just a patch over the real problem**: it is the same shape as the four conditions
already there — a lane is taken where it is a win and declined where it is not — and the device
lane genuinely is not a win for a mark it cannot represent. ADR 0016 states that trade itself, one
class of shape over: reading-size text "is where that difference is most visible and is exactly
where the CPU lane is already fast".

---

## 6. Recommendation

**Take §5.4 and do not build the area rule.** Stated as a recommendation, with what would change it.

1. **The population does not justify a milestone.** 16 marks on one page of 929 at the magnification
   the caller's product actually switches at, and 35 on seven pages at a scale where their product
   uses the other lane entirely. Principle 2 forbids speculative work on code nobody measured;
   ADR 0067 answered a caller's narrowing request the same way, with the same instrument, three
   days ago.
2. **The clause violation is real and should be closed anyway**, because a mark that is *absent* is
   not the same kind of statement as a mark that is slightly wrong — the fixture's sweep says such a
   mark draws nothing at six sub-pixel positions in ten — and §5.4 closes all of the part anybody
   can see, for one comparison.
3. **An area rule would give up ADR 0016's reason for existing.** Scale-independence is the whole
   argument for a second coverage producer, and exact area requires flattening at a device
   tolerance. A round that wants an area rule should first answer whether it still wants ADR 0016's
   lane at all — that is a different and larger question than this one.
4. **ADR 0064 is not enough to move it.** With the objection gone, what becomes available is 0.11 %
   of a frame's coverage at scale 1 and 0.63 % at 4×; the 82 % is behind the residue multiply, which
   an area rule does not touch. If a round is looking for the device lane's biggest unclaimed prize,
   it is the residue multiply and it always was.

**What would overturn this**: a page — from the caller's product, not a fixture — where a sub-quarter
-pixel mark is a material part of what a *frame* shows at a magnification past ten. The instrument to
re-ask with is in §2.1, it costs minutes, and the corpus's answer today is one page.

**If the owner takes §5.4**, it is an ADR of its own with: the threshold derived from
`coverage_samples` rather than written as 0.25; the thinness measure named for both mark kinds; the
corpus matrix above re-taken on the day it lands, in one copy; the diagonal residual written down as
a stated cost; and `tests/thin_marks.rs`'s recorded gap turned into the requirement it currently
only records — the file's own failure message already says to do exactly that.

### 6.1 What goes back to the caller, and what it asks of them

Their `QUORRA_HAIRLINE_MARKS.md` is answered for the *collapsed* fill and has never been answered
for this half, so four sentences belong in the next reply the owner carries across (we never edit
their tree):

- **On the lane you draw pages with, a hairline never disappears** — the processor lane computes
  the exact area and draws 0.0510, 0.1020, 0.2000 for the three widths your `sub_pixel_marks`
  measures. **On the lane you switch to past `GPU_COVERAGE_MAGNIFICATION`, a mark thinner than a
  quarter of a device pixel can draw nothing at all**, depending where it lands.
- **On your own corpus that is 35 marks on 7 pages at scale 1, 26 on 2 pages at 4× and 16 on one
  page at 8×**, and two pages — `bug1883609.pdf` and `vertical.pdf` — leave your oracle's agreement
  because of it at scale 1.
- **The fix we recommend is entirely ours** (§5.4): a fifth condition inside our lane chooser, no
  display-list change, no API change, nothing for you to adopt but a version. The alternative that
  *would* ask something of you — extending `sub_pixel_bands` and `enlarged_mark` to the quorra path
  — we recommend against, because it would make your display list depend on our coverage setting
  (§5.3).
- **One question is yours**, and it is the only thing that would change the recommendation: is there
  a page in the product, past magnification ten, where a mark that thin is a material part of what
  the frame shows? Our corpus says one page; your users' documents are not our corpus.

---

## 7. What this round did not do

- **No behaviour changed.** The fifth condition was built behind an environment variable to measure
  it, run, and **reverted**; the probe of §2.1 likewise. What is left in the tree is this file and
  one test.
- **The one test added** is `at_the_sample_spacing_the_device_lane_holds_the_clause_at_every_position`
  in `tests/thin_marks.rs`: at exactly one sample-column spacing the clause holds at all ten
  sub-pixel positions, where the recorded gap below it holds at four. That boundary is what every
  option above is priced against and it was unpinned — the neighbouring test asserts those widths at
  one position only. Verified able to fail by narrowing the bar below the spacing.
- **No ADR.** The decision is the owner's and the caller's; an ADR pre-empts it.
- **No area-rule spike.** A half-built lane would answer the fragment-count and cross-adapter
  questions of §3.6 badly and would cost more to un-build than to not build.

---

## 8. Recommended edits, as quoted text

For `doc/PLAN.md`, **replacing** the thin-mark bullet the last round recommended under *What is
still open* (the numbers are now measured rather than characterised):

> - **A mark thinner than a quarter of a device pixel can vanish on `Coverage::Gpu`, and the
>   population is 35 marks.** The device lane counts samples on an ordered 4 × 4 grid, so a mark
>   narrower than the column spacing falls between two columns and reads zero — six of ten sub-pixel
>   positions for a 0.1-pixel bar, where the processor lane reads the shape's exact area at all ten,
>   byte-identical on llvmpipe and RADV. That is a **violation of §10.7.4** rather than a fidelity
>   choice: painting every pixel the shape intersects "ensures that no shape ever disappears as a
>   result of unfavourable placement relative to the device pixel grid". Measured on the caller's
>   corpus at `Coverage::Gpu` (2026-08-18, `doc/notes-thin-mark-options.md`): **35 such marks on 7
>   pages at scale 1, 26 on 2 pages at 4×, 16 on 1 page at 8×** — magnification *removes* the
>   population, because a device width does not follow the viewport — and the whole observable cost
>   is **two corpus pages** that leave the oracle's agreement on that lane. A fifth `take_gpu_lane`
>   condition keeping such marks on the processor lane restores the processor lane's own verdict
>   exactly (929/25/2/18 → 931/23/2/18, two page lines, every other line identical). An area rule
>   would close it by construction and costs ADR 0016's reason for existing, because exact area
>   requires flattening at a device tolerance. **The decision is the owner's and the caller's**;
>   the costing is `doc/notes-thin-mark-options.md`.

For `doc/HANDOVER.md`, **appending to** the existing "A sampled coverage rule and an area coverage
rule disagree about what is *there*" trap:

> **Costed 2026-08-18** (`doc/notes-thin-mark-options.md`): it is a §10.7.4 violation rather than a
> quality choice, its corpus population is 35 marks at scale 1 and 16 at 8×, and its whole visible
> cost is two pages of 956 on the GPU lane. And the count that surprises: **magnification shrinks
> the population**, because a stroke's width arrives resolved in device pixels and does not follow
> the viewport — so the lane a caller switches to at zoom is the lane the defect has almost stopped
> reaching by then.

and, to **Instruments**:

> - **How thin a page's marks are, and which lane draws them**: a throwaway probe at
>   `encode/stroke.rs` and the two `take_gpu_lane` sites, carrying each mark's thin axis (a stroke's
>   resolved device width, otherwise the narrow side of its device box) and the lane it took, read
>   through a `thin_census.rs` in a scratch copy of the caller's tree —
>   `doc/notes-thin-mark-options.md` §2.1, about fifteen seconds a scale. **Take it at more than one
>   scale**: a device width does not follow the viewport, so the thin population *falls* as the
>   magnification rises, and a census at one scale says the opposite of what a census at three says.

and, to **Recorded and deliberately not taken** — only if the owner decides to leave the lane as it
is:

> - an area rule on the device lane, whose population is 35 corpus marks at scale 1 and 16 at 8×
>   and whose cost is ADR 0016's scale-independence, since exact area requires flattening at a
>   device tolerance (`doc/notes-thin-mark-options.md`).
