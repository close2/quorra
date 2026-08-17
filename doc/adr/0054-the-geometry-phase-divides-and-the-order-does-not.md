# 0054 — The geometry phase divides, and the order does not

Date: 2026-08-15. Status: **accepted**.

Asked for in `pdf-viewer/doc/QUORRA_ENCODE_THREADS.md`. The measurements and the design
detail are `doc/notes-encode-threads.md`; the instrument is
`crates/quorra-gpu/examples/encode_threads.rs` and the gate is
`crates/quorra-gpu/tests/encode_threads.rs`.

## Context

Their measurement, on a 49.7 MB single page — 58 009 commands, 3.0 M path segments, no
text, no images, no groups, **not one clip or soft mask** — at 900×1100 on llvmpipe: the
frame is 639.8 ms, `Timings::encode` is 475.9 of it (74.4 %), and inside that
`encode: geometry` is **406.3 ms — 79.2 % of the encode and 59 % of the whole frame**, on
one thread. The adapter is 4.5 %.

They had already tested the obvious alternative and reported it honestly: forcing
`Coverage::Gpu` moved nothing, because `take_gpu_lane`'s triangle test correctly refuses a
52-segment outline whose tile is about three device pixels. **That rule is right and they
did not ask us to change it** — which is why the ask is threads and not a second lane.

## What we measured before designing

Principle 2, and it mattered: **their page shape was not in our archetype set.** Our
`giant` archetype flattens 8 segments for an 80-pixel tile where theirs flattens 52 for
nine, so every number we had was about a different kind of page. `drawing` is now the
seventh archetype. One thread, llvmpipe, cold device per sample, minima of five
round-robin rounds, load 13–18:

| page | commands | segments | encode | geometry | share |
|---|---:|---:|---:|---:|---:|
| **drawing** (their shape) | 58 009 | 3 132 486 | 406.8 ms | **309.0 ms** | **76 %** |
| artwork | 684 | 23 400 | 41.9 | 36.6 | 87 % |
| dense text | 4 320 | 60 480 | 8.0 | 6.1 | 76 % |
| median page | 12 | 120 | 0.064 | 0.044 | 69 % |

Their 79.2 % reproduces at 76 % here, and geometry is the largest phase of an encode on
**every** shape we have.

## Decision

**Divide the geometry phase across a `std::thread::scope` entered and left inside
`Device::render`, in three phases: a serial walk that records a job, a parallel
rasterisation that touches no frame state, and a serial commit in encounter order.**

- **`Options::encode_threads`, default 1.** It is a *permission*, not a preference: a host
  with its own pool, its own seccomp policy or its own reason to stay single-threaded gets
  exactly what it had. ADR 0023's "take one rather than make one" applies.
- **No dependency, and no `unsafe`.** The toolchain is 1.97.1, so scoped threads are in
  `std`; and because the serial pass already knows each tile's cost, work partitions
  statically and needs no work-stealing. `deny.toml` gains nothing.
- **Nothing is built at construction**, so the launch path (§1.8) is untouched.

**Order-dependence is handled structurally, not by placement.** Every route to an
order-dependent effect — `charge`, `pack_scratch`, `push_op`, both instance appenders,
`push_gpu_tile`'s reservation, `plan_child` at both ends — drains the queue before acting.
The encounter order ADR 0021 and ADR 0034 depend on is therefore preserved by construction
rather than by remembering, which is the only form of this that survives editing.

**Not divided, each for a stated reason:** residue-clipped marks (ADR 0049's region cache
is lazily built and would become shared mutable state), the `Coverage::Gpu` lane (a second
question that needs its own flattening), and the rare lanes.

## The evidence

**Determinism — the constraint their §5 states, and the one a thread pool threatens.**

- `tests/encode_threads.rs`: equal bytes *and* equal `Counters` at **1, 2, 3, 7 and 64
  threads** over a busy overlapping page, a repeated atlas key, a budget refusal (same
  variant, same two numbers) and a blank scene.
