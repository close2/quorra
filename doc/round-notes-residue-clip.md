# Round notes — the residue clip region (ADR 0049)

The exact edits this round asks for in `doc/PLAN.md` and `doc/HANDOVER.md`, written out
rather than applied: two sibling agents are working in parallel and the owner merges those
two files by hand. Everything else this round touches is already in the tree.

Date: 2026-08-15. Commits: the border cut, the region, and the documents.

---

## `doc/PLAN.md`

### 1. "The numbers that stand" — the artwork row

The existing row was measured by `surface_measure` on the real display and has **not** been
re-run there since; it should keep saying so rather than be quietly overwritten with a
headless number from a different instrument. Replace

```
| artwork — the corpus's p99 clip shape — steady | 43.3 ms, geometry 35.4 of it | `surface_measure`, same run |
```

with

```
| artwork — the corpus's p99 clip shape — steady | 43.3 ms, geometry 35.4 of it | `surface_measure`, RADV at the real display, 2026-08-14 — **before ADR 0049**, and not re-run on the display since |
| — the same page's encode, before → after ADR 0049 | geometry **37.8 → 28.9 ms**, encode 46.3 → 37.2 | `examples/residue_clip.rs`, headless RADV into a texture, three alternating rounds, minima, load 3.8–4.8, 2026-08-15 |
```

### 1b. "The numbers that stand" — the two corpus rows

Both were re-run this round in one copy of their tree, base and change in the same hour.
Scale 1 moves by one page, toward the oracle; scale 4 does not move at all. Replace

```
| the caller's corpus at scale 1 | **934** agree / 20 differ / 2 refused / 18 not comparable | their tree, one copy, 2026-08-14 |
| the caller's corpus at scale 4 | **936** / 10 / 5 / 23 | same |
```

with

```
| the caller's corpus at scale 1 | **931** agree / 23 differ / 2 refused / 18 not comparable | their tree, one copy, 2026-08-15 (ADR 0049; `issue2177.pdf` stops differing) |
| the caller's corpus at scale 4 | **936** / 10 / 5 / 23 | same run, unmoved |
```

**and note the trap while doing it**: the 2026-08-14 row read 934/20/2/18 and the base
commit re-run on 2026-08-15 in a fresh copy reads **930/24/2/18** for the same quorra
commit. Nothing regressed between those two dates in *this* tree — their tree moved, which
is exactly what `HANDOVER.md` says a count quoted from an older run is worth. The
before/after pair above is what the change is worth; the 934 is not a baseline it can be
compared against.

### 2. "What is still open" — the first bullet

The bullet says the artwork row is "35 ms of re-rasterising the same clip coverage every
frame". **That reads the whole geometry phase as though it were all residue, and it is
about a quarter of it**: a temporary probe through `tests/archetypes.rs` measured the
residue span at 17.3 ms of the 65.6 ms artwork spends flattening and rasterising. Replace
the bullet with

```
- **The residue-clip seam, half taken.** The residue itself is now rasterised once per
  chain rather than once per clipped command (ADR 0049): artwork's encode geometry is
  37.8 → 28.9 ms and its 600 residue rasterisations are 185. What is *not* taken is the
  reason two pages at 4× refuse with `ScratchExhausted` — that is the coverage **sheet**,
  one tile per clipped command, and ADR 0049 leaves `Counters::tiles` unchanged on every
  archetype on purpose. `HANDOVER.md` item 2 holds what is left, and it is tiling work.
```

**And the refusal count in that bullet needs correcting while it is open.** It says three
pages at 4× and one at scale 1 refuse with `ScratchExhausted`. Today's copy of their tree,
both scales, base and change alike: **two** at 4× (`bug1703683_page2_reduced.pdf` and
`issue1905.pdf`) and **none** at scale 1. The other refusals are a different budget each —
`22060_A1_01_Plans.pdf` on `max_resource_bytes` at 4×, and `bug1721218_reduced.pdf` and
`issue18032.pdf` on two clause refusals at both scales — so "the only reason any frame of
the corpus is refused" was already too strong.

### 3. §1.4, "Clips are mostly rectangles, and the design says so"

The paragraph promises a residue cache that did not exist; it does now, and it is keyed
differently from what that sentence says. Replace

> Where a residue mask is built, it is cached **keyed by the resolved region under the
> current viewport, never by an identifier** — […] — and `Counters` reports the count of
> distinct regions, not a hit rate.

with

