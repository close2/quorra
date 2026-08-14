# Handover

Read `CLAUDE.md` first, then `doc/RENDER_LIBRARY.md` (the brief this library exists to
satisfy) and `doc/PLAN.md` (the design as currently believed, newest entry first).
**`PLAN.md`'s "Where we are" carries the numbers and the narrative; this file carries the
state of play, what to do next, and the traps.** A lesson belongs in exactly one place: in
an ADR if it is a decision, in `PLAN.md` if it is where we are, here if it changes how you
*work*.

## State of play

Nine milestones are done; the swap landed on 2026-08-03 and the caller consumes this
library as a git dependency, pinned by their `Cargo.lock`.

**`a35dc70` is what the owner pushed.** Everything after it is local, and the ADRs say what
each is: 0032 and 0033 answer the caller's §14.2, then the sheet packer (0034), the size
hint (0035), layers sized to their plans (0036, in two commits), a scissor fix that 0036
made necessary (no ADR — a defect), masks sized to their plans (0037, in two commits), and
one accumulator per plan instead of a ping-pong pair (0038, in two commits), the root sized
like every other plan (0039), the compositor's two pipelines moved into the warm set after
ADR 0035's first-frame number failed to reproduce (0040), a child the encoder drops when its
clip leaves it nothing to contribute (0041), the round cap that was drawing the
half-disc *inside* the stroke — the caller's `QUORRA_FEEDBACK.md` §21.1 — and a WGSL
compile failure turned from a silent test-suite hang into a refusal that names its span
(0042).

On 2026-08-13 the tree had a full review — code, plan, and both sides of the caller
conversation — and the "What to do next" list below is its result. The review's one
structural finding: the work itself is disciplined, but the brief's own success metric
(§6.2, surface-tier) has never been measured, and the caller sync is lagging the local
work. `examples/surface_measure.rs` was built for the first of those; it is the only
code the review added.

**What the caller must do to take it** is written for them in
`/home/cl/projects/pdf-viewer/doc/QUORRA_UPGRADE.md`: one line for `GroupSpec::compose`, a
test of theirs that fails by design, and the `Command::Shaped` translation that their four
refused pages need. Their `doc/QUORRA_FEEDBACK.md` is the other half of that conversation
and is worth reading before answering anything in it: a heading there can be stale in
either direction, and §13 sat marked *open* for eleven of their rounds after ADR 0023 had
answered it.

## What to do next, in this order

The order is the 2026-08-13 review's (a full pass over the tree, the plan, the caller's
feedback and the code-health of every crate), and its reasoning is one sentence: **the
metric the brief judges us by has never been measured in its own terms, and the caller
conversation is going stale while local work stacks up — both cost less to fix than one
more optimisation round, so they go first.**

### 1. The two surface-tier numbers — **done** (2026-08-14), and what they left behind

The owner ran `examples/surface_measure.rs` on RADV at the real display, twice, and the
warm-set fix is landed and verified: ADR 0043, the numbers in `PLAN.md`'s entry for the
date, **compiles: none on eight presenting first frames of eight** on the re-run. The
owner also left a trigger loop for this instrument — `touch tmp/start-measurement` in
this repository and the loop rebuilds the working tree and runs `surface_measure` on
the real GPU, writing `tmp/output.stdout.txt` — so a measurement no longer waits for a
hand-off (confirm the loop still runs before relying on it).

What the numbers re-rank, recorded here rather than jumped on:

- **The §6.2 gap is CPU recording, not the device.** A steady presenting dense-text
  frame is 2.84–3.38 ms against the 2.0 ms success bar, and the device's share is
  0.11–0.13 ms; recording (hash lookups, instance writes — 4 320 commands) is
  1.90–2.32 ms of it and submit/device-wait another 0.55–0.71. Before designing
  anything: profile what recording actually spends its time on, and price **encode
  reuse across identical frames** — a retained scene redrawn unchanged pays full
  recording today, and a page a person is reading is exactly that frame after frame.
