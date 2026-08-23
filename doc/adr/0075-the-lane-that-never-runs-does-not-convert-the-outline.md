# 0075 — The lane that never runs does not convert the outline

Date: 2026-08-23. Status: **accepted, and built**. **It moves no pixel**, and it moves a
budget: what `Device::resource_bytes_in_use` reports after an upload is smaller, and it
grows on the frame that first takes the GPU coverage lane.

The ask is the caller's `doc/QUORRA_FEEDBACK.md` §33. The measurement is ours:
`crates/quorra-gpu/examples/outline_upload.rs`. The code is
`crates/quorra-gpu/src/resources.rs` (`StoredOutline::quads`), its new
`resources/budget.rs`, `encode/coverage.rs`'s split of `take_gpu_lane`, and
`encode/fill.rs`'s reordering of the three tests that follow.

## Context

`Device::upload_outline` did four things: validate the segments (§4.7), recognise an
axis-aligned rectangle (§6.4), **convert the outline into the closed quadratic contours
the GPU coverage lane draws** (ADR 0016), and charge all of it against the resource
budget.

The third of those is read in exactly one place — `encode::fill`'s solid arm — behind a
condition whose first test is `self.coverage == Coverage::Gpu`. `Coverage::Cpu` is the
default, it is what the caller's viewer draws with below ten times magnification, and
under it the condition is `false` before the quadratics are looked at. **Every launch of
that program paid for a representation the frame would not read.**

The caller measured it on the project owner's own document — `tmp/Entwurf.pdf`, one page,
58 009 display commands, **3 011 919 path segments**:

| frame | scene | of it, inside our `upload_*` | uploads |
|---|---:|---:|---:|
| first (cold) | 187.6 ms | **156.0 ms — 83 %** | 58 029 |
| second (a zoom of the same list) | 9.7 ms | 0.2 ms | 40 |

and, by instruction count with `encode_threads` at 1: `Device::upload_outline` 1 743.2 M,
of which `QuadOutline::from_segments` is **1 476.9 M — 490 instructions per segment**, and
`push_cubic` and its recursion are 852 M of that. They did not ask about the segments' own
copy or the validation walk; those are the remaining 266 M and they are the price of the
boundary being a boundary.

They asked for three things in the order they valued them, and argued against their own
second: an outline uploaded under `Coverage::Cpu` may be drawn under `Coverage::Gpu` after
a zoom, so nothing the host can say at upload time is true for the outline's life.

## Decision

**The conversion runs on the first frame that reads its result, and never on a host that
never reads it.** `StoredOutline::quads` is a `OnceLock<QuadOutline>` behind an accessor;
`upload_outline` stores the segments, the rectangle hint and nothing else.

Three things had to be settled for that to be a decision rather than a move.

### 1. The order of the lane's tests is the optimisation

`take_gpu_lane` asked five questions, and the fifth needed a triangle count, which needs
the quadratics. So a lazy cell alone would have converted every outline anyway — on the
first `is_empty()`, which the fill arm asked *before* the setting.

The function is split where it already had a seam:

- **`gpu_lane_admissible`** — the four tests that cost nothing and need no geometry: the
  coverage setting, a residue chain, whether the atlas would rather cache this, and the
  thin-axis test of ADR 0070. Under `Coverage::Cpu` this is one comparison and it is
  `false`.
- **`triangles_under_coverage`** — the byte-for-byte comparison of ADR 0026, which needs
  the count.

`take_gpu_lane` is the conjunction of the two and is what `push_coverage_styled` still
calls, because its triangle count is a length of an already-flattened polyline and costs
nothing to have.

The fill arm now asks `gpu_lane_admissible`, converts, and only then asks about emptiness
and triangles. **That reordering changes which test answers first and not what the
conjunction answers**: `A ∧ B ∧ C` and `B ∧ (A ∧ C)` agree for every input, and every one
of the three is a pure function — `take_gpu_lane` takes `&self` and computes.

### 2. The budget follows the bytes

The old charge was honest because both forms were stored. The new one cannot anticipate
what it has not built, and an estimate is not available at a price worth paying:

> one cubic converts to between one and 2⁸ quadratics (`MAX_SPLIT_DEPTH`)

so the only bound that could not *under*-count is 256 quadratics per segment — 5 120 bytes
where a segment costs 28 — and a device charging that would refuse a page of straight
edges by two orders of magnitude. A ceiling that can under-count is a budget that lies,
and CLAUDE.md's principle 3 does not admit one.