- **The gate was verified able to fail, in two directions**: removing the
  `push_rect_instance` drain breaks the page at 2 threads; removing `plan_child`'s second
  drain moves `layers_culled` 0→12 and `layer_textures` 3→1.
- A first fixture used a 15-pixel lattice where nothing overlapped and **passed with a
  drain removed**. It is now 6 pixels for a 44-pixel mark. A determinism fixture that does
  not overlap is not a determinism fixture.
- **The caller's corpus, 956 real pages, at 1 thread against 8: every per-page line
  identical.** Scale 4 likewise unmoved at 936/10/5/23. That is the check `HANDOVER.md`'s
  first trap exists for, and it is stronger than any fixture: their pages carry clip
  chains, groups, masks and atlas pressure that no fixture here combines.
- One real defect was found *by the gate rather than by reading*: a duplicate atlas insert,
  `bytes_uploaded` off by exactly 64. The drain had to move from `enqueue` to
  `prospect_for` — after the lane is chosen is too late.

**The win**, llvmpipe, minima of five round-robin rounds, load 17.6 → 13.8:

| threads | drawing encode | drawing geometry | artwork geometry | dense text geometry | median |
|---:|---:|---:|---:|---:|---:|
| 1 | 406.8 ms | 309.0 ms | 36.6 ms | 6.09 ms | 0.0440 ms |
| 24 | **132.2** | **46.9** | 30.1 | 2.27 | 0.0439 |
| | 3.1× | **6.6×** | 1.2× | 2.7× | 1.0× |

artwork is 1.2× and the reason is stated rather than averaged away: 600 of its 684 marks
are residue-clipped and stay serial.
> **The artwork column of both tables above was measured on a page whose clips clipped
> almost nothing** (2026-08-17: `doc/notes-tiling-bound.md` §3, re-cut in
> `doc/notes-clipped-instrument.md`). **The conclusion is unchanged and the re-cut
> strengthens it** — "600 of its 684 marks are residue-clipped and stay serial" is now
> true of marks that do the serial work, where before 592 of them were rasterising a tile
> and multiplying it by zero. The milliseconds are not comparable with anything measured
> after that date.
 **An earlier round at load 25–33 read 24 threads as
worse than 8**, so no crossover is published as a constant — `HANDOVER.md`'s oldest rule.

**The small-page floor.** `PARALLEL_FLOOR_SEGMENTS = 4 096` queued outline segments; below
it no scope is entered. The median page carries **120** — a thirty-fourth of the floor —
and its five thread columns are flat to the fourth digit. Their §4 asked for this
explicitly, citing their own ADR 0228, and a unit test asserts both ends because a duration
on this desktop cannot carry the claim and a count can.

## The cost, written down

- **A queue holds every tile in flight where the walk held one.** That is a principle 3
  exposure and it was closed inside the round: each job carries an upper bound (the control
  hull, which contains the flattened geometry) and the queue drains at 1/64 of
  `max_frame_bytes`. It is a **batching granularity, never a capacity** — a single
  oversized job is queued alone, so no frame is refused for being made of large marks.
- **`recording` is now the largest phase of their page** — 132 ms of encode with geometry at
  47. ADR 0023's "revisit when" is closer than it was. Their §4 excluded recording from the
  ask deliberately; we leave it.
- Five `impl` blocks' worth of drain calls are a rule a reader must trust. The test that
  fails when one is removed is what makes that trustworthy, and it is the reason that test
  exists rather than a coverage number.

## What we do not claim

**Their stated ceiling stands, unaltered.** Geometry to *zero* still leaves about 235 ms of
their frame: recording 83, upload 65, execute 29, the remainder ~52, their own scene walk
16. This buys a zoom step of roughly 640 → 250–300 ms on their machine — a stall becoming a
step — and nothing here implies sixty frames a second. They said that themselves before we
measured it, and it would be a poor return to oversell it back to them.
