# 0050 — A repack that cannot change the layout is not taken

Date: 2026-08-15. Status: accepted, and built. Closes the shape `doc/HANDOVER.md` item 2
records under ADR 0048's last consequence, and audits the second half of ADR 0048's key
while it is open.

## The situation, as it was recorded and as it turned out to be

ADR 0048 built `RetainedScene`: a caller keeps a handle, an unchanged frame replays its
last encode instead of walking the scene again, and the dense-text archetype goes from
1.107 ms to 0.174. Its `EncodeKey` includes `atlas_generation`, because the retained quad
instances carry **absolute texel origins into the atlas sheet** and a repack moves every
one of them.

That ADR's own consequences, and `HANDOVER.md` after it, state the limit this way:

> A page whose glyph tiles overflow the atlas never replays, because the repack that
> follows such a frame (ADR 0024) invalidates its own encode. Magnified text with many
> distinct letterforms is the shape that reaches it.

**Measured, the sentence is too broad, and the true shape is narrower and worse.**
`examples/retained.rs`'s new overflow section and a throwaway sweep across seventeen
(atlas budget, magnification) pairs — twelve retained frames each, `E` for a frame that
encoded and `.` for one that replayed, llvmpipe, `Options::default()` otherwise:

| page | atlas | zoom | distinct keys | resident | scratch tiles | before |
|---|---:|---:|---:|---:|---:|---|
| dense, 107 outlines | 8 MiB | 1× … 20× | 774 → 22 | all | 0 | `E...........` |
| dense, 107 outlines | 64 KiB | 1× | 774 | 563 | 1 262 | `E...........` |
| dense, 107 outlines | 16 / 8 / 4 KiB | 1× | 774 | 146 / 74 / 40 | 4 595 … 5 535 | `E...........` |
| dense, 107 outlines | 64 KiB | 2×, 3× | 214, 284 | 147, 67 | 605, 761 | `E...........` |
| **dense, 107 outlines** | **256 KiB** | **4×** | **107** | **102** | **21** | **`EEEEEEEEEEEE`** |
| dense, 800 outlines | 64 KiB … 1 MiB | 1× | 1 600 | 536 → 1 600 | 3 789 → 0 | `E...........` |

Sixteen of the seventeen already settled after one encode. **One did not, and never
would**: twelve frames of a page nobody was touching, twelve full encodes, and no counter
anywhere saying why.

So overflow is not the condition. The condition is a *band*:

- a working set **far larger** than the atlas never repacked even before this ADR — ADR
  0024's byte test (`atlas_requested_bytes <= atlas_bytes`) blocks it, which is the case
  that ADR measured at 6.0 ms against 0.6;
- a working set that **fits** never overflows at all;
- the band between them is a page that fits the atlas **by bytes** and does not fit it
  **by shelves**. The byte test compares two areas; the packer divides twice — tiles
  across a shelf, shelves down the sheet — and wastes both remainders. The row above asks
  for 165 596 texels of a 262 144-texel atlas, **63 % of it**, and five of its 107 tiles
  do not fit.

Inside that band the old rule fired on every frame, and each firing invalidated the
encode of the frame that caused it. **An oscillating atlas is a retained encode that never
hits**, and this is the frame shape where the encode is most worth having.

## Three candidates, and why the obvious one is not merely insufficient but wrong

### (a) Key the encode on the generation it *finished* with

The key is built before the walk (`EncodeKey::new`, called from `render_retained`), and a
repack happens after the frame; so the encode is stored under a generation that a later
`settle_atlas` immediately supersedes. Keying on the generation the encode ended with
looks like the fix.

**It is a no-op today and a hazard tomorrow.** A no-op, because there is exactly one
caller of `AtlasStore::reset` — `Device::settle_atlas`, after `draw_encoded` — so no reset
can occur *during* an encode and the two generations are always equal. A hazard, because
if one ever could, the encode that spanned it would be broken in its middle: the tiles
inserted before the reset are gone and the origins naming them are stale, whatever the key
says. Keying on the *finishing* generation would store that encode under a valid-looking
key and replay it — a plausible wrong page, which is principle 6's worst outcome. Keying
on the generation read **before** the walk is the conservative order: an encode that
spanned a reset is stored under a stale key and can only ever be discarded.

