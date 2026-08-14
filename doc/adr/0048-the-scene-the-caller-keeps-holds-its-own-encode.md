# 0048 — The scene the caller keeps holds its own encode

Date: 2026-08-14. Status: accepted, and built.

Builds ADR 0045's candidate **(A)**, which that ADR left *proposed* because it changes
what the caller sees and CLAUDE.md's rule is that a decision either side can make alone
is a decision neither side has made. The project owner lifted that block on 2026-08-14:
an API change is available if it is written down for the caller, which it is —
`/home/cl/projects/pdf-viewer/doc/QUORRA_RETAINED_FRAME.md`, with a dated pointer in
their `QUORRA_UPGRADE.md`.

ADR 0045 is the pricing, the four candidates and the invalidation list, and none of it
is restated here. This ADR decides **the shape of the thing**: who holds the retained
encode, what makes it stale, and what the API is that a frame loop can actually use.

## What changed since 0045 said "proposed"

Two things, and only one of them is the authorisation.

- **The owner will carry an API change to the caller.** So the question stops being
  "what can we do without asking" and becomes "what should the caller be asked for".
- **0045's own analysis of *whose* obstacle this is stands.** Their frame's scene is
  rebuilt every frame with fresh `Arc`s (`pdf-viewer/doc/todo/44` §3, `Overlays::of`),
  so the device-side cache 0045 sketched — one `Encoded` on the `Device`, keyed by scene
  identity — would have missed on every frame of the document it was designed for. That
  is not a detail of their tree. It is the reason the retained unit cannot be something
  the device infers.

## The decision

**`RetainedScene`: a handle the caller holds, which owns the `Scene` and the encode of
its last frame, rendered by `Device::render_retained`.**

```rust
let mut page = RetainedScene::new(scene);      // the caller keeps this
device.render_retained(&mut page, &viewport, target)?;   // every frame
page.set_scene(rebuilt);                       // when the content changes
```

`Device::render` is untouched: it retains nothing, replays nothing, and reports
`EncodeSource::Encoded` on every frame. A caller that ignores all of this sees today's
library exactly.

### Why the caller holds it, and not the device

Three reasons, in the order they decided it.

1. **A device-side cache cannot be hit by the frame loop that needs it.** Keyed on scene
   identity it misses every frame; keyed on anything weaker it is a promise the caller
   makes in a comment, which is how a wrong page gets drawn.
2. **The handle owning the scene makes the hazard structural rather than contractual.**
   `render_retained` draws the scene the handle holds — there is no second scene for the
   retained encode to disagree with, so **this API adds no way to produce a wrong page
   that passing a stale `Scene` to `render` did not already have.** That sentence is the
   whole of the principle-6 argument, and it is why the alternative below was refused.
3. **Memory becomes the caller's decision, at the point where the knowledge lives.**
   0045 priced a device-side cache of a dozen encodes at a quarter of a gigabyte per
   page in the worst case and called it "a leak with a key", concluding that a device
   would have to retain exactly one. A handle has no such problem: a host retains the
   pages it wants to retain, reads `RetainedScene::retained_bytes()` for each, and drops
   them by dropping the handle. §11.5 puts the dozen-resident-pages posture on `Scene`;
   this is the same posture one layer down, and it needed no new policy.

### The alternative that was refused

**An explicit token — `render(scene, viewport, target, key: u64)` where the caller
promises to change `key` when the content changes.** It is smaller, it needs no new
type, and the caller could adopt it without restructuring anything. It was refused
because the promise is unverifiable and its failure mode is the exact outcome both
projects have a name for: a stale page under a title bar naming the new one. A retained
handle's failure mode — "the host did not rebuild its scene" — is the same failure mode
retained-mode graphics has always had, and it is visible in the host's own code as a
missing `set_scene` rather than invisible in a number.

## What makes an encode stale, and how each is detected

