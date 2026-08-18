# Handover

Read `CLAUDE.md` first, then `doc/RENDER_LIBRARY.md` (the brief this library exists to
satisfy) and `doc/PLAN.md` (the design, and what is true today). **`PLAN.md`'s "Where we
are" carries the current numbers; `doc/history/` carries how they got there; this file
carries what to do next and the traps.** A lesson belongs in exactly one place: in an ADR
if it is a decision, in `PLAN.md` if it is where we are, in `doc/history/` if it is what
happened, here if it changes how you *work*.

## State of play

Nine milestones are done; the swap landed on 2026-08-03 and the caller consumes this
library as a git dependency, pinned by their `Cargo.lock`.

**The 2026-08-16/17 improvement rounds are merged and unpushed.** Fifteen rounds in parallel
worktrees: ADR 0057 (a clipped mark's tile is bounded by its chain's box — one corpus page
refused → agrees), ADR 0058 (a present layer draws its own rectangle), ADR 0059 (a gate over
a private list lives inside the crate), ADR 0060 (a page has one definition and every example
is run), ADR 0061 and ADR 0062 (the `raster.rs` and `pipeline.rs` splits, and how a test is
filed), the `error.rs` split, ADR 0023's amendment (the residue multiply is geometry, not
recording), the archetype re-cut, the caller's `HAYRO_ISSUES_FOR_QUORRA.md` answered in full,
and **§11.2's census, open since M5**. The suite went **445 → 551** tests, every new gate
verified able to fail.

**§11.2 is answered and §1.1's premise survives** — 9.4 % of marks miss the glyph and
rectangle lanes at a page's own scale, and 81 % of what does is strokes, so §1.6's candidate 2
is confirmed by the population rather than chosen ahead of it. The finding the plan did not
anticipate: **by work it inverts**, the sheet being at least 66 % of rasterised coverage at 1×.
`PLAN.md` §1.1, §1.6 and §1.10 carry it; `doc/notes-census.md` is the record.

**Six defects that no test in the tree could see**, and they are the reason the rounds were
worth running rather than the features:

- **`raster::direction` overflowed above `1.9e19`** — eight orders below the contract's own
  `1e27` device delta — to an infinite length, a `(0,0)` normal and **a stroke drawn as
  nothing**; below `1.1e-22` it underflowed to NaN geometry, because `stroke_polylines`
  dedupes by testing coordinates for *equality* and two points `1e-30` apart pass that.
- **`raster::accumulate_edge`'s slope overflowed** for a wide, vertically thin edge to a NaN
  that survived the prefix sum into a solid row. See the NaN trap below; this is the one that
  drew a plausible wrong page.
- **`tests/shader_copies.rs` named 8 shaders where `src/shaders.rs` names 10.** Missing were
  `present.wgsl` and `function_lane.wgsl` — and `function_lane.wgsl` carries the sixth
  `soft_mask_value` and the sameness promise. The gate compared five, asserted five, found
  five and passed, on every run since ADR 0053.
- **The archetype fixture's curve clips met 0 of 40 and 8 of 600 of the marks they clipped**,
  so the signature looked like it gated the residue lane for two ADRs.

**Three claims in our own documents were withdrawn or corrected**: `recording` being 56 % the
residue multiply on a clipped page (re-measured: **0.62 % of the encode**, and the old figure
was taken on the fixture above *and* counted a bounding scan that is recording); ADR 0049's
37.8 → 28.9 ms as a *demonstration* (mechanism and saving stand); and three citations of
§11.4.5 for a group's constant alpha, which is §11.6.4.4's in §11.3.7.2's range.

**What needs the owner, not another round:** the caller must drop
`bug1703683_page2_reduced.pdf` from their scale-4 `REFUSED` ratchet — and, in doing so, note
that **`REFUSED_AT_FOUR` holds two kinds of refusal under one name** (their finding,
2026-08-18): `issue1905.pdf` refuses at *render* with `RenderError::ScratchExhausted`, while
the other two refuse in *translation*, before a scene exists at all. The two are different
types on this side and the ratchet flattens them.
`issue1905.pdf` is **answered** — gate only — and the `AIS` question is **settled** by
ADR 0066 from the clause rather than by them; two clause corrections go back to
their `HAYRO_ISSUES_FOR_QUORRA.md` (`doc/notes-hayro-coverage-map.md` has both). **ADR 0058's open number is delivered**: the present pass is 4.4 % of a refresh (2026-08-17).

**The 2026-08-15 release round is pushed** (`a64a908`): ADR 0047 (a document's rectangles
reach the rectangle lane), ADR 0048 (`RetainedScene`), ADR 0049 (a clip chain's residue
rasterised once, and the border-cut defect under it), ADR 0050 (a page too large for its
atlas stops re-encoding itself every frame), ADR 0051 (three files split along their seams),
ADR 0052 (the readback gate counts instead of timing), and one clause defect: a blended
stroke inside a knockout group was blended where §11.4.6 replaces.

**Everything through `a4380e2` is pushed** — the owner pushed it on 2026-08-16 at 00:14, and
`.git/logs/refs/remotes/origin/main` carries the four pushes that got there:
`6ed67f0 → a64a908 → 05fadc5 → 619ef3b → a4380e2`. That is the function paint (ADR 0053),
ADR 0054's parallel geometry phase, the `device.rs` split, the `encode.rs` split, ADR 0055
and the debt round. **The `AI` user cannot check this against the remote** — it has no key for
`git@github.com:close2/quorra.git` — so read the reflog, not `git ls-remote`, and never write
"unpushed" here from a failed fetch. An earlier version of this paragraph said two rounds were
unpushed when both had reached the remote; the caller's own `Cargo.lock` disproved it, since
cargo cannot resolve a git rev it could not fetch.

**The caller pins `619ef3b`**, one round back, in their `Cargo.lock`
(`git+https://github.com/close2/quorra#619ef3b4…`, no `[patch]`). Their tree builds against
`a4380e2` unmodified — verified, at 24 encode threads — and **cannot go back**: `a64a908`
fails their adapter with 15 compile errors across four call sites (`Paint::Function`, `FnOp`
/`FnRange`/`FunctionId`, `Device::upload_function` + `function::admit`, and
`Options::encode_threads`). The bump is not optional for them, and the release note should
name those four.

**The 2026-08-15 debt round** closed the small debts this file listed, in three worktrees at
once: a generated function shader's compile cost measured here (`examples/function_compile.rs`),
the workspace's eleven rustdoc warnings fixed *and gated*, a uniform's byte offsets checked
against the WGSL struct they mirror (`src/shaders/layout.rs`), `device.rs`'s three leftovers
closed, six clause-derived unit tests on `device/ramp.rs`, and `tests/` reorganised —
`retained_frame.rs`'s 1 139 lines into five files along its own seams, plus `tests/common/`.
It moved no pixel and needed no corpus run. **It also found one clause defect that does move
pixels** — item 3 below.

**One thing the release round owes the caller and they have not done yet.** Their corpus
ratchet lists the pages that differ from their oracle by name, and ADR 0049 moved
`issue2177.pdf` onto the oracle — so their `every_corpus_page_agrees_with_the_cpu_oracle`
**fails on the pushed commit** until they re-baseline that list. The verdict counts are
right (931/23/2/18) and every other page line is identical; the ratchet is doing its job.
`pdf-viewer/doc/QUORRA_API_2026_08_15.md` §0 tells them to expect it.

**The round is corpus-clean and the bump is ready.** Base against merged, one copy of their
tree, one hour, both coverage lanes: scale 1 goes 930/24 → 931/23 on the CPU lane and
928/26 → 929/25 on the GPU lane, scale 4 is unmoved at 936/10/5/23, and exactly two page
lines change out of 956 with every other line identical to the character. `PLAN.md` carries
the matrix.

**And they are already blocked on it.** Their working tree uses `RetainedScene` and
`EncodeSource`, which do not exist at the `87898c6` their own `Cargo.lock` pins — the corpus
gate cannot even be built against their pin. Their adoption of ADR 0048 is done and waiting
on a push from this side, which reverses who is holding the round up.

**What the caller must do to take it** is written for them in
`/home/cl/projects/pdf-viewer/doc/QUORRA_UPGRADE.md` and, for the retained encode,
`QUORRA_RETAINED_FRAME.md`. Their `doc/QUORRA_FEEDBACK.md` is the other half of that
conversation and is worth reading before answering anything in it: a heading there can be
stale in either direction, and §13 sat marked *open* for eleven of their rounds after
ADR 0023 had answered it.

