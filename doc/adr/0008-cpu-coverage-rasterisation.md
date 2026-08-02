# ADR 0008 — One CPU coverage rasteriser feeds the glyph and path lanes

Status: accepted, 2026-08-02. Landed with M4/M5. Revisited when §11.2's census runs
(see `doc/PLAN.md` §1.6 — the census is still open).

## Context

The glyph atlas (M4) and the general path lane (M5) both need R8 coverage of an
outline. Three producers were on the table: a compute-shader rasteriser on the GPU,
`tiny-skia` on the CPU (the brief's §6.3 offered the oracle-agreement argument for
it), or our own CPU rasteriser. And M5's design was, per the plan, supposed to wait
for §11.2's census of how many real commands miss the cheap lanes — a census that
needs corpus fixtures from the caller's tree and has not run.

## Decision

**Our own CPU scanline rasteriser (`raster.rs`), as the single producer for both
lanes** — chosen as the smallest correct design that the census can later overturn,
not as the final word on the path lane.

- Exact signed-trapezoid accumulation with a prefix sum; non-zero coverage is
  `min(|w|, 1)`, even-odd is the triangle fold `1 − |1 − (w mod 2)|` (ISO 32000-2
  §8.5.3.3 defines the rules, not the anti-aliasing — ADR 0005's silence applies, and
  the fold at multi-edge pixels is our stated behaviour).
- Cubics flatten by midpoint subdivision to 0.25 px; subdivision at t = 1/2 is exact
  f32 halving, so flattening is deterministic everywhere.
- Strokes expand to closed polygons — per-segment quads, §8.4.3's caps/joins/miter
  limit — and fill non-zero; overlaps clamp away.
- Quantisation to a byte is `round(cov × 255)`, once, at rasterisation.

## Why CPU, and why our own

- **Determinism across adapters.** ADR 0006 measured that the fixed-function
  float→unorm store conversion differs per driver. CPU-made coverage bytes are
  identical on every adapter, so the divergence surface stays exactly where ADR 0006
  bounded it — the final blend — instead of growing a per-lane term. The
  cross-adapter gate for the coverage lanes holds at the same ±2.
- **Nothing new on the startup path** (§7): no compute pipeline to compile.
- **No new dependency**: `tiny-skia`'s oracle-agreement argument is real, but the
  oracle comparison happens at M9 against the whole pipeline anyway, and a second 2D
  library in the graph is what `deny.toml`'s posture exists to resist. Our arithmetic
  is stated line by line, which is what lets tests derive expectations by hand.

## The measured cost, and what would overturn this

`examples/floor.rs`, release, 2026-08-02, RADV, 5 933 curved glyph fills at
1191×1684 into a texture target: **1.0 ms/frame steady state** (encode 0.73 ms,
execute 58 µs); the cold frame that rasterises all 107 tiles costs 1.9 ms once. The
atlas-hostile page (fresh phases everywhere, ~1 700 tiles) costs ~7 ms on its cold
frame — the failure mode is bounded by CPU rasterisation throughput.

Overturned by: §11.2's census showing the path lane is hot on real corpora, or a
page profile where per-frame CPU rasterisation dominates. The recorded lever is a
compute-shader coverage pass (candidate 1 of PLAN §1.6) fed by the same flattening,
with this rasteriser retained for the atlas.

## Also fixed here

One quantisation, stated: a cached coverage byte differs from the rectangle lane's
analytic float coverage by at most one premultiplied unorm step, and `tests/m45.rs`
pins the two lanes to exactly that bound (in premultiplied space, where the claim
lives — the straight-alpha readback re-amplifies by 255/α per ADR 0005's demultiply).
