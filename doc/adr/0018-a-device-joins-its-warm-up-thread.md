# ADR 0018 — A device joins its warm-up thread when it is dropped

Status: accepted, 2026-08-07. Found while writing `tests/backend_choice.rs` for ADR
0017; the defect it fixes is older than that test and independent of it.

## Context

`PipelineStore::spawn_warm_up` compiled the warm set on a thread and **dropped the
`JoinHandle` deliberately** — completion is observed through `is_warm`/
`wait_until_warm`, and a frame that arrives early compiles what it needs on the spot,
so nothing in the library needed the handle. That reasoning was about *waiting*, and it
missed what a handle is also for: **ownership of a thread's lifetime**.

A thread compiling a pipeline is inside the driver. When the process reaches `exit()`,
libc runs the loader's atexit handlers — and Mesa's do real work, tearing down RADV's
disk-cache worker threads — while our detached thread may still be in
`vkCreateGraphicsPipelines`. The result is a crash after every test has passed, in a
thread the host never created.

**The reproduction**, `tests/device_lifecycle.rs` on this machine: four tests in
parallel, each building a device on every adapter and dropping it without rendering.
Against the unfixed source, **13 of 15 runs** died — SIGSEGV mostly, SIGABRT sometimes
— after `test result: ok` had printed. `coredumpctl` named the faulting thread:
`quorra-warm-up`.

This is not a test-only shape. A host that probes adapters builds devices and drops
them; so does one that constructs a device, finds a limit it cannot live with, and
falls back to its CPU backend. The window between construction and the warm set being
done is a handful of milliseconds, and it is exactly the window a probe lives in.

## Decision

`spawn_warm_up` returns the `JoinHandle`; `Device` holds it; `Drop for Device` joins
it. `None` when the host could not give us a thread and the warm set compiled inline —
§2.1's "not requiring a background thread" is unchanged, and so is everything else:
construction still returns before pipelines exist, `is_warm` still says whether they
do, and nothing new blocks.

**Dropping a device now waits for a compile nobody else waits on.** Measured in
release, worst of five per adapter, dropping immediately after construction: 8.5 ms on
RADV and 8.5 ms on llvmpipe, of which 3.9 ms and 3.5 ms is the device teardown that
happened anyway. So the join adds **~5 ms to a device dropped before it is warm, and
nothing at all to one dropped after** — a device that has rendered a frame is warm by
construction, so the cost falls on the probe, which is the case that was crashing.

The join's result is discarded: the thread's body compiles two pipelines and sets two
fields, and a device being dropped has no one left to report a panic to. `is_warm`
would stay false, which is the honest residue.

## Alternatives

**Ask the thread to stop.** There is nothing to ask: `vkCreateGraphicsPipelines` is not
interruptible, and a flag checked between the two compiles would still leave a driver
call in flight for most of the window.

**Leave it, and document that a host should `wait_until_warm` before dropping.** This
is the shape CLAUDE.md's instrumentation rule names outright: a cost — here a
correctness obligation — written down beside one call is not one anybody adds up. A
crash that only happens sometimes, in teardown, on some drivers, is the worst possible
thing to leave to a doc comment.

**Detach and outlive the process's teardown by leaking the device.** Trades a crash for
a leak and a driver still holding a queue; no.

## What it does not fix

A host that `std::mem::forget`s a device, or calls `std::process::exit` while one is
alive, still exits with a thread in the driver. Both are the host stepping outside
`Drop`, and neither is something a library can take back.

## Revisit when

The warm set grows past two pipelines, which would grow the worst-case drop with it —
the number above is a measurement of today's warm set, not a bound. Or if `wgpu` ever
offers a cancellable compile, which would make the wait avoidable rather than merely
short.
