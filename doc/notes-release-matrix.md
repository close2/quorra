# The release matrix for `a4380e2 → 1cd74c9`

Date: **2026-08-17**. Base `a4380e2` — the last commit the owner pushed — against `main` at
`1cd74c9`. **73 commits**, 59 of them non-merge, fifteen rounds prepared in parallel worktrees
and merged without a corpus run covering them together. Each round argued individually that it
moved no pixel; those arguments were sound, and this is the instrument saying so about all of
them at once.

The corpus is the caller's, read-only at `/home/cl/projects/pdf-viewer`, revision
**`22ab57d4`** ("A caret for a screen reader, and a third off what a page turn cost"), copied
once at 04:20 and not re-copied between columns. Adapter: **AMD Radeon 890M Graphics (RADV
STRIX1)**, Vulkan, 24 encode threads, release build, glyph quantum off. All eight runs in
**one copy of their tree between 04:37 and 05:04 on the same day**, flipping only the
`[patch]` path.

## 1. The four rows

| lane, scale | base `a4380e2` | `main` `1cd74c9` |
|---|---|---|
| CPU, scale 1 | 931 agree / 23 differ / 2 refused / 18 not comparable | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 936 / 11 / 4 / 23 | **937** / 11 / **3** / 23 |
| GPU, scale 4 | 937 / 10 / 4 / 23 | **938** / 10 / **3** / 23 |

3 814 page verdicts compared. A line is printed only for a page that differs or is refused,
and **81 distinct page names print one in one column or the other** — 25 at CPU 1, 27 at
GPU 1, 15 at CPU 4, 14 at GPU 4. **Five of those 81 move, and each of the three distinct
causes is ADR 0057.** Nothing moves at scale 1 in either lane: all 25 and all 27 printed
lines are identical to the character.

**What this matrix does not cover.** `main` moved to `de1c013` while the eight runs were in
flight — `333f80b`, `903d05e` and `de1c013`, the atlas-budget round, which touches
`src/atlas.rs`, `src/frame.rs`, `src/startup.rs`, `src/device.rs` and two encode modules.
The change column is `1cd74c9` and nothing after it. Those three commits need their own
row before a push, and this is the same trap `HANDOVER.md` records about the caller's tree,
seen for once in our own: **a count is a statement about a revision, and both revisions have
to be named.**

## 2. Every page line that moved, with its cause

| page | lane, scale | base `a4380e2` | `main` | cause |
|---|---|---|---|---|
| `bug1703683_page2_reduced.pdf` | CPU 4 **and** GPU 4 | refused, `ScratchExhausted` | **agrees with the oracle** | ADR 0057 decision 1 — a clipped mark's coverage tile is bounded by its chain's own device box |
| `issue1905.pdf` | CPU 4 **and** GPU 4 | refused, naming only the adapter's wall | refused, **naming the sheet**: "a 4763x7103 tile would not fit a sheet at 14289x15117 holding 6 tiles and 213115672 texels" | ADR 0057 decision 2 — a refused frame accounts for the sheet it met (`CoverageSheet`) |
| `inks.pdf` | GPU 4 only | mean 0.0394, worst tile 17.29, differing 0.0012, SSIM 0.998**61** | the same to the digit, SSIM 0.998**62** | ADR 0057 decision 1 again: the tile is asked for a different rectangle, and `f32` addition is not associative — the 1-of-255 `fill_mask` residual ADR 0049 recorded and priced |

**Nothing moved away from the oracle**, at either scale, on either lane. Every other printed
line — mean, worst tile, differing fraction and SSIM — is identical to the last digit in both
columns. **No page moved for a cause that cannot be named**, which is the question this round
existed to ask.

That the three moves are ADR 0057's and only ADR 0057's is the finding, not a null. Fourteen
other rounds are in this delta and several of them touch code on the frame path: the `error.rs`
split into seven modules, the `raster.rs` split into three plus the two arithmetic fixes under
it, the `pipeline.rs` split, `Counters::coverage`/`Counters::lanes`, `SceneError::InvalidImageAlpha`,
the residue multiply moving from `recording` to `geometry`, `RenderError::ViewportTransformTooLarge`,
`SolidFill` carrying the outline the fill arm already found, and ADR 0058's present rectangle.
All of them are character-identical across 3 814 page verdicts here.

Two of those deserve naming because they are the ones that *could* have moved a page and did
not:

- **`raster::direction`'s `hypot` fallback** (`f52f11d`). The commit claims "`hypot` is the
  second path, not the first, so every segment keeps its arithmetic to the bit and no corpus
  page can move." The fast path is entered whenever `(dx²+dy²).sqrt()` is finite and positive,
  which is every segment of every corpus page; the matrix is the check on that claim over 974
  documents rather than over the nineteen fixtures that gate it. Likewise `accumulate_edge`'s
  non-finite-slope return: no corpus page reaches it.