## The caller's non-blocking-render ask, answered on 2026-08-16

Their `doc/QUORRA_NONBLOCKING_RENDER.md` asked for the surface to stop being the device's
business, so that a picture can be presented while a page is being drawn. **The answer is yes
and it is built (ADR 0056)**, and the measurement half of the same document (§9, `recording`)
is `doc/notes-recording-shares.md`. **`doc/answer-nonblocking-render.md` is the single reply
for the owner to carry across — we never edit their tree.**

`Device::detach_presenter` hands the surface, its swapchain and one pipeline to a `Send`
`Presenter`; `attach_presenter` refuses a presenter from another device by ADR 0048's device
id and **hands it back inside the refusal**, because consuming it on the failing path would
destroy a window's surface over a caller's mix-up. While the presenter is out,
`Target::Surface` is refused as `RenderError::PresenterDetached` — a different word with a
different fix from `NoSurface`. `Presenter::present(&[Layer])` draws each finished raster
under its own `Affine` and `ImageFilter` through `Kind::Present` / `present.wgsl`, which
joins the warm set of every surface device, so detaching compiles nothing.

**Two things this round could not settle, and neither is a defect in the design.** Whether the
arrangement holds 60 or 120 Hz **cannot be observed on this machine** — `Xvfb` reports a
refresh of 0.00 and `--newmode` does not take — so that number is the owner's, on the display
that states its own refresh, behind the caller's ADR 0383 trace lines. And a layer texture
from a device of a *different* `wgpu::Instance` still panics inside wgpu-core rather than
being refused: ids are per-instance, so no scope sees it. That hole is pre-existing —
`Target::Texture` shares it — and the remedy is one instance per process, which is documented
in the error type, the ADR and the reply.

**What §9's measurement says, because it changes what is worth doing next.** `recording` on a
path-heavy page is **56 % one thing nobody had named** — computing each mark's device
bounding box — and ADR 0045's memo is pure cost there, because that page places each of its
58 009 outlines exactly once. It *is* divisible by a parallel pre-pass, but that is worth
**1.31× on the encode, not the 6.6× geometry gave**, and it needs an ADR, a distinctness
floor and a charged allocation before anyone starts. **The floor is the finding**: with the
whole of `encode` at zero their frame is still 107.0 ms and 12.8 refreshes at 120 Hz, so
nothing in phase one is what stands between that page and the rate they want — which is
exactly why the presenter split was the right thing to ask for. One live defect fell out of
it: on a *clipped* page the geometry clock **mislabels** the residue multiply, so 56 % of
artwork's reported `recording` is per-pixel geometry outside the span.

## What to do next, in this order

Two items, and the ordering reason is one sentence: **the caller conversation is the older
problem and costs an afternoon, while the tiling seam is the only work left with
milliseconds in it and wants its own measurement before its own design.** A third item — the
ramp's subdomain boundary — closed as ADR 0055 and is kept below for one round because of
how it closed.

### 1. The adoption round with the caller — cheap, and overdue

Three sections of their `QUORRA_FEEDBACK.md` wait on this side, and **all three are
drafted** in `doc/feedback-answers-draft.md` — a draft for the owner to carry across, since
we never edit their tree:

- **§15** asks whether the coverage lane bounds a clipped fill by its clip's extent, and
  offers to close itself if yes. It is yes: every coverage tile is `shape ∩ clip ∩ target`
  (`encode.rs` `visible_tile` and `coverage_tile`, `encode/clips.rs` `residue_intersection`,
  `encode/rare.rs` for the image lane). Its secondary ask — can the scene say "this fill is
  bounded by this rectangle" — is the same fact: a rectangular clip collapses to a device
  rectangle and never becomes a mask, so the clip already *is* the bound. The draft
  cross-references item 2, because §15 and the tiling ceiling are one seam seen from two
  sides.
- **§19** is measured (`examples/rect_lane.rs`). Median page: microseconds, so nothing. A
  page of thousands of rectangles: 0.6–2.1 ms. Which of those their corpus actually has is
  a row their profile does not carry, and the draft asks for it before either side writes a
  recogniser. ADR 0047 has since made the ask worth a third of what it was.
- **§22.5** (a process note): the recommendation is *document*, not rename — `layer_textures`
  always meant what it still means, and what was wrong was a derived claim in our own
  rustdoc that theirs inherited. The rule the draft states: rename when the *unit* changes,
  date a correction when a derived claim does, name the counter in the ADR that moves its
  value.

Deliver the answers and let their corpus re-baseline absorb the twenty commits in one round.
The draft's release-note section doubles as what the bump delivers.

### 1b. The function paint — **built**; what is left of it

ADR 0053 is accepted and the lane draws (`src/function/`, `src/pipeline/function.rs`,
`src/encode/function.rs`, `src/compose/function.rs`). Read its **amendment** before quoting
its §3: `Agreement::Exact` was renamed `Bounded` because WGSL permits reassociation and
fusion, so bit-exactness was never ours to promise. The caller's answer document carries the
same correction.

Three of the four things this item listed are done. `tests/function_knockout.rs` draws
§11.4.6's replacement over a function paint and measures it in the pixels against an ordinary
group as control, with both of its gates verified able to fail by forcing the defect each
names; `tests/function_retained.rs` replays a function op byte-identically and holds the
release path, which is where principle 6 has teeth in this lane; and **the compile now has a
number of ours** — `examples/function_compile.rs`, 8.25 ms on RADV and 6.88 on llvmpipe for a
482-instruction program at the witness's length, minima of twelve round-robin rounds over
three alternating runs per adapter at load 5.1–8.6. The spike's 6.3 ms holds. The finding
worth carrying: **a one-instruction program still costs 2.0–2.7 ms**, which is
`function_lane.wgsl` parsed and built rather than anything generated — so the fixed half is
where to look if this ever needs to be cheaper, and the cost is mildly superlinear above it
(~10 → ~17 µs per instruction, the same shape on two independent compilers).
`doc/notes-function-wiring.md` §4.6–§4.8 carry all three.

What is left is `gt`/`ge`/`lt`/`le` comparing a boolean numerically where PLRM3 raises
`typecheck`, which is ADR 0053 §3.2's open question with the caller and theirs to answer;
and three gaps §4.5 now names, each found by reading rather than by a failure: **a clip and a
soft mask over this paint are never anything but 1** anywhere in the tree (the other two
factors of `base_weight` are unobserved), **no function test runs `Coverage::Gpu`**, and
ADR 0025's `DestOut`/`Plus` stages are compiled and selected but never drawn.

Do not build the interpreter shape. It is 133 ms against 0.060, it costs 596 ms–4.5 s of
cold compile against 6.3 ms, and at 4× it lost the device.

### 2. The tiling seam — **taken** (ADR 0057); one page and one question are what is left

*(This was **item 5** while three finished items still stood above it, and ADR 0048 and
`doc/feedback-answers-draft.md` both cite it under that number. What follows the horizontal
rule is the item as it stood before it was taken, kept because its measurements retired three
candidate schemes with numbers rather than with argument.)*

**Built and corpus-clean, 2026-08-17.** A clipped mark's coverage tile is bounded by its
chain's own device box — the links' control hulls, taken from `HullMemo` at the moment the
chain is resolved, so no flattening, no second pass, and **nothing paid by a page with no
residue clip**. `bug1703683_page2_reduced.pdf` goes from **refused to agreeing with the
oracle** at 4× on both lanes (CPU 936/11/4/23 → 937/11/3/23, GPU 937/10/4/23 → 938/10/3/23);
zero page lines move at scale 1 in either lane, and one moves at scale 4 — `inks.pdf` on the
GPU lane by a hundred-thousandth of SSIM, its mean and worst tile unchanged.

**Both instrument debts are paid**, through one additive type: `CoverageSheet`, reported as
`Counters::coverage` on a drawn frame and inside
`RenderError::ScratchExhausted { limit, sheet, tile_width, tile_height }` on a refused one.
Attaching a whole `Counters` to a refusal was considered and refused — a `Counters` from a
half-finished walk is numbers about a frame that does not exist, and principle 6 does not
weaken when the frame is an error.

**What is left, in order:**

- **`issue1905.pdf` at 4× refuses only in the gate — answered, and the seam is retired.**
  The caller measured it on the real adapter (2026-08-18): the whole page at 4× outgrows the
  16 384² scratch image, and **every window frame draws, even at 64× zoom**. A viewer's
  viewport is its window, so the refusing frame is a shape only the gate constructs. **This
  half of the tiling seam has no product behind it**; reopen it only if a caller appears that
  renders a whole page at once. The question was worth asking rather than assuming — the
  answer went the other way from the arithmetic's implication.
