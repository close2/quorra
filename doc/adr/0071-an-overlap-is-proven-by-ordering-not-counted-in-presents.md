# 0071 — An overlap is proven by ordering, not counted in presents

Date: 2026-08-22. Status: **accepted, and built**.

The measurements are `doc/notes-present-overlap.md`. The code is
`crates/quorra-gpu/examples/present_thread/main.rs`,
`render_on_another_thread_while_presenting` and `RenderHold`. It is the item
`doc/notes-present-settle.md` §7 recommended and deliberately did not take, so that one
round would not mix two subjects.

## Context

ADR 0056 splits the device from the presenter so that a window can be presented while a
`&mut Device` renders elsewhere. `examples/present_thread` is what proves it, and until this
ADR the proof was a count:

```rust
assert!(
    presents >= 2,
    "the point of the split is presenting during a render; only {presents} got through"
);
```

The property it means — *at least one present completed while the render was still
running* — is the right one. The instrument is not. **The number of presents that fit a
span is `span / refresh`, and the span is a wall clock on a shared machine.** The surface is
`PresentMode::Fifo` by construction (`rate.rs`'s module comment), so a present cannot go
faster than a refresh no matter how little it costs; the count therefore measures how many
refreshes the render happened to outlive, which is a property of the scheduler and of the
display, not of the split.

`doc/notes-present-settle.md` §5 recorded it failing **3 of 18 real-display runs** at load
36.9 to 55.8, and none of the 14 below 19, in two different ways:

- twice the presenting thread could not be scheduled — one present in 25.4 ms, three
  refreshes — so the count reached 1 while everything worked;
- once the render itself took **6.4 ms, less than one refresh**, so one present was the
  arithmetically correct answer and the assertion was wrong about its own subject.

The second is the one that matters, because no threshold fixes it: with a render shorter
than a refresh, *no* count above 1 is achievable and the gate is asking for something the
arrangement cannot produce.

## Decision

**The overlap is proven by ordering: a present that *returned* while the render thread was
still rendering.**

The render thread renders back-to-back and publishes that it is doing so (`RenderHold::holding`,
raised before the first render and lowered after the last). The presenting thread presents
once and reads the flag **after `present` returns**. That read is the whole gate.

Three consequences, and the third is the one that keeps it a gate.

**It is decided by an order, not by a duration.** A scheduler that starves either thread
delays the answer; it does not change it. Nothing divides a span by a refresh.

**The render loop is bounded** — `RENDER_HOLD_CEILING`, 300 ms, a *stopping rule* of the
same shape as `rate::CADENCE_SPAN` and not a measurement. Without a bound, the regression
this gate exists for turns into a hang: a present that cannot proceed while the device is
held would wait for a loop that is waiting for it.

**It still fails for its own regression.** If presenting could not proceed while the device
was held elsewhere, the present would return only after the loop had reached its ceiling and
lowered the flag — so the gate goes red, names how many renders ran and for how long, and
terminates. Forced and measured: 30 renders in 304.1 ms, red, at
`main.rs`'s assertion.

## Consequences

**A healthy run costs one render, which is what it cost before.** The loop stops after the
render it is inside as soon as the proof is taken, so the ceiling is a bound that is never
approached rather than a wait that is always paid — `settle`'s 64 rounds are the same shape.

**The count is not asserted anywhere, and it is still printed.** How many presents fit a
render is worth reading and is not a gate; `rate::cadence` is where it is measured properly,
at a display that states its own refresh, against that refresh.

**One thing the flag does not cover, stated rather than left to a reader.** `fixture::page()`
builds an 18 000-rectangle scene, and the scene is now built *before* the flag goes up: a
present that overlapped a scene build would prove nothing about a device held by a render.

**What it does not prove.** That presents keep landing at the refresh while a render runs is
a rate question, and `rate::cadence` owns it. This gate proves the ordering only — that the
presenting thread is not blocked by the rendering one — which is exactly ADR 0056's claim
and no more.

## Alternatives rejected

**Raise the threshold, or lower it to `presents >= 1`.** The first makes the flake worse;
the second cannot fail, because the loop counts a present before it can observe anything.

**Make the render longer than a present by construction** (§7's first option). A synchronised
render is a render that is waiting, and the flag would then cover a wait rather than work.
Bounding the loop and letting the *presenting* thread stop it gives the same certainty with
the device busy throughout.

**Wait for a present-complete signal instead.** There is none: ADR 0068's table lists what
was looked for at this seam — `SurfaceTexture::present` returns `()`,
`Queue::on_submitted_work_done` answers about the device, `VK_KHR_present_wait` is not
exposed by wgpu 30, and X11's `PresentCompleteNotify` is delivered on a connection this
example cannot read. That finding is unchanged and is why an ordering *between our own two
threads* is the strongest instrument available here.
