# `encode` on more than one thread — what was measured, what was built, what it cost

Round notes for the caller's `pdf-viewer/doc/QUORRA_ENCODE_THREADS.md`, written
2026-08-15 against this worktree. The decision this round takes belongs in an ADR, which
the owner holds; this file is the measurement and the reasoning behind it.

**The short version.** `encode: geometry` is a pure function of one mark's own geometry,
so it divides exactly. It divides now, behind `Options::encode_threads` — default **1**,
so nothing changes for a host that does not ask. On the caller's own page shape at their
own window size, geometry goes **309.0 ms on one thread to 46.9 ms on twenty-four**
(6.6×), and the whole `encode` **406.8 → 132.2 ms** (3.1×). On our other archetypes the
win is between nothing and 2.7×, and the reason is stated below rather than averaged away.
The caller's own ceiling stands: their frame has ~235 ms in it that this cannot touch.

---

## 1. What we measured first, and on which pages

`examples/encode_threads.rs` is the instrument. Four page shapes, each encoded on a
**fresh device with a cold atlas** — because the page this exists for fills the atlas on
its first frame and reads it on every frame after, so a steady state of twenty frames
would be measuring the tile cache and calling it the rasteriser. Thread counts are
round-robin within each round and minima are reported, with the load average printed
either side.

Three of the four shapes are `tests/archetypes.rs` rows. **The fourth was not in our set
and had to be added**, which is the first finding of this round:

> A lane tuned on the pages we have is a lane tuned on the wrong page.

The caller's file is 58 009 commands — 58 003 fills, six strokes — over **3 011 879 path
segments**, 51.9 a fill, with no text, no image, no group and not one clip, at a fit view
where a mark is about three device pixels across. Nothing in our archetype set had that
ratio: `giant` also reuses exactly one outline per command, but it flattens **eight**
segments for an eighty-pixel tile where this page flattens **fifty-two** for nine pixels.
`drawing` is now the seventh archetype (1 200 commands in the gate, so a debug build can
finish; the example builds the full 58 009).

Its counter row is `[1200, 0, 1200, 1194, 0, 6, 0, 0, 0]`, and the two numbers worth
naming are **1 194 atlas keys and 6 sheet tiles**: the six strokes have no atlas at all —
a stroke's coverage is its expansion, not its outline — and every three-pixel fill tile
the atlas takes. The caller's page has exactly that split, six strokes among 58 009
commands.

### The shares, on one thread

`llvmpipe`, headless into a texture, `Options::instrument_encode` on, cold device per
sample, minimum of five round-robin rounds, load average 13–18.

| page shape | commands | segments | `encode` | `encode: geometry` | geometry's share |
|---|---:|---:|---:|---:|---:|
| **drawing** (the caller's page, 900 × 1100) | 58 009 | 3 132 486 | 406.8 ms | **309.0 ms** | **76 %** |
| artwork (1191 × 1684) | 684 | 23 400 | 41.9 ms | 36.6 ms | 87 % |
| dense text (1191 × 1684) | 4 320 | 60 480 | 8.0 ms | 6.1 ms | 76 % |
| median page (1191 × 1684) | 12 | 120 | 0.064 ms | 0.044 ms | 69 % |

The caller measured 79.2 % of encode on their machine; we read **76 %** on ours for the
same page shape. **The finding is theirs and it reproduces** — and it is not special to
that page: geometry is the largest part of a cold encode on every shape we have, the
median page included, where it is 69 % of a *sixteenth of a millisecond* and therefore
worth nothing.

---

## 2. What was built

`crates/quorra-gpu/src/encode/parallel.rs` and its child `parallel/commit.rs`, their own
modules; `encode.rs` grew by 15 lines of call site and not by the mechanism (2 406 →
2 421).

**The phase splits in three**, and the split is the design:

1. **the walk**, serial and unchanged: resolve the clip, choose the lane, and record a
   `Job` — everything needed to rasterise one mark and nothing about where it will land;
2. **the fan-out**, parallel: `rasterise(&Job) -> Option<CoverageMask>`, which reads no
   frame state, writes no frame state and allocates only what it returns;
3. **the commit**, serial and in encounter order: charge the budget, offer the tile to the
   atlas or pack it on the sheet, append the instance.

Everything order-dependent is in 1 and 3: the frame budget's running total, the scratch
sheet's shelf cursors (ADR 0034 made encounter order load-bearing and **declined to sort**
it), the atlas allocator, the instance stream and the layer plans. So the third phase
produces the numbers a one-threaded frame produces, in the sequence a one-threaded frame
produces them.

