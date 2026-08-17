# 0063 — The atlas fills across pages, and the budget is not what ran out

Date: 2026-08-17. Status: accepted, and built. **Closes ADR 0024's recency question** with
a measurement, four months after it was recorded and two ADRs after it was deferred again.
Corrects `doc/PLAN.md` §1.1's stated mechanism for what a magnified page does to the glyph
lane. The measurement is `doc/notes-atlas-budget.md`.

## Context

§11.2's corpus census reported that the biggest lane movement anywhere is the glyph lane
losing tens of thousands of marks at 4× **to the atlas budget**, on 19 pages, and that
§1.1's stated mechanism for that — a tile outgrowing an atlas entry — accounts for 0.06 %
of it. Four explanations were on the table: the default budget is wrong for a zoomed page;
the atlas wants recency; it is the caller's knob and a documentation problem; or the
sub-pixel quantum interacts and has to be understood first.

Nothing in `Counters` could tell them apart, which is why the round began by building an
instrument. `atlas_distinct_keys` says what was asked for, `atlas_entries` says what is
resident *after* the frame, `tiles` mixes every lane's output, and the count of entries
*before* the frame — the one subtraction that would have answered it — is not reported at
all.

## What was measured

Page one of 974 corpus documents at **4×**, RADV, `Coverage::Cpu`, one device for the whole
run, twice: with the corpus gate's `glyph_quantum: None` and with the shipped `Some(16)`.
948 pages drew. Full tables in the notes; the four numbers the decision rests on:

| | quantum off | quantum 1/16 |
|---|---:|---:|
| glyph-lane marks the packer had **no room** for | **74 820** | **64 998** |
| marks `admits` refused (over the texture, or over `MAX_TILE_SHARE`) | **40** | **40** |
| **largest single page's working set** | **4 298 422 B (4.10 MiB)** | 4 269 439 B |
| **pages whose working set exceeds the 8 MiB atlas** | **0** | **0** |

And the finding that decides everything else — `issue12295.pdf`, which is 85 % of the whole
effect, on its two consecutive frames:

| | reached an entry | no room | sheet tiles | `atlas_repacked` |
|---|---:|---:|---:|---:|
| frame 1 | 2 435 | **63 826** | 63 849 | **true** |
| frame 2 | **66 261** | **0** | 23 | false |

Its working set is 829 764 bytes — **a tenth of the default atlas**. On a fresh device it
loses nothing on the first frame either. What it did not fit was an atlas already holding
5 644 entries belonging to the pages drawn before it.

**And the two runs' affected page sets share only six of about twenty names.** A page that
loses 3 838 marks in one run loses none in the other. Which page pays is not a property of
the page; it is a property of when the shared atlas happened to run out.

## The decision

### 1. Nothing about the atlas's policy changes

`DEFAULT_ATLAS_BUDGET` stays 8 MiB, `MAX_TILE_SHARE` stays an eighth, admission stays as
ADR 0024 left it, and the repack rule stays as ADR 0050 left it.

**Raising the budget is measured to be the wrong lever.** Not one of the 74 820 marks
belongs to a page the atlas is too small for — the worst page in the corpus asks for half
of it and the page carrying 85 % of the loss asks for a tenth. What a larger atlas would
buy is a longer interval between exhaustions, proportionally, and never no exhaustion: the
accumulation is unbounded in the number of pages and any texture is bounded in bytes.
`MAX_TILE_SHARE` refused seven marks in 259 269.

### 2. No recency — and this is ADR 0024's question, answered

ADR 0024 recorded recency as "the next question", ADR 0050 declined it again and named the
condition that would settle it: `Counters::atlas_repacked` true on frame after frame of a
real workload. Measured, this workload is not that:

- **With stable resource identifiers the atlas does not oscillate.** Nine consecutive
  frames of the glyph page at 4× against 64 KiB, 256 KiB, 1 MiB and 8 MiB atlases —
  including the 256 KiB row that *is* ADR 0050's band, working set inside the atlas by
  bytes and outside it by shelves — report identical counters on every frame and
  `atlas_repacked` false throughout. ADR 0050's fix is intact.
