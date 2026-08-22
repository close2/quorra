# Plan

The brief is `RENDER_LIBRARY.md`; this file is the design in its current state of
belief, the order of work, and the state of both. Bare section numbers (§) are the
brief's; "clause" numbers are ISO 32000-2's. **Picking the work up cold: `HANDOVER.md`
has what to do next and the traps that cost a round each.**

The file has two parts. **Part 1 says how the library will work** — the architecture as
believed, with each piece naming the measurement that could overturn it, because §11 is
explicit that the design should turn on measurements rather than on anyone's taste. Most
of those measurements have since been taken, and each piece carries what came back.
**Part 2 says the steps that got there** — nine milestones, each with its work, its exit
gates, and the question it settles; all nine are done. When a Part 1 hypothesis is
confirmed or overturned, the decision becomes an ADR and this file is corrected in the
same commit; a plan that disagrees with the tree is worse than no plan.

## Where we are

Nine milestones are done, the swap landed on 2026-08-03, and the caller consumes this
library from https://github.com/close2/quorra, pinned by their `Cargo.lock`.

**This section says what is true today and is rewritten freely. What happened is
`doc/history/` — one file per round, newest last, never edited after the fact — and why
each decision was taken is `doc/adr/`. What to do next is `HANDOVER.md`.**

### The numbers that stand

Everything below is a *minimum*, because this machine is somebody's desktop and its load
average is not a constant (see `HANDOVER.md`'s traps). The instrument is named beside each
row, since a number without one is not evidence.

| | | measured on |
|---|---:|---|
| dense text, presenting, steady — §6.2's bar is 2.0 ms | **1.420 ms** `wall − acquire` | `examples/surface_measure.rs`, RADV at the real display, **2026-08-17**, load 3.09, minima of 80 frames over 2 rounds, `MESA_SHADER_CACHE_DISABLE` |
| — encode | 1.020 ms | same |
| — recording, which is still most of that encode | 0.926 ms of a 1.179 ms *instrumented* encode | same; the instrument costs a clock read per seam (ADR 0023) |
| — geometry | 0.206 ms | same — **up from 0.161 on the pre-re-cut page**, which is the 40 residue-clipped tiles that page now actually rasterises (ADR 0057 found the clips met none of their marks) |
| — execute: the GPU is about 5 % of the frame | 0.073 ms | same |
| the same frame, unchanged, replayed rather than encoded | **0.174 ms** against 1.107 | `examples/retained.rs`, headless RADV, ADR 0048 |
| — and now also when the page's glyph tiles overflow the atlas | 1 encode per page, not 1 per frame | `examples/retained.rs`'s overflow section, ADR 0050 |
| artwork — the corpus's p99 clip shape — steady | **45.765 ms**, geometry **36.013** of it, staging 2.631, recording 2.870 | `surface_measure`, RADV at the real display, **2026-08-17** — the re-run the previous row was owed, and the first taken on a page whose curve clips meet the marks they clip. **Not comparable with the 43.3 ms it replaces**: that was a different page. Recording is 6.9 % of this encode, which is what ADR 0023's amendment looks like once the residue multiply is counted as geometry |
| — the same page's encode, before → after ADR 0049 | geometry **37.8 → 28.9 ms**, encode 46.3 → 37.2 | `examples/residue_clip.rs`, 2026-08-15 — **a number about a page whose clips met 8 of the 600 marks they clipped.** The mechanism and the saving were real; the fixture was re-cut on 2026-08-17 and no number taken on it before that date is comparable with one taken after (`doc/notes-clipped-instrument.md`) |
| — what the residue multiply costs that page, and where the clock puts it | **4 683 942 Ir, 0.62 % of the encode**; inside `geometry` since ADR 0023's amendment | callgrind on the re-cut page, counters checked against `tests/archetypes.rs`, 2026-08-17. It was reported as `recording` until then, so every `recording` share published for a clipped page was too large by it |
| the artwork and dense-text archetypes' curve clips, before the re-cut | overlapped **8 of 600** and **0 of 40** of the marks they clipped | counted from the generator's own arithmetic, 2026-08-17 (ADR 0057 found it) — a mark whose chain admits nothing still got a mark-sized tile multiplied by zero, so the rows looked like they gated the residue lane for two ADRs |
| the presenter at the display's own refresh — does the split hold 119.96 Hz | **149 of 149 presents landed on the next refresh**, 1.02 per refresh of the span | `examples/present_thread`, RADV at the real display, 2026-08-17, four runs over two loop rounds, loads 8.89–12.21; a fifth at load 23.74 misses 2 of 37 (`doc/notes-present-rate.md`) |
| the present pass, the caller's four layers at 1280×1600 (7 506 609 fragments) | **0.367 ms, 4.4 % of a refresh** | same. The **count** is the bracket — 16 copies still land every refresh, 32 never do; the 0.367 slope is two host clocks, the minimum of five, and indicative |
| first frames, presenting | pipeline compiles: **none**, eight of eight | same; ADR 0043 |
| a path-heavy page's encode geometry, 1 thread → 24 | **309.0 → 46.9 ms** (encode 406.8 → 132.2) | `examples/encode_threads.rs`, llvmpipe, minima of five round-robin, load 17.6, ADR 0054 |
| a generated function shader's compile, at the 482-instruction witness's length | **8.25 ms** RADV, 6.88 ms llvmpipe | `examples/function_compile.rs`, minima of 12 round-robin rounds × 3 alternating runs per adapter, load 5.1–8.6, 2026-08-15 |
| — the floor a *one*-instruction program still pays | 2.67 / 2.04 ms | same; that is `function_lane.wgsl` parsed and built, not the program |
| the caller's corpus at scale 1 | **931** agree / 23 differ / 2 refused / 18 not comparable (GPU lane 929 / 25 / 2 / 18) | their tree, one copy, 2026-08-17 |
| the caller's corpus at scale 4 | **937** / 11 / **3** / 23 (GPU lane **938** / 10 / **3** / 23) | same copy, same day, ADR 0057 |
| — what ADR 0057 moved there | `bug1703683_page2_reduced.pdf` refused → **agrees**; `issue1905.pdf` still refused and now names its sheet | zero page lines move at scale 1; one more at scale 4, `inks.pdf` on the GPU lane by a hundred-thousandth of SSIM |
| — and a count in an older document is still never a baseline | 936 / 10 / 5 → 936 / 11 / 4 for an *unchanged* quorra, a day apart | ADR 0055, 2026-08-15: their tree moved a page from *refused* to *differs* under us |

**Every page a gate or an instrument draws has one definition** (ADR 0060). `crates/quorra-pages`
is a `publish = false` workspace member holding the seven archetypes, the glyph page, two
instrument-only pages, the generator and each page's recorded counter row; `tests/archetypes.rs`
and six examples read it rather than carrying copies. It is a **dev-dependency**, which is the
only edge that reaches a test *and* an example, and it depends on `quorra-scene` alone so the
graph still reads in one direction. Two drifts fell out of putting the definitions side by side,
neither ever caught by a failure: `encode_threads.rs`'s "dense text" has had no clips since
ADR 0054 and is now named `DENSE_TEXT_UNCLIPPED`, and `retained.rs`'s overflow page called itself
verbatim of `zoom.rs`'s while drawing in a different ink.

**And every example's assertions are executed by CI.** Each accepts `--check` — the smallest
configuration that runs its assertions, printing no statistics — and one workflow step runs all
twelve under `Xvfb`, in release, in about two minutes. `tests/example_checks.rs` fails if an
example exists that the step does not name. Before this, `cargo test` neither built nor ran an
example, and `examples/retained.rs` panicked at its own signature gate on `main` for two days
with no signal. Three examples gained an assertion they never had — including
`surface_measure`, which is where this section's real-display row comes from and which until
now asserted nothing about the page it drew.

**The release matrix for `f378fa2 → 1adf479`** — ADR 0066, the one change in this whole round
that moves pixels. One copy of their tree at `14a81f0d`, RADV, both lanes, both scales,
2026-08-18 00:35–01:02:

| lane, scale | base `f378fa2` | `main` `1adf479` |
|---|---|---|
| CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
| GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |

**Nothing moved, and the null was checked rather than accepted.** A null from eleven touched
files, six of them shaders, is exactly the claim this project has been burned believing, so the
round walked all 974 page-one display lists: **5 pages emit a `Shaped` command and 16 emit a
knockout group** (29 groups, 142 overall), so the corpus does reach the path ADR 0066 changed.
Re-run on those 16 alone, both columns print identical lines with **0 not comparable** — so all
sixteen were really rendered by both backends rather than skipped.

**And the mechanism is confirmed rather than inferred**: of the six `Shaped` commands, the
`shape` half carries a soft mask in **none** of them, and the corpus's one masked element
(`knockout_smask.pdf`) carries it on `object`. That is the caller's `stated_shape` (their
ADRs 0234/0327) doing exactly what it says, which is what makes ADR 0066 inert *for this
translator* — by construction, not by luck. A different caller, or a scene built straight
through `SceneBuilder`, would see the 138-of-255 difference the round measured.

**`a4380e2 → 1cd74c9`, `1cd74c9 → a4f10f5` and `f378fa2 → 1adf479` together cover everything a
push delivers.** `doc/notes-release-matrix.md` holds all three, with method and per-page
evidence.

**The release matrix for `1cd74c9 → a4f10f5`** — the 24 commits merged after the matrix below
was taken: ADR 0063's atlas round, ADR 0064's rare-lane round, ADR 0065's atlas-admission round,
the `geom.rs` and `outline.rs` splits with the `resources.rs` and `encode/parallel.rs` declines,
the real-display rounds, and `CLAUDE.md`'s Wayland correction. One copy of their tree at
`411063f9`, RADV, both lanes, both scales, taken 2026-08-17 23:37 – 2026-08-18 00:08:

| lane, scale | base `1cd74c9` | `main` `a4f10f5` |
|---|---|---|
| CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
| GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |

**Nothing moved.** All 79 printed lines across the four rows — 37 distinct documents, 3 814 page
verdicts — are identical between the columns, and the four output files are byte-identical once
the wall clocks are removed. That is the null the range's three source-touching commits needed:
`333f80b`'s two additive `Counters`/`Limits` fields and its one-arithmetic `AtlasStore::byte_size`,
and the two splits, separately verified as pure code moves by multiset comparison before the run.
**The caller's `REFUSED_AT_FOUR` ratchet fails in both columns with the same two lists** — ADR 0057's
`bug1703683_page2_reduced.pdf`, present in both — so it is their outstanding re-baseline and not a
difference here; the other six runs exit 0. Both columns built their `render-quorra` unmodified,
which the base column of the matrix below could not. The output was byte-identical across a load
swing of 1.4 → 35.6, which is its own evidence that a verdict is load-independent.

**`a4380e2 → 1cd74c9` and `1cd74c9 → a4f10f5` together cover everything a push delivers.**
`doc/notes-release-matrix.md` holds both, with method and per-page evidence.

