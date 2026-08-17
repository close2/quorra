# The tiling bound — what was built, what it moved, and what it uncovered

Round notes for `HANDOVER.md`'s item 2, written 2026-08-17. The measurement that chose
the candidate is `doc/notes-tiling-ceiling.md`; the decision is ADR 0057. This file is
what building it found, including one thing nobody was looking for.

**The short version.**

- A clipped mark's coverage tile is now bounded by its chain's own device box, taken from
  the links' control hulls at the moment the chain is resolved. It costs no flattening, no
  second pass, and nothing at all on a page with no residue clip.
- `bug1703683_page2_reduced.pdf` goes from **refused to agreeing with the oracle** at 4×.
  No page line of 956 moves at scale 1 in either lane, and at scale 4 one moves besides it
  — `inks.pdf` on the GPU lane, by a hundred-thousandth of SSIM with its mean, its worst
  tile and its differing fraction unchanged.
- Two instrument debts are paid: `Counters::coverage` prices a drawn frame's sheet and
  `RenderError::ScratchExhausted` names the sheet a refused frame met.
- **And the change made a fixture defect visible.** `tests/archetypes.rs`'s two
  curve-clipped pages place their clips on one grid and their marks on another: **0 of 40**
  and **8 of 600** of the clipped marks overlap the clip that clips them. The 632 that do
  not were rasterising a tile each and multiplying it by zero, which is why the row read as
  though the residue lane were exercised. It never was.

---

## 1. What was built

### 1.1 The bound

`ResolvedClip` carries a third field, `residue_bounds`, and `ResolvedClip::mark_bounds()`
is `rect ∩ residue_bounds`. It is computed in `ClipResolver::resolve`, in the arm that
already decided the link is not a rectangle, from `encode/hull::HullMemo` — the same memo
ADR 0045 built for marks, keyed by `(outline, linear part)`, which a page's clips share
with its fills.

Three properties, and each is why the *hull* was taken rather than the flattened outline:

| | |
|---|---|
| flattening added | **none** — the box comes from control points the memo already holds |
| passes over the commands | **one**, unchanged |
| what a page with no residue clip pays | **nothing** — the line is inside the `None` arm of the rectangle test |
| direction of the error | **outward** (convex-hull property of Béziers), which is the only safe one |

The three sites that size a rasterised tile take it: `encode/coverage.rs::visible_tile`
(which `coverage_tile` now calls, so the two lanes cannot come apart — the ten lines of
duplicated arithmetic `doc/notes-encode-split.md` §5 named are gone), `encode/rare.rs`'s
image lane, and `encode/layer.rs::plan_group_residue`, where a group under one curve clip
was rasterising a page-sized mask.

`encode/device_space.rs`'s cull was **left on `rect`**: culling by `mark_bounds` would be
correct and would drop more commands, but `commands_culled` is a caller-visible number.

### 1.2 The two instruments

`CoverageSheet { tiles, texels, width, height }`, reported in both states —
`Counters::coverage` on a drawn frame, and inside
`RenderError::ScratchExhausted { limit, sheet, tile_width, tile_height }` on a refused one.
Both are additive: no existing field changes type, name or meaning.

`ScratchPacker::reserve` now charges `placed` and `tile_area` only on success, so the sheet
a refusal reports is what the frame *placed* rather than what it asked for. The
candidate's width still raises the shelf target for its own placement and its area is
still in the `√(2A)` sum it is measured against, so **no drawn frame's packing moves by a
texel** — which the corpus's per-page lines then confirm.

---

## 2. The caller's corpus

One copy of their tree under `/home/AI/corpus-tiling/viewer`, all eight runs inside it,
flipping only the `[patch]` path between a `git worktree` at `eada81e` and this one. Both
halves of each pair the same day, because that tree moves under us.

| lane, scale | base | with the bound |
|---|---|---|
| CPU, scale 1 | 931 agree / 23 differ / 2 refused / 18 not comparable | **931 / 23 / 2 / 18** |
| CPU, scale 4 | 936 / 11 / 4 / 23 | **937** / 11 / **3** / 23 |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| GPU, scale 4 | 937 / 10 / 4 / 23 | **938** / 10 / **3** / 23 |

**No page line of 956 moves at scale 1, in either lane** — all 25 and 27 printed lines are
identical to the character. At scale 4 exactly what
`doc/notes-tiling-ceiling.md` predicted happens and nothing else:

| page | lane, scale | base | with the bound |
|---|---|---|---|
| `bug1703683_page2_reduced.pdf` | CPU 4 and GPU 4 | refused, `ScratchExhausted` | **agrees with the oracle** |
| `issue1905.pdf` | CPU 4 and GPU 4 | refused, naming only the wall | refused, **naming the sheet**: a 4 763 × 7 103 tile against a sheet at 14 289 × 15 117 holding 6 tiles and 213 115 672 texels |
| `inks.pdf` | GPU 4 only | mean 0.0394, worst 17.29, differing 0.0012, SSIM 0.99861 | the same to the digit, SSIM **0.99862** |

Every other differing page's mean, worst tile, differing fraction and SSIM is identical to
the last digit at both scales in both lanes. `inks.pdf`'s fifth decimal is the 1-of-255
`fill_mask` residual ADR 0049 recorded and priced: a tile is asked for a different
rectangle and `f32` addition is not associative. Its mean, its worst tile and its differing
fraction do not move at all, and it does not move on the CPU lane.

**And the refusal that stays is now its own evidence.** `issue1905.pdf`'s message reports
the sheet the patched crate had to be built to read in `doc/notes-tiling-ceiling.md` §1 —
6 tiles seated and 213 115 672 texels, against a table that was obtained with `eprintln!`s
and read exactly 213 115 672.

**Their `REFUSED` ratchet fails, loudly, and that is the result rather than a problem.**
The gate holds the scale-4 refusal list to equality and prints both:

```
assertion `left == right` failed: the pages quorra refuses at 4× have changed
  left: ["bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
 right: ["bug1703683_page2_reduced.pdf", "bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
```

**The caller must drop `bug1703683_page2_reduced.pdf` from that list** when they take the
bump.

Wall clocks are context and not evidence here, but the scale-4 CPU sweep of the same corpus
in the same copy went **455.7 s to 247.3 s** and the GPU one 268.4 s to 343.6 s, on a
machine at load average 9–26. The first is the page that stopped rasterising a gigabyte;
the second is the machine. Neither is a claim.

---

## 3. What the round uncovered: the archetypes' clips clip nothing

`tests/archetypes.rs` lost 632 coverage tiles, and the reason is not the bound.

`define_clips` places clip `j` at `position(j, side × 6)` and gives it an outline about
`side` across; `emit` places mark `i` at `position(i, side)` and hands it
`clips[i % clips.len()]`. The two grids have different steps, so a clip's box and its
marks' boxes coincide only by accident. Counting the boxes from the generator's own
arithmetic, independently of the crate:

| archetype | clipped commands | whose mark box meets its clip's box |
|---|---:|---:|
| dense text | 40 | **0** |
| artwork | 600 | **8** |

— which is exactly the `tiles` each row now reports. Before the bound, a mark whose chain
admits nothing still got a mark-sized tile: rasterised, packed, uploaded, and multiplied
by a residue of zero. So the rows read **40 tiles / 2 residue regions** and **600 tiles /
185 residue regions** for pages that mark almost nothing under a clip.

Three consequences, stated rather than absorbed:

- **The archetype signature no longer gates the residue lane.** `tests/tiling_ceiling.rs`
  holds that property instead — 64 marks under clips that *do* overlap them, with
  `tiles == 64` asserted on both legs so the gate cannot pass by drawing nothing.
- **ADR 0049's artwork measurement was taken on this page.** `examples/residue_clip.rs`
  copies the archetype, so the 37.78 → 28.89 ms of geometry it recorded was largely the
  removal of repeated rasterisation of *empty* tiles. The mechanism ADR 0049 built is
  unchanged and the saving was real; what the fixture demonstrated is narrower than the row
  implied, and the artwork row's "185 residue regions against 600 tiles" was 185 regions
  serving eight marks.
- **A fixture round is owed** and is the first thing anybody measuring this lane must do.
  The shape wanted is a real `q W n`: a curve clip *larger than the marks under it and
  smaller than the page*, because a page-sized clip is refused by ADR 0049's admission rule
  and a mark-sized one exercises nothing.

  **Done, 2026-08-17 — `doc/notes-clipped-instrument.md`.** A curve clip is now cut around
  the run of three or four consecutive marks that draw under it, in `tests/archetypes.rs`
  and in all three examples that copy that page. All 600 of artwork's clipped commands and
  all 40 of dense text's now meet the clip that clips them, and
  `a_curve_clip_clips_the_marks_that_draw_under_it` fails if that ever stops being true —
  from the generator's arithmetic *and* from `tiles == clipped`. The rows below are
  superseded by that round's and are not comparable with them.

