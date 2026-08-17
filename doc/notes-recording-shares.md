# What `encode: recording` is made of, whether any of it divides, and where the floor is

Round notes for §9 of the caller's `pdf-viewer/doc/QUORRA_NONBLOCKING_RENDER.md`, written
2026-08-16 against this worktree. Nothing here changed a line of the library: this round
ships a measurement and a draft answer. No instrumentation was left in the tree.

> **Correction, 2026-08-17.** Every number below that was taken on the **artwork**
> archetype is a number about a page whose clips clipped almost nothing: 592 of its 600
> clipped commands rasterised a coverage tile and multiplied it by a residue of **zero**
> (`doc/notes-tiling-bound.md` §3, found by ADR 0057 the day after this file was written).
> The artwork column of §2.1 and §2.2 is therefore not a measurement of the residue lane,
> and §2.2's headline — *"on artwork: 56 % of recording is per-pixel work the clock calls
> recording"* — is wrong in both of its parts. It is corrected in place below, under its
> own dated note, and the fix it argued for is built: ADR 0023's amendment of 2026-08-17.
> **The two pages this file is actually about — `drawing` at 58 009 marks and dense text —
> state no residue clip and no clipped tile, so every number about them stands unchanged,
> including §3's divisibility answer and §4's floor.**

**The short version.** On the caller's own page shape — 58 009 marks, no clip anywhere —
`recording` is **not** dominated by any of the four things their §9 quotes back at us, and
not by the fifth our own prose adds. It is dominated by a sixth we never named: **computing
each mark's device bounding box before anything else can be decided about it**, which is
**56.0 % of recording's instructions** and **40–43 % of its wall clock**. Their four —
clip resolution, culling, instance building, plan assembly — are 0.30 %, 0.60 %, 4.17 % and
0.55 % of recording between them. That answer does not generalise: on dense text the same
work is 11.6 % and on artwork 2.0 %, and the note says why each time.

The divisibility answer is **partly yes, and the divisible part is the majority of it** —
but not through the seam ADR 0054 built. §3 says which state forces the rest to stay in
order, in the form the caller asked to be able to quote.

The floor is the number that matters most and it is against us as much as their ADR 0368's
was against them: **with `recording` at zero their frame is still 139.5 ms, and with the
whole of `encode` at zero it is still 107.0 ms — 12.8 refreshes at 120 Hz.**

---

## 1. The instrument, and what says it measured their page

**Instructions, not milliseconds.** `doc/HANDOVER.md`'s "An encode, exactly": a `#[cfg(test)]`
module inside `quorra-gpu` that builds a `ResourceStore` and an `AtlasStore` directly —
`encode` needs no adapter — encodes once outside the collected region to fault in the
allocator's arenas, then once inside an `#[inline(never)] steady_run`, with a **fresh
`AtlasStore`** so the collected encode is the cold-atlas frame every zoom step is.
Built with `CARGO_PROFILE_RELEASE_DEBUG=1` into its own `CARGO_TARGET_DIR`, run under
`valgrind --tool=callgrind --collect-atstart=no --toggle-collect='*steady_run*'
--read-inline-info=yes --cache-sim=no`. The harness and its example were **deleted with the
round**, which is `Cargo.toml`'s standing decision about benchmark harnesses.

**What says the harness encoded the right page**, before any share below is believed —
the counter rows, against `tests/archetypes.rs` and `doc/notes-encode-threads.md` §3:

| page | counters read | matches |
|---|---|---|
| dense text | `[4320, 0, 818, 2164, 1, 40, –, 2, 0]` | `tests/archetypes.rs` `BASELINE`, exactly |
| artwork | `[684, 0, 300, 300, 1, 600, –, 185, 0]` | the same row, exactly |
| **drawing at 58 009** | 58 009 commands, 58 009 distinct outlines, **58 003 atlas keys**, 0 clip regions, **6 tiles**, **3 132 486 segments** | `notes-encode-threads.md` §3, exactly |

(The seventh field is `layer_textures`, which the device counts and an encode does not.)

