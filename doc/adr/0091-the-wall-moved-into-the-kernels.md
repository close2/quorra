# 0091 — The wall moved into the kernels, and stage B is re-priced before it is built

Date: 2026-08-27. Status: **accepted — a design round: it prices, and it builds
nothing.** ADR 0084 ordered stage B ("the walk on the device") after stage A had
proven the record shape. Stage A is built (ADR 0087) and its measurement, together
with the lane's new timestamps (the ADR 0086-adjacent commit), changes what stage B
would buy — enough that building it as specified would spend the round's largest
effort on the frame's smaller term.

## The frame, re-attributed

The worst page's zoom step (58 009 fills, `Coverage::Compute`, 890M), after this
round's stages:

| term | ms | owner |
|---|---:|---|
| scene | 0.0 | caller — view-free since their ADR 0702/0703 |
| encode (record replay) | 14–20 | host: **seat and instance writes**, per ADR 0087's measurement |
| residency + records | 3–7 | host: per-frame tile-record build and arena probes |
| count pass | 11–20 | **GPU** — the stall waits for exactly this |
| emit + deposit | 22–30 | **GPU** — the frame-end wait |
| content pass | 0.1 | GPU |

Stage B moves the first two rows to the device: ~20–25 ms of host time, some of it
already overlappable behind the GPU rows. The GPU rows it does not touch: ~35–50 ms.
A perfect stage B lands the step at roughly the kernels' own floor — better, but the
kernels are now the wall, and they were invisible when ADR 0084 priced the stages.

## What stage B still owes before it can be built

- **The seat.** The scratch layout is a serial shelf packer; a device-side seat needs
  a layout computable by scan. Doable, but it changes the sheet's extent behaviour,
  which is budget-checked —
- **The refusals.** The frame budget refuses *before allocation, by name* (principle
  3). A device-side place/seat learns the totals on the device; refusing by name
  still requires them on the host, which is the count stall's shape again, now for
  the whole walk. Any design that keeps the promise has a sync in it; any design
  without the sync weakens the promise. That trade needs its own argument, not a
  paragraph at the end of a build.

## The kernels, and the named next measurement

The count pass alone is 11–20 ms of GPU for one thread per tile walking a handful of
segments — slow enough to suspect occupancy rather than arithmetic. `flatten_cubic`
keeps five 17-deep arrays (~170 scalars) of function-scope storage per thread,
whether or not the outline has a single cubic; on RDNA-class hardware that is a
register/scratch footprint that collapses wave residency for every tile, lines-only
tiles included. Two experiments, both cheap, both honest:

1. **Partition the dispatch by outline content.** The arena knows at residency
   whether an outline has cubics; two pipelines — one stackless for lines-only tiles,
   one as today — and two dispatches. If occupancy is the story, the lines-only
   population (most of a ruled or vector-drawn page) gets most of the win.
2. **Shrink the stack.** Depth 16 mirrors the CPU recursion cap; a shallower cap with
   a coarser tolerance fallback is a *contract* question (ADR 0082's relaxed bounds)
   and is measured before it is argued.

Whichever wins, the emit pass shares the same structure and the same fix.

## Decision

Stage B is not built in this round, and this ADR is the explicit statement of why —
the instruction was to build it, and the measurements the round produced argue
otherwise: the kernel experiments above are the next largest lever, they are
prerequisite to knowing stage B's real payoff, and stage B's refusal semantics
deserve a design that is not written in a hurry at the end of a long round. The
record shape it needs stays proven and tested (ADR 0087), and nothing in this round
moved it further away.