The two modules are split along the same seam: the parent is the *work* — a `Job`, a pure
function over one, and the arithmetic that divides a list of them, none of which can see a
frame — and the child is the *frame*.

**No dependency was needed and none was added.** `std::thread::scope` is entered inside
`Device::render` and left before it returns; nothing exists between frames, nothing is
built at construction, and `#![forbid(unsafe_code)]` is untouched — a scoped thread handing
out disjoint `&mut` sub-slices of the result vector needs none.

**The host names the number.** ADR 0023 recorded the caller's own answer to "should quorra
build a thread pool?" as *no — take one rather than make one*, for three reasons that are
still true (their `rayon` would be oversubscribed; their confined worker's seccomp filter
kills the `/sys` read `glibc` sizes its arenas from, so a lazily-spawned thread there dies
as a mysterious kill; a pool at construction lands on their time-to-first-page).
`Options::encode_threads` is a **permission**, not a preference: 1 by default, held to
`std::thread::available_parallelism`, and at 1 the encoder runs the walk it ran before with
no queue and no allocation.

### Where the queue can be observed, and the two rules that close it

A queued job has not charged, not packed and not drawn. So **anything that advances the
frame's order drains the queue first** — `charge`, `pack_scratch`, `push_op`, the two
instance appenders, `push_gpu_tile`'s reservation, and `plan_child` at both ends of a
child's body. Draining *empties* the queue before committing, so the commit reaches those
same methods and finds nothing to drain: there is one set of call sites and not a shadow
set that has to be kept in step.

The other observation is the atlas, and it needed a different answer. A queued job has not
inserted its key, so a repeat of that key would read `entry: None` and be *built* as a
second rasterisation of one picture. Draining at `enqueue` is too late — the lane has
already been chosen on the stale answer — so the drain moved to `Encoder::prospect_for`,
**before** the question is asked. That was found by the determinism gate and not by
reading: `bytes_uploaded` differed by exactly 64 bytes on a page of two hundred placements
of one outline, which is one 8 × 8 atlas upload of a tile that should have been inserted
once.

Nothing else about `AtlasStore::prospect` depends on what is queued, and that is a property
rather than luck: ADR 0029 kept "has the atlas room?" out of that answer deliberately, so
occupancy cannot change a lane.

### What is *not* divided, and why

- **A mark under a residue clip.** ADR 0049's region cache is built lazily during the walk
  and decides, per chain, whether a region is worth keeping — shared mutable state a
  fan-out would have to freeze first. The caller's page states no clip at all, so their
  measurement is silent on this; **our artwork archetype is 600 clipped marks out of 684**,
  which is exactly why its win below is the smallest in the table.
- **`Coverage::Gpu`.** The path lane asks `take_gpu_lane` a second question about the
  *flattened* triangle count, so a job that skipped the flattening would choose its lane on
  one reading and draw on another. Under `Coverage::Gpu` nothing is deferred. This costs the
  caller nothing: they forced that lane for a whole session and `encode: geometry` did not
  move (418.5 ms against 406.3), because the triangle test correctly refuses a 52-segment
  outline whose tile is three device pixels.
- **The rare lanes** (images, shadings, meshes, function paints) and the oblique-rectangle
  path, which arrive already flattened. They are the brief's rare case and they are rare.

### The two bounds, and why each exists