So: **the segments are charged at upload, and the conversion is charged when it becomes
resident.** `resources/budget.rs` is the counter, extracted for the reason `resources.rs`
used to give *against* extracting it — "`charge` has no caller anywhere but those five
`upload_*` methods" stopped being true the moment a frame charged the same ceiling. It is
an `AtomicU64` behind a compare-exchange loop, `release` refunds both charges, and
`Device::resource_bytes_in_use` says what is resident on either side of the change.

The cost is a refusal that did not exist: **`RenderError::OutlineConversionBudgetExceeded`**.
A device filled close to its ceiling can now be refused by the frame that first crosses
into `Coverage::Gpu`, where before it was refused by the upload. That is the second budget
principle 6's "discoverable before the frame" does not reach — `ScratchExhausted` says the
same about itself — and the reason is checkable rather than a shrug: how many quadratics a
curve becomes is a property of the curve. A host that wants the old timing back gets it by
drawing one frame under `Coverage::Gpu`, which converts every outline the page names and
charges every byte.

### 3. `OnceLock`, and why not `OnceCell`

`OnceCell` is not `Sync` and `OnceLock` is, and the question is real: ADR 0054 divides the
encode's geometry phase across `encode_threads`, and the caller runs that at the machine's
parallelism.

The ownership says it is not read from those threads. `Encoder` holds
`resources: &'a ResourceStore` for the whole walk, and the fan-out hands each worker a
`parallel::Job` holding `&'a [Segment]` — a borrow of the segments and nothing else. No
worker ever sees a `StoredOutline`, `rasterise` is a pure function of a `Job`, and the
quadratics are read from the serial walk, which is the only place a lane is chosen at all.
So there is no contention to serialise and no lock on any hot path: a warm read is
`OnceLock::get`, one acquire load.

It is `OnceLock` regardless, for two reasons that outlive today's call graph. A `Cell`
would make `Device` `!Sync`, which is a public property no caller asked us to withdraw.
And a counter whose correctness rests on "no other thread reaches this" is one that the
refactor which does breaks silently, where the atomic is right either way for the price of
one uncontended compare-exchange per upload — beside a `HashMap` insert on the same line.
The lost-race arm is written and refunds its charge rather than being asserted away.

## What this measures, here

`examples/outline_upload.rs`, on **llvmpipe** (the upload path touches no adapter; §A is
CPU work either way), on this project's 24-core machine. **One instrument, two libraries**:
the same binary is built against the tree with this change and against the tree without
it, so nothing about the measurement differs between the columns. The runs alternated
`before, after, before, after, …` so that a drift in machine load could not land on one
column; each number is the minimum of nine round-robin rounds within a run and of seven
such runs, so 63 rounds per arm per column. **The machine was somebody's desktop and its
load average moved between 12 and 92 over the afternoon**, which is printed beside every
sample the binary takes: the runs at load 12–25 give the numbers below, the runs at load
36–92 give roughly twice them and the same ratios, and a reader who does not like that
discounts the run rather than the conclusion.

§A uploads 2 000 outlines of 200 segments — 400 000 segments, an eighth of the caller's
document — in two arms: the same corpus as cubics, and with every curve replaced by the
chord it spans.

| §A, 400 000 segments | before | after |
|---|---:|---:|
| cubic corpus | 49.1 ms — 121.6 ns/segment | **2.90 ms — 7.2 ns/segment** |
| chord corpus (control) | 5.17 ms — 12.8 ns/segment | 2.70 ms — 6.7 ns/segment |

**The control is the result.** Before, uploading a cubic outline cost **9.5×** what
uploading the same outline's chords cost; after, it costs **1.07×** — the two arms are one
number, because what an upload does no longer depends on the shape of what is uploaded.
The cubic corpus's upload is **16.9× faster**, and 114.4 ns per segment left the upload
path: on the caller's 3 011 919-segment document that is **0.34 s of launch on this
machine**, and it is the whole of the 156 ms they measured on theirs. The chord arm's own
1.9× is the `Vec<QuadSegment>` per contour that a straight outline no longer allocates and
fills either.

§B draws 48 marks of 120 segments under three settings, on a corpus whose marks are
declined by the triangle test so that all three arms draw the same page. **Its wall clocks
cannot see the conversion, and the before column is the control that says why**: the first
frame costs more than the second in *both* trees — 18.8 against 13.7 ms before, 19.2
against 13.7 after — so that gap is what a first frame pays for pipelines and buffer
growth, and the conversion of 5 856 segments at 114 ns each is 0.7 ms hiding inside it. A
corpus large enough to lift it above that is a corpus whose frame is dominated by
flattening the same segments, so the ratio would not improve. The **byte** column
separates, on every run of both trees:

