# 0072 — A conversion bound is read at the pixel it applies to

Date: 2026-08-22. Status: **accepted, and built**.

The measurements are `doc/notes-scale-reference.md`. The code is
`crates/quorra-gpu/tests/m1.rs`: `Reference`, `bound_at`, `disagreement`, and the new
`golden_matches_cpu_reference_at_every_magnification`.

## Context

Two things met in one round, and the second was found while taking the first.

**The gap.** `doc/PLAN.md` recorded the suite's scale coverage as half taken:
`tests/scale_invariance.rs` walks 1×, 2× and 4× and asserts a *property* (ink is area), but
the only place this tree's pixels are checked against the independent CPU rasteriser is
`m1.rs`'s golden, **at scale 1 only** — which is the caller's hayro `#40` ("a defect that
only appears above 1× is one that a test suite rendering everything at 1× cannot see")
pointed at our own instrument.

**The finding.** Comparing at 2× and 4× needs a bound, and `m1.rs` had one:

```rust
/// The stated cross-implementation bound (module docs, ADR 0006): ±1 unorm step per
/// blend stage in premultiplied space, amplified to at most ±2 by the straight-alpha
/// conversion on this golden (minimum alpha 128).
const UNORM_TOLERANCE: i32 = 2;
```

**This golden's minimum alpha is 24, not 128** — the corner pixel where an extent of 0.75
meets one of 0.125 — measured, not argued. The stated derivation therefore gives
`255/24 ≈ 11` and not 2, so the constant was enforcing something nobody could re-derive from
the sentence beside it. It passed because the fixture never exercised the amplification it
claimed to allow for, which is principle 5's failure exactly: the number was right about the
runs and wrong about its own reason. `m3.rs` carries a second constant that cites this one's
derivation, and `doc/HANDOVER.md`'s debt list records them as two constants for a reason that
is now a third reason.

## Decision

**The bound is derived at each pixel from that pixel's own alpha and its own number of
stores, and never from the fixture's worst pixel.**

```
bound(colour channel) = ceil(stores × 255 / α)
bound(alpha  channel) = stores
```

`stores` is what the reference counted — the number of commands that wrote that pixel, which
is the number of float→unorm8 conversions ADR 0006's ±1 applies to — and it is returned by
`cpu_reference` alongside the pixels, because it is a fact only the rasteriser knows. `α` is
the smaller of the two sides' alpha, since that is the one that amplifies more. A pixel
nothing stored to must agree **exactly**: a device that inks where the reference does not is
not a rounding difference, and §3's "transparent is `[0, 0, 0, 0]`" is what makes that
checkable.

Nothing here is a property of the golden, so nothing here has to be restated when the
fixture changes, when a command is added to it, or when the viewport is magnified.

## Consequences

**The gate is stronger than the constant almost everywhere.** Where α is 255 and two
commands stored, the bound is 2 — the constant's value, unchanged. Where one command stored
an opaque pixel it is **1**. It is looser only at the sliver pixels where the amplification
is real, and there it is loose for a reason that can be read off the pixel.

**The comparison now runs at 2× and 4×** and the numbers are in
`doc/notes-scale-reference.md`: both adapters agree with the reference within **1** unorm
step at both magnifications, against 2 at scale 1.

**A magnification is not a repetition.** Every edge of the golden is fractional, so the scale
changes every coverage value, every partial alpha, and the amplification the bound is read
through — the fixture's own minimum alpha is 24 at 1×, 32 at 2× and 128 at 4×. That the
minimum *rises* with magnification is why scale 1 is the hard row and why a suite that only
ran there had its bound derivation wrong in the one place it mattered.

**It is verified able to fail, twice, and the second one is the point.** A 2 % coverage error
in `rect.wgsl` reddens both the scale-1 gate and the new one. A coverage error conditioned on
the mark being wider than 30 device pixels — the shape of defect the caller reported, present
above 1× and absent at it — leaves the scale-1 gate **green** and reddens the new one alone,
at (5, 30) of the 2× raster, 26 steps past a bound of 1.

**`m3.rs` keeps its own constant** and now needs its own derivation rather than a citation of
this one. It is not touched here: that page agrees with its reference to 0 unorm steps, so
its gate has never spent any slack, and changing it is a round with its own measurement.
`doc/HANDOVER.md`'s debt bullet carries the correction.

## Alternatives rejected

**Raise the constant to 11**, the honest raster-wide value. It is honest and it is nine steps
of slack at every pixel that does not need it — a gate that would pass a defect it can see.

**Compare in premultiplied space**, where the bound is ±1 per store with no amplification at
all. The device hands back straight alpha (§3), so this means inverting the conversion, which
is invertible only to within a step — a second approximation to justify in place of an
arithmetic that is exact where it stands.

**Keep the constant and add a second one for each scale.** Three numbers, three derivations,
and the same defect waiting in each of them.