- **`RenderError::ViewportTransformTooLarge`** (`f52f11d`). A new refusal on the frame path.
  No page of the corpus is refused by it at either scale — the gate's viewport transforms are
  1× and 4×, and the bound is `MAX_COORDINATE`, `1e9`.

## 3. Their `REFUSED` ratchet fails, loudly, and that is the result

The CPU scale-4 row of the change column exits 101. The gate holds the scale-4 refusal list to
equality and prints both:

```
assertion `left == right` failed: the pages quorra refuses at 4× have changed
  left: ["bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
 right: ["bug1703683_page2_reduced.pdf", "bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
```

**The caller must drop `bug1703683_page2_reduced.pdf` from `REFUSED_AT_FOUR`** when they take
the bump. Every other run of the eight exits 0 — including all four base rows, which is what
says the ratchet is measuring the change rather than their tree having moved under us.

Their scale-1 ratchets pass in both columns, so they have already re-baselined for ADR 0049's
`issue2177.pdf`; the debt `HANDOVER.md` recorded against the last release is closed on their
side.

## 4. What was patched in the copy, and why

**One patch, to the base column only.** `a4380e2` predates ADR 0056, and their working tree
has already adopted it: `crates/render-quorra/src/present.rs` calls
`Device::detach_presenter` and `Device::attach_presenter` and names `quorra_gpu::Presenter`
and `quorra_gpu::ForeignPresenter`. Against `a4380e2` that is five compile errors across two
methods, and the crate does not build.

For the base column, and only for it, lines 902–927 of
`/home/AI/release-matrix/viewer/crates/render-quorra/src/present.rs` — the whole of
`QuorraWindowRenderer::detach_presenter` and `QuorraWindowRenderer::attach_presenter`,
their doc comments included — were deleted. Nothing else in the copy was touched, the
original is kept beside it as `present.rs.orig`, and it was restored verbatim for the change
column. The corpus gate calls neither method: it drives `QuorraRasterizer` into a texture and
never touches a surface, so the deletion cannot reach a pixel. **The change column built
unmodified**, which is the half of this that is a statement about our API.

The copy also carries the `[patch."https://github.com/close2/quorra"]` block the recipe asks
for. Cargo warns that the `quorra` entry is unused — their workspace depends on `quorra-gpu`
and `quorra-scene` only — which is expected and is not an error.

## 5. Method, so the numbers can be re-taken rather than argued about

- `rsync -a` of their tree with `HANDOVER.md`'s seven excludes to `/home/AI/release-matrix/viewer`,
  once, at 04:20. **Never built in or written to `/home/cl/projects/pdf-viewer`.**
- Base column: `git worktree add /home/AI/release-matrix/quorra-base a4380e2`. Change column:
  this worktree at `1cd74c9`. Only the `[patch]` path differs between the two runs of each row.
- A private `CARGO_TARGET_DIR=/home/AI/release-matrix/target`, because the shared one has
  produced stale binaries naming symbols that exist in no worktree.
- `cargo test --release -p render-quorra --test corpus -- --ignored --nocapture`, with
  `PDFVIEWER_QUORRA_COVERAGE=cpu|gpu` and `PDFVIEWER_QUORRA_SCALE=1|4`. Driver:
  `/home/AI/release-matrix/run-column.sh`; per-page comparison: `/home/AI/release-matrix/compare.py`;
  raw output: `/home/AI/release-matrix/out/{base,change}-{cpu,gpu}-{1,4}.txt`.

**No timing is published from this run.** A sibling agent's corpus gate
(`/home/AI/rare-lane/target/…/corpus`) was on the same GPU for part of the change column, load
averages ran 2.1–4.4 across the eight runs, and `HANDOVER.md`'s rule stands: which pages refuse
is arithmetic and machine-independent, which lane is faster is not. The wall clocks in the raw
output are context and nothing else.

## 6. Recommended replacement for `PLAN.md`'s matrix