```
Where a residue mask is built, the chain is rasterised **once over the region it
occupies** and every mark takes a window on it (ADR 0049). The key is the chain's
deepest non-rectangular link, which under one viewport *determines* the region — the
transform is fixed for the frame — so the failure the identifier rule warns about
cannot happen here: two commands under one chain cannot get two different masks. What a
chain-identity key cannot do is *unify* two chains that happen to resolve to the same
region, and `Counters::clip_residue_regions` would show that honestly, as two regions.
`Counters` reports the count of distinct regions and the count of chains that paid per
tile instead — keys, never a hit rate.
```

### 4. §1.5, "Memory that grows" — after the third bullet

Add, as a sub-point of "Every allocation derived from scene content":

```
  A cache is the one place where the answer is *decline* rather than `Err`: the residue
  regions of ADR 0049 are checked against a quarter of `max_frame_bytes` before
  allocation, and a region that does not fit is not built — the frame draws it per tile
  instead, which is what every frame did before. Refusing a drawable frame because a
  cache filled up would be principle 6 pointed the wrong way, and the atlas has said so
  since ADR 0029.
```

---

## `doc/HANDOVER.md`

### 1. "What to do next" — item 2

Its heading and first paragraph are now half done. Suggested replacement for the whole
item, keeping its number since three documents cite it:

```
### 2. A page-sized coverage tile per clipped shape — **not** multi-sheet passes

*(This was **item 5** while three finished items still stood above it, and ADR 0048 and
`doc/feedback-answers-draft.md` both cite it under that number.)*

Two pages at 4× refuse with `ScratchExhausted` — `bug1703683_page2_reduced.pdf` and
`issue1905.pdf`, the coverage sheet against the adapter's 16 384 limit, a different ceiling
from the frame budget. **It is the only *budget* that refuses a frame we could otherwise
draw.** The other three refusals of their corpus are each something else: 548 MB of
resident images against `max_resource_bytes` (`22060_A1_01_Plans.pdf`, at upload rather
than at the frame), and two clause refusals that are correct — a four-component blending
colour space and a non-isolated knockout group. Counted on 2026-08-15; the earlier "three
at 4× and one at scale 1" was their tree at an earlier revision.

This item used to read "a frame would have to use more than one sheet". **That was measured
and refused** — the numbers are in `doc/history/`; a second sheet takes `bug1721218_reduced`
past the byte budget and lets the other two draw at a quarter of a gigabyte of per-frame
coverage upload each.

**ADR 0049 took the other half of this item and left this one untouched, deliberately.**
The residue is no longer re-rasterised per command — that was 17.3 ms of artwork's 65.6 ms
of geometry, and artwork's encode geometry went 37.8 → 28.9 ms — but a *region* is host
memory that never reaches the sheet, and `Counters::tiles` is unchanged on every archetype,
which is the evidence that no refusal moved. What is left is the tiling side: a clipped
shape still becomes one coverage tile of its own device bounds, and at 4× a full-page
clipped shape is a full-page tile. ADR 0028's panes are the nearest existing mechanism.

One more shape belongs to this seam since ADR 0048: a page whose glyph tiles overflow the
atlas re-encodes on every frame, because the repack that follows bumps the atlas generation
and invalidates its own retained encode. Magnified text is that shape.
```

### 2. "Instruments" — add one

```
- **A page of curve-clipped marks**: `examples/residue_clip.rs` — the artwork archetype,
  headless into a texture, `instrument_encode` on, minima of twenty steady frames with the
  first reported apart and the load average printed beside them. It prints
  `clip_residue_regions` and `clip_residue_tiles` with the clocks, which is what makes two
  runs on a loaded machine comparable at all: the counters are exact functions of the
  scene. Two builds of it cannot be round-robined inside one process, so the A/B is
  `git checkout` between HEAD and the change, three rounds each, alternating.
```

### 3. "Traps" — add one

```
**A tile is not a window on a wider region unless the rasteriser cuts at its border.**
`fill_mask` clamped the endpoints of an edge piece that left the region and interpolated
between them: the row's total winding survived, so every column past the crossing read the
right value and no test could see it, while the columns *at* the border took the height the
piece spent outside. 2 684 pixels of a 2.9-million-pixel probe, the worst by 185 of 255.
Any change that wants to compute coverage once and cut it up afterwards has to check this
first — the probe is `a_tile_is_the_crop_of_the_region_that_contains_it`, and it took
ninety seconds to write and settled the design of a whole round (ADR 0049).
```

### 4. "Recorded and deliberately not taken" — add two

```
- a residue region is the chain's own bounds and not the union of the tiles that ask for
  it; the union needs a second pass over the commands before the first ask, which is the
  two-pass encode ADR 0034 declined for the tiling (0049);
- residue regions are not pooled across frames: the retained encode already answers "the
  same page again", and a cache that outlived its scene is ADR 0029 §3's rejected memory
  in a second place (0049).
```
