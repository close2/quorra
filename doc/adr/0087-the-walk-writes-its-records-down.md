# 0087 — The walk writes its records down, and a new viewport replays them

Date: 2026-08-27. Status: **accepted, and built** — ADR 0084's stage A, with its
estimate corrected by the measurement the build produced.

## What was built

Under `Coverage::Compute` the encode records, per command and in encounter order, the
half of its answer that does not depend on the viewport: the outline id beside its
denormalised control box, rectangle hint and §10.7.4 collapse table, the command
transform, paint, rule, style and clip id (`encode/replay.rs`). A retained scene
rendered at a **new viewport** — same device, same lane, same resource generation —
replays the records instead of walking the scene: `EncodeSource::RecordReplayed`.
Every per-viewport answer (compose, cull, seat, instance bytes) is computed fresh by
the same arithmetic the walk uses, and `tests/record_replay.rs` holds the strong form
of that claim: a record-replayed frame is **byte-identical** to the full walk at the
same viewport, across a zoom, an oblique step, §10.7.4 marks and a Slow-record
stroke.

Admission is structural. The sites that build frame-wide state a per-record replay
cannot rebuild — a child layer, a soft mask, a residue clip (and with it every route
back into the atlas and winding lanes) — are the sites that abandon the list, and a
scene using them re-walks byte for byte as before. Commands that are merely
*expensive* per viewport (strokes, rare paints, images) become `Slow` records and
re-dispatch through the ordinary walk, in order: the few pay full price, the many pay
three steps.

## The measurement, and the correction it makes

ADR 0084 priced stage A at 5–8 ms against a 20–26 ms walk, from the dense-text
archetype's profile: 78 % recording, much of it hashing and dispatch. The worst
page's compute-lane walk has a different shape, and the record replay exposes it:
**16–19 ms replayed against 14–20 walked — no win**. On this lane the walk was never
dispatch- or hash-bound; it is **seat-and-instance-bound** — `scratch.reserve`,
`push_tile`, the quad bytes — which is exactly the work a byte-identical replay must
also do. The one hash probe per fill the records eliminate was 2.3 % of recording,
and 2.3 % is what the replay saves.

So the stage's value is not the number its estimate promised, and this ADR says so
rather than leaving the estimate to be believed. What the stage actually bought:

- **The record shape, proven.** Stage B — the walk on the device — needs a flat,
  denormalised, viewport-free record list with a byte-identity test behind it, and
  that is now a thing that exists and passes. ADR 0084 ordered the stages this way
  precisely so B would not be built on an unproven shape.
- **The remaining host floor is named**: seat and instance writes, ~16 ms on 58 009
  tiles, the exact work stage B moves to the device.
- Pages whose walks *are* dispatch-heavy (mixed content, many Slow-adjacent shapes
  under few fills) replay at the discount the estimate imagined; the worst page is
  simply not one of them.

Beside it, the timestamp work (ADR 0086's neighbouring commit) reordered the whole
optimisation: the frame's floor is the compute lane's own device passes — count
11–20 ms, emit+deposit 24–30 ms of GPU on the worst page — not the host walk at all.

## Held by

`tests/record_replay.rs` (byte identity across three viewports; the same-viewport
frame still takes the cheaper `Replayed` road; a child layer re-encodes), and the
full suite (623), clippy pedantic clean.
