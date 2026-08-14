# 0052 — A gate that cannot fail for its own regression

Date: 2026-08-15. Status: **accepted**.

## Context — the gate was wrong in both directions at once

`tests/perf_gate.rs::a_readback_frame_does_not_pay_for_its_pixels_twice` guarded
ADR 0022's claim — *the readback reads once and divides never* — with a wall clock:

```rust
assert!(best < Duration::from_millis(6), "…measured 1.32 ms on RADV (ADR 0022), \
        against 3.84 ms in release before it");
```

Read those two numbers against the threshold. **The regression the gate names is 3.84 ms,
and the gate admits anything under 6.** Restoring the exact shape ADR 0022 removed — the
staging `Vec` in `read_buffer`, then a demultiply of that — would have passed this
assertion on any quiet machine, comfortably. The gate could not fail for the one thing it
existed to detect.

What it *did* fail for was load. Measured on this machine on 2026-08-15, code unchanged:
**two runs in five**, readings of 6.5–8.8 ms at load averages of 7 to 30. The gate's own
comment already recorded the mechanism — *"load average 19 took this to 80 ms with the
code unchanged"* — and the file's module comment already stated the rule it was breaking:

> where a gate needs to be deterministic it uses timestamp queries rather than a
> stopwatch — wall clocks lie under load, and CI runners are always under load.

So the gate was decorative for its stated purpose and noisy for every other. A gate that
cries wolf is not a gate; it is a thing people learn to re-run until it passes, which is
what had been happening.

## What was tried, and why no threshold survives

Two normalisers, on the theory that a *ratio* would divide the load out — which is
`HANDOVER.md`'s own prescription for wall-adjacent numbers (run the configurations
round-robin so drift falls on all of them). Ten rounds each, round-robin, minima, at load
averages from 7 to 30:

| normaliser | observed ratio on unchanged code |
|---|---|
| readback ÷ (whole frame − readback) | **1.17 – 9.12** |
| readback ÷ one 8 MB host read-write pass | **0.48 – 1.84** |

Neither separates 1.32 ms from 3.84 ms with any margin: the noise band is wider than the
signal band on both. A constant picked from those spreads would be fitted to the machine's
load history rather than derived from anything, which is what CLAUDE.md principle 5
forbids and what `HANDOVER.md` means by *never publish a crossover as a constant*.

**The honest conclusion is that no wall clock on this machine can hold this claim.**

## Decision — count the allocations, and say what is left un-gated

ADR 0022 makes two claims, and they are not the same kind of claim:

- **"reads once"** is a statement about *how many target-sized buffers exist*. That is a
  count, and a count is exact.
- **"divides never"** is a statement about throughput. That is a duration, and this
  machine cannot measure one.

So the gate is split along that seam. `tests/readback_cost.rs` asserts the count:
a `Readback` frame allocates **exactly one** buffer of a megabyte or more, and its size is
**exactly** the raster's `width × height × 4`. `tests/perf_gate.rs` keeps printing the
duration and asserts only that something timed it, with the numbers written beside it for
whoever reads a CI log.

The instrument is a `#[global_allocator]` in the test crate that forwards every method to
`System` and counts allocations at or above a megabyte on the thread that opened the
window.

### Verified in both directions before it was written down

Not "this passes", which proves only that a test exists:

| tree | allocations ≥ 1 MB | bytes |
|---|---:|---:|
| as it stands | **1** | **8 022 576** — the raster exactly |
| `read_buffer`'s staging `Vec` restored | **2** | 16 213 552 |

Nine runs of each at load averages from 7 to 30, on **both** adapters (RADV, and llvmpipe
at LLVM 22.1.8): identical every time, to the byte. The regression run was produced by
editing `readback.rs` back to the pre-ADR 0022 shape, watching the new test fail with the
message it was given, and restoring.

Note the second row's total. The staging buffer is the copy-out's **padded** extent —
`bytes_per_row` rounded to `COPY_BYTES_PER_ROW_ALIGNMENT`, so 8 191 kB against the
raster's 8 023 — which is why the assertion is an equality on the byte count and not a
bound in megabytes. A bound loose enough to be comfortable would have to admit the thing
it is looking for.

## The cost, written down

**This puts one `unsafe` block into a tree whose principle 3 says
`#![forbid(unsafe_code)]` on every crate.** That is the real price of this ADR and it is
not smuggled: principle 3 names this exact escape hatch — *"an ADR with a benchmark, a
written invariant and a `// SAFETY:` comment, not a quiet `#[allow]`"* — and this is the
ADR. The terms:

- **There is no safe version.** `GlobalAlloc` is an unsafe trait; an allocator cannot be
  implemented without it. The alternative instrument was a byte tally maintained by hand
  inside `readback.rs`, which is precisely the shape CLAUDE.md's first instrumentation
  rule rejects — *a cost written down beside one call is not a cost anybody adds up*. The
  next staging buffer would not have been added to it. An allocator sees the allocation
  whether or not anyone remembered to.
- **It is a test crate**, its own crate, never linked into the library and so never
  reaching the PDF viewer whose security posture principle 3 is about.
- **The invariant is one sentence and checkable by reading forty lines:** every method
  forwards to `System` with its arguments unchanged and returns what `System` returned.
  The additions are three `const`-initialised thread-local `Cell<usize>` updates on the
  way past — non-allocating, so they cannot recurse into the allocator, and non-unwinding.
  This allocator therefore *has* `System`'s memory behaviour, and a defect in the counting
  can make a test wrong but cannot make a program unsound.

**Thread-local rather than global**, and that is not a detail: a software adapter
rasterises on worker threads that allocate buffers of their own, llvmpipe is pinned by
name through much of this suite, and a global counter would report those — making the
number a property of the adapter and its core count rather than of the code under test.

**What is no longer gated at all** is the divide half. Nothing in CI now notices if the
demultiply becomes a per-pixel float division again. The instrument for it is callgrind,
which `HANDOVER.md`'s "An encode, exactly" already describes and already prescribes for
this machine, and it is a manual round rather than a gate. Stating that plainly is better
than leaving a threshold in place that was never going to catch it either.

## Consequences

- One test renamed: `a_readback_frame_does_not_pay_for_its_pixels_twice` →
  `a_readback_frame_reports_what_its_pixels_cost`, because it reports and no longer gates,
  and a name that promises a gate is the kind of derived claim ADR 0050's round was about.
- The suite gains a test binary and a test-only module. No library code changed; no public
  API moved; the caller sees nothing.
- The pattern generalises, and deliberately is not generalised yet: any "this path does not
  copy the target twice" claim in this tree could be gated the same way. When a second one
  wants it, `tests/counting_allocator/` is where it lives.
