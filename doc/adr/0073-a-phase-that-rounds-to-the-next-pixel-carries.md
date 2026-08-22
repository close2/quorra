# 0073 — A phase that rounds to the next pixel carries into the origin

Date: 2026-08-22. Status: **accepted, and built**. **It moves pixels: 133 of 957 corpus
pages.**

The measurements are `doc/notes-glyph-phase-carry.md`. The code is
`crates/quorra-gpu/src/atlas.rs` (`GlyphPlacement::of` and its three new unit tests) and
`crates/quorra-gpu/examples/lane_placement.rs`, which is the instrument that found it.

## Context

ADR 0009 makes the glyph quantum §4.5's fifth decision — the one that is ours to expose.
A placement's device translation is split in two: the integer part says where the tile's
pixels go, and the fractional part is **rounded to `1/q` of a pixel** so that repeats of one
outline share a rasterisation. `Options::glyph_quantum` defaults to 16.

The rounding was written as a wrap:

```rust
let nx = (fx * fq).round() as u16 % q;
let ny = (fy * fq).round() as u16 % q;
```

`(fx · q).round()` reaches **`q` itself** for any `fx ≥ 1 − 1/2q` — a fraction within half a
bucket of the next pixel, which is 3.1 % of phases per axis at the default quantum. `% q`
maps that to bucket 0 and leaves the integer part alone, so the tile was rasterised at phase
zero and seated at `floor(e)` where the placement asked for `floor(e) + 1`.

**The mark was drawn a whole device pixel low**, in x, in y, or in both. Not a bound
exceeded: a bound absent — the one input for which the quantum was not a quantisation at all.

Three things kept it invisible for the life of the project:

- **`GlyphPlacement::of` had no unit test.** The atlas's tests covered packing, eviction and
  admission; nothing covered the arithmetic that decides where a cached mark lands.
- **Every sweep that could have found it was aliased with the quantum.** The instrument's own
  first run swept 16 positions of 1/16 and measured exactly zero error at all sixteen,
  because every sample sat on a bucket boundary. A step sharing no factor with `q` is what
  visits the inside of a bucket.
- **The caller's corpus gate runs with the quantum off.** `render-quorra/tests/corpus.rs`
  sets `glyph_quantum: None`, deliberately, "to isolate the backend's fidelity from the
  deliberate sub-1/32-pixel trade" — and their separate quantum gate,
  `real_pages.rs::the_glyph_quantum_cost_stays_bounded`, is a *statistical envelope* over
  mean, worst tile and SSIM. A 3 % population of whole-pixel misplacements raises a mean; it
  does not crater an SSIM. **So the setting their product ships is not the setting their
  gate measures**, and no gate on either side of the boundary was looking at this.

## Decision

**A phase that rounds to `q` is a carry into the integer origin, not a wrap to zero.**

```rust
if nx == q { nx = 0; ix += 1.0; }
```

The key is unchanged — such a placement belongs in bucket 0, and now it belongs there *of the
next pixel*, which is what it was always asking for.

With the carry in place, the quantum states a bound it actually holds:

> a placement is rounded to the nearest of `q` buckets, so it moves a mark by at most
> **half a bucket — `1/2q`, or 1/32 of a device pixel at the default** — in each axis
> independently.

That sentence is now three things at once: an ADR line, a unit test
(`a_quantised_phase_moves_a_mark_by_at_most_half_a_quantum`, over 513 positions), and an
assertion in an example CI runs.

## Consequences

**133 of 957 corpus pages move onto the caller's oracle**, measured at the quantum their
product ships (`glyph_quantum: Some(16)`), one copy of their tree, base and change in the
same copy on 2026-08-22:

| | agree | differ | refused | not comparable |
|---|---:|---:|---:|---:|
| base | 800 | 155 | 2 | 17 |
| with the carry | **933** | **22** | 2 | 17 |

**Zero pages regressed** — the change set is a strict subset relation, and the 22 that still
differ are exactly the 22 that differ with the quantum switched off entirely.

**The quantum's page-level cost was never the quantum.** With the carry fixed, `Some(16)` and
`None` produce the *same* verdicts — 933 / 22 / 2 / 17 either way. The 133 pages the caller's
documents attribute to "the deliberate sub-1/32-pixel trade" were this defect, and the trade
itself costs no page its agreement. What it still costs is sub-pixel: means move in the third
decimal, which is what a 1/32-pixel bound looks like.

**A `--check` run that could not fail is fixed with it.** The example's uniform four-step
sweep visits 0, ¼, ½ and ¾ and never enters the top bucket, so the assertion passed under the
forced defect. `offsets` now always appends `1 − 1/4q`. Verified by forcing the defect: green
before, `-0.9844 device pixels, past the quantum's own bound of 0.0312` after.

## What this does not decide

**It is not the caller's §31.** Their four pages were measured with `lane_diff.rs`, which
also sets `glyph_quantum: None`, so the per-command offset they report is not this and is not
the quantum. Their §31 stays open; `doc/notes-glyph-phase-carry.md` §5 says what our own
instrument can and cannot reproduce of it.

**It does not change the default.** 1/16 remains ADR 0009's measured choice; what changed is
that its cost is now bounded in fact and not only in prose.