- **A page that finds a full atlas settles on its next frame**, which is the table above:
  one repack, then everything cached, exactly the bound ADR 0050 claims.

So the state recency would answer is not thrash; it is **accumulation**, and the whole-atlas
repack already reclaims it — one frame late, once per exhaustion, which over the corpus is
19 frames in 948 pages. Against that: entries carry no last-used stamp today, and the packer
is deliberately append-only, which is what makes ADR 0050's "a repack reproduces the layout
it replaced" a proof rather than a hope. Per-entry eviction needs a free list or compaction,
and either makes that argument false and every retained encode's absolute texel origin a
question. **A redesign of the packer to save one frame per fifty pages is not a trade this
measurement supports.** Recency is closed, not deferred; reopen it against a workload where
`atlas_repacked` is true on frame after frame with the resource identifiers held still.

### 3. Two counters, because the state that was actually happening had no name

This is the round's whole deliverable, and it is CLAUDE.md's rule that a decline nobody
counts is a cost nobody adds up — the sentence `clip_residue_tiles` already exists under.

- **`Counters::atlas_overflow_tiles`** — glyph-lane marks this frame drew through the
  scratch sheet because the packer had no room. Counted per placement in `commit_glyph` on
  the failure path, which already packs a scratch tile, so a frame under no pressure pays
  nothing. **A named part of `LaneCounts::path`**, which the census landed as the whole of
  that lane and which therefore cannot say why a mark is in it — §11.2 needed exactly this
  breakdown and built a throwaway instrument for it, as did this round. It is the one
  reason-code worth keeping permanently, because it is the only one that is a property of
  the device's *history* rather than of the page, and so the only one a caller cannot
  predict from the scene it handed over. Beside `atlas_working_set_bytes` it separates the
  two states **by construction**:
  a working set over the atlas is a page the atlas cannot hold and a budget fixes it; a
  working set under the atlas with overflow tiles is an atlas holding another page's, and no
  budget touches it.
- **`Limits::atlas_bytes`** — the atlas the device actually made. `Options::atlas_budget` is
  a *request*: `AtlasStore::new` sizes near-square, caps the width at 2048 and clamps both
  sides to the adapter's texture limit, so no atlas can exceed `2048 × max_target_size` —
  **32 MiB on this adapter, whatever the caller asks for**. Until now nothing said so, and
  `atlas_working_set_bytes`'s own rustdoc named the request as "the number this has to be
  compared against", which is the wrong number precisely when the cap bites. §5's rule is
  that a limit which must exist is discoverable before the frame.

Both are additive, neither moves a pixel or a refusal, and `Limits::atlas_bytes` is taken
from `AtlasStore::byte_size()` rather than re-derived, because two arithmetics for one
number is how they come to disagree.

### 4. "Reversible by turning the quantum on" is withdrawn as a property of the quantum

The quantum **divides** distinct keys — `Some(16)` collapses placements differing only in
sub-pixel phase onto one key, `None` keys the exact bits and almost never collides — so
turning it on reduces pressure, and the census's headline column was taken with it *off*
because the corpus gate sets `glyph_quantum: None` deliberately, to isolate fidelity from
the trade `real_pages.rs` gates separately.

`doc/notes-census.md` landed while this round ran, so the two independent measurements can
be compared, and they agree to the mark on the residue count (**6 186** both), on
`issue12295.pdf`'s path-lane marks (**63 849**) and on its keys collapsing **66 261 →
66 232**; 19 overflowing pages both; 69 834 against 74 820 on the headline.

**They disagree on one thing: the census measures the quantum taking 69 834 to 5 355, and
this round measures it taking 74 820 to 64 998.** A 92 % reversal against a 13 % one.

Both are honest, and the disagreement is the finding rather than a discrepancy to resolve.
The quantum's stable effect is on distinct keys (−17 % here). What it does not stably change
is *which page is rendering when the shared atlas runs out* — only how fast it fills. In the
census's quantum-on run `issue12295.pdf` met an atlas with room and lost 23 marks; in this
round's it met one holding 15 288 entries and lost 58 891. The census recorded the same
instability from its own side — "which of them overflows is unstable, the mechanism is one",
with `tracemonkey_a11y.pdf` tipping 0 → 2 756 the *other* way — and this round's sharpest
form of it is that **the 19 pages overflowing with the quantum off and the 20 overflowing
with it on share six names.**

