# The atlas budget, measured — what the glyph lane actually surrenders at 4×

2026-08-17. The round §11.2's corpus census set up: it found the glyph lane losing tens of
thousands of marks at 4× "to the atlas budget", reversibly, and found that `doc/PLAN.md`
§1.1's stated mechanism for that surrender — a tile outgrowing an atlas entry — accounts
for a rounding error of it. This is what those marks turned out to be.

`doc/notes-census.md` was **not** on `main` when this round started, so every figure below
was measured again from scratch by an instrument written without sight of theirs. It landed
before the round finished, which turned an accident into the better experiment: two
independent measurements of the same corpus, and §6 reconciles them. They agree on the
mechanism to the mark, and they disagree about one claim.

`notes-census.md` §7 ends "That is ADR 0024's recency question, which `HANDOVER.md` records
as 'waiting for its measurement since' — this is a measurement it can be reopened with."
**It was reopened, and the answer is that recency is not what this needs**: the pages that
lose marks are not too large for the atlas — the largest asks for half of it and the worst
offender for a tenth — and a page that finds a full atlas has all its glyphs cached again on
its next frame. What the atlas has is no *eviction*, so it accumulates across pages until
something is refused; the repack already reclaims that, one frame late and once per
exhaustion.

The decision is `doc/adr/0063-the-atlas-fills-across-pages-and-the-budget-is-not-what-ran-out.md`.

---

## 1. The instrument, and why it had to be built

Nothing in `Counters` can answer "how many marks did the glyph lane surrender, and to
what?". `atlas_distinct_keys` counts what was asked for, `atlas_entries` counts what is
resident *after* the frame, and `tiles` counts the scratch sheet with every lane's output
mixed into it. The difference between "asked" and "resident" is not the answer either,
because the atlas carries entries from earlier frames — the count before the frame is not
reported, so the subtraction cannot be done from outside. §7 is what this round did about
that.

So: a temporary instrument, **deleted with the round**, described here so that it can be
rebuilt rather than re-derived. Process-global `AtomicU64`s (`src/lanecount.rs`, gone)
bumped at each of the five points a solid fill can fail to reach an atlas entry, and one
line per frame appended to the file named by `QUORRA_LANE_DUMP`:

| bumped where | what it counts |
|---|---|
| `Encoder::fill_solid`, entry | every solid fill that reaches the lane choice |
| `Encoder::fill_solid`, GPU arm | `take_gpu_lane` took it (always 0 under `Coverage::Cpu`) |
| `Encoder::fill_solid`, fall-through | `admits` refused — split into *over the texture's own extent* and *over `MAX_TILE_SHARE`* by a temporary `AtlasStore::admit_reason` |
| `Encoder::fill_solid`, fall-through | a residue clip sent it to the sheet |
| `commit_glyph`, `inserted.is_none()` | it **entered** the glyph lane and the packer had **no room** |
| `commit_glyph`, success | it was drawn from an atlas entry |