> **The release matrix for `a4380e2 → 1cd74c9`** — 73 commits over fifteen parallel rounds,
> one copy of their tree at `22ab57d4`, RADV, both lanes, both scales, taken 2026-08-17
> 04:37–05:04:
>
> | lane, scale | base `a4380e2` | `main` `1cd74c9` |
> |---|---|---|
> | CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
> | GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
> | CPU, scale 4 | 936 / 11 / 4 / 23 | **937** / 11 / **3** / 23 |
> | GPU, scale 4 | 937 / 10 / 4 / 23 | **938** / 10 / **3** / 23 |
>
> **Of the 81 page names that print a line across the four rows, five move, and every one of
> them is ADR 0057.** `bug1703683_page2_reduced.pdf` goes from refused to agreeing on both lanes at 4×;
> `issue1905.pdf` stays refused and its message now names the sheet it met; `inks.pdf` moves by
> one hundred-thousandth of SSIM on the GPU lane at 4× with its mean, worst tile and differing
> fraction unchanged. Nothing moved away from the oracle and nothing moved at scale 1. The
> fourteen other rounds in the delta — the `error.rs`, `raster.rs` and `pipeline.rs` splits,
> `Counters::coverage` and `Counters::lanes`, `SceneError::InvalidImageAlpha`,
> `RenderError::ViewportTransformTooLarge`, ADR 0023's amendment, ADR 0058's present rectangle
> and `SolidFill`'s single hash probe — are character-identical across 3 814 page verdicts,
> which is the first corpus exposure any of them has had. **The caller must drop
> `bug1703683_page2_reduced.pdf` from their scale-4 `REFUSED` ratchet**; the CPU scale-4 run
> fails loudly until they do, and that failure is the result rather than a problem.
>
> **It stops at `1cd74c9`.** The atlas-budget round (`333f80b`, `903d05e`, `de1c013`) landed
> on `main` while the eight runs were in flight and touches `atlas.rs`, `frame.rs`,
> `startup.rs`, `device.rs` and two encode modules; it owes its own row before a push.
>
> The base column does not build against their tree unmodified and the change column does:
> `a4380e2` predates ADR 0056, and their `render-quorra` already calls `detach_presenter`.
> `doc/notes-release-matrix.md` records exactly what was removed from the copy for the base run.

---

# The release matrix for `1cd74c9 → a4f10f5`

Date: **2026-08-18**. Base `1cd74c9` — where the matrix above stops — against `main` at
`a4f10f5`. **24 commits**, 18 of them non-merge, five rounds merged after that run and
covered by no matrix until this one. Each argued individually that it moves no pixel, and
this project's rule is that the corpus is the instrument rather than the argument.

The corpus is the caller's, read-only at `/home/cl/projects/pdf-viewer`, revision
**`411063f9`** ("A key that cannot be hit, and a mask that masks itself"), copied once at
23:34 and not re-copied between columns. Adapter: **AMD Radeon 890M Graphics (RADV
STRIX1)**, Vulkan, 24 encode threads, release build, glyph quantum off. All eight runs in
**one copy of their tree, in one sitting, 2026-08-17 23:37 to 2026-08-18 00:08**, flipping
only the `[patch]` path.

## 1. The four rows

| lane, scale | base `1cd74c9` | `main` `a4f10f5` |
|---|---|---|
| CPU, scale 1 | 931 agree / 23 differ / 2 refused / 18 not comparable | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
| GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |

3 814 page verdicts compared — 956 at scale 1 and 951 at scale 4, twice each. A line is
printed only for a page that differs or is refused: **79 lines across the four rows** (25
at CPU 1, 27 at GPU 1, 14 at CPU 4, 13 at GPU 4), naming **37 distinct documents**.

## 2. Every page line that moved, with its cause

**None.** Not one of the 79 lines differs between the columns, in any field — page name,
verdict, mean, worst tile and its coordinates, differing fraction, SSIM, and the full text
of every refusal. The four `.out` files of the change column are **byte-for-byte identical**
to the base column's once the five wall-clock lines are removed, which is a stronger
statement than the per-field comparison and is the one actually run
(`/home/AI/final-matrix/strict.sh`).

That is the expected result and it is still worth having taken, because three of the range's
commits reach shipped source rather than rustdoc:

- **`333f80b`, ADR 0063's atlas round.** Two additive fields — `Counters::atlas_overflow_tiles`
  and `Limits::atlas_bytes` — plus the one behavioural-looking edit in the range:
  `Device::construct` now builds the `AtlasStore` *before* `Limits` so both read one
  arithmetic, and `settle_atlas` calls `AtlasStore::byte_size()` where it multiplied
  `dimensions()` itself. Same product, one site; the matrix is the check on "same product"
  over 974 documents rather than over the unit tests that assert it.
- **`cd93db3`, `geom.rs` into `affine.rs` / `segment.rs` / `shape.rs`**, and **`cc47ea7`,
  `outline.rs` into `outline/triangles.rs`.** Verified as pure moves before the run, by
  comparing the multiset of non-comment code lines across each split: `geom` differs by one
  removed `use` and sixteen added lines, every one of them a `mod`, a `pub use`, a
  `use super::…` or a `#[cfg(test)] mod tests {` brace pair; `outline` differs by twelve,
  all of the same kinds. **No logic line moved**, and the corpus says so independently.

The rest of the range does not reach a pixel by construction and the matrix confirms it:
`903d05e` and `de1c013` (doc corrections to ADR 0063's own text), ADR 0064's rare-lane round
(`b194c85`, `c2c7f4a`, `9d5f2af` — rustdoc on `Coverage` plus `tests/rare_lane_coverage.rs`),
ADR 0065's atlas-admission round (`3e1a863`, `2a4d2b5`, `81c5727` — two doc comments on
`CacheProspect`), the `resources.rs` and `encode/parallel.rs` split *declines* (`af8eb3e`,
`3e4b837` — module comments stating why each file is one thing), the real-display rounds
(`94d4cc2`, `8f6d2a6`, `d168f78`, `1b5aa21` — `examples/present_thread` and `PLAN.md` rows),
and `a4f10f5`'s `CLAUDE.md` Wayland correction.

