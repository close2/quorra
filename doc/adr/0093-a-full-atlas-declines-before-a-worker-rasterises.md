# 0093 — A full atlas declines at admission, and the device draws the tile

Date: 2026-08-27. Status: **accepted, and built** — ADR 0090's named increment. A
tile admitted to a full atlas used to be rasterised by a worker, refused at commit,
and fall through to the sheet with a quantised phase that bought an entry nobody
kept. Where the hybrid gives a refused tile somewhere better to go, the admission now
asks the packer whether the tile *would* fit (`AtlasStore::would_fit` — `allocate`'s
own search, read-only, condition for condition) and declines before any rasterisation
— the tile takes the compute lane's device flattening at its exact phase instead.

Two honest bounds on the mechanism:

- **Jobs still queued for other keys have not consumed their room yet**, so the probe
  can admit a tile a pending insert beats to the shelf. That tile falls through at
  commit exactly as every over-admitted tile always has; it only misses the reroute.
- **The probe is asked only under the hybrid** (`compute_assist` on, `Coverage::Cpu`),
  so the fall-through path keeps its exact old behaviour everywhere else, including
  every existing test's.

## The finding beside the feature

Holding the rerouted frame to byte identity against a pure `Coverage::Compute`
reference exposed a **pre-existing hole in the zero-pixel lane claim**: on the gate's
own fixture the pure Cpu and pure Compute frames differ at ~124 bytes (boundary
pixels, worst 255) — a divergence `tests/compute_lane.rs`'s fixtures never reached.
The gate therefore asserts what the reroute actually promises — every byte it changes
is the compute lane's own, and rerouting only ever removes divergence from the
compute reference — and this paragraph is the follow-up's charter: find the boundary
rule the two rasterisers disagree on, and either close it or state it in ADR 0082's
terms.

## Held by

`tests/compute_assist.rs`: the fixture overflows without the probe (the counter says
so), declines to zero overflows with it, and the changed bytes are the compute
lane's own. Suite 632, clippy pedantic clean.