Global atomics rather than encoder fields on purpose: the plumbing is not the subject, and
every bump site is on the walk thread or the commit thread, both of which are the calling
thread (`encode/parallel`'s three phases put only the rasterisation on the pool).

The caller's corpus harness was copied per `HANDOVER.md`'s recipe and **its copy** — never
`/home/cl/projects/pdf-viewer` — gained three knobs: a `# <page name>` line into the same
dump file before each page, `PDFVIEWER_QUORRA_QUANTUM`, and `PDFVIEWER_QUORRA_ATLAS`
(the gate hard-codes `glyph_quantum: None` and the default budget).

**The instrument moved no pixel**, which the run itself says: the differing and refused
lists of the 4× run below are exactly the ones `doc/PLAN.md` records for scale 4 on the
`Coverage::Cpu` lane — 11 differing pages and 3 refusals, the same names.

---

## 2. The corpus at 4×

Page one of 974 documents at four times the page's own scale, RADV, `Coverage::Cpu`,
24 encode threads, release. **One device for the whole run**, which is what the harness
does and what a viewer does. 948 pages produced a frame; the rest are refused or not
comparable. Both runs are the same copy of the caller's tree on the same day.

| | quantum **off** (what the gate sets) | quantum **1/16** (the shipped default) |
|---|---:|---:|
| solid fills reaching the lane choice | 259 269 | 259 269 |
| — drawn from an atlas entry | 178 217 | 188 039 |
| — **the packer had no room** | **74 820** | **64 998** |
| — `admits`: larger than the atlas texture | 33 | 33 |
| — `admits`: over `MAX_TILE_SHARE` | 7 | 7 |
| — a residue clip took it to the sheet | 6 186 | 6 186 |
| — the GPU lane took it | 0 | 0 |
| distinct keys asked for | 229 663 | 189 656 |
| pages that lost a mark to the packer | 19 | 20 |
| pages reporting `atlas_repacked` | 19 | 20 |
| **largest single page's working set** | **4 298 422 B (4.10 MiB)** | 4 269 439 B (4.07 MiB) |
| p99 of the per-page working set | 1 478 793 B | 1 229 016 B |
| median | 11 234 B | 11 105 B |
| **pages whose working set exceeds the 8 MiB budget** | **0** | **0** |

631 of the 948 pages ask the atlas for anything at all. Summed over the whole corpus the
distinct keys come to 104.8 MiB — a number no single page ever asks for, and the one §3
turns on.

**So the census's 69 834 marks are the packer.** Not `admits`, not the share rule, not a lane
choice: a tile that entered the glyph lane, was rasterised, was offered to
`AtlasStore::insert`, and got `None` because the sheet was full. `admits` refused
**40 marks of 81 046** that left the glyph lane — 0.049 %, which is §1.1's stated
mechanism measured, and it is the same 0.06 % the census reported.

---

## 3. What was full, and with what

`issue12295.pdf` is 85 % of the whole effect on its own: 63 826 of the 74 820. Its two
consecutive frames — the timed one and the one the harness renders again to write
artefacts — are the entire finding in two lines:

| | fills | reached an entry | no room | sheet tiles | coverage texels | `atlas_repacked` |
|---|---:|---:|---:|---:|---:|---:|
| frame 1 | 66 279 | 2 435 | **63 826** | 63 849 | 3 958 220 | **true** |
| frame 2 | 66 279 | **66 261** | **0** | 23 | 3 513 791 | false |

Its working set is **829 764 bytes — under a tenth of the 8 MiB atlas.** The page fits the
default budget nine times over. What it did not fit was an atlas already holding 5 644
entries belonging to *the pages rendered before it*, and the repack that followed frame 1
gave frame 2 an empty sheet, at which point every one of its 66 261 keys was admitted.

The same page rendered on a **fresh device** loses nothing at all: `no room = 0` on the
first frame, no repack, 66 261 entries, 23 tiles.

Two more facts close the case that this is not a property of the pages:

- **The 19 affected pages and the 20 affected pages share only six names.** Turning the
  quantum on changes *which* pages lose marks, because it changes how fast the shared
  atlas fills and therefore which page is unlucky enough to be rendering when it does.
  A page that loses 3 838 marks in one run loses none in the other, and vice versa.
- The atlas has no eviction. It accumulates every admitted key of every page ever drawn,
  and the only thing that ever removes one is a whole-atlas repack — which `settle_atlas`
  takes **only under pressure**, i.e. only after a frame has already paid for the
  exhaustion by discovering it.

That is the mechanism, stated once: **the atlas fills up across pages, the frame that
finds it full pays for all of its own glyphs, and the repack that follows clears it for
the next frame.** ADR 0050 bounds that at one frame, and the bound holds (§5).

### Why a page deposits keys it will never re-read

`issue12295.pdf` asks for 66 261 distinct keys over 66 279 fills, and quantising to 1/16
of a pixel collapses that to 66 232 — **0.04 %**. Its shapes do not repeat; they are 66 000
different things drawn once each. ADR 0029 built the census exactly for that page shape,
and `CacheProspect::worth_caching` answers "no" for it — but `worth_caching` is read by
`take_gpu_lane` alone, and `take_gpu_lane` returns `false` on sight under `Coverage::Cpu`.
`CacheProspect::admission` does not consult it.

So **on the default lane every admitted tile enters the atlas whether the page will read it
again or not.** That is not a defect: `encode.rs` says so in the comment beside
`census: match coverage { … Coverage::Cpu => Census::default() }`, and ADR 0029 §"What it
costs" states it as the reason the census walk is `Gpu`-only. What this round adds is the
consequence, with a number: one page can leave 66 261 single-use entries in a shared atlas,
and the page that pays for them is a different one.

---

## 4. The budget is not the binding constraint, and above 32 MiB it is not even the budget

No page at 4× asks for more than 4.10 MiB. Raising `DEFAULT_ATLAS_BUDGET` cannot move a
single one of the 74 820 marks for the reason its own doc comment would suggest, because
not one of them belongs to a page the atlas is too small for. What raising it *would* do is
lower how often the atlas is exhausted — proportionally, and never to zero, because the
accumulation is unbounded in the number of pages and the atlas is bounded in bytes.

And there is a ceiling on the lever anyway. `AtlasStore::new` sizes near-square, **caps the
width at 2048**, and clamps both sides to the adapter's texture limit:

```
side   = isqrt(budget).min(2048).min(max_dimension)
height = (budget / side).min(max_dimension)
```

On this adapter `max_dimension` is 16 384, so the largest atlas that can exist is
2048 × 16 384 = **32 MiB**. A caller that sets `atlas_budget` to 64 MiB gets 32, a caller
that sets 512 MiB gets 32, and **nothing anywhere says so** — `Limits` carried no atlas
field and `Counters::atlas_working_set_bytes`'s own rustdoc named `Options::atlas_budget`
as "the number this has to be compared against", which is the wrong number the moment the
cap bites. §7 fixes that.

---

## 5. Thrash, or a working set that does not fit? Neither — and ADR 0050's bound holds

`Counters::atlas_repacked` is the instrument ADR 0050 built for this question, and it
answers it twice over.

**On a scene whose resource identifiers are stable — ours — the atlas does not oscillate.**
The glyph page at 4×, nine consecutive `Device::render` frames, one device per row, RADV:

| atlas budget | working set | reached an entry | no room | sheet tiles | `atlas_repacked`, nine frames |
|---:|---:|---:|---:|---:|---|
| 64 KiB | 165 596 | 225 | 368 | 368 | `.........` |
| 256 KiB — ADR 0050's band row | 165 596 | 572 | 21 | 21 | `.........` |
| 1 MiB | 165 596 | 593 | 0 | 0 | `.........` |
| 8 MiB | 165 596 | 593 | 0 | 0 | `.........` |

Identical counters on every frame of every row, `atlas_repacked` false throughout —
including the 256 KiB row, which is the (budget, magnification) pair ADR 0050 was written
for and where the working set fits by bytes and not by shelves. `examples/retained.rs`'s
overflow section reads `E...........` on the same shape. **The fix is intact.**

**On the caller's single-list `rasterize` path it oscillates with period two.**
`issue12295.pdf` at 4× against a 1 MiB atlas — working set 742 008 bytes, so squarely
inside ADR 0050's band — nine frames of the same page:

```
frame   0    1    2    3    4    5    6    7    8
entry  2974  300 2974  300 2974  300 2974  300 2974
repack   0    1    0    1    0    1    0    1    0
```

For ever. This is ADR 0050's "Revisit when" condition met on a real workload — and it is
**not** our repack rule failing. The driver is that the page's atlas keys are not the same
from one render to the next: on a fresh device the same page's two frames left 66 261 and
then 132 097 entries, i.e. the second render inserted 65 836 keys the first had already
inserted. Same page, same target, same options, exact-phase keying — the only field of
`GlyphKey` that can have moved is the outline identifier, and `render_quorra`'s
`ResourceCaches` releases and re-uploads on that path (`caches.begin_frame()` plus
`evict_settled` after every frame; the *window* path retains instead). Every frame therefore
finds the previous frame's entries foreign, `resident > atlas_entries_used` holds, and the
repack fires.

That is ADR 0050's recorded not-taken — "two pages that alternate and do not fit beside each
other still repack once per frame" — reached by a second route, and it is **a finding to hand
back to the caller** rather than a change here. It was not isolated further: doing so means
instrumenting their cache, which is their tree.

---

## 6. Two independent measurements, put beside each other

`doc/notes-census.md` landed on `main` while this round was running, so the two can be
compared. **They agree on everything about the mechanism, three of the figures to the mark:**

| | census (`notes-census.md` §3, §4) | this round |
|---|---:|---:|
| pages compared at 4× | 948 | 948 |
| glyph-lane marks the packer had **no room** for, quantum off | 69 834 | 74 820 |
| pages that overflow at 4× | 19 | 19 |
| marks whose **tile was too large** for an entry | 55 | 40 |
| solid fills under a residue clip | **6 186** | **6 186** |
| `issue12295.pdf` at 4×, path-lane marks | **63 849** | 63 849 (`tiles`) / 63 826 (`no room`) |
| `issue12295.pdf`, keys off → 1/16 | **66 261 → 66 232** | **66 261 → 66 232** |

Two instruments written independently, agreeing to the mark on the residue count and on the
page that carries 85 % of the effect, is as strong as a cross-check in this tree gets. The
7 % on the headline and the 55-against-40 are the run-to-run spread the census's own
methodology note predicts — "a page's overflow count is therefore not purely a property of
that page … the ordering could matter".

### The one claim the two disagree about, and why the disagreement is the finding

**The census says the effect is reversible by turning the quantum on: 69 834 → 5 355. This
round measures 74 820 → 64 998.** A 92 % reversal against a 13 % one, on the same corpus, at
the same scale, on the same lane, with the same two quantum values.

Both are honest measurements, and they differ because **the quantity is unstable in exactly
the way the mechanism predicts.** The quantum's stable effect is on distinct keys — 229 663 →
189 656 here, −17 %, and the census's own table shows it collapsing `issue12295.pdf`'s keys by
0.04 % and `comments.pdf`'s by 24 %. What the quantum does *not* stably change is which page
is unlucky enough to be rendering when the shared atlas runs out; it only changes how fast it
fills. In the census's quantum-on run `issue12295.pdf` met an atlas with room and lost 23
marks; in this round's it met one holding 15 288 entries and lost 58 891. Same page, same
options, different history.

The census saw this and wrote it down — "which of them overflows is unstable, the mechanism
is one", and its table has `tracemonkey_a11y.pdf` going 0 → 2 756 in the *other* direction.
This round's independent evidence for the same instability is sharper: **the 19 pages that
overflow with the quantum off and the 20 that overflow with it on share only six names.**

So "reversibly" should not be carried forward as a property of the quantum. What is stable
is: the mechanism (a full sheet), its cause (accumulation with no eviction), its bound (one
frame, then a repack), and that no page's own working set is the problem.

### Why the quantum reduces pressure at all, since that direction surprised the round's brief

It **divides** distinct keys; it does not multiply them. `Some(16)` rounds a placement's
sub-pixel phase to one of 16 buckets per axis so placements differing only in phase collide
on one key and one rasterisation; `None` keys the exact `f32` bits, which almost never
collide. And the corpus gate sets `glyph_quantum: None` **deliberately**, to measure fidelity
rather than the sub-pixel trade `real_pages.rs` gates separately — so the configuration the
census's headline column was taken in is not the one the caller ships.

---

## 7. What changed

Nothing in the atlas's policy. Two instrument fields, because the round's whole difficulty
was that the state it was measuring is invisible from outside — which is CLAUDE.md's
"a decline nobody counts is a cost nobody adds up", the same sentence `clip_residue_tiles`
exists under.

- **`Counters::atlas_overflow_tiles`** — glyph-lane marks this frame drew through the
  scratch sheet because the packer had no room. Counted in `commit_glyph` on the failure
  path, which already packs a scratch tile, so a frame under no pressure pays nothing.
  Together with `atlas_working_set_bytes` it separates the two states by construction:
  a working set over the atlas is "too small for this page", a working set under it with
  overflow tiles is "holding another page's".
- **`Limits::atlas_bytes`** — the atlas the device actually made, after the near-square
  sizing, the 2048 width cap and the adapter clamp. `Options::atlas_budget` is a request;
  this is the answer, and it is what `atlas_working_set_bytes` must be compared against.
  Principle 6: a limit that must exist is discoverable before the frame.

Both are additive and neither moves a pixel or a refusal, so no base-vs-change corpus run
is owed for them. The 4× corpus run above was made through the same code paths and
reproduced `doc/PLAN.md`'s recorded scale-4 verdict lists name for name.

Doc corrections that go with them: `DEFAULT_ATLAS_BUDGET`'s justification (a scale-1
argument, replaced by §2's corpus figure), `AtlasStore::new`'s silence about the two caps,
and `atlas_working_set_bytes`'s rustdoc naming the wrong number to compare against.

## 8. What was deliberately not changed

- **`DEFAULT_ATLAS_BUDGET` stays 8 MiB.** No page at 4× asks for more than 4.10 MiB (§2),
  so the default is right by a factor of two at the worst page in the corpus, and raising it
  addresses none of the 74 820 marks.
- **`MAX_TILE_SHARE` stays an eighth.** It refused 7 marks of 259 269.
- **No recency.** ADR 0024's open question, closed in ADR 0063 §2 with the numbers above
  rather than left open a fifth month.
- **The census is not extended to `Coverage::Cpu`.** It is the strongest candidate this
  round turned up (§3), it is ADR 0029's own "revisit when", and it belongs to ADR 0029 with
  its own A/B and its own corpus run — moving 66 261 tiles of one page from the atlas onto
  the scratch sheet is exactly the kind of change that can push a page over the sheet's
  ceiling, which is already why `issue1905.pdf` refuses at 4×.

## 9. Recommended edits to `doc/PLAN.md` and `doc/HANDOVER.md`

The owner maintains both; these are proposals, quoted so they can be pasted or argued with.

### 9.0 What the census round already applied, and what is left

The census landed §1.1's correction as a **paragraph after the lane table** ("One clause of
this table is wrong and is corrected below"), and as a still-open bullet at the top of the
file. Both are right about the mechanism and both are what this round confirms
independently. Two things are left:

- **The bullet the correction corrects is still uncorrected.** §1.1's "Classification
  happens at encode time" bullet still says "a general path at 6400%, when its device size
  outgrows what an atlas entry can hold", and a reader meets it three paragraphs before the
  correction. 9.1 replaces it.
- **"Reversibly" should go**, in both places. §6 above: two independent runs measure the
  quantum taking the total to 5 355 and to 64 998. What is reversible is not the effect, it
  is which page is unlucky.

### 9.1 `PLAN.md` §1.1 — the bullet the correction corrects

Replace the first bullet under "Two properties of the sorter matter more than the lanes
themselves" — the sentence "the same glyph outline is a quad at 100% zoom and a general path
at 6400%, when its device size outgrows what an atlas entry can hold" — with:

> - **Classification happens at encode time, per frame — never at scene-build time.**
>   Which lane a command takes is a device-space question: the same glyph outline is a quad
>   at 100% zoom and a general path once it outgrows what the atlas will do for it.
>   **Measured, that is almost never a question about the tile's size.** Over page one of
>   974 corpus documents at 4× (ADR 0063, `doc/notes-atlas-budget.md`), **40 marks of
>   81 046** left the glyph lane because the tile was larger than the atlas or over
>   `MAX_TILE_SHARE`, and **74 820** left it because the packer had **no room** — a sheet
>   full of *earlier pages'* tiles, on 19 pages of 948, not one of which asks for more than
>   half the default budget. So the sentence to hold in mind at magnification is not "the
>   tile grew" but "the sheet was full", the two are told apart by
>   `Counters::atlas_overflow_tiles` beside `Counters::atlas_working_set_bytes`, and the
>   repack after the frame is what clears it. Putting the sorter in `render` is what keeps
>   the `Scene` viewport-free (§2.3), which the brief calls the most important property in
>   the document. The budget for the whole encode is the number the current backend already
>   achieves: **1.1–1.6 ms, flat in resolution** (§6.1). Ours may not regress it, because it
>   is a function of the command list and not of the pixels, and that flatness is
>   structural, not accidental.

And the glyph row of the lane table, so it names both conditions:

> | **glyph** | a fill of an uploaded outline whose device-space size fits an atlas entry **and for which the atlas has room** — §1.1's dominant case, 5 933 of one dense page's commands over 107 distinct outlines | one instanced quad sampling the R8 coverage atlas (§6.3) |

### 9.2 `PLAN.md` "The numbers that stand" — two rows

> | the atlas at 4×, over 948 corpus pages: largest single page's working set | **4.10 MiB** against an 8 MiB budget; p99 1.41 MiB, median 11 KiB; **0 pages over budget** | ADR 0063, one copy of their tree, 2026-08-17 |
> | — and the marks the glyph lane still lost there | **74 820** to a packer with no room, on 19 pages, against **40** to the size rule | same; `Counters::atlas_overflow_tiles` is the counter that now says so |

### 9.3 `PLAN.md` M4, work item 2 — the counter list, and the number to compare against

> 2. The packer, eviction, and the budget check; `atlas_entries` and
>    `atlas_distinct_keys` in `Counters` — the distinct-key count, not the hit rate.
>    Two more joined them at ADR 0050 and a fourth at ADR 0063.
>    `atlas_working_set_bytes` is what holding **all** of a frame's distinct glyph keys
>    would cost, which is the number `Limits::atlas_bytes` is compared against —
>    `Options::atlas_budget` is a *request*, and the atlas is capped at
>    `2048 × max_target_size` whatever it says. `atlas_repacked` is whether the atlas
>    was thrown away and re-packed after this frame — the one event that makes a retained
>    encode stale. A page that settles reports it true on at most one frame; true on frame
>    after frame is thrash, and the counter exists so that state has a name.
>    `atlas_overflow_tiles` is how many of *this* frame's glyph marks the packer had no
>    room for, which is the half of the pair that says whether the page or the history is
>    what did not fit. Over the corpus at 4× it is always the history.

### 9.3b `PLAN.md` §1.1's correction paragraph — one clause, and the ADR to point at

Replace the last sentence of "**One clause of this table is wrong and is corrected below**":

> What moves the other 69 834 at 4× is the atlas *budget* — more precisely, the atlas having
> no **room**, because it is full of earlier pages' tiles. ADR 0063 measured it
> independently: 74 820 marks on the same 19 pages, **no page's own working set above
> 4.10 MiB of the 8 MiB atlas**, and the page carrying 85 % of it asking for a tenth. So it
> is not the budget that ran out, and `Counters::atlas_overflow_tiles` beside
> `atlas_working_set_bytes` is what now tells the two apart. **It is not reversible by the
> quantum either**: the two runs put that total at 5 355 and at 64 998, because the quantum
> changes how fast the shared atlas fills and not which page is rendering when it does — the
> 19 pages that overflow with it off and the 20 with it on share six names.

### 9.4 `PLAN.md` "What is still open" — replace the census bullet's last clause, and add one

The census bullet's "What the census left open is one thing it was not asked about: the
glyph lane surrenders 69 834 marks at 4× to the atlas **budget**, on 19 pages, reversibly —
§1.1's stated mechanism accounts for one mark in 27 507" is now answered; replace it with
"…what the census left open is settled by ADR 0063 (below)", and add:

> - **The atlas's remaining question is ADR 0029's, not ADR 0024's** (ADR 0063,
>   2026-08-17). Recency is closed with a number: a page that finds a full atlas settles on
>   its next frame, nine consecutive frames of a page in ADR 0050's band never repack, and
>   no corpus page at 4× asks for more than half the default budget. What the measurement
>   turned up instead is that **on `Coverage::Cpu` the census is never consulted** —
>   `worth_caching` is read by `take_gpu_lane` alone — so one page can deposit 66 261
>   single-use entries in the shared atlas and a *different* page pays for them. That is
>   ADR 0029's own recorded "revisit when", it now has a page name and a count against it,
>   and it needs its own A/B and its own corpus run because it can move a page past the
>   scratch sheet's ceiling.