**A second null worth recording**: every count in the base column reproduces the `1cd74c9`
column of the matrix above to the digit, although the caller's tree moved from `22ab57d4` to
`411063f9` between the two runs. That is luck rather than a guarantee — `HANDOVER.md`'s trap
about counts being statements about a revision stands — but it is evidence that the eight
runs above and the eight here are measuring the same population.

## 3. Their `REFUSED_AT_FOUR` ratchet fails, identically, in **both** columns

The CPU scale-4 row exits 101 in the base column *and* in the change column, with the same
assertion and the same two lists:

```
assertion `left == right` failed: the pages quorra refuses at 4× have changed
  left: ["bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
 right: ["bug1703683_page2_reduced.pdf", "bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
```

`diff` of the two stderr excerpts is empty. **That identity is the point**: both columns
contain ADR 0057, which moved `bug1703683_page2_reduced.pdf` off the 4× refusal list, and the
caller has not re-baselined. The failure is a debt of theirs carried forward from the matrix
above, not a difference this range introduces. The other six runs exit 0, including both
CPU scale-1 rows — which check `REFUSED` *and* the differing list, the strictest pair the
gate has — so nothing else in their ratchets moved either.

**The caller must still drop `bug1703683_page2_reduced.pdf` from `REFUSED_AT_FOUR`** when they
take the bump. This matrix does not change that instruction; it confirms it is the only one.

## 4. What was patched in the copy, and why

**Nothing but the `[patch]` block, in either column.** `head -n -5 Cargo.toml` is
byte-identical to the copy's `Cargo.toml.orig`, and both columns built their `render-quorra`
unmodified. This is a change from the matrix above, where the base column at `a4380e2`
predated ADR 0056 and needed two methods deleted from `crates/render-quorra/src/present.rs`
to compile at all: both ends of this range carry ADR 0056's presenter API, so their working
tree compiles against either. Cargo warns that the `quorra` patch entry is unused — their
workspace depends on `quorra-gpu` and `quorra-scene` only — which is expected and is not an
error.

The two columns are two distinct binaries and the record says which: all four base rows ran
`target/release/deps/corpus-fb39a089635d465a`, all four change rows ran
`corpus-44888576e3dd799c`. Cargo's metadata hash moves with the patched source, which is what
makes that a check rather than a coincidence.

## 5. Method, so the numbers can be re-taken rather than argued about

- `rsync -a` of their tree with `HANDOVER.md`'s seven excludes to `/home/AI/final-matrix/viewer`,
  once. **Never built in or written to `/home/cl/projects/pdf-viewer`.**
- Base column: `git worktree add /home/AI/final-matrix/quorra-base 1cd74c9`. Change column: a
  worktree at `a4f10f5`. Only the `[patch]` path differs between the two runs of each row.
- A private `CARGO_TARGET_DIR=/home/AI/final-matrix/target`.
- `cargo test --release -p render-quorra --test corpus -- --ignored --nocapture`, with
  `PDFVIEWER_QUORRA_COVERAGE=cpu|gpu` and `PDFVIEWER_QUORRA_SCALE=1|4`. Driver:
  `run-column.sh`; per-page comparison: `compare.py`; byte-level column diff: `strict.sh`;
  raw output: `out/{base,change}-{cpu,gpu}-{1,4}.{out,err,rc}` — all under
  `/home/AI/final-matrix`, which is removed after the numbers are written down.
- `--release` and not the caller's newer `gates` profile, so that these rows are comparable
  with the eight above, which predate it.

**No timing is published from this run.** The load average was 1.4 when the base column
started and 35.6 when the change column ended — something else on this desktop began during
the run — and `HANDOVER.md`'s rule stands: which pages refuse is arithmetic and
machine-independent, which lane is faster is not. The wall clocks in the raw output are
context and nothing else. **The verdicts are unaffected by that load**, which is exactly what
byte-identical output under a 25× swing in load demonstrates.

## 6. Recommended replacement for `PLAN.md`'s matrix

`doc/PLAN.md` is not edited from here. The block below is the recommended text. It goes
**immediately before** the `a4380e2 → 1cd74c9` matrix — that section lists matrices
newest-first — and **replaces neither of them**: the two cover different ranges and together
they cover `a4380e2 → a4f10f5`, which is what a push delivers.

One paragraph of the existing matrix is superseded and should go with the insertion. Its
closing —

> **Three commits are not in this matrix**: ADR 0063's atlas round (`333f80b`, `903d05e`,
> `de1c013`) landed while the runs were in flight. Its own scale-4 run reproduced the recorded
> verdict lists name for name, so nothing is known to move — but it has not had the four-row
> treatment, and it owes one before a push.

— is the debt this round pays, and it should be struck rather than left standing beside the
matrix that pays it.

