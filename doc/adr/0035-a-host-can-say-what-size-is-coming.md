# ADR 0035 — A host can say what size is coming

Status: accepted, 2026-08-13. The rest of the caller's `QUORRA_FEEDBACK.md` §9, after
ADR 0031 took the part that needed no API.

## Context

§9 measures a device's first frame at 12 to 18 ms more than every frame after it, flat
across target sizes, and rules out pipeline compilation: settling a second between
bring-up and the first render changes nothing. ADR 0031 found 2.4 ms of it in an
instrument and moved that to device construction, and wrote down what was left:

> About 6 ms of the first frame is still there, and it is not ours to warm. […] it scales
> with the target — page-sized textures, their bind groups, and the driver's first touch
> of a memory heap that size. A warm-up thread cannot allocate those without knowing the
> viewport, and the viewport arrives with the frame.

Measured again here on a fixture built for it — a 2 448 × 4 752 page with eight groups, so
the compositor allocates layer pairs at all — the gap is larger than §9's: **a first frame
of 24.7 ms against a steady 5.0** (medians of six devices, RADV, release).

## Decision

**`Device::warm_for(width, height)`.** A host that knows the size it is about to ask for
says so, and the frame-sized allocation happens then instead of inside the first frame.
It is a hint: a frame of any size draws correctly whether or not it was called, calling it
again with the same size is free, and a different size replaces what it held.

**The pair it makes is held until the frame that wants it takes it, and no longer.** The
compositor's pool stays per-frame, which is ADR 0012's decision and which this measurement
does not overturn — keeping pairs *between* frames was implemented and measured and moved
nothing in either direction, in a fixture where the noise is larger than the effect.

**Where a host calls it is the whole point.** §7's advice already puts device construction
off the critical path — the caller's `main` spawns a thread for it at its first line —
while a first frame is on that path by definition. `warm_for` is synchronous and belongs
beside the constructor, on that thread.

## What it buys

Same fixture, six devices per configuration, medians:

| | first frame | frames 3–5 |
|---|---|---|
| without the hint | **24.7 ms** | 5.0 |
| with it | **10.3 ms** | 6.8 |

Fourteen milliseconds off the frame a person waits for, on a page whose steady frames cost
five. The steady column moved too, in the wrong direction and by less than the spread of
the runs behind it; a later attempt to re-measure the pair landed while the machine was
loaded and reported 19.9 against 20.0, which is contention rather than a result. Wall
clocks lie under load — CLAUDE.md says so, and this is the third time this project has
been reminded — so the number above is stated as one quiet machine's and the test that
ships asserts a *property* instead.

`tests/device_lifecycle.rs` holds that property: a device warmed for this size, for
another size, or for a zero size draws the same bytes as one that was never warmed. A hint
that changed a picture would be worse than the milliseconds it saves.

## What it does not do

**It warms the layer pair, not everything a first frame allocates.** The scratch sheet, the
atlas texture and the mask textures are still made when first needed, and a page whose
first frame is dominated by one of those will see less than fourteen milliseconds. They are
warmable the same way once a measurement asks for them; this one asked for the pair.

**And it is measured on one driver.** Making the pair and dropping it immediately measured
the same as keeping it (11.3 ms against 10.3), which says RADV keeps a freed allocation of
that size warm. The version that ships keeps the pair, because relying on that is relying
on a promise no API makes.

## Revisit when

The layer textures stop being target-sized. They are allocated at the full viewport today
while the plans that use them cover **0.0 % to 6.5 %** of the page on the three corpus
pages that refuse for bytes at 4×, which is the next thing to fix on this path — and it
would make `warm_for`'s size a poor predictor of what to allocate, since the right size
would then be the largest plan's rather than the target's.
