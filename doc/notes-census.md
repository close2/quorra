# §11.2's lane census, run

Status: **record** — a measurement, its instrument, its date, and how to redo it.

`RENDER_LIBRARY.md` §11 question 2 asks:

> **Does a document renderer want tiles at all for the glyph path?** If glyphs are quads
> against an atlas and rectangles are analytic, the general tile-binned path may be reached
> by a small minority of commands. Measure that minority on our corpus before designing
> for it.

`PLAN.md` §1.1 stakes the architecture on the answer and names what would overturn it, and
M5 recorded the census as **not run**. It has now been run. **The premise survives: over
948 real first pages at their own scale, on the coverage lane the viewer draws every page
with, 9.4 % of the marks a page puts on the target miss the glyph and rectangle lanes** —
10.4 % with the 1/16 glyph quantum the viewer runs, 9.4 % with it off, which is how the
caller's own corpus gate is configured. Glyph and rectangle together are 89.1 % of the
marks (88.2 % with the quantum on); images are 1.5 %.

---

## 1. What was measured, and where

The census is taken **at our own encoder**, not by re-deriving lane rules from the
caller's display list. Lane choice is a device-space question — the same outline is a quad
at 100 % and a path at 6400 % (§1.1) — so it can only be counted where it happens.

- **Instrument:** `Counters::lanes` (`LaneCounts`), added for this round, and ADR 0057's
  `Counters::coverage` (`CoverageSheet`), which is what the "by work" rows read.
  `tests/lane_census.rs` gates both against the lanes; §6 justifies the new one.
- **Corpus:** the caller's gate corpus — 974 documents' first pages, `doc/pdf.js/test/pdfs`
  in `/home/cl/projects/pdf-viewer` at **`22ab57d4`**, copied on **2026-08-17**. *Their
  tree moves under us: a count in an older document is not a baseline* (`HANDOVER.md`).
- **quorra:** `main` at **`6ee3072`** plus this round's counter. **Every row below was
  re-taken on that base**; §11 says which numbers the rebase moved and why.
- **Adapter:** `AMD Radeon 890M Graphics (RADV STRIX1)`, 24 encode threads.
- **Scales:** 1, 2 and 4 — a census without a scale is not a census.
- **Lanes:** `Coverage::Cpu` (the caller's default, and what `viewer-ui` draws every page
  with) and `Coverage::Gpu` (what it switches to past its magnification threshold).
- **Quantum:** both **off** (the corpus gate's setting, which isolates fidelity from the
  sub-pixel trade) and at the **1/16 default** the viewer actually runs. §3 is why that
  turned out to matter more than the scale did.

**The population is the same 948 pages in every row below.** The corpus draws 954 at
scale 1 and 948 at scale 4: five pages exceed the gate's 64 Mi pixel budget when magnified
and `issue1905.pdf` is refused at 4× rather than 1×. Among the five is `issue12810.pdf`,
whose 34 970 sub-pixel strokes are the single largest path-lane page in the corpus.
Comparing scales over each scale's own population would have credited magnification with
that page's departure.

## 2. The lane shares

Marks, not commands: a group draws none of its own, a culled command draws none at all,
and a command inside a group is counted where it draws. 291 411 marks over 948 pages.

| lane, scale, quantum | rectangle | glyph | path | image | **path %** |
|---|---:|---:|---:|---:|---:|
| CPU, 1×, off | 7 332 | 252 320 | 27 507 | 4 252 | **9.44 %** |
| CPU, 2×, off | 7 332 | 247 568 | 32 252 | 4 252 | 11.07 % |
| CPU, 4×, off | 7 332 | 183 203 | 96 588 | 4 252 | 33.15 % |
| CPU, 1×, 1/16 | 7 332 | 249 644 | 30 183 | 4 252 | **10.36 %** |
| CPU, 2×, 1/16 | 7 332 | 251 884 | 27 936 | 4 252 | 9.59 % |
| CPU, 4×, 1/16 | 7 332 | 247 683 | 32 108 | 4 252 | **11.02 %** |
| GPU, 1×, off | 7 332 | 252 919 | 26 908 | 4 252 | 9.23 % |
| GPU, 4×, off | 7 332 | 245 995 | 33 795 | 4 252 | 11.60 % |
| GPU, 4×, 1/16 | 7 332 | 243 477 | 36 313 | 4 252 | 12.46 % |