- **The artwork shape's steady frame is geometry: 37–47 ms of residue-clip
  re-rasterisation** every frame, on the corpus's p99 clip shape. That is item 5's
  seam with a steady-state cost beside its refusal count, and §15's question measured
  from our side — the three answers belong together in the sync round.

### 2. A sync round with the caller — cheap, and overdue

The viewer pins `a7babab`, five commits behind, and those five include the §21.1 round-cap
fix their own §22.7 predicted and wants to re-run first thing after a bump. Twenty-five
commits are local. Their `QUORRA_FEEDBACK.md` has three sections waiting on this side,
and two of them cost an afternoon:

**All three are drafted** in `doc/feedback-answers-draft.md` (2026-08-14) — a draft for the
owner to carry across, since we never edit their tree. What each one says:

- **§15** asks whether the coverage lane bounds a clipped fill by its clip's extent, and
  offers to close itself if yes. It is yes: every coverage tile is `shape ∩ clip ∩
  target` (`encode.rs:2151` `visible_tile`, `:2099` `coverage_tile`, `:1281`
  `residue_intersection`, `:206` for the image lane). Its secondary ask — can the scene
  say "this fill is bounded by this rectangle" — is answered by the same fact: a
  rectangular clip collapses to a device rectangle at `encode.rs:433` and never becomes a
  mask, so the clip already *is* the bound. The draft cross-references item 5, because
  §15 and the tiling ceiling are the same seam seen from two sides.
- **§19** is measured: `examples/rect_lane.rs`, numbers and interpretation in `PLAN.md`'s
  2026-08-14 entry. Median page: microseconds, so nothing. A page of thousands of
  rectangles: 0.6–1.8 ms. Which of those the corpus actually has is a row their profile
  does not carry, and the draft asks for it before either side writes a recogniser.
- **§22.5** (a process note, not code): the recommendation is *document*, because the name
  `layer_textures` always meant what it still means and what was wrong was a derived claim
  in our own rustdoc that theirs inherited. The rustdoc is corrected in the same commit,
  and the draft states the rule — rename when the *unit* changes, date a correction when a
  derived claim does, and name the counter in the ADR that moves its value.

Push (or hand the owner the push), deliver the answers, and let their corpus re-baseline
absorb ADRs 0036–0043 in one round instead of six.

### 3. §21.2 — a tiny outline flattens to its inscribed polygon

The caller's other live defect report: at d ≤ 1 device pixel a circle loses 36 % of its
area to a quarter-pixel flattening tolerance, filed as a report (§10.7.3 licenses a
device tolerance) with a suggestion — a tolerance relative to the shape, or a floor of a
few segments per turn. Small, self-contained in `raster.rs`, and its evidence already
exists: the caller's `sub_pixel_marks` example rows, plus a corpus run that must not
move except where it should. A good first item for a fresh session.

### 4. Split `encode.rs` along its stated seams — **before** the tiling work

2 600 production lines, one ~1 640-line `impl Encoder`, eleven of the workspace's
thirteen `too_many_arguments` allows, and a module comment that enumerates
responsibilities instead of naming one. The seams are already visible in the file:
`ClipResolver`, `ScratchPacker`, the rare-case lane encoders, and the thrice-repeated
verbatim `ChildOp` construction (`encode.rs:1134/1603/1656`) that wants one named
helper. ADR 0042 just did the same to `pipeline.rs` (779 → 460 plus two named halves);
this is the same move. It goes before item 5 **because item 5 lands exactly there** —
the tiling logic is `encode.rs`'s tenant, and redesigning it inside the current file is
how threading-style growth continues.

### 5. A page-sized coverage tile per clipped shape — **not** multi-sheet passes