### 9.5 `PLAN.md`, the adoption-round bullet — the API bump list

Add to the list of additive items the bump owes the caller:

> `Counters::atlas_overflow_tiles` and `Limits::atlas_bytes` (ADR 0063), both additive; the
> second matters to them because `atlas_working_set_bytes`'s documented comparand changed
> from a request to a size.

### 9.6 `HANDOVER.md` "Recorded and deliberately not taken" — the atlas entry

Replace the entry beginning "the atlas has no recency, so two pages that alternate…":

> - the atlas has no recency, and **ADR 0063 closed the question with the measurement it had
>   been waiting for since ADR 0024** rather than deferring it a third time: over 948 corpus
>   pages at 4× no page's working set exceeds half the default budget, a page that finds a
>   full atlas settles on its next frame, and nine consecutive frames of a page inside
>   ADR 0050's band never repack. So the state recency answers is accumulation, the repack
>   already reclaims it one frame late and once per exhaustion, and per-entry eviction would
>   cost the append-only packer that makes ADR 0050's proof a proof. Reopen it against a
>   workload where `atlas_repacked` is true frame after frame **with the resource
>   identifiers held still** — the caller's single-list `rasterize` path oscillates with
>   period two and that is their id churn, not our rule (ADR 0063 "What this does not do");