**A floor, so no small page pays.** The caller's §4 excludes *"anything on the frame path
that a small page pays for"* by name, and their own ADR 0228 had to put a measured floor
under a `rayon` image resampler for the same reason. Ours is `PARALLEL_FLOOR_SEGMENTS =
4096` **queued outline segments**: below it the run rasterises on the walk's thread and no
scope is entered at all. A weight rather than a job count, because six 40 000-segment fills
are more work than six thousand triangles and the fan-out should take the first. The median
corpus page is **120 segments** — 1/34 of the floor — and `drawing` at 1 200 commands is
62 400, fifteen times over it. The floor's own unit test asserts both ends, and the median
page's row in §4 is what says it works.

**A ceiling on what the queue holds**, which principle 3 required and the first draft did
not have. Where the walk held one coverage tile at a time, a queue holds every tile in
flight: a page of large marks could hold a gigabyte between the walk and the commit,
un-charged, which is "never allocate from an unchecked number" arriving through the back
door. Each job carries an **upper bound** on its own tile — the control-hull box the walk
already computed for culling, cut by the same clip and target the rasteriser will use,
which bounds the flattened geometry because a Bézier lies inside the convex hull of its
control points — plus the job record itself, and the queue drains when the sum reaches a
**sixty-fourth of the host's stated `max_frame_bytes`** (4 MB at the default). It is a
batching granularity and never a capacity: a single job larger than the limit is queued
alone, so **no frame is refused for being made of large marks**, and every tile is still
charged exactly once, in encounter order, at the commit.

---

## 3. Determinism: the evidence, not the argument

§4.6 and the caller's §5 ask that a frame drawn on 24 threads be the same bytes as the same
frame drawn on one. This is a property the design *has* — each job writes only its own
result slot, the partition is a pure function of the job list, and no worker touches the
sheet, the atlas, the budget or the instance stream — so it is asserted rather than
approximated.

`crates/quorra-gpu/tests/encode_threads.rs`, on `llvmpipe`, at **1, 2, 3, 7 and 64
threads**, each compared to the one-threaded frame:

- **a busy page** — 90 overlapping marks at 0.85 alpha, with a rectangle instance, a
  stroke, a blended fill (§11.3.5's implicit one-element group), a curve-clipped fill (the
  residue path), a real group and a rect-hinted fill placed between runs of ordinary fills,
  so that every drain point is crossed and **draw order is visible in the pixels**. Equal
  bytes and equal `Counters` at every count;
- **a repeated atlas key** — two hundred placements of one outline at one transform: one
  key, one rasterisation, equal bytes at every count;
- **a budget refusal** — the same `FrameBudgetExceeded` with the same two numbers at every
  count, which is the caller's `REFUSED_AT_FOUR` held to equality from our side;
- **a blank scene**, which is a legitimate scene at any thread count.

**The gate was verified able to fail, in both of its interesting directions.** With the
drain removed from `push_rect_instance`, the busy page differs at 2 threads. With the
second drain removed from `plan_child`, `layers_culled` goes 0 → 12 and `layer_textures`
3 → 1 — a group drawn into the wrong plan — and the refusal test fails too. Both were
restored. The first version of that fixture laid its marks on a 15-pixel lattice where
nothing overlapped, and it **passed with the drain removed**; the lattice is now 6 pixels
for a 44-pixel mark, and the doc comment says why.

A fifth datum comes free from the measurement binary, which asserts the counters across
thread counts on every sample: **58 009 commands, 6 tiles, 58 003 atlas keys, 3 132 486
segments, identical at 1, 2, 4, 8 and 24 threads** on the caller's own page shape.

Cross-adapter identity is untouched: nothing here changes what is computed, only which
thread computes it. The suite is green on **both** adapters — 393 tests, RADV and
`QUORRA_ADAPTER=llvmpipe`.

---

## 4. The win, with its load average

`examples/encode_threads.rs`, `llvmpipe`, headless into a texture, cold device per sample,
round-robin over the thread counts, **minima of five rounds**, load average **17.6 before
and 13.8 after**. Milliseconds; `geometry` is the phase this round divides and `encode` is
the phase the caller times.

