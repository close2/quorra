# 0065 — A within-frame criterion is the wrong axis for the cached lane

Date: 2026-08-17. Status: accepted. **Closes the question ADR 0063 named**, and closes
ADR 0029's recorded "revisit when" for the case where the census's looseness was expected
to matter. Nothing about admission changes; two comments do. The measurement is
`doc/notes-atlas-admission.md`.

## Context

ADR 0063 established that the glyph lane's surrender at zoom is **accumulation** — the atlas
has no eviction, so earlier pages' tiles fill it until a later page is refused — and named
the mechanism it did not price:

> `CacheProspect::worth_caching` is consulted by `take_gpu_lane` alone, and that answers
> `false` on sight under `Coverage::Cpu`. So one page can deposit 66 261 single-use entries
> and a *different* page pays for them.

The counterfactual looked already measured, from the other side: `doc/notes-census.md` §3
reads the path lane at **11.60 % under `Coverage::Gpu` against 33.15 % under `Coverage::Cpu`**
at 4×, "because ADR 0029's placement census keeps single-use tiles out of the atlas".

## What was measured

948 corpus pages, `Coverage::Cpu`, RADV, one device per run, at 1× and 4× and with the
quantum both off and at 1/16. Base and candidate in **one copy** of the caller's tree, the
same hour, compared by per-page lines.

### 1. ADR 0063's statement is true, and its implied fix is a no-op

All three of its parts hold: `worth_caching` has exactly one non-test call site,
`take_gpu_lane` short-circuits before evaluating it under `Coverage::Cpu`, and `fill_solid`'s
glyph arm gates on `admission()`, which is `Some` for every `Admitted` variant whatever
`once` says.

**What it omits reverses what follows.** Under `Coverage::Cpu` the census is never taken —
`Census::default()`, and an unseen shape answers "not placed once" — so `prospect` builds
`once: false` for every placement and `worth_caching` answers **`true` for everything the
atlas admits**. Consulting it on that lane, literally, changes **nothing**. Measured across
the corpus, the shipped rule calls **0** placements `once` under `Coverage::Cpu`; the same
expression in the same binary reads **over 140 000** under `Coverage::Gpu`. The instrument
can read nonzero; the zero is the lane.

Making the filter mean anything requires taking the census on that lane too — which is the
change actually priced below.

### 2. It works, and no page moved

| | base | candidate |
|---|---:|---:|
| placements entering the atlas, 4× | 253 043 | **173 673** |
| **overflow tiles, 4×, quantum off** | **74 820** | **8 447** |
| **overflow tiles, 4×, quantum 1/16** | **64 998** | **7 571** |
| pages overflowing | 19 | 9 |
| verdicts, 4× | 937 / 11 / 3 | **identical per-page lines** |
| verdicts, 1× | 931 / 23 / 2 | 931 / 23 / 2 |

**88.7 % of ADR 0063's refusals would not happen.** `issue12295.pdf` — 85 % of the whole
effect — stops overflowing entirely. **Nothing was pushed past the scratch ceiling**, which
was ADR 0063's stated risk for this round.

### 3. And it costs a third of the cache, on every frame

| | base | candidate |
|---|---:|---:|
| marks drawn from a cached atlas entry, warm frame, 4× | **242 049** | **165 226** |

**76 823 marks — 31.7 % — stop being cache hits.** `issue12295.pdf` goes from 66 261 cached
marks on its second frame to 376; `22060_A1_01_Plans.pdf` from 14 to none, re-rasterising
274 KB of coverage on every frame for ever.

So the trade is a **permanent per-frame cost on a third of the corpus's cached marks**
against a **transient** that ADR 0063 measured at one frame per exhaustion, 19 frames in 948
pages, already reclaimed by the repack. It is worst on precisely the workload ADR 0024 built
admission for — a reader holding at a magnification.

## The decision

### 1. Admission does not change

`worth_caching` stays read by `take_gpu_lane`, the census stays taken under `Coverage::Gpu`
alone, and ADR 0063's policy stands untouched. ADR 0050's repack rule is unaffected: the
candidate never made `atlas_repacked` true on frame after frame, and declining it cannot.

### 2. Because the criterion is on the wrong axis, not at the wrong threshold

`worth_caching` asks **"does this frame read the entry more than once?"** On the cached lane
the atlas's value is almost entirely **across** frames. The corpus is unambiguous:
**226 368 of 229 663 distinct keys at 4×, and 234 300 of 237 440 at 1× — 98.6 % and 98.7 % —
are placed exactly once in their own frame**, while the atlas still serves 242 049 cached
marks on the warm frame. Every one of those is a tile whose whole worth is that the *next*
frame finds it.