### 9.7 `HANDOVER.md` Traps — one new entry

> **A cache with no eviction has a cost that lands on whoever is next.** The glyph atlas
> keeps every admitted key of every page until something is refused, and only then throws
> everything away. Over the caller's corpus at 4× that put **74 820 marks** on the scratch
> sheet — 85 % of them on one page whose own working set is a tenth of the atlas — and it
> looked exactly like "the budget is too small for zoomed pages". It is not: **which** page
> pays is not a property of any page, and the proof is that two runs differing only in the
> sub-pixel quantum have 19 and 20 affected pages sharing **six names**. When a cost appears
> on a page, check whether the page caused it before believing the page's own numbers
> explain it — and give the shared state a per-frame counter, because `atlas_entries` after
> the frame minus what the frame asked for is not the same subtraction and the count before
> the frame was never reported (ADR 0063).

### 9.8 `HANDOVER.md` Instruments — one new entry

> - **Which lane a real page's marks actually take, and why they left the glyph lane**: not
>   in the tree. `doc/notes-atlas-budget.md` §1 describes the temporary instrument that
>   measured it — process-global atomics at the five points a solid fill can miss an atlas
>   entry, one line per frame into `QUORRA_LANE_DUMP`, plus three knobs added to a **copy**
>   of the caller's corpus harness (`PDFVIEWER_QUORRA_QUANTUM`, `PDFVIEWER_QUORRA_ATLAS`, a
>   per-page label line). Rebuild it from that section rather than re-deriving it; it takes
>   about forty minutes for two 948-page runs at 4×. The permanent residue is
>   `Counters::atlas_overflow_tiles`, which answers the one question the round most needed
>   and could not ask.

### 9.9 Already taken here, since it names a constant this round's ADR deleted

`examples/zoom.rs`'s module comment said "past `MAX_GLYPH_DIM` every visible glyph leaves the
atlas for the coverage path". `MAX_GLYPH_DIM` was removed by ADR 0024 fourteen ADRs ago; the
comment now names `MAX_TILE_SHARE`, says when it was wrong, and points at the mechanism that
actually dominates.