**Naming the parts needed real call boundaries, and the distortion that costs is measured
rather than assumed.** A release build inlines most of the walk into a handful of
functions — the outline lookup, the clip resolver, the cull test, `GlyphPlacement::of`,
the batch appender and `Job::glyph` all disappear into their callers — so the subdivision
was taken with a temporary `#[inline(never)]` on the thirty-four functions that name a
seam. The control is the same page with and without them:

| page | without the seams | with them | distortion |
|---|---:|---:|---:|
| dense text | 114 924 167 Ir | 115 153 947 Ir | **+0.20 %** |
| drawing | 5 717 707 110 Ir | 5 721 942 468 Ir | **+0.074 %** |

Everything below is from the seamed build. The partition is exact by construction: the
total is the sum of every function's *self* cost, each function belongs to exactly one part,
and a shared helper (`floorf`, `malloc`, a `hashbrown` probe) has its cost pushed up to
whoever called it in proportion to the call edges.

**Where the seams are is the clock's own definition**, so these shares are shares of what
`Options::instrument_encode` reports: geometry is what a `clock.geometry` span wraps,
staging what a `clock.staging` span wraps, recording the remainder. One known departure:
`raster::polyline_bounds` sits inside a geometry span at some of its call sites and outside
one at the others, and is counted as geometry at all of them. It is bounded above by the
row it shares with `rasterise`'s own body — 2.21 % of `drawing`'s encode, 0.07 % of
artwork's — so it cannot move a share in §2.2 by more than that.

**And a wall clock too, labelled as the weaker instrument.** `encode` is a duration and the
floor is a duration, so §4 needs one. Same rules as `examples/encode_threads.rs`: llvmpipe,
headless into a texture, a **cold device per sample**, round-robin over the thread counts,
minima reported, load average printed either side.

---

## 2. The subdivision

### 2.1 The three phases first, so the shares below have a denominator

By **instruction count**, one thread, cold atlas:

| page | encode, Ir | geometry | staging | **recording** |
|---|---:|---:|---:|---:|
| **drawing** (58 009 marks, no clip, 900 × 1100) | 5 710 296 213 | 91.09 % | 1.68 % | **7.23 %** |
| dense text (4 320 marks, 1191 × 1684) | 114 758 673 | 86.33 % | 2.96 % | **10.71 %** |
| artwork (684 marks, 600 curve-clipped) | 602 344 038 | 93.71 % | 0.65 % | **5.65 %** |

By **wall clock**, `drawing` only, five round-robin rounds, minima, load average 7.28 before
and 5.99 after — milliseconds:

| threads | encode | geometry | staging | recording | recording's share |
|---:|---:|---:|---:|---:|---:|
| 1 | 400.7 | 305.9 | 15.5 | 79.3 | 19.8 % |
| 2 | 242.2 | 154.9 | 7.5 | 78.7 | 32.5 % |
| 4 | 201.6 | 112.9 | 9.5 | 79.2 | 39.3 % |
| 8 | 154.1 | 68.9 | 10.3 | 74.4 | 48.3 % |
| **24** | **127.8** | **42.9** | **9.3** | **74.9** | **58.6 %** |

**Read the two tables together and the first finding is in the gap between them.** Recording
is 7.23 % of the encode's *instructions* and 19.8 % of its *time* at one thread. Geometry
retires 5.20 G instructions in 306 ms — 17.0 G/s — and recording retires 0.41 G in 79 ms —
5.2 G/s, **three times slower per instruction**. That is the shape of the two: geometry is a
tight float loop over sequential memory, recording is 58 009 hash probes, 58 009 map inserts
and 58 009 allocator round-trips, and it is memory-bound. So an instruction share understates
recording's cost in time by about three, and every share in §2.2 should be read with that in
mind. It is also why recording is **flat across thread counts** — 79.3, 78.7, 79.2, 74.4,
74.9 ms — which is what "serial by design" looks like in a measurement.

### 2.2 Inside `recording`, and the names it actually falls on

Shares of recording, by instruction count, one thread. **▸ marks a name our own prose gives
it**: `instrument.rs` says *"clip resolution, culling, instance building, plan assembly"* —
the four their §9 quotes back — and `notes-encode-threads.md` §5 adds atlas lookups.

