# 0055 — A ramp's bound belongs to the interval that starts there, except at the ends

Date: 2026-08-15. Status: accepted. Closes `HANDOVER.md` item 3.

## What was claimed, and what the clause says

`HANDOVER.md` item 3 and `device/ramp.rs`'s own comment claimed one thing in two halves:

> §7.10.4's subdomains are **closed on the left**, so a bound belongs to the subfunction
> starting there, and a coincident stop pair should take the *later* stop's colour at
> exactly that point. […] a coincident pair at 0 or 1 always does.

**The first half is right and the second half is half wrong.** The clause defines both
ends, they do not point the same way, and at offset 0 the code was already correct. This
round went to the clause before it went to the code, which is the only reason that was
found rather than "fixed".

## The clauses

ISO 32000-2 §7.10.4, *Type 3 (stitching) functions*. Subscripts are set below the line in
the original and are written inline here:

> The Bounds array shall describe a series of k half-open intervals, closed on the left
> and open on the right with the following exceptions:
>
> - the last interval, shall always be closed on the right,
> - if Domain0 = Bounds0 then the first interval shall be closed on both the left and
>   right and the second (next) interval shall be open on the left.

and, on which subfunction owns which interval:

> The first function shall apply to x values in the first subdomain (interval), defined by
> Domain0 and Bounds0 as defined just above; the second function shall apply to x values in
> the second subdomain, defined by Bounds0 and Bounds1; and so on. The last function shall
> apply to x values in the last subdomain, which includes the upper bound defined by
> Boundsk-2 and Domain1.

The clause's own EXAMPLE 1 writes the two degenerate cases out in interval notation:

> 𝑘 = 2 and 𝐷𝑜𝑚𝑎𝑖𝑛0 = 𝐵𝑜𝑢𝑛𝑑𝑠0 < 𝐷𝑜𝑚𝑎𝑖𝑛1 results in two intervals [𝐷𝑜𝑚𝑎𝑖𝑛0 , 𝐵𝑜𝑢𝑛𝑑𝑠0 ];
> (𝐵𝑜𝑢𝑛𝑑𝑠0 , 𝐷𝑜𝑚𝑎𝑖𝑛1 ].

That answers the three questions this round was sent to ask.

1. **The intervals are half-open, closed on the left.** So an `x` exactly on a bound is in
   the interval that *starts* there, and where two ramp stops share an offset the later
   one's colour applies at exactly that point.
2. **The last bound is not left undefined by extending the pattern.** A rule that is
   closed on the left everywhere would leave `Domain1` in no interval at all; the clause
   does not extend the pattern, it states the exception — "the last interval, shall always
   be closed on the right" — and says a second time that the last subdomain "includes the
   upper bound".
3. **A coincident pair at exactly 0 or exactly 1 is defined, and the two answers are
   opposite.** At `Domain0 = Bounds0` the first interval is `[Domain0, Bounds0]` — a single
   point, closed on both sides — and the second is open on the left, so that point is the
   **earlier** subfunction's. At `Boundsk-2 = Domain1` the last interval is the single point
   `[Domain1, Domain1]`, closed on the right, and the interval below it is open there, so
   that point is the **later** subfunction's.

That these degenerate ends are *defined* rather than merely not forbidden is the clause
saying it again about the encoding, which is the sentence that settles question 3:

> If the last bound, Boundsk-2, is equal to Domain1, then x ′ shall be defined to be
> Encode2(k-1). For the degenerate case, if the first bound, Bounds0, is equal to Domain0
> then x′ shall be defined to be Encode0.

`Encode0` is the *first* subfunction's encode pair and `Encode2(k-1)` the *last*'s. The
clause names which function is evaluated at each degenerate end, and they are the two
different ones. So there is no silence here to record: §7.10.4 defines the interior bound,
both ends, and both degeneracies.

### Why §7.10.4 governs a table of stops at all