Read the **1/16 rows** as the viewer's own configuration: **10.36 % → 9.59 % → 11.02 %**
from 1× to 4×. The path lane is a tenth of the marks and stays a tenth under a sixteenfold
increase in pixels. The one row that is not a tenth is 4× with the quantum off, and §3 is
what that row is actually measuring.

The rectangle lane is 2.5 % and **not one of those marks arrived as a `Command::Rect`** —
every one is a fill of a four-edged outline reaching ADR 0047's door, which is the
corpus-profile finding confirmed at the lane rather than at the command.

### By the work each lane causes

`Counters::coverage` is ADR 0057's, and it reports the **sheet**: the tiles a frame
rasterised, uploaded and sampled. A glyph tile that reaches the atlas is not on it.

| common 948 pages | CPU 1×, off | CPU 4×, off |
|---|---:|---:|
| sheet texels (`coverage.texels`) | 20 322 933 | 347 883 493 |
| sheet tiles | 27 519 | 96 600 |
| glyph working set — an **upper bound** on the atlas's share | 10 261 469 | 109 894 935 |

**This is the one place the path lane is not a tenth of anything.** The rectangle lane
rasterises *zero* texels by construction (asserted, not assumed —
`a_rectangular_outline_takes_the_rectangle_lane_and_rasterises_nothing`), the image lane
rasterises none without a residue clip, and a glyph tile is rasterised once per distinct
key however many times it is placed. So of all the R8 coverage this corpus makes,
**at least 66 % at scale 1 and 76 % at 4× is the sheet's** — made for the tenth of the
marks in the path lane and the residue regions they are cut from.
`atlas_working_set_bytes` counts distinct keys including ones a previous page left
resident, so it is an upper bound on the glyph part and those are lower bounds on the rest.

A tenth of the marks, two thirds of the coverage. That is the honest answer to "by count
*and* by the work they cause", and it is why the path lane keeps its correctness attention
even though it is rare.

### A refused frame reports it too

ADR 0057 carries the same `CoverageSheet` inside `RenderError::ScratchExhausted`, so the
corpus's one capacity refusal now says what it would have cost rather than only which
adapter limit it hit:

> `issue1905.pdf`: the frame's rasterised coverage outgrew the 16384x16384 scratch image
> this adapter allows: a 4763x7103 tile would not fit **a sheet at 14289x15117 holding 6
> tiles and 213115672 texels**

Six tiles, and a fourteen-thousand-pixel sheet. That is §4's picture in one line: the page
has no strokes and no residue clip, its marks simply *are* the page, and nothing about the
path lane's design reaches it.

## 3. What moves the share, and it is not the scale

Between scale 1 and scale 4 the path lane rises by a tenth with the quantum on and
**triples** with it off. Both movements have one cause, and it is not what §1.1 predicted.

§1.1 expects the glyph lane to surrender marks under magnification because a tile
*outgrows what an atlas entry can hold*. Measured, that is **1 mark at scale 1 and 55 at
scale 4 — 55 of 96 588**. What actually happens is the other atlas limit: a page's distinct
keys stop fitting the **budget**, the packer refuses an insert, and the tile falls through
to the sheet. That is 783 marks at 1× and **69 834 at 4×** with the quantum off.

And it is a handful of pages sitting on a threshold, tipping in *both* directions. Nineteen
pages overflow at 4×; turning the quantum on takes the total from 69 834 to 5 355 and
raises the scale-1 total from 783 to 3 459.

