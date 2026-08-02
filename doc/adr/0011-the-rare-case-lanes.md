# ADR 0011 — The rare-case lanes: images, ramp sweeps, meshes

Status: accepted, 2026-08-02. Landed with M7.

## Context

The brief's §0 premise is that most of a page is glyph quads and axis-aligned
rectangles; images and shadings are the rare case that must still be exact. Three
constraints shaped the design: ADR 0006 (fixed-function arithmetic diverges across
adapters, so anything that can be our arithmetic should be), ADR 0008/0010 (one CPU
rasteriser feeds every coverage source; knockout needs a per-element shape pass),
and the caller's already-taken decisions — per-placement filtering (§4.5), sampled
function shadings arriving as grids, meshes arriving pre-rasterised at device
resolution (integration note 5).

## Decision

**One uniform-driven quad per op, not a third and fourth instance stream.**
`Op::Image` and `Op::Shaded` each carry a small params block; the executor builds a
per-op uniform + bind group and issues one 4-vertex strip inside the same render
pass as the neighbouring lane batches. Rare primitives do not deserve a lane's
plumbing; a page dense with them would motivate instancing, and that page has not
been measured.

- **Both shaders map device pixels back through the inverse transform** carried in
  the uniform, so the quad only has to *cover* the footprint. An axis-preserving
  image placement gets the analytic cell-overlap of the rectangle lane (ADR 0005) —
  exact antialiased edges; an oblique one paints fragments whose centres land inside
  the unit square — hard edges, the stated cost of the rare-rare case. §8.9.5's
  orientation (top row at v = 1) is applied at sampling.
- **Nearest filtering is `textureLoad`** — exact and adapter-invariant. **Linear is
  the hardware sampler** (clamp-to-edge): its interpolation precision is the
  driver's, the one place a §4.5 decision buys hardware variance; tests bound it
  with tolerance instead of hiding it.
- **Ramps are pre-sampled on the CPU to 256 straight-RGBA8 texels** at first use;
  the shader indexes with `textureLoad` at `round(t·255)`. The sweep's colour
  arithmetic is ours and deterministic; the driver never filters a ramp. The sweep
  parameter itself implements §8.7.4.5.2 (axial projection) and §8.7.4.5.3 (radial
  quadratic, larger root with non-negative radius) in shading space, so shears and
  rotations sweep correctly.
- **Where extension is off, nothing is painted** — §8.7.4.5.2/.3 paint nothing
  beyond an unextended boundary, which is not the same as painting the end colour.
  Consequently an unpainted region has no *shape* either: the knockout `fs_shape`
  emits zero there, because no mark was made and §11.4.6 replaces only what an
  element actually paints. Likewise an image's shape is its geometric footprint ×
  clip × soft mask; its own alpha and the constant alpha are opacity (§11.4.7.2's
  split), while a mesh raster's alpha is antialiased triangle coverage and counts as
  shape.
- **Coverage for shaded fills and strokes** comes from the frame's scratch (the M5
  rasteriser, residues multiplied in); a rect-hinted outline under an
  axis-preserving transform skips the tile for analytic coverage — the shading twin
  of ADR 0007's fast path. Meshes sample at absolute device pixels from their
  uploaded anchor; a zoom re-uploads them, which is the caller's documented cost.
- **GPU forms are lazy.** Upload keeps the validated CPU copy (M2); the texture (and
  the ramp's 256-texel sampling) is created on the first frame that draws the
  resource, so startup and pages without images pay nothing (§7). `release` drops
  the GPU form with the CPU copy.

## Costs, stated

Per-op uniforms mean a buffer + bind group per image/shading per frame — fine at
"rare", wrong at "hundreds", and the counter to watch is command count in `Timings`.
Oblique image edges are not antialiased (a clip path is the workaround the format
itself uses). Linear-filtered output is not byte-stable across adapters; nearest is.

## What holds it

`tests/m7.rs`: nearest ×4 magnification against exact blocks; §8.9.5 orientation;
constant alpha; linear midpoint within tolerance; axial per-pixel bytes against the
CPU-sampled ramp; radial rings; extend-off transparency; shading through a triangle's
coverage tile; mesh anchoring; unknown-id refusals by name.