| part | drawing | dense text | artwork |
|---|---:|---:|---:|
| **bounding each mark** (`hull::HullMemo::bounds`) | **55.98 %** | 10.12 % | 1.92 % |
| ▸ culling (`device_space::culled`) | 0.60 % | 1.51 % | 0.11 % |
| the walk itself — dispatch, lane arithmetic, `use_mask`, `finish` | 16.80 % | 29.91 % | 34.94 % |
| the two counter sets (`atlas_keys`, `distinct_outlines`) | 6.50 % | 9.14 % | 0.45 % |
| ▸ atlas lookups (`prospect_for`, `GlyphPlacement::of`, `prospect`) | 4.89 % | 12.55 % | 0.46 % |
| outline lookups (`ResourceStore::outline`, **twice a fill**) | 4.61 % | 11.58 % | 0.85 % |
| ▸ instance building (`push_quad_instance`, `push_rect_instance`) | 4.17 % | 10.43 % | 0.92 % |
| commit and budget (`commit_glyph`, `charge`) | 3.36 % | 6.47 % | 0.35 % |
| queue and jobs (`enqueue`, `Job::glyph`, `drain_queue`) | 1.93 % | 4.51 % | 0.23 % |
| ▸ plan assembly (`note_batch`, `push_op`, `append_op`) | 0.55 % | 1.37 % | 0.12 % |
| ▸ clip resolution (`resolve_clip`, `ClipResolver::resolve`, residue) | 0.30 % | 0.83 % | 1.76 % |
| lane choice (`take_gpu_lane`, `placed_once`, `visible_tile`) | 0.15 % | 0.39 % | 0.05 % |
| the serial coverage path (`coverage_tile`, `push_coverage_styled`) | 0.00 % | 0.72 % | **56.13 %** |
| recording, absolute | 412 672 286 Ir | 12 295 501 Ir | 34 047 918 Ir |

**A lane tuned on one of these pages is tuned on the wrong page**, which is
`notes-encode-threads.md`'s finding arriving a second time. Three different rows are the
largest on three page shapes.

#### On their page: bounding, and why the memo cannot help it

`HullMemo::bounds` is **231 022 646 Ir on 58 003 fills — 3 983 a fill.** That page carries 52
cubics a mark, so a mark has 157 control points, and the box is four multiplies, two
additions and four min/max per point: ~9.1 M control points, ~23 Ir each, so **~210 M of
the 231 M is the arithmetic itself.**

The other ~20 M is ADR 0045's memo, and on this page it is **pure cost**. That memo exists
because a dense page's 4 320 placements collapse to 818 distinct `(outline, linear part)`
boxes, and it is worth 21 % of that page's encode. **This page has 58 009 outlines and
places each of them once**, so the memo hashes a 20-byte key, probes, misses, computes, and
inserts, 58 003 times, and never answers a question twice. Two independent readings agree on
what that costs: ADR 0045 measured the probe at 292 Ir a placement (17 M here), and the
residual after the arithmetic above is ~20 M, of which 7.2 M is the rehashing the map does
on its way to 58 009 entries.

**Culling, by contrast, is 0.60 % and returns nothing on this page**: `commands_culled` is 0,
so 2.5 M instructions establish 58 009 times that a mark is on the target. That is the
correct trade — ADR 0015 measured the same test saving 9.35 ms of a 14.4 ms frame at
magnification — but on a fit view it is a cost with no matching benefit, and it is worth
knowing that the two are separable: what is expensive is *bounding*, and culling is what the
bound is then used for.

#### On dense text: nothing dominates, which is its own answer

The largest row is 29.91 % and it is the walk itself. The next four — atlas lookups 12.55,
bounding 10.12 (the memo working: 4 320 placements, 818 hulls), outline lookups 11.58,
instance building 10.43 — are within a factor of 1.3 of each other. **A page of repeated
glyphs has no hot spot in recording**; it has twelve warm ones. That page's recording is
12.3 M instructions against `drawing`'s 413 M, so this is a statement about shape rather
than about a cost worth attacking.

#### On artwork: 56 % of recording is per-pixel work the clock calls recording