A ramp is not a PDF object; it is a shading's colour function already sampled onto stops
by the caller. §8.7.4.5.3's and §8.7.4.5.4's `Domain` entries are what make that function
the one in question — "[t]he variable t becomes the input argument to the colour
function(s)" — and the caller places **two stops at one offset** exactly where that
function is discontinuous, which is at a type 3 `/Bounds` value
(`pdf-render`'s `Ramp::sample_across`, fed by `breakpoints_over`). A coincident pair in a
quorra ramp therefore *is* a §7.10.4 bound, and which side owns it is §7.10.4's answer and
not ours to choose.

Two clauses cover the rest of the function, unchanged by this round: §7.10.1's Table 38 —
"Input values outside the declared domain shall be clipped to the nearest boundary value" —
outside the stops, and §7.10.3's type 2 with an exponent of 1 between them.

*Evidence, not truth (principle 5): the caller's own stitching evaluator selects with
`position(|bound| x < *bound)`, which is the later subfunction at a bound — an independent
reading agreeing with ours on the interior case. It does not implement either end
exception; that is their tree's business and is mentioned only because agreement is
evidence and disagreement would have sent us back to the clause.*

## The defect

`ramp_color_at` walked the stops with `t <= stop.offset`, so a `t` exactly on an offset was
answered by the interval that *ends* there, with `u == 1` — the earlier colour. Three
consequences:

- **The interior bound was the earlier subfunction's**, where the clause makes it the
  later's.
- **The last offset was too.** A ramp whose final two stops share offset 1.0 answered with
  the earlier colour at `t == 1.0`, where the last interval is closed on the right.
- **The `span <= 0.0` branch that was meant to implement the clause was unreachable.**
  Reaching a stop required `t > previous.offset` and `t <= stop.offset`, which for
  `span == 0` is a contradiction. A guard cannot implement a rule it can never be asked.

The first offset was **not** part of the defect: `t <= first.offset` gives a coincident pair
at the ramp's start to the earlier stop, which is precisely §7.10.4's second exception. It
now says so, because a line that is right by accident is a line that gets "fixed".

## Why no test could see it

`ramp_color_at` had no unit test at all until the debt round the day before; it had only
ever been exercised through a rendered frame, and a frame cannot ask what colour sits at
one exact parameter value. The corpus could not see it either, and that is the more
interesting half: at scale 1 the one page in the corpus that carries an affected ramp
**agrees with the oracle both before and after**, so no line of the comparison changed. It
takes scale 4 to print the page at all.

## The fix

One comparison, `t <= stop.offset` → `t < stop.offset`, and two comments that now say which
clause each of the three cases is.

The comparison also *earns* what the dead guard was standing in for. With `<`, the loop body
is reached only when `previous.offset <= t < stop.offset` — the loop returns at the first
stop above `t`, so `previous` is never above it — and therefore `span > 0` strictly, as a
local theorem rather than as a fact borrowed from `upload_ramp`'s validation. The guard is
removed rather than kept: it could not have caught a NaN (`NaN <= 0.0` is false) and it can
no longer be reached by anything else. `u` now stays in `0..1`, which is the half-open
interval written in arithmetic.

`RAMP_RESOLUTION`'s claim changes with it, and is restated. The snap of a hard boundary onto
the sampling grid is now **one-sided and strictly under one step** — the boundary lands on
the first grid position at or after its offset, never before it. Before, a boundary that
fell exactly *on* a grid position was displaced a whole step past it, which is an error no
resolution would have removed.

## Verification

`device/ramp.rs`'s test module, eight tests, each expectation derived from the clause named
in its comment:

- `a_coincident_pair_is_a_step_and_not_a_ramp` now asserts the bound itself — the debt its
  own comment named — as well as the two sides.
- `the_ramps_two_ends_take_a_coincident_pair_opposite_ways` is the two exceptions, and it is
  the test that would have refused this round had the reading been taken from the pattern
  instead of the clause.
- `a_boundary_on_the_grid_gives_its_texel_to_the_later_subfunction` pins the witness: an
  offset of exactly `2048/4095` is asked by texel 2048 and by nothing else, and that texel
  read red before.
- `a_hard_boundary_lands_within_one_grid_step_of_its_offset` is restated to the one-sided
  bound above.

**Verified able to fail, in both directions.** With `<=` restored, three of the eight fail
and five pass. The one that fails inside `the_ramps_two_ends…` is the assertion at the
ramp's **last** offset; the assertion at its **first** offset passes under the old code,
which is the check that the exception at 0 is real and not a symmetry we assumed.

427 tests pass on both adapters (RADV and `QUORRA_ADAPTER=llvmpipe`), 53 binaries, from 425
before — `cargo fmt --all --check` and `RUSTFLAGS=-D warnings cargo clippy --workspace
--all-targets` clean, with `Checking quorra-gpu` in the clippy log rather than only
`Finished`.

## The corpus

One copy of the caller's tree, one hour, flipping only the `[patch]` between a checkout at
the base `0e7923f` and this change. Both coverage lanes at both scales:

| lane, scale | base `0e7923f` | this change |
|---|---|---|
| CPU coverage, scale 1 | 931 / 23 / 2 / 18 | 931 / 23 / 2 / 18 |
| GPU coverage, scale 1 | 929 / 25 / 2 / 18 | 929 / 25 / 2 / 18 |
| CPU coverage, scale 4 | 936 / 11 / 4 / 23 | 936 / 11 / 4 / 23 |
| GPU coverage, scale 4 | 937 / 10 / 4 / 23 | 937 / 10 / 4 / 23 |

*(`PLAN.md`'s scale-4 row read 936 / 10 / 5 / 23. Nothing regressed: their tree moved, and
one page that refused at scale 4 now differs instead. That is the reason `HANDOVER.md`
insists the base is run in the same copy and the same hour, and this is a second instance
of it.)*

**No verdict moves, and exactly one page line of 956 changes** — the same one on both lanes,
by the same digits, at scale 4:

```
- differs: issue10572.pdf: mean 0.1332 worst tile 7.97 at (256, 1792) differing 0.0005 ssim 0.99497
+ differs: issue10572.pdf: mean 0.1036 worst tile 7.97 at (256, 1792) differing 0.0004 ssim 0.99602
```

**It moves toward the oracle on every number that moves**: the mean falls, the differing
fraction falls, the SSIM rises, and the worst tile is unchanged in value and position.
Every other differing and refused line at both scales on both lanes is identical to the
character, and the four refusals are the same four documents by name.

`issue10572.pdf` is the page `RAMP_RESOLUTION`'s own comment was written from — 24 hard
stripes whose boundaries snap to the grid.

### That the scale-1 result is not vacuous

"Nothing moved at scale 1" is only evidence if a corpus ramp could have moved. A throwaway
probe in `sample_ramp` (deleted with the round) counted, over the scale-1 CPU lane:

| | |
|---|---:|
| ramps sampled | 411 |
| ramps carrying at least one coincident bound | 2 |
| coincident bounds in total | 24 |
| coincident bounds landing on the sampling grid | 2 |

Both on-grid bounds are in one 48-stop ramp, and narrowing the run to a single document
attributes it: `issue10572.pdf`. So the change *does* move two texels at scale 1 — the page
simply stays inside agreement there, and the comparison prints no line for a page that
agrees. At scale 4 the same two texels are the page line above.

That is also the honest limit of this evidence: **the corpus shows one page's worth of it**,
because a coincident bound is rare (2 ramps of 411) and one landing on the grid is rarer.
The clause, not the corpus, is why the comparison changed.

## What it costs

- **Two texels of one corpus page**, both toward the oracle. Nothing else in 956 pages at
  two scales on two lanes.
- **A branch that can no longer be reached is gone**, and with it the ability to answer a
  zero span at all. That is deliberate: the span is now strictly positive by a two-line
  argument stated beside it, so a check would be a claim nobody can test — which is the
  shape the removed guard already was.
- **No performance question.** `sample_ramp` runs once per resident ramp on the CPU and the
  comparison is the same instruction it was.

## What this does not touch

- **The caller's `Ramp`.** They sample the function and place the coincident pair; where
  they place it is theirs. Their `sample_across` filters breaks to `0 < at < 1`, so a
  coincident pair at exactly 0 or exactly 1 cannot arrive from them today — both ends are
  implemented here because `Device::upload_ramp` is a public API that accepts them, not
  because a corpus page needs them.
- **`RAMP_RESOLUTION` itself.** 4096 is unchanged; only what it can promise is restated.
- **The shader.** It reads the table with `textureLoad` at a rounded index (ADR 0011); the
  bound rule lives entirely in the table's arithmetic, which is why this is a CPU-side
  change with a byte-exact consequence on every adapter.

## Revisit when

A ramp arrives whose *first* two stops share an offset and whose earlier colour is not what
a caller expected. §7.10.4's second exception is the least intuitive sentence in the clause
and the one this round nearly overwrote; if it is ever questioned again, the answer is the
EXAMPLE 1 line quoted above, and the question should reach the clause before it reaches
`ramp_color_at`.