> **The release matrix for `1cd74c9 → a4f10f5`** — the 24 commits merged after the matrix
> above was taken: ADR 0063's atlas round, ADR 0064's rare-lane round, ADR 0065's
> atlas-admission round, the `geom.rs` and `outline.rs` splits with the `resources.rs` and
> `encode/parallel.rs` declines, the real-display rounds, and `CLAUDE.md`'s Wayland
> correction. One copy of their tree at `411063f9`, RADV, both lanes, both scales, taken
> 2026-08-17 23:37 – 2026-08-18 00:08:
>
> | lane, scale | base `1cd74c9` | `main` `a4f10f5` |
> |---|---|---|
> | CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
> | GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
> | CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
> | GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |
>
> **Nothing moved.** All 79 printed lines across the four rows — 37 distinct documents,
> 3 814 page verdicts — are identical between the columns, and the four output files are
> byte-identical once the wall clocks are removed. That is the null the range's three
> source-touching commits needed: `333f80b`'s two additive `Counters`/`Limits` fields and its
> one-arithmetic `AtlasStore::byte_size`, and the `geom.rs` and `outline.rs` splits, which
> were separately verified as pure code moves. **The caller's `REFUSED_AT_FOUR` ratchet fails
> in both columns with the same two lists** — ADR 0057's `bug1703683_page2_reduced.pdf`, which
> is in both — so it is their outstanding re-baseline and not a difference here; the other six
> runs exit 0. Both columns built their `render-quorra` unmodified, which the previous
> matrix's base column could not.
>
> **`a4380e2 → 1cd74c9` and `1cd74c9 → a4f10f5` together cover everything a push delivers.**
> `doc/notes-release-matrix.md` holds both, with method, raw-output layout and the per-page
> evidence.

---

# The release matrix for `f378fa2 → 1adf479` (ADR 0066)

Date: **2026-08-18**. Base `f378fa2` — the first parent of ADR 0066's merge, and the commit
the matrix above stops one short of — against `main` at `1adf479`. **Two commits**, one of
them non-merge, and unlike the two ranges above **this one is known to move pixels**: ADR
0066 makes a soft mask a knockout element's *opacity* rather than its *shape*, so all five
`fs_shape` lanes now return §11.6.4.2's geometry met with §8.5.4's clip and nothing else. On
the round's own fixture the worst premultiplied change is **138 of 255**, inside a knockout
group and nowhere else. Neither matrix above covers it.

The corpus is the caller's, read-only at `/home/cl/projects/pdf-viewer`, revision
**`14a81f0d`** ("Two of three go home, and the fork keeps the remainder", 2026-08-18
00:23:01 +0200), its `doc/pdf.js` submodule at **`2ea8820d9`**, copied once at 00:31 and not
re-copied between columns. Adapter: **AMD Radeon 890M Graphics (RADV STRIX1)**, Vulkan, 24
encode threads, release build, glyph quantum off. All eight runs in **one copy of their
tree, in one sitting, 2026-08-18 00:35 to 01:02**, flipping only the `[patch]` path.

## 1. The four rows

| lane, scale | base `f378fa2` | `main` `1adf479` |
|---|---|---|
| CPU, scale 1 | 931 agree / 23 differ / 2 refused / 18 not comparable | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
| GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |

3 814 page verdicts compared — 956 at scale 1 and 951 at scale 4, twice each. A line is
printed only for a page that differs or is refused: **79 lines across the four rows** (25 at
CPU 1, 27 at GPU 1, 14 at CPU 4, 13 at GPU 4), naming **37 distinct documents**.

## 2. Every page line that moved, with its cause

**None.** Not one of the 79 lines differs between the columns, in any field — page name,
verdict, mean, worst tile and its coordinates, differing fraction, SSIM, and the full text of
every refusal. The four output files of the change column are **identical to the base
column's** once the wall-clock and process-identity lines are removed; the only residue in
the whole comparison is the position of cargo's "has been running for over 60 seconds"
progress line inside the CPU scale-4 row and the thread id in its panic header.

## 3. Why a null here is a *result* and not an absence of one

A null from a range that touches eleven shipped files, six of them shaders, is exactly the
claim this project has been burned believing. So the run was not left to speak for itself:
the reachability of the changed construction was measured directly, over the same 974
documents, by walking each page-one display list and counting `Command::Shaped` and
`Command::Group { knockout: true }`.

> 974 documents: 5 pages emit a `Shaped` command (6 in total, 1 carrying a soft mask),
> 16 pages emit a knockout group (29 in total), 142 groups overall.

So the corpus **does** reach the path ADR 0066 changes — this is not a null obtained by the
population being empty. The sixteen pages are `22060_A1_01_Plans.pdf`, `alphatrans.pdf`,
`bug852992_reduced.pdf`, `issue12810.pdf`, `issue13447.pdf`, `issue17069.pdf`,
`issue18032.pdf`, `issue1905.pdf`, `issue20062.pdf`, `knockout_groups_test.pdf`,
`knockout_inner_backdrop.pdf`, `knockout_isolated_overlap.pdf`, `knockout_nested.pdf`,
`knockout_nested_group_alpha.pdf`, `knockout_nonisolated_sparse.pdf` and
`knockout_smask.pdf`.