`coverage_tile`'s own body is 11.79 M Ir over 600 tiles — **19 657 a tile** — and that is
the loop that multiplies the residue mask into the coverage tile, one `u16` multiply and
divide per pixel over a 60-pixel mark. `push_coverage_styled`'s own body is another 7.25 M,
which is `polyline_bounds` and the triangle-count sum over the flattened geometry.

Both are per-pixel or per-point arithmetic on geometry — they are geometry by any reading of
ADR 0023's own definition — and they are outside every `clock.geometry` span, so the
instrument reports them as recording. **That is a defect in the instrument, not in the
code**, and it is the strongest argument in this note for §5's recommendation: a caller
acting on "artwork's recording is 56 % one thing" would be acting on a mislabelled span.

> **Correction, 2026-08-17 — the finding was right and the number was wrong twice.**
>
> 1. **The page was not what this paragraph thought it was.** Those 600 tiles were
>    rasterised over marks their clips did not reach: 592 of them were multiplied by a
>    residue of zero, and the 19 657 instructions a tile were being spent to produce
>    nothing. ADR 0057 removed them the next day and the row went to **8 tiles**; on the
>    tree as it stood on 2026-08-17 the residue product on this page is **16 490 Ir,
>    0.06 % of its `recording`** — so the 56 % is not merely stale, it is not
>    reproducible.
> 2. **The row was two things and only one of them is geometry.** ADR 0023's amendment
>    draws the line at *making* coverage rather than at *per-pixel*: the residue product is
>    geometry, and `polyline_bounds` and the triangle-count sum are recording — for the
>    same reason `HullMemo::bounds` is recording in §2.2's own table, where it is 56 % of
>    `drawing`'s recording and nobody proposed to move it.
>
> Re-measured on the **re-cut** artwork page (`doc/notes-clipped-instrument.md`), where all
> 600 clipped commands do meet their clips: the product alone is **4 683 942 Ir, 0.62 % of
> the encode and 13.7 % of what `recording` was** before the seam moved. That is the honest
> replacement for "56 % of recording", and it is a different page from this table's, so it
> does not belong in the table.

---

## 3. Is any of it divisible?

**Partly, and the divisible part is the majority of it on their page. The rest is not, and
the order genuinely is the product.**

### 3.1 What is not divisible, and the state that forces it

Five ordered structures, and every one of them is read or written by a part of recording:

- **the frame budget's running total** (`Encoder::spent`). A refusal has to name the same
  two numbers a one-threaded frame names, so charging out of encounter order changes the
  message of a `FrameBudgetExceeded` even when it does not change which frames are refused.
  `tests/encode_threads.rs` holds that to equality, and the caller's `REFUSED_AT_FOUR` is the
  same requirement from their side. — *commit and budget, 3.36 %*
- **the scratch sheet's shelf cursors.** ADR 0034 made encounter order load-bearing and
  **declined to sort**, because assigning positions after the walk is a two-pass encode. A
  tile packed out of order is a different sheet, which is different texel origins, which is
  a different retained encode (ADR 0048). — *the coverage path*
- **the atlas allocator.** `AtlasStore::prospect` must be asked of an atlas every earlier
  mark has already reached, or a repeated key reads `entry: None` and is *built* twice. That
  is not a hypothesis: it is how ADR 0054's determinism gate found `bytes_uploaded` off by
  exactly 64 bytes — one 8 × 8 tile — and why the drain moved to `Encoder::prospect_for`,
  *before* the answer is given rather than before the job is queued. — *atlas lookups,
  4.89 %*
- **the instance stream and the batches over it.** A `Batch` is a `first`/`count` **range**
  of consecutive instances in one lane under one style and one mask; the painter's algorithm
  survives as the order the bytes were written in, and a run is extended or broken by what
  came immediately before. Reordering instances is repainting the page. — *instance
  building 4.17 % and plan assembly 0.55 %*
- **the layer plans' bounds** (ADR 0036), which grow by the rectangle each op will mark, so
  that a plan's texture fits what it draws. A plan that received its ops out of order still
  gets the right union — but `push_child` decides whether a child is appended at all, and
  that decision reads the plan as it stands. — *plan assembly*

Together these are ~15 % of recording on their page, and **each of them is order-dependent in
the strong sense: not "hard to make thread-safe", but "the sequence is the answer".** ADR 0054
put the whole of them in the third phase for that reason and the design has not changed.