| §B arm | before | after |
|---|---:|---:|
| first frame, `Coverage::Cpu` | 0 B converted | 0 B converted |
| first frame, `Coverage::Gpu` | 0 B converted | **873 856 B converted** |
| second frame, `Coverage::Gpu` | 0 B converted | 0 B converted |

Before, every arm converts nothing because the upload already had. After, exactly one
frame converts, and it is the one that reads the result. That is what §33 asked for,
stated in a number that does not move between runs.

§C is not a clock. It asserts, on a corpus small enough for CI and therefore on every
`--check` run, that an upload charges the segments and nothing else, that a
`Coverage::Cpu` frame converts nothing, that the first `Coverage::Gpu` frame converts and
is charged, that the second converts nothing, that a release returns both charges — and
that the two lanes' rasters are byte-identical for this fixture, which is what makes §B's
three arms comparable at all.

## No pixel moves, and what says so

**Measured, not argued.** §D of the instrument renders a page the device lane actually
draws — one two-cubic blob over a box the atlas will not cache and the triangle test
admits — and prints an FNV-1a digest of its raster, with `assert_ne!` against the same
page on the processor lane as the control that says the device lane was taken. Built
against both trees and run on llvmpipe, the digest is
**`0x958a5b8770e422a8` on either side of this change**.

That is what one expects, and the reason is worth stating: the conversion is the same pure
function of the same segments, and the lane's three tests were reordered rather than
changed — `A ∧ B ∧ C` against `B ∧ (A ∧ C)`, all three pure. The gate in the suite that
holds it is
**`tests/coverage_lanes.rs::the_lane_can_change_between_frames_on_one_device`**, and it is
the right one because of its control: it renders one scene four times as
`Cpu, Gpu, Cpu, Gpu` on a single device, asserts `frames[1] == frames[3]` for whole-raster
byte equality of the GPU lane against itself across a lane change, and asserts
`frames[0] != frames[1]` so that a GPU lane which had quietly stopped being taken could
not pass. That is exactly the fixture this change could have broken and did not.
`tests/frame_independence.rs::a_short_gpu_frame_after_a_tall_one_draws_its_own_coverage`
holds the same equality across frames of different sizes.

The corpus gate in the caller's tree is not run here and is predicted to move **nothing**:
`render-quorra/tests/corpus.rs` compares pixels, and no pixel is a function of when an
outline was converted.

## What was not taken, and why

- **Their second option — a flag at upload.** They argued against it themselves and the
  argument holds: an outline uploaded under `Coverage::Cpu` may be drawn under
  `Coverage::Gpu` after a zoom, so a device-level "this host never takes the GPU lane"
  would be a promise the host cannot keep and we would have to either break it or draw the
  page wrong. Laziness needs no promise from anybody.
- **Their third option — a batch `upload_outlines(&[&[Segment]])`, divided the way ADR
  0054 divides the encode's geometry phase.** Declined, and it is the interesting refusal.
  It would have made the conversion *parallel* rather than *absent*, so the caller's
  launch would still pay 156 ms of work spread over `encode_threads` cores — energy, cache
  pressure and 64 MB of quadratic contours, all for a lane that frame does not take. It
  would also have widened our API surface for a case that laziness removes entirely, and
  it would have had to be *ours* to divide: ADR 0023 recorded the answer to "should quorra
  build a thread pool?" as **no — take one rather than make one**, and an upload has no
  frame around it to take one inside of. Their own sentence is the summary: *the fastest
  conversion is the one nobody asked for.*
- **Charging an estimate at upload and keeping the old refusal site.** §2 above: the only
  estimate that cannot under-count over-charges a page of straight edges by two orders of
  magnitude, and a budget that lies in either direction is worse than a refusal that
  arrives one frame later than it used to.
- **Instrumenting the conversion as its own frame phase**, the way the caller's
  `FrameCost::handover` instruments the handover. It would make §B's clocks separate on a
  loaded machine, and it is a library change made for a measurement's convenience rather
  than for a question a host asked. If the conversion ever needs to be watched per frame,
  that is the shape — but it is a decision of its own and not a footnote to this one.

## Consequences

- A host that never sets `Coverage::Gpu` never converts an outline, and its resident bytes
  are the segments it uploaded.
- A host that does pays once per outline, on the frame where it starts drawing that lane,
  which is already the expensive frame — and never again, so ADR 0016's whole property
  survives: a frame at 100× still re-uses what the frame that converted built.
- `Device::resource_bytes_in_use` can grow without an upload, and its documentation says
  so.
- `resources.rs` lost the budget to a module of its own, and its "why this file is not
  two" argument lost the bullet that had been holding it.