| page | path marks at 4×, quantum off → 1/16 | atlas keys |
|---|---|---|
| `issue12295.pdf` | 63 849 → 23 | 66 261 → 66 232 |
| `comments.pdf` | 3 016 → 2 | 4 246 → 3 246 |
| `tracemonkey_a11y.pdf` | 0 → 2 756 | 4 276 → 3 275 |
| `pr12564.pdf` | 18 → 1 430 | 3 626 → 2 684 |

Pages of ~4 000 distinct keys are the population; which of them overflows is unstable, the
mechanism is one. **So the scale-4 quantum-off column is a measurement of the atlas budget,
not of the path lane's design**, and §1.1's stated mechanism for the surrender at zoom —
the tile outgrowing an entry — is right in principle and 0.06 % of the effect in practice.

Under `Coverage::Gpu` the same column reads 11.60 % rather than 33.15 %, because ADR 0029's
placement census keeps single-use tiles out of the atlas and leaves room for the reused
ones. The two mechanisms aim at the same waste; the census gets there first.

## 4. What the path lane actually is

The reasons were taken with a throwaway probe (§6), which counts a mark where the *decision*
is made. Since ADR 0057 a clipped mark's tile is bounded by its chain's own device box, so
more of those decisions end in a mark that reaches no pixel: the reasons sum to 29 744
against a lane of 27 507 at scale 1, an overshoot of **8.1 %** concentrated in eight
pattern, type-3 and shading pages (2 124 of the 2 237). See §11.

Common 948 pages, `Coverage::Cpu`, quantum off:

| why a mark is in the path lane | 1× | share of the lane | 4× |
|---|---:|---:|---:|
| **a stroke** — expansion, which never reaches the atlas | 22 193 | **80.7 %** | 22 117 |
| a **solid fill under a non-rectangular clip residue** | 6 205 | 22.6 % | 6 186 |
| a glyph tile the atlas had **no room** for | 783 | 2.8 % | 69 834 |
| a shading, mesh or §7.10.5 program over a non-rectangular shape | 562 | 2.0 % | 562 |
| a fill whose tile is **too large** for an atlas entry | 1 | 0.004 % | 55 |
| a `Command::Rect` that missed the analytic lane | 0 | 0 % | 0 |

Over the **full** 954-page scale-1 population — which includes `issue12810.pdf` — strokes
are 57 437 of 64 225, **89 %**.

**The path lane is the stroke lane.** §1.6 describes its population as "large fills,
arbitrary transforms, strokes", and of those three the first is one mark in a corpus of
27 507 and the second never appears at all: an oblique transform does not take a fill out
of the glyph lane, because a rotated outline is still an outline the atlas keys and holds.
Only 46 marks are rect-hinted fills that missed the *rectangle* lane for obliqueness; 1 833
missed it for a residue clip.

## 5. How concentrated it is

Not a broad tail. Scale 1, `Coverage::Cpu`, full 954-page population:

- **704 of 954 pages draw no path-lane mark at all.** 250 pages have one or more.
- The **top ten pages are 92.8 %** of every path-lane mark in the corpus.
- `issue12810.pdf` alone — 34 970 sub-pixel strokes in a technical drawing — is **54 %**.
- `bug1978317.pdf`'s 16 384 strokes are a further 26 %.
- Among the 149 pages with at least 100 marks, the **median page is 0.14 % path lane**;
  p90 is 49.9 %, and 13 of the 149 are more than half path-lane. Those thirteen are pattern
  fills, type-3 fonts and drawings, not documents of text.

The coverage is more concentrated still: five pages are **87 %** of the corpus's sheet
texels at scale 1 (`issue1905.pdf` alone is 68 %), and five are 40 % at 4×.

## 6. Method, so a later reader can redo it

Two pieces, neither of them in this tree:

1. **The harness.** `crates/render-quorra/tests/census.rs` in a *scratch copy* of the
   caller's tree: it walks the corpus exactly as their `tests/corpus.rs` does, renders each
   page through quorra alone (no oracle — half the time, and nothing here is a comparison),
   and prints one `COUNT` line per page carrying `Counters`. It needs four small edits to
   their `QuorraRasterizer` — a `last_counters: quorra_gpu::Counters` field, its
   initialiser, `self.last_counters = frame.counters();` after `let frame = rendered?;` in
   `render`, and a `pub fn last_counters(&self)`. **Never in the owner's own checkout:**
   copy it per `HANDOVER.md`'s rsync recipe, append the `[patch]` block, then

   ```
   QUORRA_PROBE=1 PDFVIEWER_QUORRA_SCALE=n PDFVIEWER_QUORRA_COVERAGE=cpu|gpu \
   CARGO_TARGET_DIR=<private> cargo test --release -p render-quorra --test census \
     -- --ignored --nocapture 2> out.txt
   ```

   Everything goes to **stderr**, so the probe's lines interleave with the page markers in
   draw order. 12 s at scale 1, 45 s at scale 4.

2. **The probe**, for §4's reasons only — added to this tree, run, read and **deleted**,
   which is what ADR 0055's round did for the corpus's ramps. It was a private `Probe`
   struct on `Encoder` incremented at the five arms (`fill.rs` where `fill_solid` falls
   through to the sheet, `stroke.rs` past the blend wrap, `rect.rs`'s oblique branch,
   `rare.rs`'s `push_rare_coverage`, and `parallel/commit.rs` at the failed atlas insert),
   printed from `encoded.rs`'s `finish` under `QUORRA_PROBE`. It is not in the tree because
   a per-mark *reason* has to pick one of several true answers in a stated priority order,
   and that is a decision that would need justifying on its own; the lane a mark took does
   not.

**The counters are the tree's, not the probe build's.** The scale-1 and scale-4 CPU runs
were repeated against the probe-free tree at `6ee3072` and every one of their 954 and 948
`COUNT` lines is character-identical to the probe build's. That check also covers the four
`main` commits that landed *during* the rebase (`746fa0b`…`6ee3072`, the `raster.rs` and
`pipeline.rs` splits): they moved no counter on any page, which is what a split has to
prove rather than assert.

### The one `Counters` field this round adds, and why it is permanent

`Counters::lanes` is additive public API on a bump that already carries several
(`atlas_working_set_bytes`, `atlas_repacked`, `coverage`, `ResourceIdsExhausted`).

It exists because §1.1 is the library's central claim and had no instrument: a page that
disagrees with it could only be found by reading an encoder. It is counts and not a rate —
the denominator a question wants is the reader's choice — and it is counted at the two
seams where a mark becomes drawable (`instance.rs` and `plan.rs`'s `append_op`) rather than
at the five arms that *choose* a lane, because a choice can still change afterwards: a
glyph tile the atlas refuses falls through to the sheet at commit, and §3 says that is
almost all of the movement at 4×.

**No second coverage counter was added.** An earlier draft of this round carried a
`Counters::coverage_texels`; ADR 0057's `CoverageSheet::texels` is the same quantity,
better defined — it is the sheet, so it excludes the atlas tiles the draft's version
included — and it is reported by a refused frame as well as a drawn one. Two public fields
for one number is worse than none. The instrument debt `HANDOVER.md` used to record here
("no field for what a frame's coverage costs") was closed by ADR 0057, not by this round.

One internal change did come with the lane counter: `push_quad_instance`'s `source: f32` is
now a `CoverageSource` enum. The float was the same fact twice — the shader's texture
selector and the lane — and a float parameter can carry a third value that means nothing to
either.

## 7. What this means for §1.1 and §1.6

**§1.1's premise survives, by measurement, for the first time.** The number is 9.4 % at
scale 1 and 11.0 % at 4×, on the caller's own corpus, at the caller's own configuration.
The section should be rewritten with the number in it — §8 has the text — and the
"Overturned by" paragraph becomes a recorded answer rather than a standing condition.

**§1.6 is answered, and by a population it did not predict.** Its three candidates are
chosen by which of the path lane's reasons dominates, and the answer is *strokes* — 81 % of
the lane, 89 % on the wider population:

- **Candidate 1, tile-binned compute à la Vello: not justified.** It exists to scale the
  hard case, and the hard case is a tenth of the marks concentrated in 250 pages of 954,
  four fifths of it geometry that has already been flattened and expanded into polygons on
  the host before any lane sees it. Bringing a general binner to that is §6.1's finding
  restated: machinery for a case this corpus does not have.
- **Candidate 2, CPU flattening at encode with GPU coverage, is what the lane already
  is** (ADR 0008 for the host rasteriser, ADR 0016 for the device one, ADR 0026 for which
  of the two a mark takes) — and the census says it was the right shape: a stroke arrives
  as polygons because §4.5 resolved its width upstream and our expansion is caps, joins and
  miters, so the flattening cost the candidate was worried about is already paid for the
  dominant case.
- **Candidate 3, stencil-then-cover with multisample, stays refused** on the ground it was
  refused on: its coverage quantisation risks the oracle's bound. Nothing in the census
  argues for reopening it, and §1.3's rule — coverage is a first-class value at the moment
  of composite — outranks it anyway.

**ADR 0008's lever stays where it is, and its two triggers are answered separately.** It is
overturned by "§11.2's census showing the path lane is hot on real corpora" — it is not, at
a tenth of the marks — "or a page profile where per-frame CPU rasterisation dominates". The
second is a *timing* claim about a page and this census is a count, so it is not answered
here; what the counts contribute is that the pages where it could be true are nameable, few,
and now named (§5), and that `Coverage::Gpu` already exists as the recorded answer for them.
The lever is a compute-shader coverage pass, which is candidate 1 above, and the population
argument against it is the same one.

§1.6's own open question, "whether hairline strokes deserve their own primitive", now has
its population: **the census's stroke population is the path lane**, and the two pages that
carry most of it (`issue12810.pdf`, `bug1978317.pdf`) are pages of sub-pixel rules. That is
a real candidate for a round, and it is a *stroke* question rather than a path-lane-design
question.

**And the census points somewhere else than either section did.** The largest lane movement
anywhere in these numbers is the glyph lane surrendering 69 834 marks at 4× to an atlas
*budget*, on nineteen pages, reversible by turning the quantum on. That is ADR 0024's
recency question, which `HANDOVER.md` records as "waiting for its measurement since" — this
is a measurement it can be reopened with.

## 8. Recommended edits to `PLAN.md` (not made here — the owner maintains it)

Replace §1.1's closing paragraph:

> **Answered by:** §11.2's census, run on 2026-08-17 over the caller's 974-document corpus
> at scales 1, 2 and 4 (`doc/notes-census.md`). **The premise holds: 9.4 % of a real
> corpus's marks miss the glyph and rectangle lanes at a page's own scale, and 11.0 % at
> four times it** — 89.1 % are glyph or rectangle, 1.5 % are images. The path lane's
> population is not what this table assumed: **81 % of it is strokes**, 23 % is fills under
> a non-rectangular clip residue, and exactly **one mark of 27 507** is there because a fill
> was too large for an atlas entry. It is also concentrated rather than spread — 704 of 954
> pages draw no path-lane mark at all and ten pages carry 93 % of them. The one place the
> lane is not rare is the work it causes: **at least two thirds of the coverage texels this
> corpus rasterises are made for that tenth of its marks**, which is why the lane keeps its
> correctness attention and does not get the atlas's engineering attention.

Replace the path lane's row in §1.1's table:

> | **path** | everything else: strokes above all — **81 % of the lane, measured** — plus
> fills under a non-rectangular clip residue, glyph tiles the atlas has no room for, and
> non-solid paints over a non-rectangular shape. **9.4 % of a real corpus's marks at scale
> 1, 11.0 % at 4× (§11.2's census, `doc/notes-census.md`)** | the general coverage path
> (§1.6) |

Replace §1.6's opening sentence and add a verdict:

> ## 1.6 The path lane: the census is in, and it chose candidate 2
>
> The census §11.2 asked for has run (`doc/notes-census.md`): the lane is 9.4 % of a real
> corpus's marks and **81 % of it is strokes**, which arrive already expanded into polygons
> because §4.5 resolved their widths upstream. Of the three candidates below, that is
> candidate 2 — which is what ADRs 0008, 0016 and 0026 built — and it retires candidate 1
> with a number rather than an argument: a general tile binner exists for a hard case this
> corpus reaches with a tenth of its marks on a quarter of its pages, four fifths of which
> are not curves by the time a lane sees them. Candidate 3 stays refused on the oracle
> bound.

Replace "Where we are"'s last open bullet:

> - **§11.2's census has run** (2026-08-17, `doc/notes-census.md`): 9.4 % of the caller's
>   corpus's marks miss the glyph and rectangle lanes at scale 1 and 11.0 % at 4×, so
>   §1.1's premise is confirmed by measurement for the first time and §1.6's shortlist is
>   settled on candidate 2. One additive `Counters` field lands with it — `lanes` — and the
>   finding that was not asked for is that the largest lane movement under magnification is
>   the glyph lane losing marks to the atlas *budget*, not to tile size, which reopens
>   ADR 0024's recency question with a number.

And in M5's entry, replace "The census (§11.2) has not run — it needs corpus fixtures from
the caller's tree — and stays the recorded condition for revisiting the lane design
(ADR 0008)":

> The census (§11.2) **ran on 2026-08-17**, at our own encoder rather than from a serialised
> fixture — the swap made that possible, and it is the only place device-space lane choice
> can be counted. `doc/notes-census.md` has the numbers; ADR 0008's lever was not pulled,
> because the lane the census describes is the lane that was built.

## 9. Recommended edit to `HANDOVER.md`

In "Recorded and deliberately not taken", the ADR 0029 line stays as it is — the blind spot
is real (§10). Add to "Instruments":

> - **Which lane a page's marks take**: `Counters::lanes` per frame, and
>   `doc/notes-census.md` §6 for the corpus-scale harness — a `census.rs` in a scratch copy
>   of the caller's tree, four small edits to their `QuorraRasterizer`, 12 s at scale 1.
>   **Take it at more than one scale and say which**, and say whether the glyph quantum was
>   on: at 4× it moves the path share more than the scale does. Read the work a lane causes
>   from ADR 0057's `Counters::coverage`, not from a second counter.

## 10. What this census cannot see

- **ADR 0029's recorded blind spot.** The *placement* census inside the encoder counts by
  outline, linear transform and fill rule, deliberately **not** by sub-pixel phase, so it
  is an upper bound on reuse. It is read only under `Coverage::Gpu`, so it does not touch
  the CPU rows above at all — those are the headline. On the GPU rows it makes the path
  lane a slight *under*count: placements the loose count thinks are reused, but which are
  really one key each at distinct phases, stay in the glyph lane instead of being diverted
  to the device. The direction is the one ADR 0029 chose and states.
- **First pages only**, one per document, which is the caller's own gate population.
- **The atlas persists across the run**, as it does in the product. A page's overflow count
  is therefore not purely a property of that page. §3's tipping pages are close enough to
  the budget that the ordering could matter; the two largest (`issue12295.pdf` at 66 261
  keys, `bug1978317.pdf`) are not close, they are simply past it.
- **Nothing here is a clock.** Every number is a count and load cannot touch it, which is
  why the runs were taken on a machine at load average 11–26 without apology. What a lane
  *costs* per mark is `tests/lane_crossover.rs`'s subject and it is a property of the
  processor and the adapter together (`HANDOVER.md`'s trap), never publishable as a
  constant.
- **Marks, not commands.** A command that draws nothing takes no lane; the corpus culled
  none at these scales, but a viewer at 20× hands over a whole page for a fortieth of it and
  `commands_culled` is the counter that accounts for the difference.

## 11. What the rebase onto `main` moved, and what it did not

The census was first taken against `eada81e`; `main` had moved about twenty commits, so
**every row was re-taken** rather than any of them being asserted unchanged. That was the
right call, because the scale-1 rows did move and the reason is instructive.

- **The population gained a page.** ADR 0057 bounds a clipped mark's coverage tile by its
  clip chain's own device box, and `bug1703683_page2_reduced.pdf` went from refused to drawn
  at 4×: 149 path-lane marks over **2 515 557** sheet texels, where the pre-ADR page asked
  for 1 008 561 911. It is not a neutral addition to a path-lane census — its 278 shading
  marks and 141 residue regions are exactly the second-largest population in §4 — which is
  why the common population is 948 rather than 947 and every row was recomputed on it.
- **The scale-1 path lane fell, and the probe did not.** Full population, scale 1:
  65 891 → **64 225** path-lane marks. Every probe number is *identical* to the pre-rebase
  run — `stroke_path` 57 437, `fill_path` 6 255, `glyph_overflow_path` 2 208 — which pins
  the cause exactly: **ADR 0057 does not change which lane a mark is sorted into, it changes
  how many of those decisions become a mark at all.** A tile bounded by its chain's box can
  now be bounded to nothing, and `pattern_text_embedded_font.pdf` (1 222 → 629),
  `issue2177.pdf` (919 → 379) and `issue6961.pdf` (542 → 298) are where it happened. That is
  also why §4's reason-versus-lane overshoot grew from 1.4 % to 8.1 %.
- **The "by work" rows are on a different quantity and are not comparable to the draft's.**
  ADR 0057's `coverage.texels` is the sheet; the draft's own field also counted the tiles a
  frame rasterised *into the atlas*. The sheet number is the better one and the
  decomposition is simpler for it — see §2.
- **Two `main` fixes touch the rasteriser** (`raster::direction`'s `hypot` overflow path and
  `accumulate_edge`'s non-finite early return, `doc/notes-ceilings-audit.md`). Neither can
  move a lane count: both are inside coverage arithmetic, downstream of every decision this
  census reads.
- **`crates/quorra-pages` (ADR 0060) is not touched by this census**, which never renders an
  archetype.

Every finding survived: the premise at 9.4 %, the inversion by work, the stroke-dominated
population, the atlas-budget surrender at 4×, and the concentration. The numbers moved in
the third significant figure and the conclusions did not move at all.

## 12. What was deliberately not done

- **No ADR.** The census confirms the design rather than changing it, which is the case the
  round was told makes an ADR unnecessary. `Counters::lanes` is documented public API
  justified in §6 rather than a decision with a cost to write down, and the one decision the
  census *would* have forced — pulling ADR 0008's lever — was not reached.
- **No fixture of serialised display lists.** M5 asked for one so that the two trees stayed
  independent; the swap removed the reason. Counting at our encoder is both closer to the
  question (lane choice is device-space) and free of `doc/corpus-profile.md`'s licensing
  problem, since nothing about a page leaves the run but its counts.
- **The `path` lane is not split into its host and device halves.** Under `Coverage::Gpu` a
  path-lane mark may have been rasterised by the CPU or drawn by ADR 0016's winding lane;
  both put one tile on one sheet, which is why they are one lane here. A frame's GPU tiles
  are `winding.tiles.len()` internally and no counter publishes them. If a later round wants
  that split it is one more field, and it should be asked for by a question rather than
  added because it is possible.
- **The corpus fidelity gate was not re-run.** Nothing in this round can move a pixel: the
  counter is read-only and the enum change is the same two floats. What was re-run instead
  is the census itself against the probe-free tree, twice, and it is character-identical
  per page — which is a stronger statement about *these* numbers than a fidelity run would
  have been.

`cargo test --workspace` is green at 562 listed tests against `main`'s 554 (558 `#[test]`
attributes plus four doctests; the eight added are this round's, and no name changed).
`tests/lane_census.rs` was verified able to fail in two directions: swapping the atlas and
sheet lanes fails five of its eight, and counting an image quad as a rectangle fails two.