**And the mechanism is confirmed rather than assumed.** Of the six `Shaped` commands the
corpus produces, the `shape` half carries a soft mask in **none** of them. The one page whose
element is masked at all, `knockout_smask.pdf`, carries the mask on `object` and not on
`shape` — which is precisely the caller's `stated_shape` construction (their ADRs 0234 and
0327, `pdf-model/src/content/ext_gstate.rs`): they build a knockout element's shape by
*removing* the mask and the constant before it reaches us, and they refuse a knockout group
outright where an element may have been painted under `/AIS true`. ADR 0066's change is
therefore a no-op for every knockout element this translator can emit, **by construction and
not by luck**, and the corpus is the check on that sentence rather than the source of it.

Run on the sixteen knockout-bearing pages alone, both columns print the same three lines:

```
  differs: 22060_A1_01_Plans.pdf: mean 0.7927 worst tile 5.69 at (576, 768) differing 0.0641 ssim 0.98619
  refused: issue18032.pdf: this backend cannot draw a non-isolated knockout group: …
16 pages compared in 2.6s: 14 agree, 1 differ, 1 refused, 0 not comparable
```

**0 not comparable** is the load-bearing figure there: every one of the sixteen was actually
rendered by both backends and compared, so their agreement is a measurement and not a skip.

## 4. Their `REFUSED_AT_FOUR` ratchet fails, identically, in **both** columns

The CPU scale-4 row exits 101 in the base column *and* in the change column, with the same
assertion and the same two lists:

```
assertion `left == right` failed: the pages quorra refuses at 4× have changed
  left: ["bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
 right: ["bug1703683_page2_reduced.pdf", "bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
```

Identical, character for character. **That identity is the point**: both columns contain ADR
0057, which moved `bug1703683_page2_reduced.pdf` off the 4× refusal list, and the caller has
not re-baselined. It is the same outstanding debt the two matrices above record, carried
forward unchanged, and not something this range introduces. The other six runs exit 0,
including both CPU scale-1 rows — which check `REFUSED` *and* the differing list, the
strictest pair the gate has.

**The caller must still drop `bug1703683_page2_reduced.pdf` from `REFUSED_AT_FOUR`** when
they take the bump. Three matrices now say so; none of them changes the instruction.

## 5. What was patched in the copy, and why

**Nothing but the `[patch]` block, in either column**, and both columns built their
`render-quorra` unmodified. Cargo warns that the `quorra` patch entry is unused — their
workspace depends on `quorra-gpu` and `quorra-scene` only — which is expected and is not an
error.

The two columns are two distinct binaries and the record says which: all four base rows ran
`target/release/deps/corpus-13106c3911e0f0c0`, all four change rows ran
`corpus-0ae549c974002a4a`. Cargo's metadata hash moves with the patched source, which is what
makes that a check rather than a coincidence — and it is the check that matters most for a
range predicted to change nothing, because the failure mode of such a range is a `[patch]`
that silently did not take.

The reachability count in §3 came from one added file in the *copy*,
`crates/render-quorra/tests/knockout_reach.rs`, which walks display lists and renders
nothing. It is deleted with the copy; it is recorded here because the number it produced is
load-bearing and someone may want to re-take it.

## 6. Method, so the numbers can be re-taken rather than argued about

- `rsync -a` of their tree with `HANDOVER.md`'s seven excludes to `/home/AI/mask-matrix/viewer`,
  once, 540 MB. **Never built in or written to `/home/cl/projects/pdf-viewer`.**
- Base column: `git worktree add /home/AI/mask-matrix/quorra-base f378fa2`. Change column: a
  worktree at `1adf479`. Only the `[patch]` path differs between the two runs of each row.
- A private `CARGO_TARGET_DIR=/home/AI/mask-matrix/target`.
- `cargo test --release -p render-quorra --test corpus -- --ignored --nocapture`, with
  `PDFVIEWER_QUORRA_COVERAGE=cpu|gpu` and `PDFVIEWER_QUORRA_SCALE=1|4`; then the same with
  `PDFVIEWER_QUORRA_ONLY` set to the sixteen knockout pages, in both columns.
- `--release` and not the caller's `gates` profile, so that these rows are comparable with the
  sixteen above.

**No timing is published from this run.** The load average was 11.3 when the base column
started, and this desktop was doing other work throughout. `HANDOVER.md`'s rule stands: which
pages refuse is arithmetic and machine-independent, which lane is faster is not.

## 7. Recommended replacement for `PLAN.md`'s matrix

`doc/PLAN.md` is not edited from here. The block below is the recommended text. It goes
**immediately before** the `1cd74c9 → a4f10f5` matrix — that section lists matrices
newest-first — and **replaces neither of the two**: the three cover different ranges and
together they cover `a4380e2 → 1adf479`, which is what a push now delivers.