Three pages at 4× and one at scale 1 refuse with `ScratchExhausted` — the coverage sheet
against the adapter's 16 384 limit, which is a different ceiling from the frame budget and
one the pane work cannot reach. **It is the only reason any frame of the corpus is
refused.** (The fourth refusal at 4×, `22060_A1_01_Plans.pdf`, is a third budget again: 548
MB of resident images against `max_resource_bytes`, refused at upload rather than at the
frame.)

This item used to read "a frame would have to use more than one sheet". **Measure before
building that**: instrumenting the packer's refusal (six lines in `ScratchPacker::reserve`)
says the three pages have placed **194, 240 and 253 MB of tiles** against a 268 MiB budget
when they run out of *height*, and the tile that does not fit is 2 448 × 3 168 or
4 763 × 7 204. A second sheet takes `bug1721218_reduced` to 287 013 092 bytes — it refuses
again, on bytes — and lets the other two draw at a quarter of a gigabyte of per-frame
coverage upload each. That is a page "drawn" at a cost §6.2 would call a failure.

So the ceiling that bites is not the sheet's dimension. It is that a clipped shape becomes
one coverage tile of its own device bounds, and at 4× a full-page clipped shape is a
full-page tile. The work is on the *tiling* side — ADR 0028's panes are the nearest
existing mechanism — and it is worth its own measurement before its own design.

### Small debts, none blocking — fill-in work between the numbered items

The 2026-08-13 review's code-health pass, so a fresh session can pick one without
re-auditing. Each is one sitting:

- **A solid fill of a rectangular outline does not take the rectangle lane**, and a
  shaded one does. `StoredOutline::rect_hint` is computed for every outline at upload
  (`resources.rs:150`) and read at `encode.rs:1473` — but only on the shading arm; a solid
  paint returns into `fill_solid` at `encode.rs:1458` first and takes the atlas or the
  coverage path. Output is byte-identical either way (`examples/rect_lane.rs` checks it),
  so this is symmetry rather than correctness — but it is worth **0.13–0.49 µs a
  rectangle**, it is where most of the caller's §19 lives, and it is four lines against a
  recogniser they would otherwise write. Take it with a corpus run: it moves which lane a
  page takes, and `HANDOVER`'s "the corpus is part of a change" trap applies exactly.
- **`SceneError` is a tenant of `scene.rs`** (~180 lines at `scene.rs:392-570`) while
  serving the whole crate — CLAUDE.md names this exact anti-pattern, and `quorra-gpu`'s
  `error.rs` is the in-tree example of the fix.
- **`pipeline/spec.rs:112`** carries a bare `#[allow(clippy::too_many_lines)]` on a
  138-line `fn of` — split into named phases or write the reason.
- **`composite.wgsl:142` and `:174`** (`hard_light_channel`, `soft_light_channel`) are
  missing the `§11.3.5.2` citations their siblings carry.
- **`soft_mask_at` and `fs_shape` are copied across five shaders** with a comment
  promising textual sameness and nothing enforcing it — a test that reads the shader
  sources and compares the extracted bodies makes the promise checkable.
- **`resources.rs:32`'s `#[allow(dead_code)]` reason** ("consumed by the lanes from
  M4/M5") describes a state that shipped; refresh or remove.
- **`criterion` is pinned in the workspace with no bench anywhere** — add the benchmark
  the pin promises or drop the pin.
- **The fuzz generator's vocabulary stopped at M5** (`fuzz_scene.rs`: `rect`, `fill`,
  `stroke`, `clip`, `group`) — images, shadings, masks and the compose vocabulary are
  where the surface area grew since, and extending `random_ops` is cheap.
- **`device.rs` hosts ramp sampling** (`sample_ramp`, `ramp_color_at`,
  `RAMP_RESOLUTION`) outside its stated responsibility — a candidate seam whenever that
  file is next open.

### The memory path is finished, and this is what finished looks like

