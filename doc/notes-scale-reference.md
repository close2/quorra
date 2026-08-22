# The reference at a magnification, and the bound that was not derived — round notes, 2026-08-22

`doc/PLAN.md` recorded this as half taken:

> **The suite's scale coverage, half taken.** `tests/scale_invariance.rs` (2026-08-17)
> renders one fixture at 1×, 2× and 4× and asserts that ink is area […] What is *not* taken
> is the reference comparison: the only test that checks this tree's pixels against the
> independent CPU rasteriser is `m1.rs`'s golden, and it is scale 1 only.

This round takes the other half. ADR 0072 is the decision; the finding below is why the round
is worth reading rather than just merging.

## 1. What a magnification changes, and why it is not a repetition

Every edge of the golden is fractional — 2.25, 3.5, 20.75, 17.125 in scene units, with a
y-flip on top — so a magnification changes how much of a device pixel each edge covers, and
therefore every coverage value and every partial alpha in the raster. What must not change is
that the device computes the areas ADR 0005 defines.

Measured, over the reference raster at each scale:

| scale | target | minimum non-zero α | max device-vs-reference difference (RADV) | (llvmpipe) |
|---:|---:|---:|---:|---:|
| 1 | 48 × 32 | **24** | 2 | 1 |
| 2 | 96 × 64 | 32 | 1 | 1 |
| 4 | 192 × 128 | 128 | 1 | 1 |

The minimum alpha *rises* with magnification, and the arithmetic says why: the smallest cell
overlap at 1× is 0.75 × 0.125 = 0.094 of a pixel, and at 4× the same corner is 0.5 of one.
**Scale 1 is the hard row for the bound**, which is the opposite of where a reader would look
for trouble in a zoom lane, and it is where the finding below was waiting.

## 2. The finding: a constant whose stated derivation was wrong about its own fixture

`m1.rs` carried one tolerance for the whole raster:

> The `255/α` amplification is read off *this* fixture's minimum alpha […] and its page's is
> 29 rather than 128.

`UNORM_TOLERANCE = 2` follows from 255/128 ≈ 1.99. **The fixture's minimum alpha is 24.** The
same sentence applied to the fixture gives 255/24 ≈ 11, so the constant enforced something
five times tighter than its own reason — passing on every run since M1 because the fixture
never produced a difference at a sliver pixel, and therefore never exercised the
amplification it claimed to allow for.

This is the shape `doc/HANDOVER.md`'s traps already name in two other places: **a number that
is right about the runs and wrong about its reason survives every run.** It was found not by
a failure but by needing the number at a second scale — which is the argument for taking a
gap like this even when the tree is green.

## 3. What replaced it

Per pixel: `ceil(stores × 255 / α)` on the colour channels, `stores` on alpha, and exact
equality where nothing was stored. `stores` is counted by the reference itself, so the bound
is derived from the raster rather than typed beside it (ADR 0072 has the derivation and the
rejected alternatives).

Where it lands against the old constant:

| pixel | old bound | new bound |
|---|---:|---:|
| α 255, one command stored | 2 | **1** |
| α 255, two commands stored | 2 | 2 |
| α 139, two commands stored (RADV's worst at 1×) | 2 | 4 |
| α 24, one command stored (the corner) | 2 | 11 |

Tighter where the fixture actually lives, honest at the four slivers. The cross-adapter gate
uses the same derivation with the stores doubled, because there are two conversions between
two adapters and not one — the only doubling in the file, and it is doubled for a reason
rather than for slack.

## 4. Verified able to fail — and the second defect is the point

Both forced in `src/shaders/rect.wgsl`'s `shape_at`, reverted after.

**A. Coverage off by 2 %** (`extent.x * extent.y * 1.02`). Both gates red:

```
golden_matches_cpu_reference_on_every_adapter ... FAILED
golden_matches_cpu_reference_at_every_magnification ... FAILED
at (40, 0) channel 3: got [0, 255, 0, 195], expected [0, 255, 0, 191] — 4 unorm steps past a
bound of 1 (1 stores at α 191)
```

**B. Coverage off by 10 %, only where the mark is wider than 30 device pixels.** The golden's
widest rectangle is 20 device pixels at 1× and 40 at 2×, so this is a defect that is absent
at scale 1 and present above it — the class the caller reported as hayro `#40`/`#8`/`#63`:

```
golden_matches_cpu_reference_on_every_adapter ... ok
golden_matches_cpu_reference_at_every_magnification ... FAILED
adapter 'AMD Radeon 890M Graphics (RADV STRIX1)' differs from the CPU reference at 2×:
at (5, 30) channel 3: got [255, 0, 0, 229], expected [255, 0, 0, 255] — 26 unorm steps past a
bound of 1 (1 stores at α 229)
```

**The scale-1 gate is green in B.** That is the whole justification for the round: the new
gate sees a defect the suite could not have seen the day before, and the demonstration is a
forced defect rather than an argument about what might exist.

## 5. What this round did not do

- **`m3.rs`'s constant is untouched.** It cites `m1.rs`'s derivation, which is now known to be
  wrong about `m1.rs`'s fixture, so it needs a derivation of its own — but that page agrees
  with its reference to **0** unorm steps, so no run is spending the slack, and changing it is
  a round with its own measurement. `doc/HANDOVER.md`'s debt bullet carries the correction so
  the citation is not re-inherited.
- **No `src/` changed**, no page is drawn differently, and no corpus run is owed.
- **`m1.rs` is 933 lines** and this round added to it rather than splitting it. The golden,
  its reference and its bound are one subject and splitting them across files would cost more
  than the length does; the file is a candidate for the debt table's treatment — a module
  comment that earns the exemption or a split along the adapter-policy seam — and that is a
  decision for a round that is about the file rather than about the gap.