This is why no tuning helps, and why ADR 0029's blind spot points the opposite way from what
was expected. Sharpening the census toward the ground truth makes it **worse**: a phase-aware
census would keep out 226 368 tiles rather than 79 370 and empty the atlas nearly completely.
The looseness was doing useful work by accident. Relatedly, the census's answer is
**identical with the quantum on and off** — 79 370 both — while distinct keys fall 17 %,
because it keys on the shape and the quantum acts on the phase.

### 3. And because the two lanes route a `false` to different places

This is the finding, and it is a property of the design rather than of this corpus.

- Under **`Coverage::Gpu`**, `false` routes the tile to the **device**, which ADR 0029
  measured at two to three times the scratch path for a single use. A one-off cost becomes a
  **smaller** one-off cost, and the answer pays whether or not the tile recurs.
- Under **`Coverage::Cpu`** there is no device lane; the caller asked for the CPU rasteriser.
  `false` routes the tile to the **sheet** — the same rasteriser doing the same work into a
  different texture, no faster now, and with no entry next frame. A one-off cost becomes a
  **per-frame** cost, and the only thing bought is atlas room.

So `worth_caching`'s answer is actionable exactly where a faster single-use lane exists.
ADR 0029 tied the census to `Coverage::Gpu` and justified it on the walk's 25 µs; that
coupling turns out to be **load-bearing for the trade itself**, and the cost argument was the
lesser half of a reason it stated only in part. `notes-census.md` §3's 11.60 % against
33.15 % remains true and remains the best evidence available — but it is evidence about the
lane with an alternative, and it does not transfer to the one without.

### 4. Two comments are corrected, because they are false on the default lane

`CacheProspect::TooLarge` justifies declining a room test with "the census keeps single-use
tiles out of the atlas, so a full atlas is one holding tiles that are being reused". Stated
unconditionally, that is **measurably false under `Coverage::Cpu`**, which is what the caller
draws every page with: a full atlas there is one holding earlier pages' tiles, 98.6 % of
whose keys were placed exactly once. The *conclusion* survives — a room test still buys
nothing — but for a different reason on each lane, and HANDOVER's rule is that a claim which
decays should not go on being quoted forward. `worth_caching`'s own doc gains the asymmetry
in §3, which is the thing a reader needs before proposing this change a third time.

No behaviour changes, so no gate moves; `cargo test --workspace` is 564 passing, 2 ignored
unit tests and 1 ignored doctest, clippy, rustdoc and `fmt` clean, and
`examples/retained.rs --check` reports `[E]` with 0 repacks.

## What this does not do

- **It does not price the census walk on the default lane.** It does not have to: the change
  is refused on the count in §3, and a duration is not something this machine can measure
  (ADR 0052's seam). ADR 0029's 25 µs on a 5 933-command page stands unrevisited.
- **It does not touch the quantum**, though it measured something about it that belongs to
  whoever takes that decision next: with the quantum **on** at 4×, keeping 31 % of tiles out
  of the atlas moved **48 pages from *differ* to *agree*** (725/223 → 773/175), because a
  tile outside the atlas is drawn at its exact phase rather than a quantised one. That is a
  fidelity price of the sub-pixel quantum, observed from an unusual direction, and it is
  obtainable without touching admission by turning the quantum off.
- **It does not claim the accumulation is harmless.** It claims the cure measured here costs
  more than the disease. ADR 0063's bound — one frame per exhaustion, self-correcting — is
  what makes that true, and if that bound ever fails the arithmetic changes with it.

## Revisit when

- **A way exists to change lanes between frames without changing pixels.** That is ADR 0029
  §3's rejected frame memory, and it is the only shape in which the atlas could hold what is
  reused across frames while refusing what is genuinely used once. The criterion available
  today is within-frame, and §2 is why that one is refused.
- **A caller reports atlas pressure with its resource identifiers held still**, on
  `Coverage::Cpu`, where the repack does not reclaim it within one frame. The quantum-on
  column (64 998 → 7 571) says this filter's effect survives the configuration the viewer
  ships, so this decision rests on the cost side rather than on the effect being small — and
  a workload that changes the cost side changes the answer.
- **A page reports `atlas_working_set_bytes` above `Limits::atlas_bytes`** — ADR 0063's own
  trigger, unchanged, and still met by no page at 4×.