ADRs 0036 to 0039 took a frame's internal memory apart and left no term with an obvious
factor in it. Every layered frame of the corpus at 4× prices **1 325.5 MB** in total, and
the heaviest single one — 93.0 MB — is a page whose root marks its whole target, so that is
the page's own size and nothing else. Nine frames in ten are flat and allocate no layer at
all.

Before opening this seam again, price it first. The probe is six lines in `Device::render`
— a `Region::of(root.bounds)` and an `eprintln!` — in a `git worktree`, and it is what
turned 0039 from a paragraph saying "not worth it" into a 41 % reduction.

### Recorded and deliberately not taken

Each of these has an ADR that states the measurement and why it was left:

- **the census cannot see how often a shape is placed** at phase granularity (0029);
- **a pane is cut in sheet order** rather than by what packs tightest (0028);
- **tiles are packed in encounter order**, and sorting them needs positions assigned after
  the walk — a two-pass encode (0034);
- **`warm_for` warms a target-sized texture, and that is worth 0.06 ms** — the whole
  budget of the mechanism, because RADV commits a texture's memory when the GPU first
  touches it and not when it is allocated. ADR 0040 re-measured 0035's 24.7 ms → 10.3 and
  could not reproduce it in five configurations, including the one where the pool takes the
  warmed texture. So the pool goes on matching on **exact** extent: serving a smaller plan
  from a larger texture would buy 0.06 ms and would put a viewport in every pass, which is
  ADR 0036's hazard with a new name;
- **`warm_for` does not draw a warm frame**, which is the only warming that measured
  anything at all: at the host's size it costs 4.7 ms at 1 191 × 1 684 and 22.3 at
  2 448 × 4 752 on the calling thread to save between 0.5 and 2, and the part of the benefit
  that is real is size-independent — a 64 × 64 warm frame buys three quarters of it. What
  was attributable in it (two pipeline compilations) is in the warm set instead (0040);
- **the mask's transparent value is computed on the CPU as well as in `reduce.wgsl`**,
  rather than fed into the reduce so the two cannot disagree — an independent
  implementation a test compares is stronger evidence than an agreement by construction
  (0037);
- **a non-isolated group still takes its parent's region** rather than its own, now that
  the blit it is copied by has an origin: §11.4.4's interpolation is stated over the whole
  of the group's buffer, so shrinking it is a clause question and not a plumbing one
  (0038);
- **a culled child's plan stays in `Encoded::layers`**, unreferenced, because that list is
  indexed by `ChildOp::layer` *and* by `MaskPlan::root` and shifting either is a wrong page
  rather than an error. It is priced at nothing — nothing reaches it — but `is_flat` asks
  whether `layers` is empty rather than whether any plan is reachable, so a page whose only
  group is clipped away renders through a root accumulator instead of straight into the
  target (0041). The same shape of question is open for a culled group's soft mask, which
  still realises;
- **the hand-off gained a branch per pixel of the target**, which is the biggest thing a
  frame touches and is real work added to every layered frame; nothing surfaced above the
  run-to-run spread on the corpus, and that is the honest statement rather than a claim
  either way (0039).

## Traps

**Wall clocks lie under load, and this machine is somebody's desktop.** A first-frame
improvement measured at 24.7 ms → 10.3 on a quiet machine re-measured as 19.9 → 20.0 an
hour later with Firefox and a slicer running; the load average was 12. Check `uptime`
before believing a timing, prefer minima over means, and make the *test* assert a property
— a device warmed for one size, another, or none draws the same bytes — rather than a
duration. **That improvement was not real** (ADR 0040): 40 round-robin rounds, one device
per process, could not find it at either page size, and the allocation it was credited to
takes 0.06 ms. When a difference is a difference of wall clocks, run the configurations
round-robin so drift falls on all of them, and look for a *direct* span — a duration inside
`Timings`, an `Instant` around the one call — before believing the subtraction.

