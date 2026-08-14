# 0043 — The warm set learns the surface's format

Date: 2026-08-14. Status: accepted.

## The gap

Every pipeline is keyed by `(kind, target format)`. The warm set compiled four
pipelines, all at `WARM_FORMAT` (`Rgba8Unorm`) — the readback and internal-texture
format every headless frame needs. A surface negotiates its own format at
construction, and on this machine's RADV that is `Bgra8Unorm`. So a presenting host's
first frame compiled the lane it drew with *inside that frame* — the exact cost
ADR 0040 took off a headless first frame, still sitting on the caller's launch path,
which is the path §7 exists to protect. `HANDOVER.md` had carried the item since
ADR 0040, blocked on a measurement that needs a window this account cannot open.

## The measurement

The owner ran `examples/surface_measure.rs` on RADV at the real display,
`MESA_SHADER_CACHE_DISABLE=true`, eight rounds, fresh device per round, every round
first waiting until warm — the caller's hand-over order. **Every presenting first
frame reported exactly one `pipeline compile (first use)` entry, 0.3–1.0 ms**, on
both scene shapes: the dense-text page (flat, so the entry is `CoverOver` in the
surface's format) and the artwork page (layered, so the entry is `Blit`, the hand-off
to the surface). Honest context from the same run: the machine's load average was 55,
so medians are contamination and the minima carry the result; and the compile is a
small share of a first frame that costs 38–206 ms wall, most of which is cold-atlas
encode and upload. This fix removes a named, measured 0.3–1.0 ms; it does not claim
to move the rest.

## The decision

`Device::build` passes the negotiated surface format (None for a headless device)
into `PipelineStore::spawn_warm_up`, and the warm-up thread compiles the presenting
lanes — `RectOver`, `CoverOver`, `Blit` — a second time in that format when it
differs from `WARM_FORMAT`.

**`Composite` is deliberately not in the second set.** Since ADR 0038's hand-off, a
composite's target is always an internal accumulator in `WARM_FORMAT`; the surface
only ever receives the direct lanes (a flat frame) or the blit (a layered one). A
`Bgra8Unorm` composite would warm a pipeline no frame can reach. The image, shading
and mesh quads stay on first use in every format, as §7 always had them.

The cost is up to three more compiles on the thread nobody blocks on — `is_warm`
arrives correspondingly later, which ADR 0040 already accepted for the compositor's
pair — and nothing at all for a headless device or a surface that negotiated
`WARM_FORMAT`.

## Verification

- `pipeline.rs`'s unit tests hold both halves of the property headless: warming with
  a present format populates the three presenting pipelines in that format and *not*
  the composite; warming with `None` or with `WARM_FORMAT` compiles nothing outside
  `WARM_FORMAT`.
- The owner's re-run of `surface_measure` after the change is the end-to-end check:
  first frames on both shapes should report **no** compile entries. (Recorded in
  `PLAN.md`'s entry for this date once the run is in hand.)

## What this deliberately does not do

A surface whose format changes *after* construction — a window dragged to a monitor
whose compositor negotiates differently — would still compile on first use, refused
never, exactly as before: the warm set is an optimisation, and `get` remains the
truth. And no attempt is made to warm per-format variants of the rare-case lanes;
they are rare by the corpus's own census (ADR 0011, `doc/corpus-profile.md`).