**The release matrix for `a4380e2 → 1cd74c9`** — fifteen rounds, one copy of their tree, all
eight runs inside half an hour on 2026-08-17 against their `22ab57d4`, RADV, both lanes, both
scales (`doc/notes-release-matrix.md`):

| lane, scale | base `a4380e2` | merged `1cd74c9` |
|---|---|---|
| CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 936 / 11 / 4 / 23 | **937** / 11 / **3** / 23 |
| GPU, scale 4 | 937 / 10 / 4 / 23 | **938** / 10 / **3** / 23 |

**Of 3 814 page verdicts compared, five lines move, and all three distinct causes are
ADR 0057**: `bug1703683_page2_reduced.pdf` refused → **agrees with the oracle** on both lanes;
`issue1905.pdf` still refused but now naming its sheet; and `inks.pdf` on the GPU lane by a
hundred-thousandth of SSIM with its mean and worst tile unchanged — the 1-of-255 `fill_mask`
residual ADR 0049 priced, since a different tile rectangle sums in a different order and `f32`
addition is not associative. **Zero page lines move at scale 1 on either lane.** Nothing moved
away from the oracle and nothing moved for a cause that cannot be named.

The fourteen rounds that were *not* ADR 0057 are character-identical across all 3 814
verdicts — the four module splits, `Counters::coverage` and `lanes`,
`SceneError::InvalidImageAlpha`, `RenderError::ViewportTransformTooLarge`, ADR 0023's
amendment, ADR 0058's present rectangle and `SolidFill`'s single hash probe. In particular the
two CPU-rasteriser arithmetic fixes claimed that `hypot` is `direction`'s *second* path and no
corpus page can move; that is now checked over 974 documents rather than over nineteen
fixtures.

**Their `REFUSED_AT_FOUR` ratchet fails loudly on the change column**, printing both lists, and
the caller must drop `bug1703683_page2_reduced.pdf` from it. All four *base* rows exit 0, which
is what says the ratchet is measuring this change rather than their tree moving under us.

*(The gap this paragraph used to record — three commits that landed while these runs were in
flight — is closed by the matrix above, which covers them and everything after.)*

**The release matrix for `a64a908 → a4380e2`** — 72 commits, one copy of their tree, 29
minutes, RADV, both lanes, both scales, taken 2026-08-16 00:04–00:33:

| lane, scale | base `a64a908` | pushed `a4380e2` |
|---|---|---|
| CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 936 / 11 / 4 / 23 | **936 / 11 / 4 / 23** |
| GPU, scale 4 | 937 / 10 / 4 / 23 | **937 / 10 / 4 / 23** |

**Of 956 page lines, exactly one moves** — `issue10572.pdf` at scale 4 on both lanes, mean
0.1332 → 0.1036 and SSIM 0.99497 → 0.99602 with its worst tile unchanged, which is ADR 0055
to the digit and toward the oracle. No refusal moved in either direction. Two things that had
to move nothing moved nothing, and each is worth more than the null it looks like: the
`encode.rs` split into eleven modules got its **first** corpus exposure here and is
character-identical across 1 907 compared pages; and ADR 0054's parallel geometry ran the
change column at **24 threads against the base's 1** and produced identical output on every
page — the determinism claim §4.6 asks for, held over 956 real pages carrying clip chains,
groups, masks and atlas pressure rather than over a fixture. The function lane was exercised
rather than merely compiled: a census over the gate's 974 files found **one** page with a
§8.7.4.5.2 program (`function_based_shading.pdf`, 8 shadings evaluated on the device and 1
refused to the grid), and it agrees with the oracle at both scales on both lanes.

**The earlier matrix, base against the merged round, in one copy of their tree within one
hour** — which is the only form `HANDOVER.md` accepts. Base is `6ed67f0`, the commit before
this round; the corpus cannot be run against the `87898c6` their lock pins, because their
working tree already uses `RetainedScene` and does not compile against it.

| lane, scale | base `6ed67f0` | merged round |
|---|---|---|
| CPU coverage, scale 1 | 930 / 24 / 2 / 18 | **931 / 23** / 2 / 18 |
| GPU coverage, scale 1 | 928 / 26 / 2 / 18 | **929 / 25** / 2 / 18 |
| CPU coverage, scale 4 | 936 / 10 / 5 / 23 | 936 / 10 / 5 / 23 |

**Two page lines move, out of 956, and every other line is identical to the character.**
`issue2177.pdf` joins the oracle on both lanes; `issue11473.pdf` moves by 0.0001 of a mean
and 0.03 of a worst tile. Nothing moved away from the oracle at either scale on either lane,
and no refusal moved. The GPU-lane row is the one that had to be run rather than reasoned
about: ADR 0050 changes atlas residency between frames, and under `Coverage::Gpu` residency
can change which lane a tile takes.

An older row here read `934 / 20` at scale 1 on 2026-08-14. **It is not a baseline**: the
same quorra commit re-run a day later reads 930 / 24, because their tree moved under us.
930 → 931 is what this round is worth.

**§6.2's success bar is met and its clear-win figure is not.** A third of the CPU
backend's 5.9 ms is 2.0 and we are at 1.816; a tenth is 0.6, and the thing that points at
it is the retained encode above — which needs the caller to stop rebuilding a frame's
scene when nothing changed (`pdf-viewer/doc/QUORRA_RETAINED_FRAME.md`). A minimum on a
desktop is not a margin, and it is not the same claim as 2.0 ms inside the caller's own
frame loop on their machine.

**What the frame is made of is the durable finding**, and it survived every round that
moved the total: the device is ~4 % of a presenting dense-text frame, so nothing done to
a shader can matter to it, and the work is CPU recording — which is why the last two
rounds were a bound memo, three probe removals and a retained encode rather than anything
on the GPU.

### The surface can leave the device

ADR 0056 (2026-08-16), asked for by the caller's `QUORRA_NONBLOCKING_RENDER.md` and answered
in `doc/answer-nonblocking-render.md`. `Device::detach_presenter` hands the surface and one
pipeline to a `Send` `Presenter`, so a host can present on the thread that owns its event
loop while `Device::render` runs on another. **The number that justifies it is theirs**:
`execute` is 6.7 ms of a 4 454.9 ms run, so the graphics device is idle for 99.85 % of a
frame of their page, and the reprojection that exists for exactly those milliseconds could
not be issued because it needed the same `&mut Device`.

**The split is proven under `Xvfb` and gated in CI** (`examples/present_thread/`): a page
renders on a second thread while the main thread presents — **three to five presents through
a single render, where the old arrangement allows none** — the finished raster lands under
`scale(2) ∘ translate(64, 32)` with chrome at identity, six window points are read back with
`xwd`, and the gate was verified able to fail in three of the ways it exists to catch.

**And every one of those window points is now read at a moment that was proven rather than
waited for** (ADR 0068, 2026-08-18, `doc/notes-present-settle.md`). The proof used to present for
a fixed 300 ms and then capture, which failed once in five real-display runs by reading one
present behind. **There is no synchronisation to replace the clock with** — wgpu 30 exposes no
present-completion signal and no `VK_KHR_present_wait`, and the X Present extension's
`PresentCompleteNotify` goes to the connection `wgpu` opened inside itself — so the instrument is
a convergence criterion: **two consecutive captures must agree on something other than what the
window was last proven to show**, bounded by 64 capture rounds. Over 29 completed real-display
runs at loads 7.75 to 55.77, **37 of 145 settles took a first capture that was not the settled
window**, which is the old instrument's failure seen from the inside; no settle needed more than
4 of its 64.
**It holds 119.96 Hz, measured on the owner's display on 2026-08-17** (ADR 0056's amendment,
`doc/notes-present-rate.md`): **149 of 149 presents landed on the next refresh** while a second
thread held the device rendering, at 1.02 presents per refresh of the span, over four runs at
load 8.9–12.2. A fifth at load **23.74** misses 2 refreshes of 37, which is the boundary of the
claim rather than a counterexample: it holds on a machine that is not oversubscribed. `Xvfb`
still reports 0.00, so the gate that runs under it asserts counts and never a duration. §1.7's determinism is untouched: nothing
on this path draws a page, and the corpus and the oracle both use `Target::Readback`.

**And a layer now draws its own rectangle rather than the whole window** (ADR 0058,
2026-08-17, measured in `doc/notes-present-quad.md`). At the caller's own 2048×2560 window a
page, a selection, a sidebar and a modal card cost **20 971 520 fragments**, of which
**10 711 584 — 51 %** — are shaded to no effect once a host sizes its layer textures to their
content, and 1 766 624 (8.4 %) even when it does not. The decision was taken on the **count**,
not on a clock: the durations that agree with it vary by more than the saving between runs on
this machine, and one llvmpipe cell has the smaller arrangement slower. The bytes are
identical — 0 differing pixels of 5 242 880 on both adapters — and no public API moved. Two
honest halves: the reprojection case ADR 0056 exists for wins nothing (95.8 %), and **before
this change a host sizing its layers to their content bought nothing at all**, because the
pass cost `layers × window` whatever a layer's size — a cost the API hid from the only person
who could remove it.

### What is still open