- **The caller must re-baseline their scale-4 `REFUSED` list**, dropping
  `bug1703683_page2_reduced.pdf`. Their ratchet fails loudly with both lists printed, which
  is it doing its job.

---

### 2 (as it stood). A page-sized coverage tile per clipped shape — **not** multi-sheet passes

*(This was **item 5** while three finished items still stood above it, and ADR 0048 and
`doc/feedback-answers-draft.md` both cite it under that number.)*

**Measured on 2026-08-15 — `doc/notes-tiling-ceiling.md`, and it changes what this item is.**
The two pages are **two different problems**, and both refuse from **2×** upward rather than
only at 4×:

- `bug1703683_page2_reduced` refuses because **a residue clip does not bound the tile its
  mark asks for**. 31 of its 34 tiles are the whole page at 4× — 7 755 264 texels each —
  while 31 of the 33 seated have an *open* clip rectangle, and the page asks for
  1 008 561 911 texels where its own chains admit 2 297 897 (**439×**). Median chain box:
  99 texels.
- `issue1905` refuses because **its marks are the page**: seven fills wider than the page
  under a rectangular clip that already bounds them, 1 339 315 879 texels, no residue clip
  anywhere.

Both want 3.8× and 5.0× the frame budget, so **no sheet-side scheme draws either** — the
second sheet, a pane cut and a tighter packer all move the refusal to `FrameBudgetExceeded`,
and the packer is already at **98.6 % and 97.4 % occupancy** when each refuses, so ≤2.6 % of
slack stands against overshoots of 17 % and 37 %. That retires three of the five candidates
with numbers rather than with argument.

**What does work is bounding a clipped mark's tile by its chain's own device box** — the box
`chain_region` already computes and currently uses only to price ADR 0049's cache. Measured
on the corpus in one copy: `bug1703683_page2_reduced` goes from **refused to agreeing with
the oracle** at 4× (936/11/4/23 → 937/11/3/23), its coverage from 1 008 561 911 to 2 511 363
texels (**402×**), every other page line identical to the character at both scales, no second
pass over the commands, and **nothing paid by a page with no residue clip**. It buys
`issue1905` nothing, which is the honest half — that page needs a different answer, and
before spending a round on it, ask the caller whether it refuses in the product or only in
the gate: the frame that refuses is a whole page at 4× in one target, and a viewer's viewport
is its window.

**Two instrument debts fall out of the measurement**, and both are the reason it cost a
patched crate to obtain: `ScratchExhausted { limit }` names the adapter's wall and nothing
about the frame that hit it, and **a refused frame has no `Counters` at all**; and `Counters`
has no field for what a frame's coverage costs, so a 402× reduction moves no row of
`tests/archetypes.rs`. `crates/quorra-gpu/tests/tiling_ceiling.rs` holds both findings in
public API, both verified able to fail.

Two pages at 4× refuse with `ScratchExhausted` — `bug1703683_page2_reduced.pdf` and
`issue1905.pdf`, the coverage sheet against the adapter's 16 384 limit, a different ceiling
from the frame budget. **It is the only *budget* that refuses a frame we could otherwise
draw.** The other three refusals of their corpus are each something else: 548 MB of resident
images against `max_resource_bytes` (`22060_A1_01_Plans.pdf`, at upload rather than at the
frame), and two clause refusals that are correct — a four-component blending colour space
and a non-isolated knockout group. Counted on 2026-08-15; the earlier "three at 4× and one
at scale 1, the only reason any frame is refused" was their tree at an earlier revision,
and was too strong in both halves.

This item used to read "a frame would have to use more than one sheet". **That was measured
and refused**: the pages have placed 194, 240 and 253 MB of tiles against a 268 MiB budget
when they run out of *height*, and the tile that does not fit is 2 448 × 3 168 or
4 763 × 7 204. A second sheet takes `bug1721218_reduced` to 287 013 092 bytes — it refuses
again, on bytes — and lets the other two draw at a quarter of a gigabyte of per-frame
coverage upload each. That is a page "drawn" at a cost §6.2 would call a failure.

**ADR 0049 took the other half of this item and left this one untouched, deliberately.**
The residue is no longer re-rasterised per command — that was 17.3 ms of artwork's 65.6 ms
of geometry, *not* the whole of the "35 ms of a 43 ms frame" this item used to claim — and
artwork's encode geometry went 37.8 → 28.9 ms. But a *region* is host memory that never
reaches the sheet, and `Counters::tiles` is unchanged on every archetype, which is the
evidence that no refusal moved.

So the ceiling that bites is not the sheet's dimension: it is that a clipped shape becomes
one coverage tile of its own device bounds, and at 4× a full-page clipped shape is a
full-page tile. The work is on the *tiling* side — ADR 0028's panes are the nearest existing
mechanism.

*(A paragraph filed here since ADR 0048 — a page whose glyph tiles overflow the atlas
re-encodes on every frame — has been **removed rather than moved**. ADR 0050 did it, and
it was never this seam: the glyph atlas has nothing to do with residue-clip tiling. The
claim was also too broad, which is its own lesson and is now a trap below.)*

### 3. A ramp's subdomain boundary — **closed, ADR 0055**

Kept here for one round because of *how* it closed, not that it did. §7.10.4's subdomains
are "half-open intervals, closed on the left and open on the right", so `ramp_color_at` now
compares with `<` and a bound belongs to the subfunction that starts there.

**The item as this file first stated it was half wrong, and reading the clause instead of
extending its pattern is what caught that.** The clause states *two exceptions*, and they
point in opposite directions: the last interval is always closed on the right, so a
coincident pair at offset 1 takes the later colour — but where `Domain0 = Bounds0` the first
interval is closed on **both** sides and the second opens on the left, so a coincident pair
at offset 0 takes the **earlier** colour, and `t <= first.offset` was already right. Only the
interior bound and the last offset were wrong. That the assertion at a ramp's *first* offset
still passes under the old code is the evidence, in the pixels, that the exception at 0 is
real rather than inferred.

The corpus, base against change in one copy of their tree within one hour, both lanes, both
scales: **no verdict and no refusal moves, and exactly one page line of 956 changes** —
`issue10572.pdf` at scale 4, mean 0.1332 → 0.1036 and SSIM 0.99497 → 0.99602, toward the
oracle, worst tile unchanged. A throwaway probe (run, read, deleted) counted **411 corpus
ramps, 2 with a coincident bound, 2 bounds landing on the sampling grid**, both in that one
page's 48-stop ramp — which is what makes the scale-1 null result a measurement rather than
an absence.

One more instance of the "same copy, same hour" trap came free: the scale-4 base read
936/11/4/23 (CPU) and 937/10/4/23 (GPU) against the 936/10/5/23 `PLAN.md` carried, because
their tree moved a page from *refused* to *differs* under us. Nothing regressed.

### Small debts, none blocking