The archetype baseline as this round left it, with the tenth column (`coverage.texels`)
ADR 0057 added. **Two of these rows were re-taken on 2026-08-17 when the fixture was
re-cut** — dense text now reads `[4320, 0, 818, 2164, 1, 40, 0, 0, 40, 8956]` and artwork
`[684, 0, 300, 300, 1, 600, 3, 66, 384, 3542360]` — and neither pair is comparable with
the other, because the page changed rather than the library:

| archetype | signature |
|---|---|
| median page | `[12, 0, 9, 12, 0, 0, 0, 0, 0, 0]` |
| dense text | `[4320, 0, 818, 2164, 1, 0, 0, 0, 0, 0]` |
| artwork | `[684, 0, 300, 300, 1, 8, 3, 2, 6, 12284]` |
| image page | `[232, 0, 60, 158, 4, 0, 0, 0, 0, 0]` |
| clip mountain | `[1200, 0, 200, 800, 1200, 0, 0, 0, 0, 0]` |
| giant | `[1500, 0, 1500, 1500, 0, 0, 0, 0, 0, 0]` |
| drawing | `[1200, 0, 1200, 1194, 0, 6, 0, 0, 0, 245]` |

The signature is now `[u64; 10]` and the gate **measures every archetype before judging
any of them**: a loop that asserts as it goes reports the first row that moved and hides
the rest, which is the wrong shape for a signature.

---

## 4. The gates, and the defect each was made to fail for

Three in `crates/quorra-gpu/tests/tiling_ceiling.rs`, each verified able to fail by forcing
the defect it exists to catch and watching it go red:

| gate | forced defect | what it did |
|---|---|---|
| `a_residue_clip_bounds_the_tile_its_mark_asks_for` | `visible_tile` reads `resolved.rect` instead of `mark_bounds()` | the curved leg is refused by the frame budget at 2 037 760 bytes instead of drawing |
| `a_bounded_tile_draws_every_pixel_the_chain_admits` | `hull_box` returns its box inset by 8 device pixels on both minima | the worst channel differs by **255 of 255** from the control |
| `a_frame_is_refused_for_the_sheets_height_with_its_bytes_untouched` | `ScratchPacker::exhausted` reports `CoverageSheet::default()` | the sheet reads `0x0 holding 0 tiles and 0 texels` against a refusal that placed seven |

**Two things the first gate needed that are worth writing down**, because both took a
round-trip to find:

- **its rectangular control leg placed no tile at all** on the default atlas. Sixty-four
  placements of one 800 × 800 outline are exactly what the glyph lane exists to take off
  the sheet, so the two legs were being compared through different lanes. The gate now
  uses a device with a 64 KiB atlas, which holds nothing the file uploads.
- **the file's `blob` is not a disc.** Its cubics' control points make a self-overlapping
  loop whose winding cancels at its own centre, so it is a band. The fidelity gate needs a
  mark that covers every pixel with full coverage — `(255·c + 127) / 255 = c` exactly is
  what makes its expected picture derivable rather than stored — so it uses a *rectangle*
  four times the target, which under a residue clip cannot take ADR 0007's analytic lane
  and so rasterises through the same `coverage_tile` as every other clipped mark.

---

## 5. What was found and deliberately not done

- **`issue1905.pdf` at 4× still refuses, and correctly.** Seven fills each wider than the
  page, under a rectangular clip that already bounds them: 1 339 315 879 texels, no residue
  clip anywhere. Nothing on the tiling side draws that inside a 256 MiB budget. The
  question to ask the caller first is `doc/notes-tiling-ceiling.md` §5's: whether it
  refuses in the product or only in the gate, since the frame that refuses is a whole page
  at 4× in one target and a viewer's viewport is its window.
- **The archetype fixture was not repaired.** It is a shared instrument — `perf_gate`,
  `residue_clip`, `encode_threads` and four ADRs read numbers off it — and re-cutting its
  clip placement inside a round about tiling would change what every one of those means,
  in a round where two other agents are merging. Written down instead, with the count that
  proves it. **(Taken on 2026-08-17, with exactly that blast radius handled one reader at a
  time: `doc/notes-clipped-instrument.md` §4.)**
- **`commands_culled` does not count a mark whose chain admits nothing.** The bound now
  drops those marks from the sheet, so they cost no coverage; what is missing is the
  *count*, and moving a caller-visible number is its own decision.
- **The cull still tests `rect`.** Same reason.
- **Nothing was done about the sheet's own ceiling.** `doc/notes-tiling-ceiling.md` §4
  priced a pane cut, a second sheet and a tighter packer and declined all three with
  numbers; this round adds nothing to that and re-proposing one means reading §4 first.
- **No `Scene::cost()` field for the sheet**, and there cannot be one: its height is a
  function of the viewport, which a `Scene` does not have. The error type says so.
