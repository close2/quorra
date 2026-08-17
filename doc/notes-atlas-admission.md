# What the atlas admits on the default lane, measured

2026-08-17. The round ADR 0063 named: it found that the glyph lane's surrender at zoom is
*accumulation* — the atlas has no eviction, so one page's tiles fill it and a **different**
page is refused — and pointed at the mechanism behind the accumulation:

> `CacheProspect::worth_caching` is consulted by `take_gpu_lane` alone, and that answers
> `false` on sight under `Coverage::Cpu`. So one page can deposit 66 261 single-use entries
> and a *different* page pays for them.

This is that statement checked, that filter priced on both sides, and the decision.
The decision is `doc/adr/0065-a-within-frame-criterion-is-the-wrong-axis-for-the-cached-lane.md`.

**Result: measured, and left.** The filter works — it removes 88.7 % of the refusals — and
it costs 31.7 % of the corpus's cached marks on every frame after the first. The ratio is
the wrong way round, and §5 is why that is a property of the criterion rather than of a
threshold that could be tuned.

---

## 1. ADR 0063's statement: confirmed literally, and its implied fix is a no-op

ADR 0063 was written from reading. Read again, and then measured, it is **true in all three
of its parts** — and incomplete in a fourth that reverses what follows from it.

| the claim | how checked | verdict |
|---|---|---|
| `worth_caching` is read by `take_gpu_lane` alone | every call site in the tree: `encode/coverage.rs:253`, plus two in `atlas.rs`'s own tests | **true** |
| it answers `false` on sight under `Coverage::Cpu` | `take_gpu_lane`'s first disjunct is `self.coverage != Coverage::Gpu`, and `||` short-circuits, so `worth_caching` is not even evaluated | **true** |
| every admitted tile enters the atlas whether the page will re-read it or not | `fill_solid`'s glyph arm gates on `cache.admission()`, which is `Some` for **every** `Admitted` variant regardless of `once` | **true** |

**What it does not say, and what decides the round:** under `Coverage::Cpu` the census is
never taken. `encode.rs` builds `Census::default()` on that lane, `Census::placed_once`
answers `false` for a shape it never saw, so `prospect` receives `placed_once: false` and
constructs `once: false` for **every** placement — and `worth_caching` therefore answers
**`true` for every admitted tile**.

So *"consult `worth_caching` under `Coverage::Cpu`"*, taken literally, **changes nothing at
all**. It is not a small win or a risky win; it is a no-op. Making it mean anything requires
also taking the census on that lane, which is the cost ADR 0029 declined by name.

### The control

The claim "the shipped rule marks nothing single-use under `Coverage::Cpu`" is an assertion
that a counter reads zero, and CLAUDE.md's rule is that a gate verified in one direction is
not verified. The same expression — `matches!(cache, Admitted { once: true, .. })` at the
same site, one build, one binary — was run over the corpus at 4× on both lanes:

| lane | placements the shipped prospect calls `once` |
|---|---:|
| `Coverage::Cpu` | **0** |
| `Coverage::Gpu` | **> 140 000** |

The instrument can read nonzero. The zero is a property of the lane, and the forced
condition that produces it is `census: Census::default()`.

## 2. The instrument

Temporary, **deleted with the round**, described so it can be rebuilt rather than
re-derived. Four fields on `Encoder`, one line per frame into the file named by
`QUORRA_ADMISSION_DUMP`, recorded at `fill_solid`'s glyph arm — at the branch that actually
enters the atlas, not at the lane choice, because a residue clip and a refused size both
fall past it:

| field | what it counts |
|---|---|
| `glyph_placements` | placements reaching the glyph lane |
| `once_shipped` | of those, ones the **shipped** prospect calls `once` |
| `once_with_census` | of those, ones a **real** census calls placed-once with no entry yet |
| `key_uses` | per key: placements and tile bytes — so the frame can be asked how many **distinct keys** it placed exactly once |

The last is there for CLAUDE.md's rule that the instrument counts distinct keys and not a
rate, and it is the one that produced §5's finding. A second census is taken
unconditionally beside the shipped one, so both answers are read in the same frame from the
same scene; it feeds no lane choice, and the corpus run proves it inert — base and probe
produced **identical per-page lines** (937 agree, 11 differ, 3 refused at 4×).