So the mechanism, its cause, its bound and the absence of any page too large for the atlas
are all stable and are what this ADR decides on. "Reversibly" is not, and `PLAN.md`'s
still-open bullet should stop carrying it.

## What this does not do

- **It does not extend the census to `Coverage::Cpu`.** The mechanism behind the
  accumulation is that on the default lane every admitted tile enters the atlas whether the
  page will read it again or not: `CacheProspect::worth_caching` is consulted by
  `take_gpu_lane` alone, and that answers `false` on sight under `Coverage::Cpu`. So one
  page can deposit 66 261 single-use entries and a *different* page pays for them. That is
  ADR 0029's own recorded "revisit when" — the census's looseness measured to matter — and
  it belongs to ADR 0029 with its own A/B and its own corpus run. It is a real lane change:
  moving one page's 66 261 tiles from the atlas onto the scratch sheet is exactly what can
  push a page past the sheet's ceiling, which is already why `issue1905.pdf` refuses at 4×.
  **The counterfactual is already measured**, from the other side: `doc/notes-census.md` §3
  reports the same column reading **11.60 % rather than 33.15 % under `Coverage::Gpu`**,
  "because ADR 0029's placement census keeps single-use tiles out of the atlas and leaves
  room for the reused ones". That is this mechanism, observed on the lane where the census
  *is* consulted, and it is the strongest evidence either round has for what the next one
  should do.
- **It does not fix the caller's oscillation, and does not claim it is ours.** On
  `render_quorra`'s single-list `rasterize` path, `issue12295.pdf` against a 1 MiB atlas
  repacks on every second frame for ever — `atlas_repacked` alternating, ADR 0050's
  "revisit when" met. The driver is that the page's atlas keys are not stable between
  renders on that path: the same page's two consecutive frames left 66 261 and then 132 097
  entries, so the second render inserted 65 836 keys the first had already inserted, and the
  only field of `GlyphKey` that can have moved is the outline identifier. Their
  `ResourceCaches` releases and re-uploads after every frame on that path; the window path
  retains instead. It is a finding to hand back, and isolating it further means instrumenting
  their tree.
- **It puts no clock on what an overflowing frame costs.** The counts are exact and this
  machine cannot measure a duration (ADR 0052's seam). What can be said is what was counted:
  63 849 sheet tiles against 23, and 3 958 220 coverage texels against 3 513 791, for the
  same page one frame apart.

## What was tested

`tests/atlas_budget.rs`, three tests, each verified able to fail by forcing the defect it
exists for:

- `the_budget_is_a_request_and_the_limit_is_what_it_bought` — both directions, so a field
  that merely echoed `Options::atlas_budget` cannot pass it. Forced by making `Limits` echo
  the request; the test went red and the other two stayed green.
- `a_frame_says_how_many_marks_the_full_atlas_cost_it` — the same page against a roomy and a
  4 KiB atlas: zero overflow tiles then some, `overflow + entries` accounting for every key,
  and **the pixels equal**, which is why the counter had to exist at all. Forced by making
  the increment add zero.
- `a_page_too_large_and_an_atlas_holding_another_page_are_told_apart` — the pair of
  counters, in both states, each asserting its own precondition. Same forced defect.

Its outlines are **triangles**, and that is load-bearing: the first version used squares,
which are `rect_hint`'s shape and take the analytic rectangle lane (ADR 0047), so every
assertion read zero and the file proved nothing. `HANDOVER.md`'s "a fixture that names a
lane should say which lane it means", met head on.

## Revisit when

- A workload reports `atlas_repacked` true on frame after frame **with its resource
  identifiers held still**. That is the thrash ADR 0024 named and this ADR did not find;
  recency is what answers it.
- A page reports `atlas_working_set_bytes` above `Limits::atlas_bytes`. None does at 4×
  today; that is the page a larger budget is for, and it is now a comparison a host can make
  before the frame rather than a slowness to diagnose after it.