> **The release matrix for `f378fa2 → 1adf479`** — ADR 0066, the only range so far that was
> *expected* to move pixels: a soft mask is a knockout element's opacity, not its shape, and
> all five `fs_shape` lanes now return §11.6.4.2's geometry met with §8.5.4's clip alone,
> worth 138 of 255 on the round's own fixture. One copy of their tree at `14a81f0d`, RADV,
> both lanes, both scales, taken 2026-08-18 00:35 – 01:02:
>
> | lane, scale | base `f378fa2` | `main` `1adf479` |
> |---|---|---|
> | CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
> | GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
> | CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
> | GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |
>
> **Nothing moved**, and this time the null needed defending rather than merely reporting.
> All 79 printed lines across the four rows — 37 distinct documents, 3 814 page verdicts —
> are identical between the columns in every field. The corpus **does** reach the changed
> construction: 16 of the 974 documents emit a knockout group and 5 emit a `Shaped` command,
> and all 16 were compared with **0 not comparable**. The reason none moved is the caller's,
> and it was measured rather than taken on trust — in all six `Shaped` commands the corpus
> produces, the `shape` half carries no soft mask, because their `stated_shape` removes it
> and their ADR 0327 refuses a knockout group painted under `/AIS true`. ADR 0066 is a no-op
> for that translator by construction. **Their `REFUSED_AT_FOUR` ratchet fails in both
> columns with character-identical lists** — ADR 0057's `bug1703683_page2_reduced.pdf`, in
> both — so it remains their outstanding re-baseline; the other six runs exit 0.
>
> **`a4380e2 → 1cd74c9`, `1cd74c9 → a4f10f5` and `f378fa2 → 1adf479` together cover
> everything a push delivers.** `doc/notes-release-matrix.md` holds all three, with method,
> raw-output layout and the per-page evidence.

# A refusal that did not move: `issue18032.pdf`, ADR 0069 against ADR 0070

Date: **2026-08-18**. Not a range matrix — a **settlement**. Two rounds this week disagreed
about whether ADR 0069's `SceneError::KnockoutElementGroupUnsupported` turned a drawn corpus
page into a refused one, and a page moving from drawn to refused is the one thing the three
matrices above exist to catch.

**ADR 0069's round was right.** No corpus page moved from drawn to refused, at ADR 0069 or
anywhere in the week. ADR 0070's round reported in passing that its scale-4 exit 101 was
explained by `issue18032.pdf`, "which ADR 0069 began refusing two commits before mine"; that
sentence is wrong in **both** halves, and because the round reported it rather than writing it
down, nothing on disk contradicted it. It is retracted here and at the site in ADR 0070 where
the round's corpus section should have carried it.

## 1. The fact, measured at both revisions in one copy, the same sitting

One copy of the caller's tree at **`829d7faa`** (clean; only an untracked `.claude/`), RADV,
`Coverage::Cpu`, scale 4, release, `[patch]` flipped between two extractions of this tree.
Base **`3f6df72`** — the mainline commit *before* ADR 0069's merge `b5a09d7` — against
**`c443bc2`**, today's `main`, which carries ADR 0069 and ADR 0070 both.

`PDFVIEWER_QUORRA_ONLY=issue18032.pdf`, which the harness says itself skips the ratchets:

| | base `3f6df72` | `main` `c443bc2` |
|---|---|---|
| verdict | `0 agree, 0 differ, 1 refused, 0 not comparable` | **identical** |
| cargo exit | 0 | 0 |

The refusal line is **byte-identical** between the columns:

```
  refused: issue18032.pdf: this backend cannot draw a non-isolated knockout group: each
  element composites with the group's own initial backdrop, which a scene cannot retain
  beside the accumulation (ISO 32000-2 §11.4.6)
```

That text is **the caller's own**, `crates/render-quorra/src/scene.rs:154`, raised before a
`quorra_scene::Scene` is built. Quorra's variant reads "a group used as an element of a
knockout group needs the separate shape value §11.4.6 requires of it …", and that string
appears **zero** times in either column's output. The page is refused one crate upstream of
anything ADR 0069 can reach.

And the whole lane, same copy, same sitting, ratchets checked:

| | base `3f6df72` | `main` `c443bc2` |
|---|---|---|
| CPU, scale 4, whole corpus | 938 / 11 / 3 / 22 | **938 / 11 / 3 / 22** |
| page lines printed | 14 | 14, **all identical**, `diff` empty |
| refusals | `bug1721218_reduced`, `issue18032`, `issue1905` | the same three |
| cargo exit | 101 | 101 |

**Three refusals before and three after**, which is exactly what ADR 0069's own matrix
recorded.

## 2. What `REFUSED_AT_FOUR` actually lists, read rather than inferred