So the pre-encode key stays, and the invariant it rests on is now written beside
`AtlasStore::reset` rather than inferred from a call graph.

### (b) Predict packability instead of comparing bytes

Replace `atlas_requested_bytes <= atlas_bytes` with a simulation of the packer over the
frame's distinct tile sizes. It is exact, and it is a second implementation of the packer
that has to stay in step with the first, run over up to a few thousand tiles, on every
frame with pressure. Refused: the *outcome* can be observed for nothing.

### (c) Take the repack only when it can reclaim something — **chosen**

A repack does exactly one thing: it gives back the space held by entries the frame is not
using. Everything else it does, it does to itself. Concretely:

> **If the atlas holds nothing but entries this frame reached, a repack reproduces the
> layout it replaced — tile for tile, including the tile that overflows.**

The argument is short enough to check. On a repack, the next frame packs into an empty
atlas by calling `AtlasStore::insert` once per distinct key in the scene's encounter
order. Those are the same keys in the same order that built the current layout, because
insertion appends and never moves, and the frame walks its scene in one fixed order. The
packer is a pure function of that sequence. So the sequence of `allocate` calls is
identical, every shelf is opened at the same `y` with the same height, every cursor
advances by the same width, and the same call returns `None` at the end. The repack is a
no-op that costs a generation bump — and the generation bump costs every retained encode
keyed on that layout.

## The decision

`Device::settle_atlas` gains a third condition and returns whether it fired:

```rust
let repack = encoded.atlas_pressure                               // a tile went to scratch
    && encoded.atlas_requested_bytes <= atlas_bytes               // ADR 0024's byte test
    && resident > encoded.atlas_entries_used;                     // there is something to reclaim
```

`Encoded::atlas_entries_used` is counted in `push_glyph`, once per distinct key that
reached an entry — a resident hit or a fresh insert. The atlas holds at least that many
entries when the frame ends, because insertion never removes one; anything above it
belongs to an earlier frame. The difference is exact, it is `O(1)` to test, it needs no
state carried between frames, and it needs no second copy of the packer.

The bound this puts on a page is: **at most one repack, at most two encodes, and replays
for ever after.** The first frame pays for whatever atlas it inherited, the second pays
for the layout that replaced it, and nothing else pays at all. A fresh device skips even
the first — there is nothing foreign in an empty atlas, so a page that will never fit
encodes exactly once.

### Two counters, because a fix that assumes the pathology away is not an instrument

`Counters` gains:

- **`atlas_working_set_bytes`** — the bytes this frame's distinct keys asked for, hits
  included. `atlas_distinct_keys` already says how many things a page repeats; this says
  what holding all of them would cost, which is the number `Options::atlas_budget` has to
  be compared against and the only one that separates "the atlas is too small for this
  page" from "the atlas is holding another page".
- **`atlas_repacked`** — whether the atlas was repacked after this frame. This is the
  oscillation counter. A page that settles reports it true on at most one frame; the
  pathology above would report it true on every frame, for ever. It is reported on the
  frame that *caused* the repack rather than on the one that pays for it, because that is
  the frame whose layout stopped being the layout.

Both are per-frame, like everything else in `Counters`. A cumulative repack count would
have been a device number wearing a frame's clothes, and §8's contract is that a `Frame`
says what *this* frame did.

## The audit of `resource_generation`, which was the other half of ADR 0048's key

`EncodeKey`'s rustdoc claimed:

> An upload cannot invalidate anything: ids are minted from a monotonic counter, so a
> retained encode's ids cannot come to name other bytes.

`ResourceStore::allocate_id` was `self.next_id.wrapping_add(1)` over a `u32` shared by the
four resource families. So the counter was not monotonic; it was monotonic for the first
`2³²` uploads. After that an id is reissued, `HashMap::insert` silently replaces whatever
still holds it, `generation` does not move — it counts *releases* — and a retained encode
naming that id draws the new resource through the old instances while every field of
`EncodeKey` compares equal. A wrong page, reached by arithmetic nobody had bounded.