| threads | drawing `encode` | drawing geometry | artwork geometry | dense text geometry | median page geometry |
|---:|---:|---:|---:|---:|---:|
| 1 | 406.8 | **309.0** | 36.6 | 6.09 | 0.0440 |
| 2 | 244.6 | 162.4 | 35.7 | 5.52 | 0.0489 |
| 4 | 190.5 | 112.5 | 30.7 | 3.20 | 0.0432 |
| 8 | 172.7 | 73.3 | 32.1 | 2.41 | 0.0438 |
| 24 | **132.2** | **46.9** | **30.1** | **2.27** | 0.0439 |
| | **3.1×** | **6.6×** | **1.2×** | **2.7×** | **1.0×** |

**How to read it, and what not to claim.**

- **`drawing` is the case this was built for**, and it is the caller's own page shape at
  their own window size. Geometry divides 6.6× on twenty-four threads and the whole encode
  3.1×, because what is left of encode is recording and staging.
- **artwork is the small win and the honest one.** 600 of its 684 marks are under a curve
  clip and stay on the walk's thread; what divides is the remaining fills and strokes, and
  1.2× is what that is worth. An archetype that did not have this shape would have let us
  publish a much better average.
- **dense text divides**, but only while the atlas is cold: the same page's second frame
  pays no geometry at all, and no thread count improves on zero.
- **median page is flat to the fourth digit** — 44.0, 48.9, 43.2, 43.8, 43.9 µs — because
  its 120 segments never reach the floor and it never enters a scope. That is the caller's
  §4 answered with a measurement rather than a promise.
- **Do not read a crossover out of this table.** An earlier round of the same binary, at
  load average 25–33 instead of 13–18, read 24 threads as *worse* than 8 on `drawing`
  (184 ms against 138). Which count is best is a property of this machine on this
  afternoon, which is `HANDOVER.md`'s rule and the other reason the field is a permission
  rather than a preference: what a host should pass is the host's business.

**The caller's ceiling is theirs and it stands.** Their frame is 639.8 ms of which encode
is 475.9; with geometry at *zero* the frame is still about 235 ms — recording 83, upload
65, execute 29, the remainder ~52, their own scene walk 16. Nothing here contradicts that,
and the honest claim is the one they already wrote: a zoom step from a stall to a step, not
sixty frames a second.

---

## 5. What was found and not done

- **`recording` is now the largest phase of the caller's page.** At twenty-four threads
  their page's encode is 132 ms of which geometry is 47; the rest is recording and staging.
  ADR 0023's "revisit when the subdivision stops being able to attribute" is closer than it
  was. Recording is clip resolution, culling, atlas lookups, instance building and plan
  assembly, and it is the caller's explicitly-excluded second ask (their §4: *not
  parallelism in `recording` or `staging` first*). Left.
- **A residue-clipped mark could join the fan-out.** The shape is visible: resolve the
  chain's verdict serially (which is where the rule and the counters live), hand the worker
  an `Arc` to the held region, and let it crop and multiply. It needs `ResidueRegions` to
  hold `Arc<CoverageMask>` and it needs the per-tile fallback to stay serial. It would move
  artwork's 1.2× and nothing the caller has measured. Not built: the caller's ask is the
  page with no clip on it, and a change to ADR 0049's cache wants its own measurement.
- **`flatten_chain` re-flattens a declined chain's links once per clipped command.** Noticed
  while reading `encode/clips.rs` for the residue question above. It is ADR 0049's
  `PerTile` path — the case the region cache declines — so it is bounded by the admission
  rule, and the artwork archetype's `clip_residue_tiles` is 0, meaning our own p99 shape
  never takes it. Not measured, not touched, recorded here because the next person to open
  that file should know it is there.
- **The in-flight ceiling drains the caller's page about three times** rather than once, so
  the fan-out's shape there is "several large runs" and not "one". Each run is still tens of
  thousands of jobs, far above the floor, so it is not a cost worth removing — but it is why
  the numbers above are what they are and not a little better.
- **`Counters` has no field for segments per tile**, which is the one number that
  distinguishes `drawing` from `giant` and the one this round turned on. The archetype gate
  cannot tell the two pages apart and says so in its own comment. Whether that should become
  a counter is a decision, not a fix.