### 3.2 What *is* divisible, and what it would cost

**Bounding is 56 % of recording on their page and it depends on nothing the frame's order
decides.** `HullMemo::bounds` is a pure function of `(outline, linear part of the device
transform)` — ADR 0045's own module comment proves it returns the direct box **bit for
bit** — plus a memo that changes no answer. `Encoder::culled` reads the resolved clip, the
target rectangle and nothing else, and writes one saturating counter.

So the shape exists, and it is **not** the fan-out ADR 0054 built. It is a **pre-pass**:
bound every command's control hull in parallel into a vector indexed by command, then let
the walk read the vector instead of computing. It touches no budget, no shelf, no atlas
and no instance stream, which is exactly why it would be safe — and it is the same shape
`Census::of` and `ResidueRegions::of` already have, two passes over the commands taken before
the walk. Four costs, and they are why this note does not build it:

1. **It is a second pass over the commands**, which is the shape ADR 0034 declined for the
   tiling and ADR 0049 declined for the residue union. Both declined it for a *specific*
   reason — a value that must be known before the first ask — and neither applies here,
   because a bound is knowable from the command alone. So the objection is cost, not
   principle; but the precedent should be re-taken deliberately, in an ADR, and not inherited.
2. **It would undo ADR 0045 on the page ADR 0045 exists for.** A shared memo across threads
   needs a lock and reintroduces an order; a per-worker memo computes a dense page's 4 320
   hulls where 818 sufficed, which is the 21 % of a dense-text encode that ADR 0045 bought.
   So the pre-pass needs a floor of its own, and the floor is not size but **distinctness** —
   it wins exactly where `commands ≈ distinct (outline, linear) pairs`, which is `drawing`
   and `giant` and not `dense text`. `Census` already counts that number and is only taken
   under `Coverage::Gpu` (ADR 0029), so taking it always is part of the price.
3. **It allocates from a scene-derived number**: one `Option<[f32; 4]>` per command, 1.4 MB
   on their page, which principle 3 says must be charged against the frame budget before it
   exists. Cheap and not free.
4. **The walk recurses into groups**, so a pre-pass over the top level is not a pre-pass over
   the commands, and the vector's index is a tree position rather than a `usize`.

**And the honest ceiling on what it would buy.** Bounding is 40–43 % of recording's *wall
clock*, not 56 % — measured with a direct span on `hulls.bounds`, twelve round-robin rounds,
load average 1.50 before and 17.18 after, and stable across thread counts:

| threads | 1 | 2 | 4 | 8 | 24 |
|---|---:|---:|---:|---:|---:|
| bounding, ms | 33.7 | 29.1 | 30.0 | 30.0 | 33.9 |
| the rest of recording, ms | 44.5 | 41.8 | 44.2 | 43.1 | 46.8 |
| bounding's share of recording | 43.1 % | 41.1 % | 40.4 % | 41.1 % | 42.0 % |