Four billion uploads is not a document, and this ADR does not claim it was reachable. It
claims that "unreachable" was never written down, was not what the code said, and cost
nothing to make true: `allocate_id` now refuses with
`DeviceError::ResourceIdsExhausted { limit }`, and is called before the budget is charged
so that a refusal charges nothing. `identifiers_run_out_loudly_rather_than_wrapping` winds
the counter to its last value and holds it.

## What it is worth, and how that was established

**Not by a clock.** The instrument is the sequence of encode sources, which is the same on
an idle machine and on this one at load average 90. The seventeen-configuration sweep
above, re-run against the built change, differs in exactly one row and in nothing else:

| page | atlas | zoom | before | after |
|---|---:|---:|---|---|
| dense, 107 outlines | 256 KiB | 4× | `EEEEEEEEEEEE` | **`E...........`** |
| every other configuration | — | — | `E...........` | `E...........` |

The counters of that row are identical across the change — 107 distinct keys, 102
resident, 21 tiles on the scratch sheet — which is what says the page is drawn the same
way and only the repack decision moved. What it is worth per frame is ADR 0048's number,
because the frames that stopped encoding are ADR 0048's frames: **1.107 ms → 0.174** on
the archetype, now reachable on a magnified page instead of only on an unmagnified one.

`examples/retained.rs` carries the section permanently, with an assertion that the page it
runs is still inside the band — a fixture that silently leaves the band would go on
passing, since a page that never overflows replays trivially.

### What was tested

`tests/retained_frame.rs`, three tests around a 96×96 atlas and a page of 34 tiles of
which 30 fit:

- `a_page_the_atlas_cannot_hold_replays_after_its_first_frame` — the headline: frame 1
  encodes, frames 2–5 replay, none repacks, and every one of them is **byte-identical**
  to a frame encoded from scratch against the same atlas afterwards;
- `an_atlas_holding_another_page_repacks_once_and_then_settles` — the counter: a foreign
  page first, then one repack, then one re-encode, then six replays with `atlas_repacked`
  false throughout;
- `an_atlas_repack_re_encodes` (ADR 0048's, unchanged and still passing) — the negative
  case: an atlas that *does* move still invalidates every encode keyed on it.

Both new tests assert their own precondition — a tile on the scratch sheet, and a working
set inside the atlas by bytes — because a fixture that stops reproducing its condition is
a test that proves nothing.

## What it does not do

- **No recency, still.** ADR 0024 deferred it and this ADR does not take it. What has
  changed is which page reaches the question: a single page too large for its atlas is now
  stable, so the remaining case is genuine thrash — two pages alternating, each fitting
  alone and neither fitting beside the other. Each frame then finds the other's tiles
  foreign and repacks, which is one repack per frame and two encodes per page. That is
  the *same* cost as before this ADR for that access pattern, it is a real cost, and
  `atlas_repacked` now names it instead of leaving it to be inferred. Recency is what
  answers it, and it wants its own measurement.
- **It does not make an overflowing page cheap.** The tiles that do not fit are
  rasterised into the scratch sheet on every frame that *encodes*; what this ADR buys is
  that a still page encodes once instead of for ever. A page that keeps changing keeps
  paying, and `atlas_working_set_bytes` against `Options::atlas_budget` is how a host
  decides to stop it.
- **It changes no pixel.** A tile drawn through the atlas and the same tile drawn through
  the scratch sheet are the same bytes — one rasteriser feeds both paths, and ADR 0024's
  `the_two_paths_draw_the_same_pixels` is the standing assertion. What moved is when a
  layout is thrown away, and determinism (§4.6) is untouched: the same scene at the same
  viewport on a device in the same state produces the same atlas layout, and the repack
  decision is a function of that state and this frame rather than of a history.

## Revisit when

`Counters::atlas_repacked` is true on frame after frame of a real workload. That is the
thrash ADR 0024 named, it is now visible, and the answer to it is recency rather than a
fourth condition here.