- **A cached mark could land a whole device pixel from where it was placed, and 133 corpus
  pages were waiting on the fix** (ADR 0073, 2026-08-22, `doc/notes-glyph-phase-carry.md`).
  `GlyphPlacement::of` quantised a placement's fractional offset with `.round() as u16 % q`,
  and that expression reaches `q` itself for `fx ≥ 1 − 1/2q` — 3.1 % of phases per axis —
  where the modulo sent it to bucket 0 of the *same* pixel and left the integer origin alone.
  Measured at the quantum the caller ships, one copy of their tree: **800/155/2/17 →
  933/22/2/17**, no page regressed, and the 22 that still differ are name for name the 22
  that differ with the quantum off. **The quantum now costs no page its verdict**, which
  retires a claim both trees carried: the page-level cost attributed to "the deliberate
  sub-1/32-pixel trade" was this defect, not the trade.
  Three things kept it invisible and each is a trap now: `GlyphPlacement::of` had **no unit
  test**; every sweep that could have found it was **aliased with the quantum** (16 steps of
  1/16 measure the quantiser's fixed points and call them the quantiser); and **the caller's
  corpus gate runs with `glyph_quantum: None`**, so the 974-page instrument both projects
  rely on cannot reach the path at all.
- **The caller's §31 is narrowed, not answered.** Their four pages were measured with an
  instrument that also sets `glyph_quantum: None`, so the per-command offset they report is
  neither the quantum nor ADR 0073. What our own sweep establishes
  (`examples/lane_placement.rs`): a **stroked** hairline is exact in both coverage settings to
  0.0019 device pixels, which is a byte of alpha; and **on a default atlas the two settings
  are the same lane** for a mark that size, because `take_gpu_lane` declines the device lane
  for anything `worth_caching`. Their second question — is the sampled lane's y coverage
  quantised, and to what — is **open**, and the obstacle is named: `take_gpu_lane`'s
  area-against-triangle-bytes condition refuses a six-triangle band of 528 texels, so no
  hairline in the tree reaches that grid yet.

- **The residue-clip seam, taken.** The residue is rasterised once per chain rather than
  once per clipped command (ADR 0049), and a clipped mark's coverage tile is now bounded
  by its chain's own device box (ADR 0057) — which took `bug1703683_page2_reduced.pdf`
  from refused to agreeing with the oracle at 4× on both lanes, with **zero page lines
  moving at scale 1**. **ADR 0049's 37.8 → 28.9 ms is withdrawn as a demonstration**: it
  was measured on a page whose 185 clips met 8 of the 600 marks they clipped, so most of
  what it removed was repeated rasterisation of tiles that were then multiplied by zero.
  The mechanism and the saving are real and unaffected; what the fixture *showed* was
  narrower than the row implied. On the page that now exists, artwork reads **600 tiles,
  66 residue regions and 384 per-tile rasterisations** — 450 rasterisations for 600
  clipped commands, both branches of the admission rule gated by one page.
  **One page still refuses at 4×, correctly, and it needs no round** — the caller answered
  the question this bullet used to ask. `issue1905.pdf`'s marks *are* the page: seven fills
  wider than it under a rectangular clip that already bounds them, 1 339 315 879 texels, no
  residue clip anywhere. **It refuses only in the gate.** Measured on the real adapter by
  the caller (2026-08-18): the whole page at 4× outgrows the 16 384² scratch image, and
  **every window frame draws, even at 64× zoom** — because a viewer's viewport is its
  window, which is what made the question worth asking rather than assuming. So the
  remaining half of the tiling seam has no product behind it and is **retired rather than
  deferred**; reopen it only if a caller appears that renders a whole page at once.
- **The caller's `HAYRO_ISSUES_FOR_QUORRA.md` is answered in full** (2026-08-17;
  `doc/notes-hayro-coverage-map.md` is the row-by-row map, with
  `notes-hayro-questions.md`, `notes-ceilings-audit.md`, `notes-hayro-paints.md` and
  `notes-hayro-boundary.md` behind it). Every issue it names that is checkable against this
  tree now has a gate, including the ones we already got right, because a written argument
  decays and a gate does not. Two defects that no test could see came out of it — see
  `HANDOVER.md`'s state of play — and **four citation corrections go back to them**, each
  leaving their substantive point standing: §8.9.6.3 not §8.9.6.4 for a mask on its own grid;
  §8.5.3.2's last sentence, not §8.4.3.3, for a bare `m` (no dot under *any* cap, rather than
  a dot under two); §8.7.2 and §8.7.4.1, not §8.7.4.3, for a shading's independence from the
  mark it paints; and ISO 32000-**2**'s `/Interpolate` wording, which is "should … PDF
  processor" where they quote 32000-1's "shall … conforming reader" — a correction that
  *strengthens* their point, since §8.9.5.3 adds that the flag "is only a hint".
  One row is left open on purpose, and it is the next bullet.
- **Whether `Options::coverage` should reach a rare paint — asked, priced, and left**
  (ADR 0064, 2026-08-17, `doc/notes-rare-lane.md`). The setting is consulted in the solid arm
  alone, so `Coverage::Gpu` draws every shading, image, mesh and §7.10.5 paint exactly as
  `Coverage::Cpu` does. Over the caller's corpus the marks it would move are **0.110 % of a
  frame's rasterised coverage at scale 1 and 0.629 % at 4×** — 82 % of all rare-painted
  coverage is under a residue clip and takes the processor lane under either setting anyway.
  The finding that decided it is not the size: **56 of the 88 eligible marks at 4× are under
  100 × 100 device pixels**, because a rare paint's coverage is never offered to the atlas, so
  `take_gpu_lane`'s *cache* condition — the one that keeps reading-size text off the sampled
  grid — cannot apply, and only a comparison monotone in the magnification would be left. The
  omission is an **oversight rather than a design constraint**: the device lane's output is
  already the R8 sheet tile a rare paint is drawn through, verified by making the change and
  reverting it.
- **`examples/present_thread`'s `presents >= 2` was a count decided by a wall clock — closed
  by ADR 0071** (2026-08-22, `doc/notes-present-overlap.md`). It meant "a present completed
  while the render was still running", which is a real property, but the number of presents
  that fit a span is `span / refresh` and the span is a wall clock on a shared machine: it
  refused **3 of 18 runs at load 36.9 to 55.8** and none of the 14 below 19 — once the
  presenting thread got one present into 25.4 ms, once the render itself was **6.4 ms,
  shorter than one refresh**, so 1 was the correct answer and the assertion was wrong about
  its own subject. What replaces it is an **ordering**: the render thread renders
  back-to-back and says so, and the proof is that a present *returned* while it was still
  doing it. The render loop is bounded (300 ms, a stopping rule) so that the regression it
  gates — a present that cannot proceed while the device is held — fails the phase by name
  instead of hanging it; forced, that is 30 renders in 304.1 ms and a red assertion. **12 of
  12 loaded runs at 35.6 to 67.8 pass**, each costing exactly one render, which is what the
  phase cost before. The third failure case is now structurally unreachable rather than
  merely rarer. What it cannot do from this user is a run on the owner's display through
  XWayland, where the failing population was gathered.
- **A mark thinner than the sample grid keeps the processor lane, and §10.7.4's gap is closed
  (ADR 0070).** `Coverage::Gpu` samples a `√n × √n` ordered grid, so a mark narrower than
  `1/√coverage_samples` of a device pixel could fall between two sample columns and read zero —
  the disappearance §10.7.4 names in the very sentence that carries its rule, *"unfavourable
  placement relative to the device pixel grid"*. Note the boundary the costing round drew:
  drawing **more** than the shape's area is *not* a violation, because §10.7.4's second
  requirement is a floor; drawing **nothing** is. A fifth `take_gpu_lane` condition now declines
  the device lane for such a mark, the threshold derived from where the columns actually sit —
  **including across the pixel seam**, since the grid is symmetric about the centre and so the
  columns are one lattice of period `1/√n` across the whole device. Measured on the caller's
  corpus, one copy, 2026-08-18: **`Coverage::Gpu` at scale 1 goes 930/25/2/17 → 932/23/2/17,
  which is the processor lane's own row** — `bug1883609.pdf` and `vertical.pdf` join the oracle,
  every other page line is identical to the character, and at 4× one line moves toward the
  oracle (`issue12295.pdf`, mean 0.9517 → 0.9201) with no verdict changing. Cost: 35 marks at
  scale 1, 26 at 4×, 16 at 8× rasterised on the processor instead — and **magnification shrinks
  the population**, because a stroke's width arrives resolved in device pixels and does not
  follow the viewport.
  **What is left open is the residual, not the rule**: a hairline at 45° given as a *fill* has a
  wide device box and no width of its own to be read instead, so it keeps the device lane and
  *dots* rather than vanishing — one such mark in the corpus. The area rule that would close it
  by construction is declined and priced in ADR 0070: it costs ADR 0016 its scale-independence,
  because exact area requires flattening at a device tolerance, and it would make
  `Options::coverage_samples` meaningless. And the old bullet's lane-policy corollary survives
  the fix: **ADR 0064 declined to let rare paints take the device lane largely because that
  would move shading-painted text onto this grid** — a rare paint never asks `take_gpu_lane`, so
  ADR 0070 does not reach it, and only an area rule would.
- **A group cannot be an element of a knockout group, and is refused by name (ADR 0069).**
  §11.4.6 is normative — *"The separate shape value shall be computed in any group that is
  subsequently used as an element of a knockout group"* — and a layer's alpha is shape ×
  opacity, so §11.3.7.2's union of shapes is not recoverable from it. The construction drew
  **byte-identically to the same group in an ordinary group**, which is principle 6's third
  state. Refused at the builder as `SceneError::KnockoutElementGroupUnsupported`, on a
  population of **zero in 974 documents — and zero by the caller's design**, since their
  `element_shape_is_coverage` excludes a group for the same reason we do. ADR 0033's staged pair
  remains correct to the byte and is the way to state such an element.
- ~~**Whether a soft mask is a knockout element's shape or its opacity is unresolved.**~~
  **Settled by ADR 0066** (2026-08-18), from the clause rather than by the caller, and this
  bullet stood open here for four days after it was. §11.6.4.3's first sentence does not
  decide it — "The mask may serve as a source of either shape ( fm ) or opacity ( qm )
  values" — but Table 57 and §11.6.4.4's last sentence do, and both name **two** parameters
  where the question had been asked about one: the `AIS` flag governs the soft mask and the
  alpha constant together, and its initial value is `false`. A scene carries no such flag, so
  both are opacity, `fs_shape` no longer multiplies the mask into the shape it returns, and
  ADR 0025's text was right where the tree was wrong. It moves pixels, and the ADR carries
  what moved. `doc/notes-function-wiring.md` §4.5 had flagged the same thing for one lane.
- **The suite's scale coverage, now taken whole** (ADR 0072, 2026-08-22,
  `doc/notes-scale-reference.md`). `tests/scale_invariance.rs` (2026-08-17) renders one
  fixture at 1×, 2× and 4× and asserts that ink is area, over a fill, a stroke and a
  residue-clipped fill; what was missing was the reference comparison, since the only test
  checking this tree's pixels against the independent CPU rasteriser was `m1.rs`'s golden at
  scale 1. It now runs at 2× and 4× as well, and both adapters agree with the reference
  within **1 unorm step** at both — against 2 at scale 1, because the fixture's minimum alpha
  *rises* with magnification (24 at 1×, 32 at 2×, 128 at 4×) and scale 1 is therefore the
  hard row.
  **The finding that came out of it is a defect in one of our own gates**, and it was found
  by needing the number at a second scale rather than by a failure: `m1.rs`'s
  `UNORM_TOLERANCE = 2` was derived in its own comment from "this golden, whose minimum alpha
  is 128", and **this golden's minimum alpha is 24** — so the constant enforced something its
  stated derivation does not give (255/24 ≈ 11), which is principle 5's failure exactly. The
  bound is now read at each pixel from that pixel's alpha and its own number of stores, which
  is *stronger* than the constant almost everywhere and honest at the four slivers where the
  amplification is real. Verified able to fail twice, and the second one is the point: a
  coverage error conditioned on a mark wider than 30 device pixels — the caller's hayro
  #40/#8/#63 shape, absent at 1× and present above it — leaves the scale-1 gate **green** and
  reddens the new one alone. `m3.rs`'s constant cites the corrected derivation and is
  deliberately left for its own round.