- **`error.rs` is split** — a map over seven private modules named for the subsystem that
  raises each refusal (`doc/notes-error-split.md`, ADR 0051's shape). It moved no behaviour:
  the same test names before and after, archetype counters identical, and the seven bodies
  diff against the old file in five hunks that are all doc text. The decline was argued
  first and lost to three facts: five of its seven vocabularies are raised in exactly one
  subsystem each, its module comment was already eight lines of map, and three commits had
  added a whole vocabulary at a stroke.
- **The files past the ~500-line smell, with the unit stated: `wc -l` over
  `crates/**/src/**.rs`, whole file including its test module, because that is what a
  reviewer holds.** This is the bullet's fifth statement; **each of the previous four named
  files that were not the largest ones**, so read the table rather than a remembered
  sentence, and re-measure rather than quoting it. Measured 2026-08-17 after
  `doc/notes-final-splits.md`:

  | file | lines | state |
  |---|---|---|
  | `resources.rs` | 635 | **declined**, with the evidence in its module comment: the candidate seam would cut `charge` and `allocate_id` from the only callers they have |
  | `atlas.rs` | 567 | untouched; a round was in it on 2026-08-17 |
  | `encode/parallel.rs` | 559 | **declined**, with the evidence in its module comment: `rasterise` is the join rather than a member of either half, and no test is a statement about a `Job` alone |
  | `frame.rs` | 537 | untouched, and the only one nobody has read for this purpose |

  Nothing else in the workspace reaches 490. `error.rs` (68 over seven modules), `raster.rs`
  (61 over three), `pipeline.rs` (428 over five), `encode.rs`, `device.rs`, `geom.rs` (three
  ways) and `outline.rs` (two) have all left the list. **A declined file is not an untouched
  one**: two of the four above carry a module comment that earns the exemption, which is what
  CLAUDE.md asks for and what makes the decline re-checkable.

- **`max_frame_bytes` is not the host-memory ceiling its name suggests.** `charge_tile`
  charges `width × height` bytes; `fill_mask` holds an `f32` accumulator of
  `(width + 1) × height` *and* the coverage bytes, so the peak is **5×** the charge. Priced,
  not changed — moving the constant moves which pages refuse. `doc/notes-ceilings-audit.md` §1.
- **The frame budget's pre-check counts top-level commands only**, while `Scene::cost().commands`
  counts through group nesting. Not a bomb (the instance streams are smaller than the
  `Command`s the caller already holds), but the two numbers differ for a nested scene and
  `FrameBudgetExceeded`'s `needed` is the smaller one.
- **`commands_culled` does not count a mark whose residue chain admits nothing.** ADR 0057
  drops those marks from the sheet, so they cost no coverage; the *count* is missing, and
  `encode/device_space.rs`'s cull still tests `rect` rather than `mark_bounds` for the same
  reason — moving a caller-visible number is its own decision with its own measurement.
- **The `Counters` → `Recorded` mapping is written three times** — `tests/archetypes.rs`,
  `examples/retained.rs`, `examples/surface_measure.rs` — because `Counters` lives in
  `quorra-gpu` and `quorra-pages` must not depend on it. Named fields are the mitigation, not
  the fix. A fourth consumer is the trigger to re-open it, with the dev-dependency cycle on
  the table: it **works** (ADR 0060 verified it in a scratch workspace) and was refused on
  principle 4, not on feasibility.
- **`examples/encode_threads.rs` sweeps `DENSE_TEXT_UNCLIPPED`, not the dense-text
  archetype.** It has since ADR 0054, and its comment said otherwise until 2026-08-17.
  Whether the thread sweep should run on the archetype — putting its 40 residue-clipped marks
  into the "does not divide" column where artwork already is — is a question with a
  measurement attached.
- **`deny.toml` bans by name, which is a blocklist**, and a blocklist is silent about next
  year's crate. `tests/no_colour_management.rs` closes the shape with an allowlist over the
  published crates' direct dependencies plus a pattern walk of the shipping graph. What
  `deny.toml` still does not name is `ttf-parser` / `owned_ttf_parser` / `ab_glyph` and
  **`tiny-skia` / `tiny-skia-path` — the caller's own oracle** — all of which are in
  `Cargo.lock` through `winit → sctk-adwaita` and reachable from no published crate.
  `doc/notes-hayro-boundary.md` carries a `wrappers` block for them. `cargo-deny` is not
  installed for the `AI` user, so it has not been run.
- **Four things the `encode.rs` split found and left**, in `doc/notes-encode-split.md` §5:
  `push_op`'s doc comment is two openings for one function — the same defect this file's
  traps record from `take_pass_query`, now in `encode/plan.rs`; `CULL_MARGIN`'s comment cites
  `Encoder::push_glyph`, which ADR 0054 deleted; **`fill_solid` repeats `encode_fill`'s
  `HashMap` lookup of the outline**, once per solid fill on the hottest walk in the tree
  (4 320 of them on the dense-text archetype), and fixing it needs a lifetime that fights
  `&mut self`; and `command`'s `#[allow(clippy::only_used_in_recursion)]` no longer fires and
  is a one-line deletion. `visible_tile` and `coverage_tile` are now adjacent, and their ten
  lines of identical arithmetic are visible in one screen for the first time.
- **`SceneBuilder::image` refuses a bad image alpha with `SceneError::InvalidGroupAlpha`**
  — a shared variant. Public API, so it is a bump's business rather than a refactor's.
- **`tests/shader_copies.rs` keeps its own `include_str!` list of the shader files**, which
  since the layout gate is the second such list beside `src/shaders.rs`. An integration test
  cannot reach a private module, so closing it means deciding whether that list is public.
- **The shared probes have a home, and one number in the list was wrong.**
  `tests/common/probe.rs` holds `pixel`, `alpha` and `max_byte_diff`; the eight private
  two-argument `render`s point at `common::headless::render`. The obstacle this list carried
  for two rounds — one home for them is one home for `SIZE` — **did not survive being read**:
  a probe needs the raster's **stride**, not the suite's `SIZE`, and a stride is an argument,
  so nothing shares a dimension and `alpha`'s hazard dissolves rather than being managed.
  Verified per caller by shimming each import to pass `width + 1`: **18 of 20 red**, and both
  greens are fixtures where every stride reads the same byte (a uniformly-marked target; an
  absence assertion on an empty page), each red when forced on the value instead. **The list
  of four was not complete** — `sample_tail.rs::at_unit` was a fifth copy, found by checking
  a claim before writing it down. Five inline `alpha` indexings remain by choice, named in
  `doc/notes-test-probes.md` §1.
- **`UNORM_TOLERANCE` stays two constants, for the opposite of the recorded reason.** They
  were never two derivations: `m3.rs` cited `m1.rs`'s, which is `255/α` on a golden whose
  minimum alpha is 128, while `m3.rs`'s page produces `{0, 29, 57, 86, 115, 172, 230}` — the
  same premise gives ≈9 there. **And m3's page agrees with its reference to 0 unorm steps**,
  so that gate has never spent any slack; tightening it is a round with a real question
  attached, since that file pins llvmpipe while `m1.rs` is the one that asks every adapter.
- **Three gaps in the function lane**, named in `doc/notes-function-wiring.md` §4.5 and found
  by reading rather than by a failure: a clip and a soft mask over a function paint are never
  anything but 1 anywhere in the tree, no function test runs `Coverage::Gpu`, and ADR 0025's
  `DestOut`/`Plus` stages are compiled and selected but never drawn.

**Closed by the 2026-08-15 debt round**, listed once so nobody re-proposes them: the rustdoc
warnings (all eleven in the workspace, and `cargo doc --workspace --no-deps` under
`RUSTDOCFLAGS=-D warnings` is a CI step now, which is why seven could accumulate unseen);
`take_pass_query`'s stale opening line; the duplicate accessors (`pipeline_store()` and
`gpu()`/`queue()` deleted, with the `#[allow(dead_code)]`); the uniform byte-offset check,
which is `src/shaders/layout.rs` deriving each field's offset from the shader source by
WGSL §14.4.4 and §14.4.6 and covering all nine host writers, verified able to fail from both
sides (a host struct's `vec2f` pair exchanged, and two fields exchanged in `blit.wgsl`);
`device/ramp.rs`'s missing unit tests, six of them, each expectation derived from its clause;
and both test-suite debts — `retained_frame.rs` is five files along its own comment's seams
(`retained_replay`, `retained_atlas`, `retained_invalidation`, `retained_refusals`,
`retained_handle` — the fifth responsibility was in the file and *not* in its comment), with
`tests/common/` holding the fixtures that had copies. 403 tests before and after, the same
names.

## Instruments — how to measure without re-deriving it

- **A presenting frame on the real GPU**: `examples/surface_measure.rs`. The owner left a
  trigger loop — `touch tmp/start-measurement` here and the loop rebuilds the working tree,
  runs it on the real display and writes `tmp/output.stdout.txt`, `stderr` beside it. It
  turned a round in about ninety seconds on 2026-08-14; confirm it is still alive before
  relying on it.
- **An encode, exactly**: `perf` is not installed for this user and wall clocks here are
  worthless at the load averages this machine runs at — 4.49 ms for an encode the owner
  clocked at 1.96–2.35. Use **callgrind**: it counts instructions and load cannot touch it.
  `encode` needs no adapter, so the harness is a `#[cfg(test)]` module inside `quorra-gpu`
  that builds a `ResourceStore` and an `AtlasStore` directly, copies the archetype out of
  `examples/surface_measure.rs`, encodes twice to fill the atlas and then N times from an
  `#[inline(never)]` wrapper. Build with `CARGO_PROFILE_RELEASE_DEBUG=1` into its own target
  dir, then `valgrind --tool=callgrind --collect-atstart=no --toggle-collect='*steady_run*'
  --read-inline-info=yes --cache-sim=no`, and read it with `callgrind_annotate
  --inclusive=yes` (the lines carrying the binary's path are the per-function inclusive
  totals) and `--tree=caller` for call sites and counts. **Check the counter row against
  `tests/archetypes.rs` before believing any of it** — that is what says the harness encoded
  §6.2's page. Delete the harness with the round; `Cargo.toml`'s note on `criterion` is the
  standing decision that a benchmark harness does not live in this tree.
  **Take the A/B without the `#[inline(never)]` seams when the question is "what does this
  change cost", and with them only when the question is "what is it made of"**: `lto = "fat"`
  inlines the walk either way, so the delta between two unseamed builds is the delta the
  caller gets, and the seams' 0.07–0.20 % distortion buys nothing. Two unseamed builds
  reproduced a seamed round's per-item prices to within 2.5 % on three pages
  (`doc/notes-fill-solid-lookup.md` §1).
- **What a generated shader costs to compile**: `examples/function_compile.rs` — a §7.10.5
  program's shader, timed by the program's length. It reads a **direct span** that
  `PipelineStore::function_pipeline` already brackets rather than a subtraction, its own
  `CARGO_TARGET_DIR`, round 0 reported apart, minima of round-robin rounds with the load
  average beside them. Every sample must be a program **no process has compiled**, because
  RADV's on-disk cache keys on SPIR-V — the trap that cost the spike a round.
- **Whether an instrument still asserts what it claims**:
  `cargo run --release -p quorra-gpu --example <name> -- --check`. Every example takes it; it
  is the smallest configuration that executes the example's assertions and prints no
  statistics, so it is a gate rather than a measurement. CI runs all twelve under `Xvfb`. If
  you add an example, add it to the workflow's list — `tests/example_checks.rs` fails until
  you do. **`--check` may only *shrink* a sweep** (fewer rounds, adapters, sizes), never
  substitute a page: an example that checks a page it does not measure is ADR 0060's defect
  wearing a different hat.
- **Which coverage producer drew a mark**: `Counters::bytes_uploaded`, and nothing else can say
  it. Both lanes seat a tile on one sheet and both count as `lanes.path`, so neither `lanes` nor
  `coverage` distinguishes them — but the processor lane uploads its tile while the device lane
  uploads a winding target instead, so `Coverage::Gpu` reports **exactly** the `Coverage::Cpu`
  figure when no mark took the device lane and strictly more when one did. An equality and a
  strict inequality, with no threshold to pick between two moving numbers (ADR 0070).
- **A page of curve-clipped marks**: `examples/residue_clip.rs` — the artwork archetype,
  headless into a texture, `instrument_encode` on, minima of twenty steady frames with the
  first reported apart and the load average printed beside them. It prints **all three
  phases** and the sheet's coverage texels, plus `clip_residue_regions` and
  `clip_residue_tiles`, which is what makes two runs on a loaded machine comparable at
  all: the counters are exact functions of the scene. **Read the `fastest encode` line
  rather than the per-phase minima** — the minimum of a remainder across twenty frames is
  not the remainder of the minimum. Two builds of it cannot be round-robined inside one
  process, so the A/B is a `git checkout` between the base and the change, three rounds
  each, alternating. **Its page was re-cut on 2026-08-17** and no number taken on it before
  that date is comparable with one taken after.
- **What a present pass costs**: a **count**, and `doc/notes-present-quad.md` §2 is the
  arithmetic — pixel centres whose inverse-mapped point lands inside `[0, source)`, per
  layer, against the target. A throwaway example that also timed it with timestamp queries
  was run, read and deleted (ADR 0058); rebuild it the same way if a second question needs
  it, holding the retired arrangement inline so both can be round-robined in **one** process
  rather than across a `git checkout`. **Do not decide on its durations**: on the page-only
  arrangement llvmpipe read the 4 %-smaller pass 14 % slower in one run and 0.04 % faster in
  another.
- **Whether the atlas settles**: `examples/retained.rs`'s second section — twelve retained
  frames of a page whose tiles overflow a stated atlas budget, printed as a string of `E`
  (encoded) and `.` (replayed). A **property, not a clock**, so it reads the same at load
  average 90 as on an idle machine. `E...........` is a settled atlas; `EEEEEEEEEEEE` was
  the pathology ADR 0050 removed. The section asserts its page is still inside the band —
  a fixture that drifts out of it would go on passing, because a page that never overflows
  replays trivially.
- **What a code path allocates, exactly**: `tests/counting_allocator/` — a
  `#[global_allocator]` for one test binary that counts allocations of a megabyte or more
  on the calling thread. Deterministic where a wall clock here is not, and it is how
  ADR 0052 gates the readback. Thread-local on purpose: llvmpipe's worker threads allocate
  too, and a global counter would make the number a property of the adapter's core count.
- **Memory inside a frame**: six lines in `Device::render` — a `Region::of(root.bounds)` and
  an `eprintln!` — in a `git worktree`. That is what turned ADR 0039 from a paragraph saying
  "not worth it" into a 41 % reduction. The path is finished for now: every layered frame of
  the corpus at 4× prices 1 325.5 MB in total, the heaviest single one is 93.0 MB and is a
  page whose root marks its whole target, and nine frames in ten are flat and allocate no
  layer at all. Price before opening it again.

## Running the caller's corpus

Never build in `/home/cl/projects/pdf-viewer` and never edit it: the owner works there, and
its `[patch]` and lock are often their work in progress. Copy it instead —

```
rsync -a --exclude=target --exclude=corpus-cache --exclude=fuzz --exclude=tmp \
      --exclude=.git --exclude='doc/pdf.js/.git' --exclude=.claude \
      /home/cl/projects/pdf-viewer/ <dir>/viewer/
```

— the excludes are not optional (that tree is 100 GB). `--exclude=.claude` because that
tree's `.claude/worktrees/` holds other agents' build dirs, which reached 15 GB and was
still growing when it was learned; `<dir>` under `/home/AI` rather than the `/tmp`
scratchpad, because that tmpfs has run out mid-copy and the copy is about 537 MB. Then
append a `[patch."https://github.com/close2/quorra"]` block pointing `quorra`, `quorra-gpu`
and `quorra-scene` at `crates/*` here, and run

```
CARGO_TARGET_DIR=<scratch>/target cargo test --release -p render-quorra --test corpus \
  -- --ignored --nocapture
```

`PDFVIEWER_QUORRA_ONLY=a.pdf,b.pdf` narrows it (the ratchets are then *not* checked),
`PDFVIEWER_QUORRA_COVERAGE=cpu|gpu` picks the lane and `PDFVIEWER_QUORRA_SCALE=n` the
magnification.

## Counting a feature's population without the corpus test

ADR 0067's census read **67 464 PDFs** — `corpus-cache` alone is 66 211 files and 94 GB — with
one `rg -a` pass for the dictionary key, a 90-line Python extractor that inflates `/FlateDecode`
and `/ObjStm` bodies, and a throwaway crate with a path dependency on the worktree that runs the
**real analyser** over what it found. Minutes rather than the corpus run's 25, no copy of their
tree, and it answers the question that decides whether a change is worth making at all: **how
many documents would this touch?** It is sound for a *stream* object because ISO 32000-2 §7.5.7
forbids stream objects inside object streams; a dictionary-only key needs the `/ObjStm`
inflation. Reach for this **before** a narrowing round, not after.

## Traps

**The corpus is part of a change, not a check after it.** Layers sized to their plans passed
208 unit tests and moved 31 corpus pages off *agree*, then 12 more, before it was right:
884, 903, 915. Two defects, neither reachable by any test in this tree.

**Always run the baseline in the same copy, on the same day.** Verdict counts are stable
across copies of *one* viewer revision, and that tree changes under you: ADR 0036 recorded
915 / 37 / 5 at scale 1, and a copy taken a day later read 919 / 37 / 1 for the same quorra
commit. Nothing regressed — their tree moved. So a count quoted in an older ADR is not a
baseline; a `git worktree` at the base commit, patched into a second copy of the viewer and
run the same hour, is. Compare the **per-page lines**, not only the totals: ADR 0037's
evidence that it moved memory and not pixels is that all 37 differing pages matched to the
last digit of every mean, worst tile and SSIM.

**Wall clocks lie under load, and this machine is somebody's desktop.** A first-frame
improvement measured at 24.7 ms → 10.3 on a quiet machine re-measured as 19.9 → 20.0 an hour
later with Firefox and a slicer running. Check `uptime` before believing a timing, prefer
minima over means, and make the *test* assert a property — a device warmed for one size,
another, or none draws the same bytes — rather than a duration. **That improvement was not
real** (ADR 0040): 40 round-robin rounds could not find it at either page size, and the
allocation it was credited to takes 0.06 ms. When a difference is a difference of wall
clocks, run the configurations round-robin so drift falls on all of them, and look for a
*direct* span before believing the subtraction.

**A first-use pipeline compile is invisible to "wait a while and try again".** Two of them
were 2.6 ms of a layered first frame for three ADRs, and `Timings::phases` had been
reporting them by name the whole time. When a first frame is slow, read its phases before
theorising about memory.

**Stage "every stage learns an offset" changes at zero first.** Panes (ADR 0028) shipped with
one of three subtractions missing and drew nothing at all for every band after the first. The
same change done as *plumbing at zero, verified by equality, then the value* caught a
vertex-only uniform binding immediately and cost one extra commit.

**A refusal is arithmetic, a fidelity difference is not.** Which pages refuse is
machine-independent and can be reasoned about; which lane is faster is a property of the
processor *and* the adapter together, so never publish a crossover as a constant — the two
ADRs that tried (0027, then 0028) both had to delete one.

**A tile is not a window on a wider region unless the rasteriser cuts at its border.**
`fill_mask` clamped the endpoints of an edge piece that left the region and interpolated
between them: the row's total winding survived, so every column past the crossing read the
right value and no test could see it, while the columns *at* the border took the height the
piece spent outside. 2 684 pixels of a 2.9-million-pixel probe, the worst by 185 of 255.
Any change that wants to compute coverage once and cut it up afterwards has to check this
first — the probe is `a_tile_is_the_crop_of_the_region_that_contains_it`, and it took
ninety seconds to write and settled the design of a whole round (ADR 0049).

**A determinism fixture that does not overlap is not a determinism fixture.** ADR 0054's
first thread-count gate used a 15-pixel lattice where no two marks touched, and it **passed
with an ordering drain removed**. At 6 pixels for a 44-pixel mark it fails. The same round's
atlas defect — a duplicate insert, `bytes_uploaded` off by exactly 64 — was found by the
gate rather than by reading, which is what a fixture that actually contends buys you.

**A cache's "is this worth caching?" test must be asked on the axis the cache pays off on.**
ADR 0029's `worth_caching` asks whether *this frame* reads an entry twice. On `Coverage::Gpu`
that is right, because a "no" routes the tile to a lane measurably faster for one use. On
`Coverage::Cpu` there is no such lane — "no" routes it to the sheet, the same rasteriser with
no entry next frame — so the same question turns a one-off cost into a **per-frame** one.
98.6 % of the corpus's atlas keys are placed exactly once *within* their frame while the atlas
still serves 242 049 cached marks on the next one: the value was never on the axis the test
measures. Sibling of the trap below — that one is about **units**, this one about **which
axis**, and both look correct while reading the call site.

**A cache's "would this help?" test must be asked in the units the cache allocates in.**
ADR 0024 gated the atlas repack on `bytes requested <= bytes available`, and the packer
allocates *shelves*: a page at 63 % of the atlas by area did not fit it by packing, so the
repack fired, changed nothing, and fired again on the next frame — for ever, invalidating
its own retained encode each time (ADR 0050). Sixteen of seventeen swept configurations
settled after one encode and looked like proof the design was fine. **Sweep the parameter,
and read the sequence rather than the first two frames.**

**Read a gate's threshold against the number it names before trusting it.** The readback
gate asserted `< 6 ms` against a regression measured at 3.84 — so the shape it existed to
catch would have passed it, and the only thing it ever failed for was this desktop's load
(two runs in five). It had been that way for three ADRs. ADR 0052 replaced it with an
allocation count and recorded the general form: **a claim about "how many" is a count and
is exact; a claim about "how fast" is a duration and this machine cannot measure one.**
Split a gate along that seam rather than picking a threshold between them, and verify a
new gate *in both directions* — a test that passes proves only that a test exists.

**A fixture that names a lane should say which lane it means.** ADR 0047 found three tests in
`m45.rs` using a rectangle as a stand-in glyph: one failed, and two would have gone on
passing while comparing one lane with itself.

**A drain site can be covered by every fixture and gated by none of them.**
`tests/encode_threads.rs` fails when `plan_child`'s inner drain is removed — so ADR 0054's
fixture does contend — but **`push_op`'s drain can be deleted and all four of its tests pass**,
because every op `busy_page` pushes follows a `plan_child` that drained already. Reaching it
needs a *rare-lane* command (image, shading, function paint) mid-run of queued fills, and that
file's comment claimed one was there when the file contained none.
`tests/encode_threads_nested.rs` now holds it. **Before believing a set of drains is gated,
delete each one and watch which test goes red.**

**A fixture on integer pixel boundaries cannot tell the two coverage lanes apart.** ADR 0016
says they agree *exactly* where no edge crosses a pixel — 255 is 255 and 0 is 0 — so a bar at
`x = 20.0` draws the same bytes under `Cpu` and `Gpu`, and any test whose subject is the lane
then compares one lane with itself. `tests/rare_lane_coverage.rs` was written that way first;
its solid-painted control is what caught it, and moving the geometry to `20.3` fixed it. Same
family as `m45.rs`'s rectangle-as-glyph: **when a fixture's subject is a difference, assert the
difference is reachable.**

**A sampled coverage rule and an area coverage rule disagree about what is *there*, not only
about how much.** `Coverage::Gpu`'s 4 × 4 ordered grid puts its columns a quarter-pixel apart, so
a 0.1-pixel bar falls between them at six of ten sub-pixel positions and is drawn as **nothing**,
while the other four draw 2.5× its ink; `Coverage::Cpu` draws 0.10196 at all ten. The tempting
reading of `coverage_lanes.rs` — that the lanes agree to an eighth of a pixel — is **not** a bound
on this: it was derived for an edge *crossing* a pixel, and says nothing about a shape narrower
than the sample grid. Any claim that the two lanes agree needs to name the shape class it was
measured on. **Closed 2026-08-18 by ADR 0070**, in the lane chooser rather than in the grid: such
a mark keeps the processor lane, threshold `1/√coverage_samples` derived from where the columns
actually sit — **including across the pixel seam**, which is the step that is easy to get wrong.
Two counts survive it and are why it was worth one comparison rather than a milestone: the corpus
population is **35 marks at scale 1 and 16 at 8×**, and **magnification shrinks it**, because a
stroke's width arrives resolved in device pixels and does not follow the viewport — so the lane a
caller switches to at zoom is the lane the defect had almost stopped reaching by then. What
survives the fix is the **residual**: a 45° hairline given as a *fill* has a wide box and no
stroke width to be read instead, so it keeps the device lane and *dots*.

**A condition that declines a lane can be gated in one direction by every fixture you have.**
ADR 0070's fifth `take_gpu_lane` condition passes its clause sweep when it fires for *every* mark,
because declining the device lane is always safe for §10.7.4 — the sweep cannot tell "thin marks
are on the processor lane" from "everything is". The half that catches it is the fixture **above**
the threshold, asserted through a counter rather than through pixels: at exactly one sample-column
spacing the mark must still reach the device lane. Forcing the condition to `|| true` fails that
half *and* ADR 0026's own lane control, and leaves the clause sweep untouched. **When a change
makes a lane rarer, gate the case that must still take it.**

**A rule about what is *inside* a knockout group is three rules with three prices.** ADR 0069 had
to choose between refusing a group that is a *direct element* of a knockout group, the same
transitively, and the same transitively without an exemption for the staged compose. The first two
cost **zero** corpus pages; the third would have turned **three drawn pages into refusals**. The
round published the three-page figure against the wrong predicate and retracted it after a
re-walk. State the predicate before costing it, and cost the one you will actually write.

**`xwd` cannot see a Wayland window, and the failure does not say "Wayland".**
`examples/present_thread` had only ever run under `Xvfb`. On the owner's machine it opened its
window, warmed, rendered on a thread, presented — and died on `xwd: error: No window with name
… exists!`. It is not renamed and not reparented: `xdotool search`, `xwininfo -root -tree` and a
50 ms poll for five seconds all find **nothing**, because `XDG_SESSION_TYPE=wayland` and `winit`
prefers the Wayland backend whenever `WAYLAND_DISPLAY` is set, so the window is not in the X
tree at all. **`CLAUDE.md` said "Session: X11" and that was stale** — corrected 2026-08-17. Run
anything that reads its own window back under `env -u WAYLAND_DISPLAY`. Cost two loop rounds.

**Under `Fifo` the minimum interval is not the refresh.** A swapchain has more than one image,
so a burst gets through whatever settling you do: measured minima over 120 empty presents were
**0.064 to 0.201 ms** against a refresh of 8.34. Taking the minimum would have divided every
number in that round by forty. **The median of a run of presents with nothing to draw is what
measures the refresh** — print both, which is how this became visible rather than becoming a
result.

**A gate's forced defect has to be forced where the number can move.** `arrangement.rs`'s
fragment gate was first forced by moving the page layer's *width* by one texel and **it
passed**: the page sits at x = 480 with width 1568, so its right edge is the window's own and
the outward pixel is clamped away at either width. Moving the *height* failed it by exactly
1569. **A fixture at a boundary can be wrong on the axis of that boundary without anything
noticing.**

**`wgpu` 30 cannot pin a rounding mode or forbid a fusion, and no amount of Vulkan
documentation changes that.** `naga` 30 emits no `NoContraction` decoration (the `spirv` crate it
depends on defines one, so the absence is a choice) and no `RoundingModeRTE` / `DenormPreserve`
execution mode; `wgpu-hal` 30 never requests `VK_KHR_shader_float_controls`; and **the word
"contraction" does not occur in the WGSL specification at all**. WGSL §15.7.4.1 grants `x / y`
**2.5 ULP** where it calls `+ - *` "correctly rounded", and §15.7.4 adds that its own "correctly
rounded" still "does not specify a rounding mode". So any proposal beginning "we can make the GPU
bit-exact by setting …" is answered by ADR 0067 §1 before it is costed — it reasons about
IEEE 754 and raw Vulkan, and we ship WGSL through `wgpu`. A named cost of ADR 0002.

**A criterion that waits for the window to hold still must also say what it must stop holding.**
`doc/notes-present-rate.md` §4 named the fix for its own flake as *"capture until two consecutive
captures agree"* — and that criterion **converges on the stale window**, because while the new
picture is in flight every capture reads the old one and every pair of them agrees. It would have
turned a 1-in-5 flake into a gate green for ever, which is the same failure as the "retry until it
passes" the paragraph was rejecting. ADR 0068 adds the missing half — the agreed capture must
differ from what the window was last *proven* to show — which makes the settles a chain and needs
a first link the library names rather than a previous capture (an erase to the presenter's own
clear). **When a gate waits for stability, ask what is stable when it is wrong.**

**`git checkout -- <path>` in a scripted forced-defect loop deletes the change under test.** A
script that forced four defects into `examples/present_thread/` and restored with
`git checkout -- crates/quorra-gpu/examples/present_thread/` reverted the round's own *uncommitted*
work: the new untracked file survived and every edit to a tracked one did not, which reads exactly
like a build that mysteriously lost its feature. **Commit before forcing a defect, and restore
from a copy the script took itself, never from the index.**

**A stale worktree argues that your brief is wrong.** A round given the four files past the
size smell reported three "corrections" — that the highest ADR is 0056, that the suite is 445
passing, and that `raster.rs` is 1 400 lines and untouched. All three are true of the session's
first commit and false of `main`; the worktree was sixty commits behind and the agent read the
difference as an error in the instructions rather than in its own base. Its code was sound —
the four files it touched had not moved — but **its whole account of the rest of the tree was
of a repository that no longer existed**, including a recommendation to split two files that
had already been split. The cost is not in the diff, it is in the report, and a report is what
the next round reads. **A statement about a file you did not open is a statement about your
base.** Confirm `git log --oneline -1` against `main` before writing any claim about the tree,
and re-measure a size rather than quoting one.

**A split is a whole-file replacement, so its base is part of its correctness.** A `raster.rs`
split prepared on a base twenty commits stale would have merged out `direction`'s `hypot` path
and `accumulate_edge`'s non-finite-slope return — **and the three tests that cover them**,
because they lived in the same file. Every gate would have stayed green: there is nothing left
to fail once a fix and its test leave together, and **`git` offers no conflict**, because the
file the other change touched no longer exists on the splitting side. The instrument that
catches it is the multiset comparison of every non-comment, non-blank line — run against the
base the branch will actually merge into — finishing by grepping the split side for the
incoming fix *by name*. Check a branch's **base**, not only its diff.

**A module comment claiming one responsibility is evidence about the comment, not about the
file.** `raster.rs` said "the one producer of coverage bytes" over 864 lines in which a third —
§8.4.3's stroke expansion — produces no coverage byte at all: it takes polylines and returns
polylines. `pipeline.rs` was more honest and was read past anyway, because its seam was an
"and": "the store — its one lock, its laziness *and its warm-up*". **Two openings for one file
is the same tell as `push_op`'s two openings for one function.** Read what a file does before
believing what it says it does.

**A public module with no public items still has public documentation.** `pub mod pipeline;`
exports nothing — every item under it is `pub(crate)` — but its module comment is public
rustdoc, so `[`Kind`]` in it fails `RUSTDOCFLAGS="-D warnings"` as "links to private item"
while resolving fine under `--document-private-items`. Use code spans there, and run **both**
doc commands: they disagree, and the strict one is CI.

**A claim that something cannot be done is a claim, and it decays.** "`fill_solid`'s duplicate
lookup needs a lifetime that fights `&mut self`" was written once from a reading, and travelled
from `notes-encode-split.md` §5 through `notes-recording-shares.md` §5 into this file's debt
list, gaining a price on the way but never a compile. It was wrong: the encoder holds the store
as `&'a ResourceStore`, so a borrow out of it keeps no loan on `self`, and the whole change is
one struct field plus naming a lifetime the `impl` block already had. Two functions in the same
file were already holding that borrow across `&mut self` calls, which is evidence a reader had
in front of them for two rounds. **Cost to disprove: one edit and one `cargo build`.** Before
quoting a "cannot" forward a third time, spend the five minutes.

**A clip that overlaps nothing looks exactly like a clip that works.** `tests/archetypes.rs`
placed its curve clips on a grid of step `side × 6` and its marks on a grid of step `side`:
**0 of 40** and **8 of 600** of the clipped commands had a mark that met its clip. The rows read
40 and 600 tiles and 2 and 185 residue regions anyway, because a mark whose chain admits nothing
still got a mark-sized tile and multiplied it by zero — so the signature looked like it gated the
residue lane for two ADRs, and `examples/residue_clip.rs` inherited it. ADR 0057's tile bound is
what made it visible, four rounds later. Same family as "a determinism fixture that does not
overlap is not a determinism fixture": **when a fixture's subject is an interaction, assert the
interaction happened** — count the tiles, not the commands.

**A gate whose assertion is an absence needs a control.** `tests/no_ink.rs` asserts that four
kinds of "nothing" leave the target byte-identical. An ink floor planted in `coverage_at` failed
one of them in all four contexts and left the other three **passing** — an empty clip is culled
at encode, a zero-area rectangle produces a quad with no fragments, a zero-area outline produces
no coverage byte, and none of the three ever reaches a fragment shader. "The target did not
change" reads the same whether the fixture answered the question or never drew anything. The
remedy is in the file:
`the_marks_this_file_asserts_are_invisible_are_visible_when_they_are_given_ink` restores each
mark's one nothing-making property and asserts it *does* reach the target.

**A gate on a fixture can be wrong in exactly the way the fixture was.** The first version of
`a_curve_clip_clips_the_marks_that_draw_under_it` compared each mark against the box its clip was
*built from*, which is an identity: it passed with the clips forced back to the far side of the
page. Forcing the defect is the only thing that told the two apart — **a gate verified in one
direction is not verified.**

**A fixture copied into an example is a fixture that has to be re-cut in both places — and
nothing runs an example.** Four examples carried private copies of `tests/archetypes.rs`'s pages.
When ADR 0057 changed what dense text draws, `examples/retained.rs` kept asserting the old row and
**panicked at its own signature gate on `main` for two days**, because `cargo test` neither builds
nor runs an example. **Closed by ADR 0060**: pages live in `crates/quorra-pages` (a
dev-dependency, which is the only edge reaching a test *and* an example), every example takes
`--check`, and CI runs all twelve. The general form outlives the instance: **an assertion nothing
executes is not a gate, it is a comment that can rot into a panic** — so when you add a gate, ask
what runs it before you ask whether it is right.

**The minimum of a remainder is not the remainder of the minimum.** `encode: recording` is
computed as `encode − geometry − staging`, so taking the minimum of each column across frames
mixes three different frames and can hide a phase shift entirely. Read one frame's three parts
together — `examples/residue_clip.rs`'s `fastest encode` line, not the per-phase minima.

**A NaN is not a stopped frame.** `fill_mask`'s prefix sum carries a NaN to the end of its row,
and the non-zero rule's `running.abs().min(1.0)` returns **1.0** for it, because `f32::min`
returns the non-NaN operand. One invisible mark therefore paints its whole row solid, drawn, and
reported as drawn. Any change that lets a non-finite value into the accumulation grid produces a
plausible-looking wrong page rather than a failure — which is why `MAX_COORDINATE` now bounds the
viewport transform as well as the scene's, and why `accumulate_edge` returns on a non-finite
slope (`doc/notes-ceilings-audit.md` §2). Release **wraps**: `[profile.release]` sets `lto` and
`codegen-units` and nothing else, so `overflow-checks` is cargo's default `false`.

**A cross-worktree build can fail with another worktree's *source* in the error.** A `cargo test`
failed with `E0027: missing field 'stencil' in Command::Image` against a field that exists nowhere
in that worktree or at `HEAD`, and twice more with a `RenderError` variant that had just been
added being "not found"; a sibling agent was building into `/home/AI/cargo-target/quorra` at the
time, and re-running succeeded with no file changed. The existing shared-target-dir trap covers a
stale *binary*; this is the same hazard producing a stale *compile error*, which reads like a
defect in your own tree. Re-run before believing an error that names a symbol you cannot find,
and take a private `CARGO_TARGET_DIR` for anything you will **report a gate result from** — not
only for anything you read numbers from.

**A WGSL compile error now fails the suite by name** (ADR 0042). It used to hang it: a
reserved keyword in `blit.wgsl` panicked the warm-up thread inside wgpu, and every test file
that calls `wait_until_warm` then waited on a `Condvar` with no notifier left alive — an
infinite hang with no output at all. If you ever see a silent hang again, suspect the same
shape: a `Condvar` whose only notifier can leave without notifying.

**Cargo can call a stale artefact fresh, and a test *count* is how you catch it.** Four merge
verifications in the 2026-08-15 debt round reported `cargo test --workspace` green while the
`quorra-gpu` lib binary being run was the one built at `619ef3b` — **89 unit tests where the
tree had 112**, so the layout gate and the ramp tests that had just merged were never
executed, and nothing failed because nothing ran. The tell was arithmetic: the total dropped
by 21 across a merge that added two files. `RUSTFLAGS="-D warnings" cargo test -- --list`
read 89 while the same command without `RUSTFLAGS` read 112, and `touch`ing one source file
restored it. So: **compare a suite's test count against `grep -rc '#\[test\]'` before
believing a green run**, especially in the shared target dir, and touch the tree when a merge
has just landed. A count is exact where a duration is not — the same seam ADR 0052 drew.

**A clippy run that says "Finished" may not have looked at your file.** Eight edited test
files in the shared `/home/AI/cargo-target/quorra`, and `RUSTFLAGS=-D warnings cargo clippy
--workspace --all-targets` reported clean while five of them held an unused import — which
failed the moment the files were `touch`ed. Before believing a green gate on a fresh edit,
check that cargo printed `Checking <crate>` and not only `Finished`. Clippy *does* lint
integration tests; that was verified by planting a lint and watching it fail, which is the
right way to settle "did the gate look at this?" in either direction.

**A pipeline's exit status is the last command's.** `cargo test --workspace 2>&1 | tail -80`
exits 0 when cargo fails, because `tail` succeeded, and the tail of a passing run and of a
failing one both end in doctests. Redirect to a file and read cargo's own status, or set
`pipefail`; a status read through a pipe is not evidence that a suite passed.

**The shared cargo target dir is not yours alone.** Sibling agents in other worktrees build
the same crate and example names into `/home/AI/cargo-target/quorra`, and
`release/examples/<name>` is whoever linked last. A measurement round was published from
another tree's binary and only caught because its counters were impossible. Anything you read
numbers from gets its own `CARGO_TARGET_DIR`; `cargo build` reporting "Compiling" is not
proof the binary on disk is yours.

**A fresh `CARGO_TARGET_DIR` is a cold cache, and a per-checkout remap flag is a colder one.**
`sccache`'s key includes the paths cargo derives from the target directory, so inventing a new
target dir per purpose reads as 0 % hits and a 27 s build where the same build against a
stable dir is 100 % and 4 s. Across worktrees the cache already shares at ~97 %, and
`--remap-path-prefix` with a per-checkout value makes it *worse* — differing values took the
same cross-worktree build from 2 misses to 121. One stable target dir per user (the untracked
`.cargo/config.toml` here pins `/home/AI/cargo-target/quorra`), shared across worktrees when
builds are sequential.

**`pgrep -f "<your own command>"` in an `until` loop never exits**, because the loop's own
shell matches its own pattern. Two rounds were lost to a `cargo test` that never started.

## Recorded and deliberately not taken

Each has an ADR stating the measurement and why it was left. Do not re-propose one without
reading it:

- `Options::coverage` reaches a solid fill or stroke and no other paint; the marks it would
  move are 0.11 % of a frame's rasterised coverage at scale 1 and 0.63 % at 4×, and two thirds
  of them are glyph-sized, which is the shape class the processor lane is the accurate one for
  (0064). It is an **oversight, not a constraint** — the device lane's output is already the R8
  sheet tile a rare paint is drawn through, verified by making the change and reverting it — so
  reopen it by measuring, not by re-deriving. The trigger that changes the arithmetic is a
  residue multiply on the device: 82 % of all rare-painted coverage is under one;
- an **area** coverage rule on the device lane, which would close the thin-mark residual by
  construction: it costs ADR 0016 its reason for existing, since exact area needs curves
  flattened at a *device* tolerance and ADR 0016 exists because nothing in it depends on the
  device scale; it makes `Options::coverage_samples` meaningless; and its population is 16 marks
  on one page at the magnification the caller actually uses (0070). Re-ask with the thin-axis
  probe in `doc/notes-thin-mark-options.md` §2.1 — minutes, not a round;
- the census cannot see how often a shape is placed, at phase granularity (0029) — and
  **ADR 0065 measured that this looseness is load-bearing rather than a debt**: a phase-aware
  census would keep 226 368 tiles out of the atlas rather than 79 370 and empty it almost
  completely. Sharpening it makes the answer worse, not better;
- a pane is cut in sheet order rather than by what packs tightest (0028);
- tiles are packed in encounter order; sorting them needs positions assigned after the walk,
  which is a two-pass encode (0034);
- the layer pool matches on **exact** extent — serving a smaller plan from a larger texture
  would buy 0.06 ms and would put a viewport in every pass (0040);
- `warm_for` does not draw a warm frame: 4.7–22.3 ms on the calling thread to save 0.5–2, and
  what was attributable in it is in the warm set instead (0040);
- the mask's transparent value is computed on the CPU as well as in `reduce.wgsl`, because an
  independent implementation a test compares is stronger evidence than agreement by
  construction (0037);
- a non-isolated group still takes its parent's region rather than its own: §11.4.4's
  interpolation is stated over the whole of the group's buffer, so shrinking it is a clause
  question and not a plumbing one (0038);
- a culled child's plan stays in `Encoded::layers`, unreferenced, because that list is indexed
  by `ChildOp::layer` *and* by `MaskPlan::root`; the cost is that `is_flat` asks whether
  `layers` is empty rather than whether any plan is reachable, so a page whose only group is
  clipped away renders through a root accumulator instead of straight into the target (0041).
  The same question is open for a culled group's soft mask, which still realises;
- the hand-off gained a branch per pixel of the target; nothing surfaced above the run-to-run
  spread on the corpus, and that is the honest statement rather than a claim either way (0039);
- the atlas has no recency, so two pages that alternate and do not fit beside each other
  still repack once per frame — the same cost that shape had before ADR 0050, now *visible*
  as `Counters::atlas_repacked` true on every frame rather than inferred. A single page too
  large for its atlas is stable (0050); genuine thrash is what recency answers, and ADR 0024
  has been waiting for its measurement since;
- the *divide* half of ADR 0022 is no longer gated at all: it is a throughput claim, no wall
  clock on this machine can hold one, and its instrument is the callgrind round above rather
  than anything in CI (0052);
- a residue region is the chain's own bounds and not the union of the tiles that ask for it;
  the union needs a second pass over the commands before the first ask, which is the
  two-pass encode ADR 0034 declined for the tiling (0049);
- residue regions are not pooled across frames: the retained encode already answers "the
  same page again", and a cache that outlived its scene is ADR 0029 §3's rejected memory in
  a second place (0049).

## This machine

Arch, AMD Ryzen AI 9 HX 370 with a Radeon 890M. Two adapters and the difference is a feature:
RADV is the default and llvmpipe is pinned by name in most tests, so CI can run on a software
rasteriser. Claude Code runs as user `AI` through the `coders` group and has no X cookie —
headless is fine, a window on the owner's display is not.
