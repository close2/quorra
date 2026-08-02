# ADR 0006 — Cross-adapter byte identity: measured "no" for the fixed-function path

Status: accepted, 2026-08-02. Landed with M1. Answers `RENDER_LIBRARY.md` §11
question 4 for this design; the M6 compositor decision must treat it as input.

## Context

The brief's §4.6 asks two different determinism promises to be kept apart:

1. Same scene, same viewport, **same adapter** → the same bytes.
2. **Across adapters** — RADV and lavapipe byte-identical — which the caller's CI
   relies on for the current Vello-based backend, and which §11.4 explicitly asks us
   to answer *by measurement*, early, because the answer changes how their CI works.

M1's design draws through the fixed-function raster path: the fragment shader
computes coverage in f32 and the *output-merger* converts the result to `Rgba8Unorm`
and blends. Whether that conversion is bit-identical across drivers was the open
question.

## The measurement

Probes in `tests/m1.rs` (the tie probe was run 2026-08-02 on this machine, wgpu 30,
RADV on RDNA 3.5 vs lavapipe/llvmpipe 22.1.8):

- **A single opaque full-coverage rectangle** — no blending against content, no
  fractional coverage — of colour component 0.1 (f32: 0.100000001…, ×255 =
  25.50000038) stores as **26 on llvmpipe and 25 on RADV**. Component 0.9 → 230 vs
  229. Component 0.5 (an exact 127.5) → 128 on both.
- On the 48×32 golden scene: llvmpipe differs from the ADR 0005 CPU reference by at
  most **1 unorm step** (80 of 6 144 bytes), RADV by at most 2 in straight-alpha
  space (30 bytes); llvmpipe and RADV differ from each other by at most 2.

So the divergence is in the **float→unorm8 store conversion itself**, before blending
adds its own step: Vulkan leaves the rounding of that conversion
implementation-defined, and the two drivers genuinely round differently. The current
Vello-based backend achieves cross-adapter identity only because it composites in
compute shaders and quantises in its own arithmetic — the fixed-function path never
runs.

## Decision

- **Promise 1 stands, byte-exact, and is gated**: repeated renders on one adapter and
  across freshly constructed devices compare with `assert_eq` in `tests/m1.rs`.
- **Promise 2 is not made for the fixed-function path.** The cross-adapter and
  CPU-reference gates pin a *stated bound* instead: ±1 unorm step per blend stage in
  premultiplied space, amplified by at most 255/α by demultiplication (≤ ±2 on the
  golden, whose minimum alpha is 128). Drift beyond the bound still fails the build —
  the gate distinguishes "store-conversion rounding" from "something real diverged".
- **The finding is an input to M6, not a settled fate.** Clause 11's fifteen
  non-Normal blend modes need programmable blending anyway; when M6 designs the
  compositor it decides whether the Normal fast path keeps fixed-function blending
  (fast, loses cross-adapter identity) or moves final quantisation into shader code
  (restores identity, at a measured cost). That ADR must weigh the caller's CI
  reliance on identity against the measured price.

## Consequences

- The caller hears "no, not for this path, and here is the bound and the lever"
  now — at M1, from our suite — rather than from a golden mismatch at M9, which is
  exactly what §11.4 asked for.
- Our own goldens cannot be per-adapter byte-exact PNGs; they are a CPU reference
  plus a bound, per ADR 0005, and same-adapter byte equality carries the exactness
  load.
- If M6 moves quantisation into shaders, the bound gates tighten back to `assert_eq`
  — `tests/m1.rs` says so beside the tolerance constant, so the relaxation cannot
  outlive its reason silently.
