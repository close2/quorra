# 0068 — A capture is proven against a baseline, not against a clock

Date: 2026-08-18. Status: **accepted, and built**.

The measurements are `doc/notes-present-settle.md`. The code is
`crates/quorra-gpu/examples/present_thread/settle.rs`, with `xwd::Shot` gaining equality
and a difference report so that the criterion has something to compare.

## Context

`examples/present_thread` reads its own window back with `xwd` and asserts where the affine
put the pixels. Until this ADR it did so after presenting for a fixed 300 ms:

```rust
/// How long to keep presenting the same picture before reading the window back. The X
/// server is on the other side of a socket; this is the one place a wall clock appears
/// in this example, and it is a wait rather than a measurement.
const SETTLE: Duration = Duration::from_millis(300);
```

That comment is honest about what it is and wrong about whether it is enough.
`doc/notes-present-rate.md` §4 records it failing **once in five real-display runs** at load
average 25.22: the capture read the *previous* present, and the assertion that caught it was
right.

**There is no wait to replace it with.** `SurfaceTexture::present` returns `()`;
`Queue::on_submitted_work_done` answers about the device, not the window; Vulkan's
`VK_KHR_present_wait` is not exposed by wgpu 30; the X Present extension's
`PresentCompleteNotify` is delivered on the connection `wgpu` opened inside itself, which
this example cannot read and cannot order against; and past that sit XWayland and a
compositor. `doc/notes-present-settle.md` §1 is the table. This is a *finding* and not a
gap in the search: a convergence criterion here is the correct instrument, not a
second-best one.

## Decision

**A capture is accepted only when two consecutive captures agree on something other than
what the window was last *proven* to show.**

`doc/notes-present-rate.md` §4 named the first half — *"capture until two consecutive
captures agree"* — and that half alone is **the defect made permanent**. While the new
picture is in flight, every capture reads the old one and every pair of them agrees; the
criterion would settle instantly on exactly the stale window that failed the run, and would
then be green for ever. The baseline is what makes it a criterion rather than a pause.

Three consequences follow.

**Settles form a chain.** Each one hands back a capture proven to be the window as it is,
and that capture is the baseline the next settle must differ from. `Settle` owns exactly
that one fact.

**The chain's first link is an erase.** A present carrying no layers leaves the presenter's
own clear (ADR 0056; `src/present/pass.rs` loads the swapchain image with
`Clear(TRANSPARENT)`), and "all of the window is the clear" is a state named by the
*library* rather than by a previous capture — the only terminal state this file can
recognise without a proven capture in front of it. A stale capture during an erase reads
the picture, which is not the clear, so the first link cannot converge early either. The
example erases twice: once before the pixel proof, and once after the rate phase, which
resizes the window and so ends the first chain.

**The bound is a count.** Sixty-four capture rounds, each carrying one present. ADR 0052's
seam decides the shape — *a claim about "how many" is a count and is exact; a claim about
"how fast" is a duration and this machine cannot measure one* — and the number is derived
from what can stand between a present and a readable window: `desired_maximum_frame_latency:
2` (`src/surface.rs`), one pending XWayland presentation, one compositor frame, so about
five refreshes. Reaching the bound is `NotSettled`, whose two arms name the count and which
of the two ways it failed. Real-display runs never needed more than **4** of the 64.

### What this deliberately does not do

**It does not know what the picture should look like.** Any picture that is stably not the
previous one satisfies it, so a window that settles on the *wrong* picture settles
immediately and fails the assertions in `main.rs`. `settle` answers "is the window showing
the present I issued"; `main` answers "is that present right". Collapsing the two would
produce "retry until it passes", which is ADR 0052's gate that cannot fail for its own
regression — and which §4 rejected in advance.

## Consequences

**It was seen doing its work, not merely not failing.** Over 29 completed real-display runs
at load averages from 7.75 to 55.77, **37 of 145 settles took a first capture that was not
the window's settled contents** — exactly the capture the old instrument asserted on. A
26 % per-capture stale rate over two capture sites is what a 1-in-5 run failure rate looks
like from the inside. The green runs on their own would be weak evidence (0 of 29 leaves a
95 % upper bound of 9.8 % on the old rate); the mechanism count is the evidence.

**It is faster, which is incidental and worth stating anyway.** A healthy settle costs two
presents and two `xwd` round trips instead of 300 ms of presenting, five times per run.

**It costs an erase that changes what the window shows between steps.** Step 8's argument
improves as a result — the window is *proven* blank before the device draws through
`Target::Surface`, where before the argument was that the last present happened to leave
the corner black.

**It cannot see a torn window that survives two captures.** Captures are one present apart,
which under `Fifo` is at least one refresh plus an `xwd` process, so a state that survives
that is the window's contents rather than a tear — and if it ever is not, `main`'s
assertions report a wrong picture, which is a hole and a sentence rather than a plausible
lie. Stated in the module comment rather than left for a reader to discover.

**One thing this round found and did not take.** The render phase's `presents >= 2` is a
count whose value is decided by a wall clock, and it refuses 3 of 18 runs at load 36.9 to
55.8 and none of the 14 below 19 — a second instance of `HANDOVER.md`'s *"read a gate's
threshold against the number it names"*, pre-existing and unrelated to this criterion.
`doc/notes-present-settle.md` §5 has the mechanism and §7 recommends the round.
