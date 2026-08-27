# 0090 — The hybrid flattens what the atlas will not hold on the device

Date: 2026-08-27. Status: **accepted, and built** — the per-frame hybrid ADR 0080
named as an open question and the caller's ADR 0700 asked for from outside: under
`Coverage::Cpu`, glyphs keep the atlas and its replay, and a solid fill the atlas
declines — no admission, or a residue-free tile the prospect calls too large — takes
the compute lane's device-side flattening instead of the processor's scanline.

## The gate, and why it is safe

`Options::compute_assist`: `None` decides by adapter — on for a real device, off
where the "device" is a software rasteriser (`wgpu::DeviceType::Cpu`), because a
compute dispatch on llvmpipe loses to the scanline it replaces (the caller measured
600 ms against 229). `Some` overrides either way, which is what lets
`tests/compute_assist.rs` run the hybrid on the CI's own software adapter and hold
it to **zero pixels** against the un-assisted frame — the same identity
`tests/compute_lane.rs` holds the whole lane to, and the whole reason a routing
policy can be a heuristic here without being a correctness question.

Fixed at device construction, so the retained-encode key needs no new field; under
`Coverage::Cpu` the record-replay list is `None` regardless, so replay admission is
untouched.

## The population, stated honestly

The reroute catches fills with **no atlas admission** — above the atlas's tile
ceiling, or refused by the prospect. What it does not yet catch is the *overflow*
population: a tile admitted to a full atlas today falls through to the sheet **at
commit**, after a worker has already rasterised it, so rerouting there saves
nothing. Catching those before rasterisation needs the admission to know the atlas's
remaining room at enqueue time — a further increment with its own bookkeeping, named
here rather than implied.

## Held by

`tests/compute_assist.rs`: whole-frame byte identity with the hybrid forced on over
llvmpipe, the routing observable through the lane's own named spans, and off-is-off.
Suite 631, clippy pedantic clean.