`crates/render-quorra/tests/corpus.rs:330` at the caller's `829d7faa` — and, checked by
`git show`, character-identical at `736e01f3` (ADR 0069's copy) and `14a81f0d` (ADR 0066's):

```rust
const REFUSED_AT_FOUR: [&str; 4] = [
    "bug1703683_page2_reduced.pdf",
    "bug1721218_reduced.pdf",
    "issue18032.pdf",
    "issue1905.pdf",
];
```

`issue18032.pdf` **is** in it, and has been since their five-hundred-and-twelfth session — it
is in their scale-1 `REFUSED` too (`[&str; 2]`, line 269). Their own doc comment names the
reason and dates it: it "joined this list in the five-hundred-and-twelfth session, but the hole
is the four-hundred-and-ninety-second's", their ADR 0327, `git log` 2026-08-08. That is eight
days before ADR 0069 existed.

## 3. The mechanism of the disagreement

The scale-4 exit 101 is real, pre-existing, and about a **different page in the opposite
direction**:

```
assertion `left == right` failed: the pages quorra refuses at 4× have changed
  left: ["bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
 right: ["bug1703683_page2_reduced.pdf", "bug1721218_reduced.pdf", "issue18032.pdf", "issue1905.pdf"]
```

`issue18032.pdf` is on **both** sides. The single element of the difference is
`bug1703683_page2_reduced.pdf`, which our ADR 0057 moved from **refused to drawn** — an
improvement the caller has not re-baselined, recorded already by all three matrices above and
by `PLAN.md` §s at lines 122 and 158. ADR 0070's round read the assertion's *failure* as
naming `issue18032.pdf` and attributed it to the newest thing in the tree. The page it should
have named went the other way.

Their tree did also move under us, as `HANDOVER.md`'s trap says it does, but not here: ADR
0069's copy at `736e01f3` read `937 / 11 / 3 / 23` on this row and today's at `829d7faa` reads
`938 / 11 / 3 / 22` — **one document migrated from *not comparable* to *agree* on their side**,
974 documents and 3 refusals unchanged in both.

## 4. Method note, because the first attempt at the base column was unsound

`git archive` stamps every extracted file with the **commit's** timestamp, not the extraction
time. The base commit is older than `main`, so its sources landed with mtimes *behind* the
artefacts the `main` column had just produced, and cargo rebuilt `quorra-gpu` but declared
`quorra-scene` — the crate that holds the refusal — fresh. The first base run therefore
measured `main`'s scene builder wearing the base's name, and it is discarded. Every column
above was taken after `find <patch dir> -type f -exec touch {} +`, and the check that the swap
took is `Compiling quorra-scene` appearing in the log rather than a metadata hash: holding the
`[patch]` path stable across columns is what makes the *test binary* hash identical, so this
round could not use ADR 0069's two-hashes proof and used the compile lines instead.

## 5. The correction, and three defects found beside it

ADR 0070's aside was never written into any file — there is no `doc/notes-thin-mark-condition.md`,
and neither `doc/adr/0070-…md` nor `doc/notes-thin-mark-options.md` contains the claim. **The
reason is that ADR 0070's corpus section was never transcribed at all**: line 136 of that ADR
is the literal string `<<MATRIX>>` and line 206 is `<<DEFECTS>>`, two unreplaced placeholders
sitting on `main`. That is principle 1's "no placeholder left in merged code" in a document,
and it is also the direct cause of this disagreement — a claim that lives only in a round's
report is a claim no later round can check. A dated retraction is placed at the `<<MATRIX>>`
site; the matrix itself is **not** filled in from here, because this round measured one lane
and one scale and inventing the other three rows is the failure these notes exist to prevent.

> **Closed 2026-08-18.** Both placeholders are gone: a later round re-took all four rows and
> re-forced the defects, and `doc/notes-thin-mark-condition.md` is the round notes this
> paragraph says did not exist. It confirms this section's CPU scale-4 row unchanged at
> `938 / 11 / 3 / 22` in both columns, and finds two further things — the scale-1 device row's
> `930/25/2/17 → 932/23/2/17` holds, and one of ADR 0070's own claims does not.

**Two CI gates were red on `main` when this round started, both from the same commit
(`787b830`) and neither pre-existing before it.** `cargo fmt --all --check` failed on two
`assert!` chains in `crates/quorra-gpu/src/encode/thin.rs`, and
`RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` failed with four
`clippy::similar_names` errors in `crates/quorra-gpu/tests/thin_marks.rs` — `wide_cpu` beside
`wide_gpu`, `thin_cpu` beside `thin_gpu`, twice. Both are fixed here, mechanically:
`cargo fmt --all`'s own output, and a rename to the lane names that ADR 0070's prose already
uses (`…_on_processor` / `…_on_device`), which reads better than the abbreviations it replaces.
No assertion, bound or fixture changed. The pattern worth naming is that all three of this
round's findings — two red gates and two unreplaced placeholders — are a round that ran its
verification and reported the result without the result having been produced.
