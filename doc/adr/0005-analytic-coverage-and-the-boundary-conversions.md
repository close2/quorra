# ADR 0005 — Analytic coverage, premultiplied compositing, and the boundary conversions of the rectangle lane

Status: accepted, 2026-08-02. Landed with M1.

## Context

M1 draws axis-aligned rectangles (`RENDER_LIBRARY.md` §6.4) and hands back
straight-alpha RGBA8 (§3). Three pieces of arithmetic had to be defined, and none of
them is defined by ISO 32000-2:

1. **Anti-aliasing.** The specification does not define it. §10.7.4 discusses image
   interpolation; nothing normative says how a shape's edge maps to partial pixel
   coverage. Clauses around the subject were read before recording this silence
   (CLAUDE.md principle 5's rule about silences): §8.5 defines geometry, §10.7 defines
   scan conversion tolerances "typically half a pixel" without prescribing coverage.
   So the choice is ours, and must be documented as a choice.
2. **The compositing arithmetic between commands**, at 8-bit target precision.
3. **The premultiplied→straight conversion** at the readback boundary, where §3 fixes
   the format but not the rounding.

## Decision

**Coverage is the exact area of the pixel's unit cell inside the rectangle.** The
fragment shader computes `max(0, min(max_x, px+1) − max(min_x, px))` per axis and
multiplies — no supersampling, no approximation at corners (the two partial extents
multiply, which is exact for an axis-aligned rectangle). The quad is expanded outward
to the pixel grid (`floor`/`ceil`) so border pixels get fragments. `rect.wgsl` carries
this ADR's number.

**Compositing is premultiplied over, in fixed-function blending, quantised to unorm8
between commands.** Colours are premultiplied once, at encode (straight at the
boundary, premultiplied internally, §3); the source is scaled by coverage in the
shader; the blend factors are `(ONE, ONE_MINUS_SRC_ALPHA)` for colour and alpha alike.
The target being `Rgba8Unorm`, each command's result is stored at 8 bits and the next
command blends against that stored value — the CPU reference in `tests/m1.rs`
implements exactly this quantisation model.

**Demultiplication is integer arithmetic with round-half-up:**
`straight = (c·255 + a/2) / a`, clamped to 255. The clamp covers the ≤1-ulp cases
where unorm blending leaves a channel a hair above its alpha. Zero alpha zeroes the
pixel. `readback.rs` implements it; its unit tests pin the edges.

## Consequences

- The expected value of every golden is derivable from this ADR by hand, which is
  what principle 5 demands of a test where the specification itself is silent — the
  reference is *our stated definition*, never another renderer's output.
- Exact-area coverage is what makes §6.4's claim reachable later: axis-aligned edges
  are exactly reproducible under cuts where oblique edges are not (the caller measured
  this in its `strip_cut_exactness` tests), so the rectangle lane composes exactly.
- Rounding ties in the float→unorm conversion cannot occur in exact arithmetic —
  `255·f` never lands exactly on k+½ for f32 `f`, because (2k+1)/510 is not
  representable — but the *hardware's* conversion does not promise round-to-nearest at
  all, which is ADR 0006's subject.
- The cost: fixed-function blending delegates the final quantisation to the driver.
  What that trades away, measured, is ADR 0006.