- ~~**The caller's adoption round**~~ — **done on their side, 2026-08-18**, and this bullet
  described it as pending for four days after. Their `Cargo.lock` pins `cad50156`, their §28
  closed the three drafted answers against their own tree (§15 and §19 close, `layer_textures`
  keeps its name), their §26 answered *both* of ADR 0053 §3.2's contract questions, and their
  §27 has `Paint::Function` adopted and shipping. **Two asks now run the other way** — their
  §31 (our two coverage lanes place a one-pixel-wide rule up to an eighth of a device pixel
  apart, on four corpus pages, and only one lane can be the exact one) and their §33
  (`upload_outline`'s eager quadratics are 83 % of a launch's first frame on a three-million-
  segment drawing, for a representation the default lane never reads). `HANDOVER.md` item 0. Two `Counters` fields
  land with ADR 0050 — `atlas_working_set_bytes` and `atlas_repacked` — and one
  `DeviceError` variant, `ResourceIdsExhausted`; all three are additive. What the bump
  owes them for those is `QUORRA_API_2026_08_15.md` §0 in their tree and this file — the
  `api-change-*.md` drafts were transfer documents and were deleted in `688449d` once
  folded in, so a citation of `doc/api-change-retained-atlas.md` here was pointing at
  nothing from that commit until 2026-08-17. The bump now also carries `CoverageSheet`
  and `Counters::coverage` with `RenderError::ScratchExhausted`'s three new fields
  (ADR 0057), `RenderError::ViewportTransformTooLarge`, `SceneError::KnockoutElementGroupUnsupported`
  (ADR 0069), and one further `SceneError` addition,
  `InvalidImageAlpha`, whose transfer document is `doc/api-change-image-alpha.md`.
- **A paint the device evaluates — built** (ADR 0053, 2026-08-15). `Paint::Function` is a
  §7.10.5 type 4 program uploaded once, admitted at `Device::upload_function`, generated
  into a WGSL shader cached by the program's content hash, and drawn through the rare
  lane's coverage, clip and soft-mask weighting. A full page went 4 988 ms on the caller's
  processor to **0.060 ms** on RADV in the spike. 385 tests pass on **both** adapters; the
  125-case conformance corpus runs on the device against an independently written
  reference evaluator, and both its gate and the lane's are verified able to fail.
  The knockout group, the retained replay and the compile duration are all now measured or
  gated (2026-08-15): §11.4.6's replacement is checked in the pixels over a function paint
  with an ordinary group as its control, a function op replays byte-identically, and a
  generated shader's compile is **8.25 ms on RADV** for a program of the witness's length —
  above a **2.0–2.7 ms floor a one-instruction program pays too**, which is the fixed
  `function_lane.wgsl` and not anything generated. The three gaps this row listed as open
  **closed on 2026-08-16** (`doc/notes-function-gaps.md`, ten tests in three files), and this
  sentence went on calling them open for six days: `tests/function_weights.rs` observes each
  factor of `base_weight` at a value the other two cannot produce (a rectangular clip cutting
  a pixel, a residue clip, §11.5.2's alpha mask, §11.5.3's luminosity mask, and the two as a
  product) and found no defect; `tests/function_coverage.rs` establishes something stronger
  than the bound it was asked for — `take_gpu_lane` is asked only in the solid arm, so the two
  coverage settings draw a page of function paint **byte for byte the same**; and
  `tests/function_staged.rs` draws ADR 0025's `DestOut`/`Plus` pair against §11.4.6's line.
  **Nothing is still open here**: their §26 answers *both* of the contract questions
  ADR 0053 §3.2 names — `true 1 eq` is fixed on their side too and their operand stack has
  types — and their §27 has the paint adopted and shipping, with the classification refusing
  both of the documents the ask was originally written about.
  **The population is now measured, and it is four documents** (ADR 0067, 2026-08-18). A census
  over **67 464 PDFs** — the whole of the caller's `corpus-cache`, their tracked corpora and
  pdf.js's suite — found **four** carrying a `/ShadingType 1` together with a type 4 function,
  of which exactly **one is a real document**, and it is already drawn on the device. It
  extracted **7 139** type 4 programs and ran the real `admit()` over every one: the agreement
  classification refuses **5 of 7 139 (0.07 %)**, and the only two that reach this lane are the
  caller's own hand-written witnesses. Two narrowings that would admit those two are specified
  and deliberately not built — the yield is two hand-made demos against a third Table 42
  implementation that would have to agree with the other two for ever.
  `doc/notes-function-refusal-narrowing.md` is the record.
- **A ramp's subdomain boundary now follows §7.10.4** (ADR 0055, 2026-08-15). A bound belongs
  to the subfunction that starts there, except at the clause's own two exceptions, which
  point opposite ways: the last interval is closed on the right, and where `Domain0 =
  Bounds0` the first is closed on both sides. The corpus moved one page line of 956 —
  `issue10572.pdf` at scale 4, toward the oracle — with no verdict and no refusal moved. It
  was found by the first unit tests that function has ever had, and half of the first
  statement of it was wrong: the fix was checked against the clause text rather than against
  the pattern the clause's opening sentence suggested.
- **The crate's uniform layouts are gated rather than reviewed.** `src/shaders/layout.rs`
  derives every field's offset from the shader source by WGSL §14.4.4 and §14.4.6 and checks
  all nine host writers field-for-field; it opens no device, so it is adapter-independent,
  and it was verified able to fail from both the host and the shader side. Before it, wgpu
  checked total size only — a reordered pair of same-width fields would have passed every
  gate in the tree and drawn a plausible wrong picture.
- **`recording` is now the largest phase of a path-heavy encode** — 132 ms of encode with
  geometry at 47 after ADR 0054 divided it. ADR 0023's "revisit when" is closer than it
  was, and the caller's `QUORRA_ENCODE_THREADS.md` §4 excluded recording from its ask
  deliberately.
- **§11.2's census has run** (2026-08-17, `doc/notes-census.md`), and the path lane's design
  no longer stands on `doc/corpus-profile.md`'s shapes alone. §1.1's premise survives at
  **9.4 %** and §1.6 now picks candidate 2 by measurement. ADR 0008's lever was not pulled.
  What the census left open is one thing it was not asked about, and **ADR 0063 has since
  answered it**: the glyph lane surrenders ~70 000 marks at 4× not to the atlas *budget* and
  not to §1.1's stated mechanism (which accounts for 40 marks of 81 046) but to **the shelf
  packer holding earlier pages' tiles**. The atlas has no eviction, so it accumulates across
  pages until something is refused — and the pages that overflow with the quantum off and
  with it on share only six names, so which page pays is not a property of any page.
  **ADR 0065 has since priced the mechanism behind that accumulation and refused the fix.**
  Filtering single-use tiles out of the atlas on the default lane removes **88.7 % of the
  refusals and moves no pixel** — and costs **31.7 % of the corpus's cached marks on every
  frame after the first**, because 98.6 % of distinct keys are placed once *in their own
  frame* while the atlas still serves 242 049 cached marks on the next one. The criterion
  `worth_caching` states is within-frame, and that is the wrong axis for the lane that keeps
  its tiles; the accumulation stands as ADR 0063 bounded it — one frame per exhaustion, 19
  pages in 948.

---

# Part 1 — How the library will work

## 1.1 The shape of it: a sorter, five lanes, one compositor

The one-sentence brief calls for a renderer whose fast paths assume what a document
actually contains. The architecture that follows from it is a **sorting renderer**: at
frame time, every command in the scene is classified into one of five lanes by what it
*is*, each lane maps to the cheapest device primitive that draws it exactly, and all
five lanes draw into a compositor that implements clause 11 natively. Vello's design
premise — every fill is a general curve fill, handled by one uniform tile-binned
pipeline — is exactly the premise §6.1 measured and found backwards for this workload;
ours is the opposite premise, held to the same standard of measurement.

| lane | what lands in it | device primitive |
|---|---|---|
| **glyph** | a fill of an uploaded outline whose device-space size fits the atlas — §1.1's dominant case, 5 933 of one dense page's commands over 107 distinct outlines | one instanced quad sampling the R8 coverage atlas (§6.3) |
| **rectangle** | axis-aligned rectangles under axis-preserving transforms: rules, backgrounds, underlines, table cells — and most clips | exact analytic coverage in the fragment shader; no tiling, no binning, no edge list (§6.4) |
| **path** | everything else — and the census says what that is: **81 % strokes**, 23 % fills under a residue clip, 2.7 % atlas overflow, ~1 % non-solid paints, and *nothing* from arbitrary transforms. **9.4 % of marks, and at least 66 % of the coverage** | the general coverage path of ADRs 0008/0016/0026 — §1.6's candidate 2, chosen by the census rather than ahead of it |
| **image** | decoded RGBA8 with the filter decision already resolved upstream (§4.5, integration note 1) | a textured quad |
| **mesh** | the caller's pre-rasterised mesh, shared between its backends on purpose (integration note 5) | drawn as the raster it already is; never re-triangulated |

Two properties of the sorter matter more than the lanes themselves:

- **Classification happens at encode time, per frame — never at scene-build time.**
  Which lane a command takes is a device-space question: the same glyph outline is a
  quad at 100% zoom and a general path at 6400%. **Why** it becomes one is measured and is
  not what this bullet said until 2026-08-17: a tile too large for an atlas entry accounts
  for 40 marks of 81 046 at 4×, and what actually moves a glyph to the path lane is the
  shelf packer being full of *earlier pages'* tiles (ADR 0063). The device-space point
  stands; the mechanism named for it did not. Putting the sorter in `render` is what keeps the `Scene`
  viewport-free (§2.3), which the brief calls the most important property in the
  document. The budget for the whole encode is the number the current backend already
  achieves: **1.1–1.6 ms, flat in resolution** (§6.1). Ours may not regress it, because
  it is a function of the command list and not of the pixels, and that flatness is
  structural, not accidental.
- **The sort is a pure function of the command list and the viewport.** Same scene,
  same viewport → same lanes, same batches, same draw order. Determinism (§4.6) is
  designed in here, not tested in later.