**A first-use pipeline compile is invisible to "wait a while and try again".** §9 ruled
compilation out of its first-frame excess because settling a second between bring-up and the
first render changed nothing — which is true of the warm-up thread and says nothing about a
pipeline compiled *inside* the frame that first needs it. Two of them were 2.6 ms of a
layered first frame for three ADRs, and `Timings::phases` had been reporting them by name
the whole time. When a first frame is slow, read its phases before theorising about memory.

**The caller's corpus is part of a change, not a check after it.** Layers sized to their
plans passed 208 unit tests and moved 31 corpus pages off *agree*, then 12 more, before it
was right: 884, 903, 915. Two defects, neither reachable by any test in this tree.

**Always run the baseline in the same copy, on the same day.** Verdict counts are stable
across copies of *one* viewer revision and that tree changes under you: ADR 0036 recorded
915 / 37 / 5 at scale 1, and a copy taken a day later read 919 / 37 / 1 for the same quorra
commit. Nothing regressed — their tree moved. So a count quoted in an older ADR is not a
baseline; a `git worktree` at the base commit, patched into a second copy of the viewer and
run the same hour, is. Compare the **per-page lines**, not only the totals: 0037's evidence
that it moved memory and not pixels is that all 37 differing pages matched to the last digit
of every mean, worst tile and SSIM.

**Stage "every stage learns an offset" changes at zero first.** Panes (0028) shipped with
one of three subtractions missing and drew nothing at all for every band after the first.
The same change done as *plumbing at zero, verified by equality, then the value* caught a
vertex-only uniform binding immediately and cost one extra commit.

**A WGSL compile error now fails the suite by name** (ADR 0042). It used to hang it: a
reserved keyword (`from`) in `blit.wgsl` panicked the warm-up thread inside wgpu, and
every one of the nineteen test files that calls `wait_until_warm` then waited on a
`Condvar` with no notifier left alive — `cargo test` looked like an infinite hang with no
output at all. Today the same mistake ends the run in about a second with the WGSL span in
the failure message. If you ever see a silent hang again, the shape to suspect is the same
one: a `Condvar` whose only notifier can leave without notifying.

**A refusal is arithmetic, a fidelity difference is not.** Which pages refuse is
machine-independent and can be reasoned about; which lane is faster is a property of the
processor *and* the adapter together, so never publish a crossover as a constant — the two
ADRs that tried (0027, then 0028) both had to delete one.

## Running the caller's corpus

Never build in `/home/cl/projects/pdf-viewer` and never edit it: the owner works there, and
its `[patch]` and lock are often their work in progress. Copy it instead —

```
rsync -a --exclude=target --exclude=corpus-cache --exclude=fuzz --exclude=tmp \
      --exclude=.git --exclude='doc/pdf.js/.git' /home/cl/projects/pdf-viewer/ <scratch>/viewer/
```

— the excludes are not optional (that tree is 100 GB), then append a
`[patch."https://github.com/close2/quorra"]` block pointing `quorra`, `quorra-gpu` and
`quorra-scene` at `crates/*` here, and run

```
CARGO_TARGET_DIR=<scratch>/target cargo test --release -p render-quorra --test corpus \
  -- --ignored --nocapture
```

`PDFVIEWER_QUORRA_ONLY=a.pdf,b.pdf` narrows it (the ratchets are then *not* checked),
`PDFVIEWER_QUORRA_COVERAGE=cpu|gpu` picks the lane and `PDFVIEWER_QUORRA_SCALE=n` the
magnification. **Timings are not comparable across copies, and neither are verdicts** —
compare before and after inside one copy, flipping only the `[patch]` between a
`git worktree` at the base commit and the working tree.

## This machine

Arch, AMD Ryzen AI 9 HX 370 with a Radeon 890M. Two adapters and the difference is a
feature: RADV is the default and llvmpipe is pinned by name in most tests, so CI can run on
a software rasteriser. Claude Code runs as user `AI` through the `coders` group and has no
X cookie — headless is fine, a window on the owner's display is not.
