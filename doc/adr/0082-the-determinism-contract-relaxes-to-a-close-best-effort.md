# 0082 — The determinism contract relaxes to a close best effort

Date: 2026-08-26. Status: **accepted — stated by the project owner**, verbatim:

> I have decided to officially relax the byte-for-byte determinism contract! A close
> best effort implementation is good enough!

## What this changes, and what it does not

**Relaxed: byte-for-byte identity as a *contract*** — across adapters, and between the
CPU rasteriser and any device lane. The gates that hold such identity today
(`tests/compute_lane.rs`, `tests/compute_coverage_determinism.rs`, the cross-adapter
rows of `m45.rs`/`m6.rs`) keep their current assertions for as long as they pass, because
identity that is *measured* is the strongest evidence "close" can have — but a failure on
a new driver or a new pass is now a tolerance to state and bound, not a crisis. A bound
must still be a number with a derivation (ADR 0072's discipline); "close best effort"
is a licence to be off by stated ulps, never a licence to stop counting.

**Unchanged, because it is free and load-bearing:**

- **Same scene, same viewport, same adapter → same bytes** (§4.6, `doc/PLAN.md` §1.7).
  This is arranged by construction — ordered walks, no scheduling-dependent
  accumulation — and it is what makes a replayed retained frame the frame it replays
  and a flaky test a real defect.
- **Byte equality across encode thread counts** (`tests/encode_threads.rs`): the
  commit's encounter order costs nothing and keeps a thread count from being a picture.
- **The CPU lane as the caller's correctness oracle.** What relaxes is how exactly a
  device lane must match it, not whether it is the reference.

## What it unlocks

The fixed-point mandate ADR 0079 stated for device arithmetic ("wherever byte-identity
across adapters is claimed") loses its premise: the compute lane's float port — measured
byte-identical on this machine's three Mesa adapters, and contractually only *close*
elsewhere — is now sufficient as built, `sqrt` standing in for `hypot` without a
resolution owed. More consequentially, passes whose value was previously blocked on
exactness arguments — per-sample compositing for conflation-free rendering, supersampled
fine stages, a fused-multiply-add the compiler chooses — are now judged by their bounded
error and their speed, which is the trade the owner has asked for by name: *"we want to
be as fast as possible."*

`CLAUDE.md`'s adapter note and `doc/PLAN.md` §1.7 now carry the decision; §11's
question 4 ("is byte-identical output across adapters achievable?") is thereby
**retired rather than answered** — the caller no longer requires it to be.