**Measured, and it holds** (§11.2's census, 2026-08-17, `doc/notes-census.md`). Over 948
corpus pages on the coverage lane the viewer uses, **9.4 % of a page's marks miss the glyph
and rectangle lanes** at the page's own scale — 10.4 % with the 1/16 quantum the viewer
actually runs, and 11.0 % at 4×. Glyph and rectangle together are 88 %, images 1.5 %. This
paragraph used to say the section would be "rewritten with the number in it" if the premise
failed; it did not fail, and the number is here because a premise confirmed by measurement is
worth more than one merely not contradicted.

**By work it inverts, and the plan did not anticipate that.** The rectangle lane rasterises
no coverage at all by construction, so the scratch sheet — which is the path lane's own bill —
is **at least 66 % of the frame's rasterised coverage at 1× and 76 % at 4×**. A tenth of the
marks cause two thirds of the coverage. That is the number to design against, and it is why
`Counters::coverage` (ADR 0057) rather than a mark count is what a later round should read.

**And that coverage is almost entirely a solid paint's.** Split by the site that seats each
tile (ADR 0064, `doc/notes-rare-lane.md`): **98.6 % of the sheet at scale 1 and 93.1 % at 4×
is solid fills and strokes**, 0.98 % and 4.70 % is a shading, image or §7.10.5 paint, and the
rest is soft masks. So the inversion is a statement about *strokes and clipped fills*, not
about the rare-paint lanes.

**The population is concentrated, not a tail**: 704 of 954 pages draw no path-lane mark at
all, ten pages are 93 % of it, and `issue12810.pdf` alone — 34 970 sub-pixel strokes — is 54 %.

**One clause of this table is wrong, and so was the first correction of it.** The glyph lane's
surrender at zoom is not "when its device size outgrows what an atlas entry can hold" — that
moves **40 marks of 81 046**. Nor is it the atlas *budget*, which is what this paragraph said
until ADR 0063 measured it: the largest single page's working set is **4.10 MiB of 8**, the
median is 11 KiB, and **no page is over budget**. What runs out is **the shelf packer, full of
earlier pages' tiles** — the atlas has no eviction, so it accumulates across pages until one is
refused. The claim that turning the quantum on reverses it **does not reproduce** and has been
withdrawn; the quantum *divides* keys rather than multiplying them (`Some(16)` collapses phases
onto one key, `None` keys exact bits and never collides), so it changes how fast the shared
atlas fills, not which page is rendering when it does.

## 1.2 A frame, from call to pixels

`Device::render(scene, viewport, target)` runs five phases, each bracketed by
timestamp queries where the adapter offers them, so that `Timings` reports what §8
requires and §6.1 could not get: the split between encode, upload, execute and
readback, measured rather than inferred.

1. **Classify and count.** One CPU walk over the commands: sort into lanes, resolve
   each clip chain to its rectangle-and-residue form (§1.4), discover the group and
   mask jobs and their dependency order, and **count everything** — instances per
   lane, layer targets, mask targets, bytes to upload. Nothing has been allocated yet.
2. **Allocate and upload.** Every buffer is sized from phase 1's counts and checked
   against the stated budget before creation. This is §5's first preference — count,
   then allocate — and it is why the failure mode this library exists to eliminate
   cannot occur: there is no fixed-size table for a scene to overflow on the device,
   so a page is drawn or the *allocation* fails with an `Err` naming the limit. A count
   of zero is legitimate everywhere (a blank scene is a legitimate scene, §5) and never
   becomes a zero-length buffer handed to `wgpu`.
3. **Execute.** Passes in dependency order: atlas fills for glyphs not yet resident;
   mask groups rendered and reduced (§1.3); then each layer bottom-up, its commands
   drawn as batched instanced draws in scene order. A batch is a maximal run of
   commands in the same lane with `BlendMode::Normal` and the same clip state; a
   non-Normal blend, a group boundary or a clip change cuts it. Batch cuts are a pure
   function of the list, and `Counters` reports how many there were, because a page
   that cuts often is a page this design serves badly and we want to learn that from a
   counter rather than from a regression.
4. **Resolve.** `Surface` and `Texture` targets composite the finished page and are
   done — no readback, which per §6.1 deletes the largest single cost in the current
   backend's frame. `Readback` copies out, maps, and converts premultiplied to
   straight alpha once at the boundary (§3).
5. **Account.** `Timings` from the query results (and saying so when a wall clock had
   to stand in — a number whose provenance is ambiguous cannot gate anything),
   `Counters`, `Report`s, and a `Frame` whose every claim about itself is true.

**Overturned by:** §11.1, answered in M1. If the readback is nearly all of the fixed
cost, tiers 2 and 3 are the whole performance story, and the effort this plan spends on
per-pixel work in phases 3–4 is re-ranked accordingly.

## 1.3 Clause 11 is the compositor, not an effect

This is the part an SVG-shaped model cannot be patched into, so it is the part designed
first and compromised never.

**Groups are layers, painted once.** A `Group` becomes an offscreen premultiplied
target; its children draw into it; the finished layer is composited onto its parent
exactly once, under the group's constant alpha and blend mode (§4.4; clause 11.4.1,
11.4.5). What the layer is *initialised to* is `GroupSpec::isolated`: transparent for
clause 11.4.5's isolated group, which is the default and what the brief's §4.4 promised
would be the only case, or a copy of the backdrop for clause 11.4.4's non-isolated one,
whose composite is then an interpolation rather than §11.3.6 (ADR 0019, from the
caller's feedback §16). Nesting is bounded at 16 (§1.1), so the layer stack is
countable in phase 1 like everything else. The page itself renders onto transparency, always, because clause
11.4.7 makes the page group isolated and compositing onto the medium is the caller's
job (§3).

**Sixteen blend modes, ours.** `Normal` is hardware fixed-function blending and is the
fast path that keeps batches long. The other fifteen — the twelve separable and the
four non-separable, written from clause 11.3.5's `Lum`, `ClipColor`, `SetLum` and
`SetSat` — need the backdrop as an input, which `wgpu` has no framebuffer-fetch for, so
a non-Normal draw costs a batch cut and a backdrop read (a copy of the affected bounds
into a sampled texture, or a ping-pong of the layer — M6 measures which, on scenes
where it matters). Each WGSL blend function carries its clause number in a comment, and
the implementation is deliberately not shared with the caller's CPU backend: a shared
one would make the cross-backend comparison compare an implementation with itself
(§4.3).

**Shape is its own channel.** The knockout rule (§4.1; clause 11.4.6) replaces *a
coverage-fraction* of the accumulated group with the element composited against the
group's initial backdrop — `lerp(accumulated, element, coverage)`, per pixel. The
design consequence is a rule that binds every lane: **coverage is a first-class value
in the fragment shader at the moment of composite** — computed analytically in the
rectangle lane, sampled from the atlas in the glyph lane, produced by the coverage
machinery in the path lane — and is never irreversibly folded into premultiplied alpha
before the compose decision is applied. Two candidate mechanisms, decided at M6 with
the diagonal-edge fixture as judge: dual-source blending where the adapter offers it,
and a per-element compose pass where it does not. Knockout groups are rare; per-element
passes inside one are affordable if that is what correctness costs.

**Soft masks are rendered groups, reduced on the device.** A mask group is built
through the same `SceneBuilder` as everything else and rendered through the same layer
machinery at device resolution; a single reduction pass then produces the R8 mask —
`Alpha` (clause 11.5.2) takes the group's alpha, `Luminosity` (clause 11.5.3)
composites the group onto a fully opaque backdrop of the mask's colour *first* and
takes the luminosity of the *result* (the order is the clause's, and getting it
backwards produces a plausible picture), then the optional 256-entry `/TR` table is
applied by lookup. No readback, no round trip — the current backend's per-mask-per-frame
CPU round trip is the thing §4.2 exists to delete. The reduction arithmetic mirrors the
caller's `SoftMask::value` in the same 8-bit integer domain, because that function is
the shared definition of what the pixels mean and our shader is a second implementation
that must agree with it **to the byte** — the conformance test over all 256 mask values
ships with the shader (M6), not after it.

**Open, and flagged now:** the layer targets' precision. Premultiplied internally is
settled (§3); whether a layer is 8-bit or wider is not, and it is entangled with
byte-agreement obligations that live in 8-bit space. M6 decides it with the blend and
mask conformance tests in hand, and writes the ADR.

## 1.4 Clips are mostly rectangles, and the design says so

A clip chain is an intersection (§4.7). Phase 1 resolves each chain once into two
parts: the intersection of its axis-aligned rectangular links — which is itself a
single rectangle, four floats and a comparison in the fragment shader, or a scissor
when it bounds the whole batch — and the non-rectangular residue, which becomes an R8
clip mask through the path lane's coverage machinery.

The caller's numbers say the residue is the exception: its page 6 states one clipping
rectangle 303 times and its display list already collapses them to one identifier
(§1.1), and §6.4 is blunt that a rectangular clip must never become a mask texture.
Where a residue mask is built, the chain is rasterised **once over the region it
occupies** and every mark takes a window on it (ADR 0049). The key is the chain's deepest
non-rectangular link, which under one viewport *determines* the region — the transform is
fixed for the frame — so the failure the identifier rule warns about cannot happen here:
two commands under one chain cannot get two different masks. (That rule is the caller's
clip-mask cache, which once answered all 303 lookups a page made and built 303 identical
page-wide masks because its key was a name — their ADR 0132, restated in CLAUDE.md.) What
a chain-identity key cannot do is *unify* two chains that happen to resolve to the same
region, and `Counters::clip_residue_regions` would show that honestly, as two regions.
`Counters` reports the count of distinct regions and the count of chains that paid per tile
instead — keys, never a hit rate. An empty clip admits nothing, which is a different thing
from an absent clip, and both have tests.

## 1.5 Memory that grows

The rule is principle 6's, the mechanism is §1.2's phase discipline, and the posture is
worth stating as design rather than leaving implicit in the phases:

- **Per frame:** count, then allocate. No working buffer is sized by a constant.
- **Across frames:** pools persist on the device and grow geometrically when a frame's
  counts exceed them; they never shrink mid-frame, and shrinking at all is a deliberate
  policy decision for M8, not an emergent behaviour.
- **Every allocation derived from scene content** — instance buffers, layer targets,
  mask targets, the atlas — is checked against a stated budget before creation, and
  exceeding it is a typed `Err` naming the limit and the number that hit it, so the
  caller can fall back to its CPU backend, which its window already knows how to do
  (§5).

  A **cache** is the one place where the answer is *decline* rather than `Err`: the
  residue regions of ADR 0049 are checked against a quarter of `max_frame_bytes` before
  allocation, and a region that does not fit is not built — the frame draws it per tile
  instead, which is what every frame did before. Refusing a drawable frame because a cache
  filled up would be principle 6 pointed the wrong way, and the atlas has said so since
  ADR 0029.
- **Before the frame:** `Scene::cost()` against `Device::limits` gives the caller the
  same arithmetic we will do, so a refusal can happen before a frame is attempted at
  all — §5's second preference, satisfied in addition to the first, not instead of it.

## 1.6 The path lane: the census has now chosen, and it chose what was built

**The census ran on 2026-08-17** (`doc/notes-census.md`) and answers this section's question
twice over — how large the lane is, and *what is in it*. The second answer is the one that
picks a design, and it is not what this section assumed:

**81 % of the path lane is strokes** (89 % on the wider population), 23 % are fills under a
non-rectangular clip residue, 2.7 % are glyph tiles the atlas had no room for, and about 1 %
are non-solid paints. **Arbitrary transforms contribute nothing** — a rotated outline is still
an outline the atlas holds — which is the assumption this section carried without checking.

That retires **candidate 1 by measurement rather than by argument**: tile-binned compute is
machinery for a population of large general fills, and the population is strokes. It confirms
**candidate 2**, which is what ADRs 0008, 0016 and 0026 already built — and the reason it fits
is the one the section worried about in reverse: strokes reach the lane *already expanded to
polygons*, so the CPU flattening cost candidate 2 was charged with is paid for the dominant
case either way. **Candidate 3 stays refused** on the oracle bound. ADR 0008's lever was not
pulled.

The three candidates are kept below as written, because what they were weighed against is why
the answer means anything.

The one lane whose design was deliberately not chosen at M5, because §11.2 asked the
question and the honest answer was a measurement we did not have: **how many of a real
corpus's commands miss the glyph and rectangle lanes?** The candidates, so that M5
chooses among named options rather than improvising:

1. **Tile-binned compute, à la Vello but with counted allocation.** Known to scale to
   the hard case; brings the machinery §6.1 measured as overkill for the common case —
   defensible only if the census says the hard case is common enough.
2. **CPU flattening at encode into device-space geometry, GPU coverage accumulation.**
   Flattening tolerance is a device-space question and phase 1 is device-space, so this
   fits the frame anatomy; costs CPU time that scales with the lane's population, which
   is exactly why the census comes first.
3. **Stencil-then-cover with multisample.** The classical fallback; its coverage
   quantisation (an MSAA sample count's worth of levels) risks the oracle's bound where
   analytic coverage would not, so it must clear the oracle before it clears anything
   else.

Strokes take the path lane after expansion to fill outlines at encode time — the caller
has already resolved device widths, dashing and degenerate subpaths (§4.5), so
expansion is caps, joins and miters and nothing else. Whether hairline strokes deserve
their own primitive is a question the census's stroke population answers, not one this
plan decides.

## 1.7 Determinism, stated as a design posture

§4.6 requires byte equality for same scene, same viewport, same adapter — and the
caller's CI currently *relies* on RADV and lavapipe agreeing byte-for-byte across
adapters, which is §11's question 4 and is not assumed here.

Within one adapter, determinism is arranged rather than hoped for: the sort, the
batches and the draw order are pure functions of the list (§1.1); blending happens in
draw order, which the GPU guarantees per draw call sequence; no accumulation whose
result depends on scheduling order (atomics races, workgroup timing) is permitted in
any pass that touches pixels; nothing reads a clock or a random source.

Across adapters, the hypothesis was that simple fragment arithmetic would preserve
the RADV/lavapipe identity the current backend enjoys. **M1 measured it and the
hypothesis failed** — not in the shader arithmetic, which is deterministic, but in the
fixed-function float→unorm8 store conversion, whose rounding is the driver's
(ADR 0006). Same-adapter identity holds and is gated exactly; cross-adapter output is
gated to a stated bound; and the decision about restoring identity by owning the final
quantisation in shader code belongs to M6's compositor ADR, where its cost can be
measured against the caller's CI reliance on it.

## 1.8 Startup: the device returns before it is warm

§7's sequence, as this library will implement it:

1. `Device::headless` / `for_surface` enumerates the adapter and creates the device —
   `pollster` over wgpu's two awaits, on whatever thread called it, which may be a
   background thread and need not be (§2.1).
2. It returns as soon as the device exists. **No pipeline compilation is on the
   critical path of construction.** The warm set — glyph quads, rectangle fills, the
   composite — compiles immediately but asynchronously; a render that arrives before a
   pipeline it needs waits for exactly that pipeline, and `is_warm` answers whoever
   wants to hand over from the CPU backend only when frames will be full-speed.
3. Everything else — shadings, meshes, the fifteen non-Normal blends, mask reduction —
   compiles on first use, so a page of plain text never pays for machinery it does not
   touch.
4. `Options::pipeline_cache` takes a blob from a previous launch; a rejected blob is a
   `Report`, never a silent recompile, because a silently recompiled cache is a startup
   regression nobody can attribute (§7). Whether the backend supports the cache at all
   is the driver's answer through wgpu's door (ADR 0002), and we report which answer we
   got.
5. Startup cost is reported split three ways — adapter enumeration, device creation,
   pipeline compilation — and CI gates the numbers from the first commit that produces
   them.

## 1.9 The scene, and the resources it references

`quorra-scene` is pure data and cannot reach a device by construction (ADR 0001).
A `Scene` is the flat command list plus its side tables — the clip chains, the group
tree, the mask definitions — behind one `Arc`: `Send + Sync` by a compile-time
assertion, cheap to clone, buildable on the caller's interpreter thread while the GPU
is still initialising. Outlines, images, ramps and meshes live on the device, uploaded
once and referenced by `u32` handles (§2.2) — the caller keys uploads by `Arc::as_ptr`
identity, so one dense page's 107 outlines are uploaded once and referenced 5 933
times, and a zoom re-uploads nothing. **Those handles are never reused**, and the space is
a `u32`: the upload that would exhaust it is refused with `DeviceError::ResourceIdsExhausted`
rather than wrapping. The counter wrapped until ADR 0050 audited ADR 0048's key — a
reissued id would have made a retained encode draw a resource it never named, with every
generation counter agreeing that nothing had moved. `Scene::cost()` is computed at `finish`
time, so asking it costs nothing per frame. §11's question 5 — what a scene costs to hold,
against a target of a dozen resident pages — gets its number in M2 and its verdict in
M8.

## 1.10 The five questions, where each is answered, and what turns on each

| §11 question | answered | what turns on the answer |
|---|---|---|
| 1. How much of the fixed cost is the readback? | **Answered at M1**: ~90% of an offscreen dense-page frame on RADV (4.1 ms of 4.6; execute is 48 µs). The M1 record is in `doc/history/`. | tiers 2–3 are confirmed as the headline; per-pixel work is second-order for tier 2/3 hosts |
| 2. Does the glyph path want tiles at all? | **Answered 2026-08-17**: the path lane is 9.4 % of marks at the page's own scale and 81 % of it is strokes (`doc/notes-census.md`) | §1.1's premise **survives**; §1.6 picks candidate 2, which is what ADRs 0008/0016/0026 built. Candidate 1 is retired by the population, candidate 3 by the oracle bound |
| 3. What does the atlas cost on a page it cannot help? | M4 | whether the atlas is unconditional, adaptive, or off by default on low-reuse pages |
| 4. Is byte-identical output across adapters achievable? | **Answered at M1**: no, for the fixed-function path (ADR 0006); same-adapter identity holds, cross-adapter is bounded and gated | M6's compositor ADR decides whether shader-owned quantisation buys identity back; the caller's CI model needs the conversation now |
| 5. What does a `Scene` cost to hold? | M2 number, M8 verdict | whether the dozen-resident-pages roadmap item needs a compact encoding or gets it for free |

---

# Part 2 — The steps

## How the order was chosen

Not by intuition, and not from the top of the brief. §6.1 measured the current
Vello-based backend and the result reversed the obvious plan: **between 55% and 92% of
an offscreen frame is paid before any of the page is drawn**, scene encoding is flat in
resolution at 1.1–1.6 ms, and 5 933 glyph fills cost about what one rectangle costs.
The brief's own ranking follows from those numbers, and this plan follows the ranking:

> the surface and texture target paths first, because they delete the largest single
> item; then whatever makes the per-pixel floor cheaper for a target that is mostly
> untouched; then the atlas and the rectangle path; then the retained scene; then
> damage.

Two consequences worth stating, because they are easy to get backwards:

- **The first milestone is a measurement, not a feature.** §11's first question cannot
  be answered with a wall clock, and the answer decides whether the atlas is a headline
  or a second-order effect. It needs timestamp queries, which means it needs a device,
  which is why M1 is a device and a rectangle and nothing else.
- **Correctness work is not deferred to the end.** §4 is the reason this library exists
  at all; what is deferred is *breadth*. The knockout group's diagonal edge, the
  sixteen blend modes and a full page at a real window size are the three scenes that
  will find bugs on day one (§10), so each lands with the milestone that first makes it
  possible rather than in a conformance push afterwards.

## M1 — A device, a rectangle, and the measurement that settles §11.1

**Deliverable:** `Device::headless`, `Device::for_surface`, all three targets of §2.4,
one analytically-covered axis-aligned rectangle, `Timings` with real timestamp queries,
`Counters`, `Report`, and a `Frame` that tells the truth. Nothing else — no path, no
glyph, no group. A rectangle is the primitive that needs no tiling, no binning and no
edge list, so it isolates the per-pixel floor from everything else.

**The work, in order:**

1. `Options`, `DeviceError`, `RenderError` — the error variants name what failed, from
   the first commit.
2. Adapter enumeration and device creation, timed separately; `description`, `limits`.
3. The lazy-pipeline scaffolding of §1.8 — built now while there are two pipelines, not
   retrofitted at M6 when there are ten — and `is_warm`.
4. The rectangle fill pipeline: exact analytic coverage in the fragment shader.
5. The three targets, including the readback path with its one straight-alpha
   conversion at the boundary, `#[must_use]` and named for what it costs (§8).
6. Timestamp queries around every phase of §1.2, with the wall-clock fallback that
   *says it is one*; `Timings`, `Counters`, `Frame::reports`.
7. The measurement: execute versus readback, per target kind, at 1×, 2× and 4× of a
   window-scale target — §6.1's table, re-taken with the instrument it lacked.
8. The harness: headless golden renders to PNG, the byte-equality gate (same scene,
   same viewport, same adapter, repeated renders), the RADV-versus-lavapipe
   cross-adapter gate, and CI perf gates for the startup split and the frame numbers.

**Done when:** §11.1 has a number per target kind and resolution; startup has its
three-way split gated in CI; a blank scene renders to `Ok` on all three targets; the
cross-adapter gate has run on both adapters and its verdict — either way — is written
down; and a failed frame cannot report itself drawn (tested, not asserted).

**Done** (2026-08-02). The record and the two answered questions are in "Where we
are"; the verdicts live in ADRs 0005 and 0006; the gates live in
`crates/quorra-gpu/tests/{m1,perf_gate}.rs`. The surface path is proven end to end:
`examples/window_smoke.rs` presents real frames to a real window under `Xvfb`
(lavapipe, the caller's own CI arrangement), verified by reading the window's pixels
back with `xwd` — the centre and field pixels match the scene within ADR 0006's
bound. CI runs the smoke on every push; presenting on RADV to the user's live display
still awaits the user, since Xvfb has no DRI3 for it.

## M2 — The scene, retained and viewport-free

**Deliverable:** `quorra-scene`'s real types — `geom`, `paint` (solid half), `scene` —
plus `SceneBuilder`, `Scene: Send + Sync`, `Scene::cost()`, and the upload/release path
of §2.2 on the device side. The API integration notes below are settled with the caller
before this milestone freezes signatures.

**The work, in order:**

1. `geom`: `f32` throughout matching the caller; move/line/cubic/close and no
   quadratics; `Affine` with `preserves_axes` and `max_stretch`, because §1.1's sorter
   and §6.3's scale bucket ask for exactly those.
2. Input validation at the boundary (§4.7): coordinates and transforms outside stated
   limits are refused loudly with typed errors; no NaN survives into geometry; no
   allocation is sized from an unchecked number.
3. `SceneBuilder` and `Scene`: the flat command list, clip chains as data, group
   nesting checked against the bound of 16, `finish` computing `cost`.
4. `Device::upload_outline` / `upload_image` / `upload_ramp` / `upload_mesh` /
   `release`, each upload checked against the resource budget.
5. The fuzz target on the builder and the encoder — structured scene input, run from
   this milestone onwards, every crasher a permanent regression test (principle 3).
6. The M2 tests the scene skeleton already names: no viewport anywhere (a scene renders
   byte-identically wherever it was built); `Send + Sync` statically asserted; a blank
   scene is legitimate; encode-order independence.

**Done when:** the caller-shaped round trip works — 107 outlines uploaded once,
referenced thousands of times, a zoom re-uploading nothing; `Scene::cost()` returns the
number §11.5 asks about, recorded for M8; byte equality across repeated encodes of one
scene is gated.

**Done** (2026-08-02), with one honest boundary: the upload/identity/budget half of
the round trip is proven (`tests/m2.rs`); the *drawable* half — the outlines actually
painting — is M4/M5's proof by definition. §11.5's first number: a 5 933-fill scene
costs ~380 KB retained (64 bytes per command), so a dozen resident dense pages are
~5 MB of commands — comfortably inside the brief's 70 MB affordability line, before
any compaction. Deviations and refinements are integration notes 1, 5 and 7.

## M3 — Rectangles and rectangular clips, analytically

**Deliverable:** `SceneBuilder::rect`, clip chains resolved and honoured, the
rectangle-and-residue split of §1.4 (with the residue refused loudly for now — the
path lane that draws it is M5, and a refusal that names the reason beats a silent
approximation, §5).

**The work, in order:**

1. Phase-1 clip resolution: chains to a single intersected rectangle plus residue;
   scissor where the rectangle bounds the batch; four floats and a comparison
   otherwise. Never an R8 mask for a rectangle (§6.4).
2. Empty-clip and absent-clip semantics, tested as distinct.
3. The distinct-region counter (§1.4) — the count of resolved regions, not a hit rate.
4. The scene suite grows its full-page fixture: **one full page at a real window
   size**, because a suite of small scenes tests small scenes, and the first real page
   at a real size came back blank in the caller's tree with nothing able to see it
   (trap 12b).

**Done when:** a page-6-shaped scene — thousands of rect fills, 303 identical clip
states collapsing to one region — renders correctly and the counter proves the
collapse; the caller's worst case of 3 608 chains is synthesised and stays within
budget; perf gates cover the rectangle path.

**Done** (2026-08-02). The fixtures live in `crates/quorra-gpu/tests/m3.rs`; the
resolution design is ADR 0007. One deliberate narrowing to note: the recogniser
accepts line-edged rectangles only — a rectangle drawn as collinear cubics takes the
M5 residue path, and loosening that is a measurement-backed decision for M5 if real
corpora need it.

## M4 — The glyph atlas

**Deliverable:** the R8 coverage atlas with eviction and a caller-set budget, keyed on
`(outline, scale bucket, sub-pixel phase)` with the **quantum settable, documented, and
switchable off** — §4.5's fifth decision, the one that is ours to expose. Default 1/16
of a pixel: 1/16 reused 5.0× on a dense page and left the oracle's verdicts unmoved;
1/8 contradicted pages.

**The work, in order:**

1. The key and the quantum plumbing through `Options` — a silent quantum would move the
   text and nobody could attribute it.
2. The packer, eviction, and the budget check; `atlas_entries` and
   `atlas_distinct_keys` in `Counters` — the distinct-key count, not the hit rate.
   Two more joined them at ADR 0050. `atlas_working_set_bytes` is what holding **all**
   of a frame's distinct glyph keys would cost, which is the number `Options::atlas_budget`
   is compared against and the only one that tells "the atlas is too small for this page"
   apart from "the atlas is holding another page". `atlas_repacked` is whether the atlas
   was thrown away and re-packed after this frame — the one event that makes a retained
   encode stale. A page that settles reports it true on at most one frame; true on frame
   after frame is thrash, and the counter exists so that state has a name.
3. The rasteriser decision, as an ADR with a measurement: coverage rasterised on the
   GPU, or on the CPU by `tiny-skia` — the latter makes the glyphs come from the same
   code that is the caller's correctness oracle, a correctness argument no other
   arrangement gets for free, against a dependency and a per-glyph transfer this
   library would otherwise not have (§6.3 offers it as persuasive, not prescriptive).
4. The glyph lane in the sorter: fills whose device size fits an entry become quads;
   the atlas-miss path (too large, budget exhausted) falls through to the path lane —
   or, until M5 exists, to a loud refusal.
5. The measurement for §11.3: the atlas's cost on `tracemonkey.pdf`-shaped reuse
   (1.3×) as well as its win on dense-text reuse (5.0×), because a cache that is 5× on
   one page and a net loss on another is a decision, not a feature — and the decision
   needs both numbers.

**Done when:** a dense-text scene at window scale beats its M3-era self by a measured
margin; the low-reuse cost is known and the decision it forces is written down; the
quantum is proven settable and off-able by test.

**Done** (2026-08-02): ADRs 0008/0009, the record in `doc/history/`, gates in
`tests/{m2,m45}.rs`. The tiny-skia question resolved as "neither": our own
rasteriser, for determinism and the dependency posture — the oracle argument waits
for M9 where the oracle actually is.

## M5 — General path coverage, designed after the census

**Deliverable:** fills and strokes of arbitrary cubic outlines, both fill rules, nested
subpaths wound the same way, non-rectangular clip residues — the path lane, built as
whichever of §1.6's candidates the census justifies.

**The work, in order:**

1. **The census first (§11.2):** over the caller's corpus display lists, count what
   share of commands miss the glyph and rectangle lanes, and what they are. The corpus
   lives in the caller's tree and the two trees stay independent until M9, so the
   fixture is *data* — display lists serialised to a neutral form and carried over —
   never a dependency edge.
2. The design ADR: which candidate, chosen by the census plus the oracle constraint
   (coverage quantisation must not move oracle verdicts).
3. Fills: both rules, the caller's deep-nesting cases, clip residues through the same
   machinery, the mask targets it produces feeding §1.4's region-keyed cache.
4. Strokes: expansion to fill outlines at encode — caps, joins, miter limits only,
   because widths, dashing and degenerate subpaths arrive resolved (§4.5) and our job
   is not to undo any of it.
5. The atlas-miss fall-through from M4 becomes real: a glyph at extreme zoom takes this
   lane and the seam is invisible (tested at the boundary scale).

**Done when:** the census number is in the ADR; the three day-one scenes that involve
paths pass; every command a real corpus page contains renders through some lane with no
refusals left except genuine budget refusals.

**The census has since run** (2026-08-17, `doc/notes-census.md`): §11.2 is answered, §1.1's
premise holds at 9.4 %, and §1.6's candidate 2 — what was built — is confirmed by the
population rather than assumed. The sentence below about the census "not having run" was true
from M5 until then.

**Done in implementation, open in measurement** (2026-08-02): fills (both rules,
nested same-winding pinned end to end), strokes (caps/joins/miter, cross-checked
against the rectangle they degenerate to), residue clips (the triangle-clip fixture
masks correctly), oblique rectangles — all drawable; the remaining M6-refusals are
groups, non-Normal blends and `Compose::Src`, each named. The census (§11.2) has not
run — it needs corpus fixtures from the caller's tree — and stays the recorded
condition for revisiting the lane design (ADR 0008).

## M6 — Clause 11, natively

**Deliverable:** the four things a general vector API lacks, and the reason this
library exists (§1.3's design, made real).

1. **Coverage-modulated Porter-Duff Source** (§4.1) — the compose mechanism decided
   between dual-source blending and a per-element compose pass, with the diagonal-edge
   knockout fixture as judge, because axis-aligned rectangles would agree while being
   wrong.
2. **Soft masks reduced on the device** (§4.2) — `Alpha`, `Luminosity { backdrop }`,
   the optional 256-entry `/TR` table, the mask group built through the same
   `SceneBuilder`. No readback, no round trip. The conformance test is part of the
   definition of done: all 256 mask bytes through both rules against the caller's
   `SoftMask::value`, byte-equal; a luminosity mask with a non-black backdrop; a
   non-identity transfer sampled at both endpoints.
3. **Sixteen blend modes** (§4.3), each WGSL function carrying its clause number, the
   four non-separable ones written from clause 11.3.5's `Lum`, `ClipColor`, `SetLum`
   and `SetSat` — our own implementation, deliberately unshared, tested against the
   caller's sixteen-mode fixture that once found three of `tiny-skia`'s wrong by up to
   113 of 255.
4. **Groups compositing onto transparency** (§4.4), isolated, painted once under the
   group's own alpha and blend mode, depth bounded at 16 — plus the backdrop-read
   mechanism for non-Normal blends, measured on scenes where the batch cuts bite.

The layer-precision ADR (§1.3's open question) is written here, with the conformance
tests in hand.

**Done when:** the caller's three day-one scenes pass — the knockout diagonal, the
sixteen modes, the full page at window scale — and the soft-mask byte-agreement test is
green on both adapters.

**Done** (2026-08-02): ADR 0010, gates in `tests/m6.rs` — the 256-byte agreement is
exact, the sixteen modes match a clause-transcribed reference, the knockout diagonal
matches §11.4.6's formula, isolation and group alpha hold, and the compositor is
byte-deterministic per adapter. The layer-precision question closed on the side of
rgba8 layers (quantisation between commands is the CPU reference's model and the
mask byte-agreement's precondition); full-target layer textures are the recorded
optimisation candidate for a bbox-bounded version, with M8's measurements. Item 4's
"onto transparency" is the isolated group only, which is what §4.4 promised at the
time; clause 11.4.4's other initial backdrop arrived later, in ADR 0019.

## M7 — Shadings, meshes, images

**Deliverable:** axial, radial and function-based shadings from a `RampId` (clause
8.7.4.5); the caller's pre-rasterised mesh consumed as-is (integration note 5); decoded
RGBA8 images with the filter decision arriving resolved (integration note 1). Straight
alpha at the boundary, premultiplied internally, rendered onto transparency, always.
All of these pipelines compile on first use (§1.8) — a page of plain text never pays
for them. The open `ShadingKind` question — geometry on the paint versus resolved at
upload — is decided here with a measurement and an ADR.

**Done** (2026-08-02): ADR 0011, the record in `doc/history/`, gates in
`tests/m7.rs`. The `ShadingKind` decision landed on **geometry on the paint** (six
floats per placement; the uploaded ramp serves every placement, which is §2.2's
economy — integration note 9). The caller's *sampled* function shadings map to
images on this side, and meshes arrive pre-rasterised at device resolution
(integration note 5) and sample at absolute device pixels. Image, shading and mesh
pipelines compile on first use; the warm set is unchanged, so startup did not move.

## M8 — Damage, and the rest of the performance contract

**Deliverable:** per-tile dirty tracking against a retained scene and a cheap viewport,
so a caret blink redraws a few tiles rather than a page (§6.5) — `Viewport::damage`
honoured exactly, and a damage list we cannot honour meaning a full redraw *and a
`Report`*, never a stale region. The persisted pipeline cache round-trips through a
real second launch — **which requires an ADR first**: wgpu 30's
`create_pipeline_cache` is an `unsafe fn` (the blob is trusted input), and this tree
is `#![forbid(unsafe_code)]`; the ADR weighs a scoped exception (principle 3's
benchmark-plus-invariant route) against not offering the cache, with the startup
measurement in hand. Pool-shrink policy decided. §11.5's verdict: `Scene` memory
against the dozen-resident-pages target, with M2's number as the input.

**Done** (2026-08-02): ADRs 0012 (damage as scissored rendering plus rectangle
patching — honoured exactly on `Texture` targets, reported on the others, refused
when malformed) and 0013 (no pipeline-cache blob: the benchmark showed nothing
user-visible to win, so the `unsafe` exception was declined and
`#![forbid(unsafe_code)]` stands). The record and numbers are in `doc/history/`;
gates in `tests/m8.rs`. The plan's own phrasing was ahead of the measurement in one
place: there is no texture pool to give a shrink policy to — per-frame creation is
inside the measured budget, and the pool waits for a number that demands it.

## M9 — The swap

**Deliverable:** the caller's `render-gpu` replaced — an adapter in *their* tree
implementing their `Rasterizer` over quorra, which is where the bridging of integration
note 2 lives. Held to §10 in full: the cross-backend scene suite; the 1 794-page
oracle, which has refused two of the caller's own optimisations after they passed every
unit test and will not be kind; byte equality where we claim it; and a window driven by
real key presses under `Xvfb`. Not before: this is the last milestone, and the two
trees stay independent until it.

**Progress** (2026-08-02): the adapter exists and passes the caller's bar.
`crates/render-quorra` in the caller's tree implements their `Rasterizer` over this
library: the display list maps command-for-command (the two vocabularies were shaped
by one contract); dashes cut through the same `kurbo::dash` their Vello backend uses
and §8.5.3.2's zero-length rules through their shared helpers; sampled shadings —
which their GPU backend *refuses* — draw as domain images clipped to the fill;
meshes go through their shared `MeshRaster`; the medium is imposed by their own
function. Two integration findings became fixes on this side: `Paint::Shading` grew
its own transform (a shading anchors to the page, §8.7.4.3 — the command-space
geometry M7 chose could not express the caller's model), and `Stroke::width`'s doc
now tells the truth (device pixels). Two adapter defects were found by the caller's
own instruments and fixed: an ABA bug in the `Arc`-identity caches (pointer keys
without pinning served stale outlines by allocator mood — the caches now hold the
`Arc`), and stroke widths mistaking their `device_width()` for device units (it
answers in path units; ×`max_stretch` carries it over, and page 6's rules stopped
being 2× thin at window scale). **Gates, all green on RADV:** the eleven
cross-backend scenes at the Vello suite's own thresholds (all sixteen blend modes,
knockout, soft masks, an interpreted PDF); the real-page suite — with exact phases
quorra sits at the Vello backend's distance from the CPU oracle on every case
(worst tile ≤ 3.0 vs their 3.2; one case *better*: 2.6 vs 4.2), and the 1/16
quantum's cost is pinned as its own envelope. **Perf at the trait boundary** (page
6, fastest of 12): 4.4 ms vs CPU 4.9 / Vello 6.9 at scale 1.0; 10.4 ms vs CPU 6.1 /
Vello 11.6 at window scale — both GPU backends readback-bound exactly as §11.1
measured, which is why the remaining work is the part that escapes the trait.
**Done** (2026-08-03): the swap itself landed. `render-quorra` grew the tier-2
`QuorraPresenter` — quorra's device owns the window's surface, and one call draws
the page, the selection, the sidebar and the modal card (all display lists through
the same translation the headless gates exercise) and presents, with no readback
anywhere. To make one scene carry several lists at their own placements, the
adapter bakes each list's target transform into its commands and leaves the
viewport at identity. `viewer-ui`'s render path moved onto the presenter
(−205 lines net: the RenderContext, intermediate texture, blitter and banding
machinery all left with the backend that needed them); the CPU backend keeps its
oracle and fallback roles, the fallback now presenting through the same quorra
surface as an image. **Verified under Xvfb with real key presses**: the ISO
32000-2 cover (images, chrome) and Page-Down navigation to the dense table of
contents, zero fallback notes. One follow-up remains with the owner: a
corpus-scale quorra-vs-oracle sweep (the 1 794-page oracle itself judges the CPU
backend by design, so this is an additional gate, not a missing one). The
CI-reachability question closed on 2026-08-03: this library lives at
https://github.com/close2/quorra, and the viewer consumes it as a git dependency —
revision pinned by its Cargo.lock, source named in its deny.toml, with a
documented `[patch]` route for developing against a local checkout.

## What we must build ourselves, and when

§10 tells us what we will be judged by, which means the harness is ours to write rather
than to wait for:

| | lands with |
|---|---|
| Headless golden renders, PNG artefacts, byte-equality gate | M1 |
| Adapter-to-adapter byte equality, RADV against lavapipe (§11.4) | M1, re-checked every milestone |
| Perf gates: startup split, encode, execute, readback, one real page at a window size | M1 |
| A scene suite that starts small and includes **one full page at a real window size** | M3 |
| Fuzzing the scene boundary (principle 3) | M2 |
| The corpus fixture: the caller's display lists as neutral data, for the census and beyond | M5 |
| The knockout group with its diagonal edge; the sixteen-mode scene; the soft-mask agreement test | M6 |

## Which milestone fills which module

The skeleton's modules each state their contract (ADR 0003); this is the map of who
deletes which `text` block.

| module | filled by |
|---|---|
| `quorra-gpu`: `device`, `pipeline`, `target`, `frame`, `viewport`, `report`, `error`, `surface`, `readback`, `timing` | M1 ✓ (and every milestone after adds to `pipeline`) |
| `quorra-gpu`: `resources` | M2 ✓ |
| `quorra-scene`: `geom`, `scene`, `paint` (solid half), `image`/`mesh` (upload specs) | M2 ✓ (geom landed with M1) |
| `quorra-gpu`: `atlas` | M4 ✓ |
| `quorra-scene`: `mask`; `quorra-gpu`: `mask` | M6 ✓ |
| `quorra-scene`: `paint` (shading half), `image`, `mesh` | M7 ✓ |

## Integration notes against the caller — the recorded divergences from the brief

These are the places where the brief and `pdf-render` did not line up exactly. None was a
problem; each was a question whose answer belongs in the API rather than in a translation
layer's judgement (§4.5: a decision either side can make alone is a decision neither side
has made). They were raised before M2 froze the signatures, and each note carries where it
was settled and what it settled as. **The numbering is load-bearing** — module docs in the
tree cite these notes by number, so a note is corrected in place and never renumbered.

1. **The image filter flags are not flags.** §4.5 names `Image::is_smoothed` and
   `Image::area_averaged` as flags to honour. In the tree they are *methods taking the
   placement transform* — `is_smoothed(placement)`, `area_averaged(placement) ->
   Option<Image>`. So what must reach us is the **resolved** decision for the placement
   the command carries, not the flag it was derived from. Refined at M2, from the
   caller's types: since the decision is per *placement* and an upload is per *image*,
   the resolved filter belongs on the image **command** (M7), and `ImageSpec` carries
   pixels only — which is also what lets one upload serve every zoom level.
2. **`Viewport` versus `TargetSpec`.** §2.4's viewport carries a full affine — scale,
   y-flip and tile offset. The trait in the tree carries a scale and a pixel budget
   (`TargetSpec::for_page(list, scale, max_pixels)`). The affine is the more general
   of the two and is what tiled output and damage need; the bridging belongs in the
   caller's adapter, and the rounding rule for a fractional page stays theirs.
3. **The soft-mask transfer function.** The brief says a 256-entry lookup table because
   a mask value is one byte. The tree has a `Transfer` type; we take `[u8; 256]` and
   the caller samples. Confirm the sampling convention — inclusive of both endpoints —
   in the conformance test, not in prose.
4. **Which `raw-window-handle`.** §2.1 asks for `raw-window-handle` and nothing more
   specific, but the version has to be the one `wgpu` was built against or a surface
   cannot be created. Reach it through a `wgpu` re-export if one exists in 30.0.0;
   otherwise pin the same version here and say so.
5. **`MeshRaster` is shared on purpose.** Both of the caller's backends share the
   pre-rasterised mesh because neither rasteriser has the primitive and a second copy
   would drift. We inherit that: we consume the mesh, we do not re-triangulate it.
   Confirmed at M2 from the caller's type: a `MeshRaster` is a **device-resolution**
   positioned raster, so a mesh upload is viewport-dependent and a zoom re-uploads its
   meshes — the cost of the shared-rasteriser correctness argument, taken upstream and
   inherited knowingly.
6. **Colour is not ours.** Device RGB arrives; `ColourSpace::to_rgb` upstream is the
   only place a colour becomes RGB, and adding a second one is forbidden. If we offer
   colour management the caller cannot use us.
7. **`release` returns a `Result`, deviating from §2.2's `()` signature.** A release
   of an id the device never issued — or issued and already released — is
   `DeviceError::UnknownResource`, because a double release is a caller bug and a
   no-op would hide it. The brief calls its signatures illustrative; the property
   (no silent error swallowing) outranks the shape, and this is flagged for the API
   conversation before M9.
8. **The `mask` parameter comes last**, in every builder method, rather than beside
   `clip` as the brief's illustrative signatures place it: growing the vocabulary
   milestone by milestone was then a mechanical widening at every call site. Flagged
   with note 7 for the API conversation before M9.
9. **Shading geometry travels on the paint; sampled shadings become images.**
   Decided at M7 (ADR 0011), refined at M9: `Paint::Shading { ramp, kind,
   transform }` carries the axial or radial geometry (six floats) in the
   **shading's own space**, with the paint's transform mapping it into the scene —
   §8.7.4.3's shading matrix, anchoring a shading to the page rather than to the
   path it fills, exactly as the caller's `Shading { kind, transform }` states it.
   One uploaded ramp serves every placement and zoom — §2.2's economy, and the
   scene stays viewport-free (§2.3). The caller's `ShadingKind::Sampled` — its
   display list holds a *grid*, not a function — maps to an image upload plus an
   image command in the M9 adapter; `ShadingKind::Mesh` maps to `upload_mesh` +
   `Paint::Mesh` (note 5's device-resolution caveat applies).

## Not planned, ever

§9's non-goals: colour management, font loading, shaping, hinting, text layout, bidi,
filters and effects beyond §4, any document format, a scene graph or animation, hit
testing. `deny.toml` enforces as much of the list as a dependency policy can express.
