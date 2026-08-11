# ADR 0023 — An instrument for `encode`, and which clock each phase is on

Status: accepted, 2026-08-11. Answers the caller's feedback §13, which is explicitly a
request for an instrument before a request for speed. Also records why quorra does not
spawn threads, which is the question that led here.

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