`encode` is a pure function of nine inputs — its nine parameters, and nothing else
reaches it. The scene is the handle's, by construction. The remaining eight are covered
by `EncodeKey`, compared by bits on every call: four of them directly, and the four that
are fixed for a device's life through the device's own identity.

| input | in the key as | why it moves |
|---|---|---|
| the scene | — the handle owns it | `set_scene` drops the encode outright |
| viewport size and affine | `width`, `height`, six `f32` bit patterns | every device bound, cull, clip rectangle and atlas key is a position in *these* pixels |
| the coverage lane | `coverage` | `Device::set_coverage` is a per-frame choice by design (ADR 0016), and the two lanes' bytes differ |
| the atlas layout | `atlas_generation` | a repack (ADR 0024) moves every tile, and the quad instances carry absolute texel origins |
| resident resources | `resource_generation` | a released id must never be drawn from a stale instance stream |
| the device | `device` | an encode names atlas positions and ids belonging to one device |
| frame budget, max dimension, glyph quantum, encode instrument | — | fixed for a device's life, so the `device` field covers them |

Two of those are new counters and both are as narrow as the hazard: the atlas already
bumped a `generation` on `reset` and nothing else moves an entry, because insertion
appends; `ResourceStore` gained one bumped by `release` and by nothing else, because
`upload_*` mints ids from a monotonic counter and can never make a retained id name
different bytes.

**The damage list and the target are deliberately not in the key.** `encode` never reads
`Viewport::damage` — damage is planned target-side (ADR 0012) — and phase 1 runs before
any allocation and knows no target. So a damaged frame and a frame into a different
texture both replay a full frame's encode, which is the case the caller's scroll path is
made of. `tests/retained_frame.rs` asserts both as *hits*, because an invalidation that
is too eager is a silent loss of the whole benefit and nothing else would notice.

## Refusals, which is where a replay could do real harm

A replay skips phase 1 and nothing else. Everything below that line runs on every call:
the viewport validation, the internal-texture budget, the target's own contract, the
passes, the submit. And phase 1's refusals cannot be masked either, because:

- **an encode is retained only after it succeeded**, so a refusing scene retains nothing
  and refuses identically on every attempt;
- **a failed encode leaves the handle holding nothing**, rather than an older encode: the
  key that was there did not match — that is why the encode ran — so what it held was
  already stale, and keeping it alive would be keeping the one thing this ADR exists to
  prevent;
- **every input a refusal could turn on is in the key.** The sharpest case is a released
  outline: without invalidation a replay would cheerfully draw a resource the device no
  longer has. With it, the frame re-encodes and `RenderError::UnknownOutline` names the
  id — which is what `Device::render` over the same scene does.
  `tests/retained_frame.rs::a_released_outline_re_encodes_and_the_refusal_stands` is that
  assertion.

`Frame::encode_source()` is the observable, and it exists rather than a comment about
`Timings::encode` being small: "small" is not a state a test can assert on, and every
test here turns on the enum rather than on a clock.

**And one thing a replay would otherwise have lied about.** ADR 0023's encode
subdivision is a clock *inside* the `Encoded`, so a retained encode carries the geometry
and staging totals of the frame that made it — real durations, possibly hundreds of
frames old. A replay reporting them would be a `Frame` claiming time it did not spend,
which is the one thing a `Frame` may never do. `EncodeClock::replayed` reports the three
rows at zero instead, and `a_replay_reports_no_geometry_and_no_staging` holds it. The
rows are kept rather than dropped so that a caller summing a trace does not have to
special-case a frame that omitted them.

## The one plumbing change the design needed

`Device::upload` used to *take* the coverage sheet and the GPU lane's winding sheet out
of the `Encoded` — which a frame that owns its encode can afford, and a frame that
replays one cannot. Both are borrowed now. ADR 0045's invalidation list named this
exactly, and it is the only line of the frame path that had to move.

The frame path also split along the seam the design is about: `Device::render` is now
phase 1 plus `draw_encoded`, and `render_retained` is a replay-or-encode plus the same
`draw_encoded`. **The two callers share every refusal by construction**, rather than by a
promise that two copies of the frame path stay in step.

