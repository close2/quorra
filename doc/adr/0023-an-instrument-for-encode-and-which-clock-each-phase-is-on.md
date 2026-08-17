# ADR 0023 — An instrument for `encode`, and which clock each phase is on

Status: **accepted, with one amendment** — 2026-08-11, amended 2026-08-17. Answers the
caller's feedback §13, which is explicitly a request for an instrument before a request
for speed. Also records why quorra does not spawn threads, which is the question that led
here. The amendment is at the end: one seam was in the wrong place, and every `recording`
share this project published for a **clipped** page was wrong because of it.

## Context

Their trace put `encode` at **45% of a page turn** — 481 ms of a 963 ms `device` total
over 38 page turns — and it is the only phase that tracks the scene's size, at
3.86 µs a command by least squares. Then the sentence that decides what this ADR is:

> Whether those 3.86 µs a command are path flattening, bind-group churn, buffer writes,
> sorting, or `wgpu`'s own command recording is invisible from here.

They had also ruled out their own end of it, and asked separately about a second thing:
their summary prints an `elsewhere` row — their wall clock around `Device::render` minus
our three phases — and they no longer believe it is a duration of anything, because
`execute` is the *adapter's* clock and subtracting it from a host measurement leaves
whatever the two disagree by mixed into the remainder.

## Decision

### 1. `encode` subdivides into geometry, staging and recording

Reported through `Timings::phases`, which already carries per-pass durations:

- **geometry** — flattening outlines, expanding strokes, running the scanline
  rasteriser: making coverage out of shapes.
- **staging** — packing that coverage into the scratch sheet and the glyph atlas: the
  memory traffic that carries it, as distinct from the arithmetic that made it.
- **recording** — the remainder, computed rather than measured: clip resolution,
  culling, atlas lookups, instance building, plan assembly.

### 2. It is a switch, `Options::instrument_encode`, off by default

Encode's parts interleave per command, so subdividing means a clock read at each seam
rather than three for the frame. `Instant::now()` is ~20 ns here; a page of 5 933
commands crosses enough seams to cost ~0.2 ms, which is **three times the whole encode
of a page of rectangles** (0.089 ms) and about 1% of a page of paths. A measurement that
changes what it measures by 300% is not an instrument. A host that traces frames turns it
on for the trace.

### 3. The acquire and the present are named, always

Two clock reads a frame, so they need no switch: `"target acquire"` and `"present"` are
phases now, which is their first suggested way out of the unnamed remainder.

### 4. `Timings::host_total()`, and the clocks written down

`encode + upload + readback` — the three spans on the *caller's* clock. `execute` is
excluded on purpose and the rustdoc says why: subtract `host_total`, not the sum of all
four, and read what is left against the two named phases. That is their second suggested
way out, and both are cheap enough to take together.

## What the instrument says, immediately

1191×1684 on RADV, release, 3 675 curved fills at reading size — the shape of a page of
text, with the atlas in front of it:

| | encode | geometry | staging | recording |
|---|---|---|---|---|
| 107 distinct outlines, **cold** atlas | 2.549 ms | **1.533** | 0.155 | 0.861 |
| 107 distinct outlines, warm atlas | 0.995 ms | 0.000 | 0.000 | 0.995 |
| 3 675 distinct outlines, **cold** atlas | 8.919 ms | **6.229** | 0.795 | 1.895 |
| 3 675 distinct outlines, warm atlas | 1.758 ms | 0.000 | 0.000 | 1.758 |

**It is geometry, whenever the atlas is cold** — 60% and 70% of encode in the two cold
rows — and it is *recording* when the atlas is warm, where what remains is hash lookups
and instance writes. The all-distinct cold row is 2.4 µs a command, the same order as the
3.86 µs a command their trace fitted, which suggests their page turns are largely cold.

That is a reading of *our* fixtures, not of their pages. The instrument is theirs to
point at their own corpus, which is the whole reason it exists.

## Why quorra does not spawn threads

Asked whether we should build a thread pool for this, their answer was **no — take one
rather than make one**, for reasons we cannot see from here and now record:

- Their tree already runs `rayon`, sized to the machine; a second pool sized the same way
  oversubscribes a page turn, and neither pool knows the other exists. They have been
  bitten by nested pools confounding a measurement before.
- Their confined worker runs single-threaded **not by choice**: `glibc`'s allocator sizes
  its arena count from a `/sys` read that their seccomp filter kills, so a many-threaded
  confined process dies on the 24th worker's first allocation. Threads spawned lazily at
  first use would hit exactly that, and it would look like a mysterious kill.
- Their `viewer-core` rule 4 is "no threads the core was not handed", because every
  toolkit forbids touching the interface off the main thread.
- Pool construction on the launch path is precisely what their principle 2 forbids: page
  one goes to the graphics device, so our bring-up is already on their time-to-first-page.

So: **if parallelism is ever wanted here, quorra takes a pool rather than making one** —
the shape `create_instance` and `create_instance_with` already established, where the host
supplies what it already has. Nothing is added for it today, because the measurement above
says the first move is not threads: it is the rasteriser, or the atlas being cold, and
neither is answered by running the same work on more cores.

