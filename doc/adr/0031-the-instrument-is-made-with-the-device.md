# ADR 0031 — The timestamp instrument is made with the device, not with the frame

Status: accepted, 2026-08-12. Takes the part of the caller's `QUORRA_FEEDBACK.md` §9 that
is ours to take, and measures the part that is not.

## Context

§9 reports that a device's **first frame costs 12 to 18 ms more than every frame after
it**, that the excess is flat across target sizes, and that it is *not* pipeline
compilation — settling for a second between bring-up and the first render changes nothing.
Their ask: warm the allocations the way `spawn_warm_up` already warms the shaders, because
page one goes to the device on the launch path.

Reproduced here on RADV, a 600-outline page at 1191 × 1684 to a texture target: frame 1
costs 11.2 to 11.9 ms and frame 2 costs 0.9. The three phases a `Frame` reports name only
3.6 ms of it, so most of the excess is outside what the caller can see, which is why the
attribution had to be done on this side.

Timing the inside of `Device::render` on the first frame puts it here:

| | first frame | later |
|---|---|---|
| encode | 2.01 ms | 0.22 |
| upload | 1.49 | 0.16 |
| **making the frame's timestamp query** | **2.43** | 0.01 |
| the rest of `run_frame` | 5.87 | 0.58 |

## Decision

**One `PassQuery` per device, made in the constructor, lent to each frame.**

It was made per frame — a `QuerySet` and two sixteen-byte buffers, to time one pass. The
driver charges for the first one and pools the rest: measured over five fresh devices,
`PassQuery::new` costs **2.35, 2.37, 2.60, 2.65 and 3.34 ms** the first time and **0.018
to 0.036 ms** the second. So the instrument was charging a page's whole draw time to the
frame a person waits for, and a rounding error to every frame after.

The constructor is the right place rather than the warm-up thread: a host that follows §7
already builds the device off the critical path — the caller's `main` spawns a thread for
it at its first line — while a first frame is *on* that path by definition.

**The frame takes it and gives it back**, rather than borrowing it: everything else a
frame does needs `&mut self`. Giving it back is conditional, and that is the interesting
half — a frame whose timestamp read failed may leave the map buffer mapped, and the next
`map_async` on a mapped buffer is a validation error rather than a number. So a query that
was read goes back and one that was not is dropped, which costs the next frame 0.02 ms to
replace and cannot poison it. `tests/perf_gate.rs` renders eleven frames through one
device and holds every one of them to a timestamp-query `execute`, which is the property
that would break if the set came back wrong.

## What it does not fix, and what that would take

**About 6 ms of the first frame is still there, and it is not ours to warm.** A trivial
8 × 8 render before the real one absorbs only ~1.5 ms of it: what is left is inside
`run_frame`, and it scales with the target — page-sized textures, their bind groups, and
the driver's first touch of a memory heap that size. A warm-up thread cannot allocate
those without knowing the viewport, and the viewport arrives with the frame.

Which makes the remainder an **API question rather than an optimisation**: a device could
be told the size it is about to be asked for — `Device::warm_for(width, height)`, or a
size hint on `Options` — and spend the allocation on the background thread that already
exists. That is a decision about the caller's contract, so it is not taken here; it is
recorded with its number so the conversation starts from one.

## Revisit when

The size hint is decided either way, or a measurement shows the remaining 6 ms moving with
something other than the target's area. Both want `tests/perf_gate.rs`'s clock rather than
a wall clock on a loaded machine: the end-to-end first frame measured 5.4 to 11.3 ms
across runs here, which is why this ADR quotes the cost of the *allocation* — five
devices, one number apiece — rather than the frame it came out of.
