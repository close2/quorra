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
