# 0077 — A bound is arithmetic, and arithmetic is shared

Date: 2026-08-23. Status: **accepted, and built**. **It moves no pixel**: nothing outside
`crates/quorra-gpu/tests/` is touched.

The measurements are `doc/notes-two-stale-debts-and-two-taken.md` §3. The code is
`crates/quorra-gpu/tests/common/bound.rs` (new), `tests/m1.rs`, `tests/m3.rs` and
`tests/common/probe.rs`.

## Context

ADR 0072 found that `m1.rs`'s `const UNORM_TOLERANCE: i32 = 2` was derived in its own
comment from "this golden, whose minimum alpha is 128" when that golden's minimum alpha is
**24** — right about the runs, wrong about its reason — and replaced it with a bound read at
each pixel from that pixel's own alpha and its own number of stores. It left a second
constant standing:

> **`m3.rs` keeps its own constant** and now needs its own derivation rather than a citation
> of this one. It is not touched here: that page agrees with its reference to 0 unorm steps,
> so its gate has never spent any slack, and changing it is a round with its own measurement.

This is that round. `m3.rs`'s constant was also 2, and its comment had already been
corrected once (2026-08-17) to say that `m1.rs`'s derivation *does not reach this page* — the
alphas here are `{0, 29, 57, 86, 115, 172, 230}`, so the same premise gives `⌈255/29⌉ = 9`.
What that correction left behind was a number with **no** derivation at all: an honest
sentence saying where it does not come from, and nothing saying where it does. It also
carried `m1.rs`'s disproven "minimum alpha is 128" forward as a description of `m1.rs`,
which by then was a third copy of a claim ADR 0072 had already killed. (The second copy was
`tests/common/probe.rs`'s doc for `max_byte_diff`.)

And ADR 0072 wrote a reason into `bound_at`'s own doc comment for not sharing it:

> **`m3.rs` states its own bound and should keep doing so**: that page's is a claim about a
> fixture, and this is arithmetic over a pixel. The two are different kinds of thing.

That sentence is true, and it is an argument for the opposite of what it concludes.

## Decision

**`m3.rs`'s bound is `bound_at`, read at each pixel, and the arithmetic lives in
`tests/common/bound.rs` where both files reach it.**

`tests/common/mod.rs` already states the seam this turns on — *"a measurement is shared; a
claim about a fixture is not"* — and `bound_at` is squarely on the shared side of it. Both of
its inputs are properties of the pixel it is called for, so it is the same function for every
fixture, every scale and every command list. It was `m3.rs`'s *constant* that was a claim
about a fixture, and the way to stop `m3.rs` making an unsupportable claim is not to give it
a better one but to stop it making a claim.

Moved verbatim, per that module's rule that what arrives there is what each caller already
built: `Reference`, `bound_at`, `disagreement`. `m3.rs`'s `cpu_reference` gains the one line
that counts a store, which is the fact only a rasteriser knows.

**`max_byte_diff` is deleted.** It had three call sites when the probes were given a home;
ADR 0072 took two and this takes the third, and an unused helper under
`tests/common/mod.rs`'s `allow(dead_code)` is one nothing will ever notice.

## Consequences

**Where the new bound is stronger, and by how much.** Measured on m3's fixture, llvmpipe,
2026-08-23. The 4 000 marks are 10.25 × 24.5 on a 14.75 × 32.5 pitch, so they do not
overlap: the store histogram over the raster's 2 005 644 pixels is `{0: 1 071 244,
1: 934 400}` — **no pixel is stored to twice.**

| pixels | share | old bound | new bound |
|---|---:|---:|---:|
| nothing stored | 1 071 244 (53 %) | 2 | **0** |
| ink at α = 230 | — | 2 | 2 |
| ink at α ∈ {172, 115, 86, 57, 29} | — | 2 | 2, 3, 3, 5, **9** |

So it is tighter on 53 % of the raster and looser on the sliver pixels, and the looser half
is the honest direction rather than a regression: 2 was never derivable there, and a gate
enforcing a number nobody can re-derive is principle 5's failure whichever way the number
happens to point.

**The tightening is the one that matters for a clip suite, and it is a real defect class,
not a rhetorical one.** A clip that leaks admits ink at pixels whose store count is zero.
Forced, with `rect_link_box` outset by 0.004 device pixels — a clip rectangle four
thousandths of a pixel too large:

- **3 696 pixels are inked where nothing stored**, and
- **`max_byte_diff` over the whole raster is 1**, so the constant-2 gate **passed**.

The new gate fails at the first of them: `at (60, 79) channel 3: got [0, 0, 0, 1], expected
[0, 0, 0, 0] — 1 unorm steps past a bound of 0 (0 stores at α 0)`.

The size of the outset is load-bearing and was found by measurement rather than chosen. At
0.01 the leak is *also* caught by the old constant — 128 unorm steps, because the
straight-alpha conversion divides a leaked colour by a leaked alpha of 2 and amplifies it
enormously. Only a leak small enough for the alpha to round to 1 and the colour to round to
0 separates the two gates. **A forced defect that both gates catch proves nothing about
either**, which is why the first one is recorded here as an intermediate result and not as
the verification.

**Nothing changes about what the suite reports today.** The page and its reference agree
byte for byte — largest raw difference over the whole raster: **0**, as on 2026-08-17. No
slack of either shape is being spent.

**`m1.rs` loses 109 lines** and keeps what is its own: `cpu_reference`, which is this
project's reading of ADR 0005 and is shared with nothing, since `m3.rs`'s reference is a
different function with clips in it.

## Alternatives rejected

**Derive the constant honestly and set it to 9.** It is the raster-wide value the stated
premise gives, and it is nine steps of slack at every pixel that does not need it — including
all 1 071 244 that need none. ADR 0072 rejected the same option for the same reason at 11.

**Tighten the constant to 0, since the page agrees exactly.** That is curve-fitting to the
runs: it would encode what llvmpipe produced today as the specification's bound, and the
first adapter that rounds one sliver pixel differently fails a gate that was never entitled
to forbid it. It is the same mistake as the original constant with the sign reversed.

**Copy `bound_at` into `m3.rs`.** Two copies of one arithmetic, in a tree whose
`tests/shader_copies.rs` (ADR 0059) and whose duplicated tile arithmetic are both on record
as what that costs. There is no version of this where the second copy is the safe option.

**Keep `max_byte_diff` for a future caller.** A helper with no callers under an
`allow(dead_code)` is invisible to review and to the compiler at once. If a comparison of two
rasters neither of which is a reference is wanted later, four lines will write it then, and
whoever writes it will know why they want the whole-raster maximum.

## What this does not decide

**`m3.rs` still pins llvmpipe** through `common::headless::device`, where `m1.rs` asks every
Vulkan adapter. That is a live gap — the per-pixel bound is exactly what would make an
every-adapter comparison of this page meaningful, and `m1.rs` gained one in ADR 0072 — but it
is a different question with its own cost (a 1191 × 1684 page of 4 000 clipped marks on every
adapter, in the suite's time budget) and its own measurement. It is recorded rather than
taken.