## Revisit when

The subdivision stops being able to attribute — the candidate is `recording`, which is a
remainder and would need splitting again if it grew. That is the same question one level
down, and the same answer: measure before optimising.

## Amendment, 2026-08-17 — the residue multiply is geometry, and the line is written down

The decision stands and the three phases are unchanged. **One seam was in the wrong
place**, and the consequence is that every `recording` number this project has published
for a page with a **curve clip** on it was too large by the same amount that `geometry`
was too small.

### What was outside the span

`encode/coverage.rs`'s `coverage_tile` opened a geometry span around `raster::fill_mask`
and closed it there. The next thing it did was multiply the chain's residue into the tile
it had just rasterised — one multiply, one add and one divide per pixel — with no span
open. Almost everything inside `residue_intersection` was already spanned — the flatten,
the links' `min`, and the crop on the path a second mark under the same chain takes — so
the product was the whole of the unattributed per-pixel work, and it was reported as
`recording`, the remainder.

The fix is one seam: `residue_product` is its own function and its own span. A second,
much smaller one goes with it: a region's crop was spanned at one of its two call sites
and not at the other (the first mark under an admitted chain), which is the same defect at
a hundredth of the size — once per chain rather than once per clipped mark.

### The line between geometry and recording, now stated

The definitions in §1 above did not decide this case, so the amendment states the rule
they imply:

> **Geometry is the work that *makes* coverage. Recording is the work that decides what to
> do with it.**

- Flattening, stroke expansion, the scanline pass, the links' intersection, a region's
  crop, **and the residue product** are the mark's coverage being computed. Geometry.
- Bounding a shape, choosing a lane, charging a budget, keying the atlas, writing an
  instance are decisions taken *about* coverage. Recording.

The second half is not a technicality: the largest single row of a path-heavy page's
`recording` is `HullMemo::bounds` at 56 % (`doc/notes-recording-shares.md` §2.2), which is
per-point float arithmetic over 9.1 million control points — so "it is per-pixel, therefore
it is geometry" would move that row too, and it should not move. `raster::polyline_bounds`
and the flattened triangle count in `push_coverage_styled` stay in `recording` for the
same reason.

### What it costs

Two `Instant::now()` reads — about 20 ns each here — **per clipped command that
rasterises a tile under a residue chain**, and only when `Options::instrument_encode` is
on. Nothing for a page with no curve clip, and nothing at all when the switch is off,
where `EncodeClock::start` is a branch on a `bool` that returns `None`. On the artwork
archetype, whose 600 clipped commands are the most residue-heavy page in the tree, that is
1 200 clock reads, ~24 µs on an encode of ~53 ms: **0.05 %**, against a seam that was
mislabelling 13.7 % of that page's `recording`.

### What moved, measured

Instructions, not milliseconds (`doc/HANDOVER.md`'s "An encode, exactly"), on the re-cut
artwork archetype at 1191 × 1684, one cold-atlas encode, counters checked against
`tests/archetypes.rs` before anything was read:

| | Ir | of encode |
|---|---:|---:|
| the whole encode | 757 574 442 | 100 % |
| `residue_product` — what this amendment moves | **4 683 942** | **0.62 %** |
| `recording` after the fix | ~29.4 M | ~3.9 % |
| `recording` before it | ~34.1 M | ~4.5 % |

So the product is **13.7 % of what `recording` was** on the most clipped page this tree
has, and 600 calls over 3 542 360 tile pixels put it at 7 806 instructions a tile and
about 1.3 a pixel. The wall-clock reading of the same change is in
`doc/notes-clipped-instrument.md` §2, and it is the weaker instrument on purpose.

### The published number this corrects

`doc/notes-recording-shares.md` §2.2 and `doc/answer-nonblocking-render.md` both say
**56 % of artwork's `recording` is the residue multiply**. That number is wrong twice over
and both halves are corrected in place, dated, in those files:

1. It was measured on the artwork archetype *before* its clips were cut around its marks,
   where 592 of 600 clipped commands rasterised a tile and multiplied it by a residue of
   **zero** (`doc/notes-tiling-bound.md` §3).
2. The 56 % row was `coverage_tile`'s **and** `push_coverage_styled`'s own bodies together,
   and by the line stated above only the product part of that is geometry. Measured
   directly, the product is a third of the smaller of the two.

The finding the number was carrying — *the geometry clock mislabels the residue multiply
on a clipped page* — was right, and is what this amendment fixes.

### Does this trip "revisit when"?

**No.** That clause is about `recording` growing until a remainder can no longer attribute,
and this was a span in the wrong place rather than a phase too coarse: the subdivision
attributed exactly as well before and after, it just attributed one loop to the wrong
phase. What *does* trip it is still open and is not this round's:
`doc/notes-recording-shares.md` §5 recommends an optional detail level
(`encode: bounds`, `encode: atlas`, `encode: instances`, `encode: commit`) with
`encode: recording` kept as the remainder, and the caller asked for it in as many words.