## What it is worth, measured

`crates/quorra-gpu/examples/retained.rs`, and it is a better instrument than ADR 0045's
prototype in one respect that matters: both variants are in **one binary on one device**,
so the round-robin is two calls rather than two builds and there is no second target
directory to drift. Dense-text archetype at 1191×1684 — counters checked against
`tests/archetypes.rs`'s recorded row before any number was believed — headless on RADV
into a retained `Target::Texture`, 40 rounds round-robin, minima:

| `Device::render*`, dense text | wall | encode | upload | execute |
|---|---:|---:|---:|---:|
| `render` — re-encoded every frame | **1.107 ms** | 0.897–1.049 | 0.012 | 0.067–0.069 |
| `render_retained` — replayed | **0.174 ms** | **0.000** | 0.011–0.012 | 0.064–0.067 |

**A frame is 6.4× cheaper, and what is left is 0.174 ms**: the instance upload, the pass
and the submit. Three runs at load average 18–24, wall minima 1.107 / 1.260 / 1.225
against 0.174 / 0.175 / 0.187 — the spread on the replayed column is 7 %, which is a
floor doing its job under that much load.

**The ratio is smaller than ADR 0045's tenfold and the reason is on the other side of
it.** That prototype read 1.538 ms against 0.154, on a tree that did not yet have the
`(outline, linear)` bound memo — which is ADR 0045's own candidate (D), landed the same
day and worth 21.2 % of the encode. The numerator here is 1.107 rather than 1.538 because
the encode got faster; the denominator reproduces at 0.174 against 0.154 at six times the
load average. **The saving is the encode, and the encode is all of it.**

The same run on **llvmpipe**, where the rasteriser rather than the host dominates, is the
control: wall 5.072 → 3.295 ms with `execute` unmoved at 3.0–3.3 and `encode` going
1.324 → 0.000. Exactly the host-side term disappears and nothing else does, which is what
says the measurement is measuring what it claims to.

The pixels are compared as well as the clocks: the example renders one pair through
`Target::Readback` and reports how many bytes differ. **0 of 8 022 576**, on both
adapters. The handle held **287 688 bytes** for that page.

## Consequences

- An unchanged frame no longer walks its scene. On §6.2's page shape that is the
  difference between the success bar and the clear-win bar, and the remaining cost is
  the instance upload, the pass and the submit.
- **The caller has work to do, and it is not nothing**: their frame loop must stop
  rebuilding the frame scene when nothing changed. Their obstacle (a) — a background and
  overlays rebuilt with fresh `Arc`s every frame — is now theirs to solve by caching
  those, rather than ours to solve by inventing scene-fragment composition. ADR 0045's
  candidate (B) stays unbuilt and stays the answer if they cannot.
- **A page whose glyph tiles overflow the atlas never replays**, because the repack that
  follows such a frame (ADR 0024) invalidates its own encode. Correct, and a real limit:
  magnified text with many distinct letterforms is the shape that reaches it. Separately,
  a page refused for the coverage sheet retains nothing at all, so `HANDOVER.md`'s item 5
  gets nothing from this ADR either — for the other reason.
- `Counters` did not change, `Device::render` did not change, and `tests/archetypes.rs`
  passes untouched.

## What would overturn this

- **A caller who cannot stop rebuilding their frame scene.** Then the handle is held over
  a scene that changes every frame, every frame re-encodes, and the API has bought
  nothing but a type. That is candidate (B)'s case, and it is a design to do *with* them.
- **A host retaining a page whose encode is a quarter of a gigabyte.** `retained_bytes()`
  reports it and nothing here refuses it: the budget on a retained encode is the caller's,
  because the count of resident pages is theirs. If that turns out to be the wrong place
  for the decision, the fix is a device-side ceiling with an `Err` — not a silent drop,
  which would make a replay's absence invisible.
