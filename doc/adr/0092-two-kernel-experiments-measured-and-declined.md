# 0092 — Two kernel experiments, measured and declined

Date: 2026-08-27. Status: **accepted — a measurement round: both of ADR 0091's named
experiments were built, measured on the worst page, and reverted.** The numbers are
the deliverable; the code is deliberately not, and this file is what keeps the next
round from re-running either experiment on hope.

## Experiment 1: partition the flatten by cubic content

Built: `StoredOutline::has_cubics` at upload, the tile record carrying it, twin
`count`/`emit` pipelines with the cubic arm compiled out (`HAS_CUBICS = false` drops
`flatten_cubic` and its ~170 scalars of per-thread stack), each variant guarded to
its own population over the full dispatch range.

Measured: **the populations are the finding.** The worst page is 58 000 of 58 000
cubic tiles; the dense-text archetype 7 426 of 8 470. The lines-only dispatch that
keeps its occupancy has nearly nothing to run on either archetype — vector drawings
are curves and glyphs are curves — so the count and emit times did not move. A page
of pure rulings would benefit; neither target page is one.

## Experiment 2: the count as a closed-form bound

Built: a `BOUND` pass computing, per cubic, an upper bound on the recursion's leaves
from the flat-test's own ratio (`4^k ≥ distance/tolerance`, one level of slack,
capped at the recursion's depth cap), so the allocation stall waits for arithmetic
instead of the flatten; the emit bounds-checked every write and raised an overflow
flag read **before the deposit may run** (a wrong picture impossible by
construction); an exact-count road taken on overflow and eagerly when the bound
exceeded the budget, so no frame the old path accepted could newly refuse. A
sabotage-feature test forced the fallback and held it to the fast road's bytes.

Measured, worst page: the bound pass is **3.5–4.1 ms of GPU against the exact
count's 11–20** — the closed form works — and the frame got *worse*: 81–95 ms
against 75–90. Two reasons, both structural:

- **The deposit pays for the slack.** Edges land at bound offsets, so the buffer has
  gaps (a full-tree bound over an adaptive recursion over-reserves ~2×), and the
  deposit's per-row walk reads the sparser layout ~6 ms slower.
- **The flag is a sync.** The host must know the emit did not overflow before the
  deposit may be submitted, which serializes ~20 ms of emit against a host that
  previously submitted emit, deposit and the content pass in one chain and waited
  once.

## What stage B inherits

The bound's viability (4 ms for the allocation question) is real and stays on the
table — but only inside a design that removes the sync and the sparsity together,
which is stage B's device-side layout work (a scan over exact per-tile counts on the
device, feeding an indirect deposit), not a bolt-on to the host-driven chain. That is
ADR 0091's conclusion re-confirmed from the other side: the kernels' costs move
together, and piecemeal scheduling changes trade one for another.