The caller's corpus harness was copied per `HANDOVER.md`'s recipe into `/home/AI` and **its
copy** gained a `# <page name>` line and a `PDFVIEWER_QUORRA_QUANTUM` knob.
`/home/cl/projects/pdf-viewer` was never built in and never edited.

**Corpus:** the caller's gate corpus at the revision copied on 2026-08-17, 974 documents'
first pages, 948 drawing at 4×. RADV (`AMD Radeon 890M`), 24 encode threads, one device for
the whole run, release. quorra `main` at `9d5f2af`.

## 3. What the filter would keep out

948 pages, `Coverage::Cpu`, first frame of each page. Bytes are the hull box the atlas was
asked about.

| | 4×, quantum off | 4×, quantum 1/16 | 1×, quantum off |
|---|---:|---:|---:|
| placements entering the glyph lane | 253 043 | 253 043 | 259 471 |
| distinct keys | 229 663 | 189 656 | 237 440 |
| `once_shipped` (**the shipped rule**) | **0** | **0** | **0** |
| `once_with_census` (**what a census would keep out**) | **79 370** | **79 370** | **83 164** |
| their atlas bytes | **43 449 221** (41.4 MiB) | 43 449 221 | 21 401 167 |
| **distinct keys placed exactly once** | **226 368** | 163 408 | **234 300** |
| their atlas bytes | 108 094 150 | 85 484 694 | 26 826 222 |
| overflow tiles (ADR 0063's refusals) | **74 820** | **64 998** | 3 219 |

Two cross-checks land on the mark: **74 820** and **64 998** are ADR 0063's two headline
figures exactly, from an instrument written separately; and the 41.4 MiB those single-use
tiles ask for against an 8 MiB atlas is the accumulation ADR 0063 described, priced.

**The census's answer does not move when the quantum does** — 79 370 in both columns, while
distinct keys fall 229 663 → 189 656. That is ADR 0029's recorded blind spot seen directly:
the census keys on `(outline, linear, rule)` and the quantum acts on the phase, so the
quantum changes the number of keys by 17 % and changes what the census believes by zero.

## 4. What it buys, measured by running it

The candidate is the smallest change that makes the filter mean something: take the census
on both lanes, and require `cache.worth_caching()` before the glyph arm may claim a
placement. Base and candidate, **same copy of the caller's tree, same hour**, compared by
per-page lines.

| | base | candidate |
|---|---:|---:|
| placements entering the atlas (4×) | 253 043 | **173 673** |
| distinct keys (4×) | 229 663 | **150 293** |
| **overflow tiles, 4×, quantum off** | **74 820** | **8 447** |
| pages overflowing | 19 | **9** |
| **overflow tiles, 4×, quantum 1/16** | **64 998** | **7 571** |
| verdicts, 4×, quantum off | 937 agree / 11 differ / 3 refused | **identical, per-page lines byte for byte** |
| verdicts, 1×, quantum off | 931 / 23 / 2 | 931 / 23 / 2 |
| verdicts, 4×, quantum 1/16 | 725 / 223 / 3 | **773 / 175 / 3** |

**66 373 of ADR 0063's 74 820 refusals — 88.7 % — would not happen.** `issue12295.pdf`,
which is 85 % of the whole effect, stops overflowing entirely: 63 826 → 0, because 65 885 of
its 66 261 tiles never enter the atlas to fill it against the next page.

**No page was pushed past the scratch ceiling.** That was ADR 0063's stated risk for this
round, and the refusal list is unchanged at all three configurations; `issue1905.pdf`
refuses at 4× before and after, for the same sheet extent.

Two side effects worth recording. At 1× two pages move in the fourth decimal of the mean
(`issue12295.pdf` 1.6064 → 1.6075, `standard_fonts.pdf` 1.7327 → 1.7326) — the atlas
rasterises at a split integer-origin-plus-phase transform and the sheet at the composed one,
so recombination differs by an ULP; the verdicts do not move. And with the quantum **on**,
48 pages move from *differ* to *agree*, because a tile kept out of the atlas is drawn at its
exact phase instead of a quantised one. That is a measurement of what the sub-pixel quantum
costs in fidelity — the one decision CLAUDE.md says is ours to make and expose — and not an
argument for this change, since turning the quantum off buys it without touching admission.

## 5. What it costs, and why the ratio cannot be tuned

A tile kept out of the atlas is rasterised again on **every frame that needs it**. Comparing
the warmest frame each page reached:

| | base | candidate |
|---|---:|---:|
| marks drawn from a cached atlas entry, 4× | **242 049** | **165 226** |
| lost | | **76 823 — 31.7 %** |

| page | cached marks on the warm frame |
|---|---|
| `issue12295.pdf` | 66 261 → **376** |
| `issue15012.pdf` | 4 241 → 862 |
| `tracemonkey_a11y.pdf` | 4 276 → 1 970 |
| `22060_A1_01_Plans.pdf` | 14 → **0** (274 KB of coverage, re-rasterised every frame) |

So the trade is: **a permanent per-frame cost on a third of the corpus's cached marks, to
remove a transient that ADR 0063 measured at one frame per exhaustion, 19 frames in 948
pages, self-correcting through the repack.** That is the wrong way round by roughly the
ratio between "every frame" and "one frame", and it is worst on exactly the workload ADR
0024 built the admission rule for — a reader *holding* at a magnification.

### The reason it is not a threshold problem

`worth_caching` asks **"does this frame read the entry more than once?"**. On the cached lane
the atlas's value is overwhelmingly **across** frames, not within one — and the corpus says
so unambiguously: **226 368 of 229 663 distinct keys at 4×, and 234 300 of 237 440 at 1× —
98.6 % and 98.7 % — are placed exactly once in their frame**, while the atlas still serves
242 049 cached marks on the warm frame. A within-frame criterion applied to that lane is
measuring the wrong axis, and no tightening of it recovers the cross-frame value it is blind
to. Tightening it *toward* the ground truth makes it worse, not better: a phase-aware census
would keep out 226 368 tiles rather than 79 370, and empty the atlas almost completely.

### Why the same criterion is right on the other lane

Because the two lanes differ in what a `false` answer *routes to*:

- Under `Coverage::Gpu`, `false` sends the tile to the **device**, which ADR 0029 measured at
  two to three times the scratch path for a single use. A one-off cost becomes a **smaller**
  one-off cost. The criterion pays whether or not the tile recurs next frame.
- Under `Coverage::Cpu` there is no device lane — the caller has asked for the CPU
  rasteriser. `false` sends the tile to the **sheet**: the same rasteriser, the same work, a
  different destination, no faster now, and **no entry on the next frame**. A one-off cost
  becomes a per-frame cost, and nothing is bought but atlas room.

So ADR 0029's coupling of the census to `Coverage::Gpu` is **load-bearing for the trade**,
and not merely the cost optimisation its own text presents it as ("the caller's default
configuration pays nothing"). `notes-census.md` §3's 11.60 % against 33.15 % is real and is
the strongest evidence for this round — but it is evidence about the lane where a *faster
single-use alternative exists*, and it does not transfer to the one where it does not.

## 6. What was not done, and what would reopen it

- **Nothing about admission changes.** `worth_caching` stays read by `take_gpu_lane`, the
  census stays on `Coverage::Gpu`, `MAX_TILE_SHARE` and `DEFAULT_ATLAS_BUDGET` stay as
  ADR 0063 left them.
- **Two comments were corrected**, because they state ADR 0029's justification
  unconditionally and it is measurably false on the caller's default lane: `CacheProspect::TooLarge`'s
  "the census keeps single-use tiles out of the atlas, so a full atlas is one holding tiles
  that are being reused", and `worth_caching`'s own silence about which lane it serves. The
  measured conclusion each supports is unchanged; the reason differs per lane, and a
  justification that is true on one lane and false on the other should not read as though it
  were true on both.
- **No clock was put on any of it.** Every figure here is a count and exact; this machine
  cannot measure a duration (ADR 0052's seam). The census walk's own cost stands at
  ADR 0029's 25 µs on a 5 933-command page against an encode of 80 — a cost this decision
  does not have to weigh, because the change is refused on the count alone.
- **Reopen when** a caller draws with `Coverage::Cpu` **and** a way exists to change lanes
  between frames without changing pixels. That is ADR 0029 §3's rejected frame memory, and
  it is the only shape in which the atlas could hold a page's *reused-across-frames* tiles
  while keeping its genuinely single-use ones out. Until then the criterion available is
  within-frame, and §5 is why that one is refused.
- **Also reopen** if a caller reports pressure with the quantum **on** and a page whose keys
  are stable: the quantum-on column (64 998 → 7 571) says the filter's effect survives the
  configuration the viewer actually ships, so this decision rests on the cost side and not on
  the effect being small.