(The span costs two clock reads a command and inflates the encode it measures by about 5 ms
on this page, which is why these rows are not the §2.1 table's.)

So dividing it perfectly across twenty-four threads takes recording on their page from about
75 ms to about 44 ms, and their encode from 127.8 to about 97 — **a 1.31× encode, not a
6.6× one.** ADR 0054's geometry win is not repeatable here, and saying so is the point of
measuring it.

### 3.3 What the existing seam already costs recording

The queue is not free, and the caller should know what they are paying for the geometry win.
Same page, same collection, one thread against two — and the comparison is valid for
recording alone because callgrind's collection state is per-thread, so a worker's
instructions are not counted while the walk's are:

| | 1 thread | 2 threads | delta |
|---|---:|---:|---:|
| recording, Ir | 412 672 286 | 435 238 679 | **+22 566 393 (+5.5 %)** |

Where it goes: the `queued_keys` set that closes the atlas guard (+9.70 M), `Job` construction
and the queue itself (+7.58 M), the `queued_keys.contains` check in front of every
`prospect` (+2.86 M), and the drain checks in `charge` and the instance appenders (+2.44 M).
**389 instructions a mark, and it buys geometry 6.6×.**

---

## 4. The floor

Their ADR 0368 asked this of geometry and answered it against themselves. Here is ours,
computed the same way, against their own trace (`QUORRA_NONBLOCKING_RENDER.md` §2: 24 frames,
4454.9 ms, `encode` 1887.2 of it).

**The share to apply.** On our reproduction of their page shape at twenty-four threads,
encode is geometry 33.6 %, staging 7.3 %, recording 58.6 % — and of that recording, bounding
is ~42 %. Their window is 2048 × 2560 device pixels against our 900 × 1100, and a larger
target means more coverage pixels a mark, so **geometry's share on their machine is at least
ours and recording's is at most ours**. Every number below is therefore the optimistic end.

| what is set to zero | their 24 frames | a frame | refreshes at 120 Hz |
|---|---:|---:|---:|
| nothing (their measurement) | 4454.9 ms | 185.6 ms | 22.3 |
| bounding — §3.2's divisible part | 3990.3 ms | 166.3 ms | 20.0 |
| **the whole of `recording`** | **3348.8 ms** | **139.5 ms** | **16.7** |
| recording *and* geometry | 2715.3 ms | 113.1 ms | 13.6 |
| **the whole of `encode`** | **2567.7 ms** | **107.0 ms** | **12.8** |
| everything quorra does — encode, transfer, elsewhere, execute | 653.7 ms | 27.2 ms | 3.3 |

**What is left, and in what proportion.** With `encode` at zero, 2567.7 ms remain, and they
are: `transfer` 955.6 (37.2 % of what is left), `elsewhere` 951.7 (37.1 %), their own `scene`
walk 586.1 (22.8 %), `settle` 22.7 (0.9 %), `execute` **6.7 (0.3 %)**.

**What no work on our side can remove.** Strictly, one row: their display-list-to-scene walk,
586.1 ms, 24.4 ms a frame, **2.9 refreshes** — so even a library that returned instantly
leaves their page able to carry a rendering on at best one refresh in three. The two larger
rows are ours and are untouched by this round: `transfer` is 58 003 coverage tiles crossing
to the device, and `elsewhere` is the pass encoding, texture pricing and submission around
them.

**And the ratio their §9 opens with.** During the 4.366 s their view was moving, 15 of 524
refreshes carried a rendering — 2.9 %. With recording at zero that becomes 19.9 of 524,
**3.8 %**. With the whole of encode at zero, 26.0 of 524, **5.0 %**. To carry a rendering on
every refresh a frame must cost 8.333 ms; theirs costs 185.6, and the floor with our phase
one deleted outright is 107.0. **`recording` is not what stands between them and 120 Hz, and
neither is anything else in `encode`.**

---

## 5. What was found and not done

- **`fill_solid` repeats `encode_fill`'s `HashMap` lookup of the outline** — the debt
  `doc/notes-encode-split.md` §5 recorded, now priced. It is exactly half of the
  outline-lookup row: **9 514 587 Ir on their page (2.31 % of recording, ~1.7 ms of a 128 ms
  encode)**, 711 286 on dense text (**5.79 % of recording** — the larger relative share,
  because that page's recording is smaller in every other row too), 81 202 on artwork
  (0.24 %). Confirmed by the call counts: `ResourceStore::outline` is entered exactly as
  often from `fill_solid` as from `encode_fill` on every page. Still needs the lifetime that
  fights `&mut self`, and now the number that says what winning it is worth.
- **`flatten_chain` re-flattening a declined chain's links per clipped command: confirmed
  bounded, and it cannot fire on their page.** `notes-encode-threads.md` §5 recorded it as
  unmeasured. Measured now: on `drawing` it is entered **zero times** — the page states no
  clip, so `residue_intersection` returns at its first line — and on artwork it is entered
  **185 times for 185 chains and 600 clipped commands**, which is ADR 0049's admission rule
  holding it to once a chain rather than once a mark. Dense text: twice, for two chains. The
  §5 entry stands as written and is now evidence rather than a reading.
- **`ResidueRegions::of` walks every command on a page with no clips.** 812 147 Ir of their
  page's recording (0.20 %) spent by two passes over 58 009 commands filling a
  **zero-length** `uses` vector, because `count` is called before anything asks whether the
  scene declares a clip at all. A one-line guard. Left because it is 0.2 % and because a
  guard on `scene.clips().is_empty()` wants its own unit test next to the two passes it
  skips, not a drive-by.
- **The two counter sets are 6.5 % of recording on their page** — `atlas_keys` 4.14 % and
  `distinct_outlines` 2.36 %, 58 009 inserts each. `atlas_keys` is load-bearing (ADR 0050
  reads `first_use` for `atlas_requested_bytes` and `atlas_entries_used`), but
  `distinct_outlines` exists **only** to publish `Counters::distinct_outlines`: 9 743 210
  instructions, ~2.4 % of recording, for one reported integer. That is the cost of CLAUDE.md's
  "instrument the count of distinct keys, not the hit rate" on a page where every key is
  distinct, and it is worth writing down rather than removing — but it should be written down.
- **The clock's `geometry` span misses per-pixel work on a clipped page.** §2.2's artwork
  paragraph: the residue multiply in `coverage_tile` and the polyline scan in
  `push_coverage_styled` are 56 % of that page's recording and are geometry by ADR 0023's own
  definition. Not fixed here because moving a span changes what every recorded number in
  `PLAN.md` means, and that is a decision with a corpus-free but documentation-heavy blast
  radius.

  **Taken, 2026-08-17 (ADR 0023's amendment).** The residue multiply is inside a geometry
  span; the polyline scan is not, and the amendment says why — the line is *making*
  coverage, not *per-pixel*. The blast radius was as predicted: no pixel moves, no corpus
  run is owed, and the documentation half is `doc/notes-clipped-instrument.md`.
- **A recommendation, for the round that takes it: `Options::instrument_encode` should be
  able to subdivide further, and the caller asked for it in as many words** — *"we cannot
  subdivide it from outside"*. The shape is additive and free when off: `EncodeClock` grows
  one `Cell<Duration>` per named seam behind a second flag (`instrument_encode_detail`, or a
  `Detail` level on the existing one), `phases()` emits the extra rows **and keeps
  `encode: recording` as the remainder**, so a host summing the rows still gets `encode` and
  an existing trace parser sees the three rows it always saw. The rows worth having are the
  ones §2.2 found large on some page: `encode: bounds`, `encode: atlas`, `encode: instances`,
  `encode: commit`. The cost is two clock reads a command per extra seam — measured this
  round at about 5 ms on a 58 009-command page for one seam — which is precisely why it is a
  second flag rather than the default, and the same argument the three phases already won.
- **Not attempted: dividing the bounding.** §3.2 has the shape, the four costs and the
  1.31× ceiling. It wants an ADR, a distinctness floor with its own measurement, and a
  `Census` taken on a lane that does not take one today. It is not this round.

---

## 6. A draft answer to their §9

**For the owner to carry across, in the shape of `doc/feedback-answers-draft.md`. We never
edit their tree.** It answers §9 and nothing else in that document.

---

### §9 — what `recording` is made of, whether it divides, and the floor

**1. What it is made of.** We subdivided it with callgrind rather than with a clock — an
instruction count is exact and this machine's wall clocks are not — on your page shape at
the full 58 009 marks, and on two others so that a lane tuned on one page is not published
as a general finding. **None of the four things you quoted back at us dominates**, and nor
does the fifth our own round notes added to them. On your page, clip resolution is
**0.30 %** of `recording` (you state no clip, and the resolver returns an open rectangle
without touching anything), culling is **0.60 %**, instance building **4.17 %**, plan
assembly **0.55 %**, and atlas lookups — the fifth — **4.89 %**. What dominates is
something none of our prose ever named: **computing each mark's device bounding box,
56.0 % of `recording`'s instructions and 40–43 % of its wall clock.** Your page carries 52
path segments a mark, so a mark is 157 control points, and the box is four multiplies and
four min/max per point — 9.1 million control points a frame, 231 million instructions, 3 983 a mark. The memo we
built to make that cheap (our ADR 0045) keys on `(outline, linear part)` and is worth 21 %
of a *dense text* encode because 4 320 placements there collapse to 818 boxes. **Your page
has 58 009 outlines and places each of them exactly once**, so the memo misses every time
and costs about 20 M of the 231 M for nothing. The rest of `recording` is a long tail with
no second peak: the walk's own dispatch 16.8 %, two counter sets 6.5 %, atlas lookups
4.9 %, the outline store's `HashMap` 4.6 %, the budget and commit 3.4 %, the queue 1.9 %.

**2. Is any of it divisible.** Partly — and unusually, the part that is divisible is the
majority of it, which is why the answer is not the clean "no" we expected to give you. The
bounding depends on nothing the frame's order decides: it is a pure function of the outline
and the linear part of the transform, provably bit-identical however it is computed, and it
writes only a memo that changes no answer. So it could be a **pre-pass** — bound every
command in parallel into a vector, then let the walk read it — which touches no ordered
structure at all. We have not built it, and the reasons are in
`doc/notes-recording-shares.md` §3.2: it is a second pass over the commands (a shape two of
our ADRs have declined before), it undoes ADR 0045 on the page ADR 0045 exists for unless
it carries a *distinctness* floor of its own, it allocates 1.4 MB from a scene-derived
number that principle 3 says must be charged first, and — the number that decides it — **it
would take your encode from 127.8 ms to about 97, a 1.31× improvement and not a 6.6× one.**

**What is not divisible is the remaining ~15 %, and there the order genuinely is the
product**, in your words. Five structures, each of which is a *sequence* rather than a
value: the frame budget's running total, so that a refusal names the same two numbers a
one-threaded frame names; the scratch sheet's shelf cursors, whose encounter order our
ADR 0034 made load-bearing and declined to sort, because assigning positions after the walk
is the two-pass encode; the atlas allocator, which must be asked of an atlas every earlier
mark has already reached — that is not theory, it is how our determinism gate caught
`bytes_uploaded` off by exactly 64 bytes, one 8 × 8 tile rasterised twice; the instance
stream, where a `Batch` is a *range* of consecutive instances and the painter's algorithm
survives as the order the bytes were written in, so reordering instances is repainting the
page; and the layer plans, whose bounds grow by the rectangle each op will mark. Those five
are why the walk and the commit are serial by design and will stay that way.

**3. The floor.** Taking your own trace and zeroing our phases the way your ADR 0368 zeroed
geometry: with **`recording` at zero** your 24 frames go 4454.9 → 3348.8 ms, **139.5 ms a
frame, 16.7 refreshes at 120 Hz**. With **`recording` and `geometry` both at zero**,
113.1 ms a frame. With **the whole of `encode` at zero — phase one deleted outright —
107.0 ms a frame, 12.8 refreshes.** What is left in that last case is `transfer` 37.2 %,
`elsewhere` 37.1 %, your own scene walk 22.8 %, and the graphics device 0.3 %. And if
everything quorra does cost nothing at all, your scene walk alone is 24.4 ms a frame —
**2.9 refreshes long**, so the page still could not be drawn inside one. Applied to your
§9's own ratio: the 15 renderings in 524 refreshes become 19.9 with `recording` at zero (2.9 % → 3.8 %) and 26.0 with `encode` at
zero (5.0 %). **So `recording` is not what stands between this page and 120 Hz, and neither
is anything else in `encode`.** We will keep taking the milliseconds — the bounding pre-pass
is now a costed candidate rather than a guess — but the arithmetic says your §4 is asking
for the right thing: at 107 ms a frame the question is not how to make the frame fit a
refresh, it is how the refresh gets a picture while a frame is still running.

**One thing we owe you off the back of this.** You wrote that you cannot subdivide
`recording` from outside, and you are right — and worse, on a *clipped* page our own three
phases mislabel it: 56 % of artwork's `recording` is the per-pixel multiply of a clip's
residue into a coverage tile, which is geometry by any reading and lands outside the
geometry span. We are recommending that `Options::instrument_encode` grow an optional
detail level that emits `encode: bounds`, `encode: atlas`, `encode: instances` and
`encode: commit` as extra rows, with `encode: recording` still the remainder so a trace that
sums the rows still gets `encode` and an existing parser sees the three rows it always saw.
It is additive and free when off. It is not in this round.
