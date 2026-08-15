# Handover

Read `CLAUDE.md` first, then `doc/RENDER_LIBRARY.md` (the brief this library exists to
satisfy) and `doc/PLAN.md` (the design, and what is true today). **`PLAN.md`'s "Where we
are" carries the current numbers; `doc/history/` carries how they got there; this file
carries what to do next and the traps.** A lesson belongs in exactly one place: in an ADR
if it is a decision, in `PLAN.md` if it is where we are, in `doc/history/` if it is what
happened, here if it changes how you *work*.

## State of play

Nine milestones are done; the swap landed on 2026-08-03 and the caller consumes this
library as a git dependency, pinned by their `Cargo.lock`.

**The 2026-08-15 release round is pushed** (`a64a908`): ADR 0047 (a document's rectangles
reach the rectangle lane), ADR 0048 (`RetainedScene`), ADR 0049 (a clip chain's residue
rasterised once, and the border-cut defect under it), ADR 0050 (a page too large for its
atlas stops re-encoding itself every frame), ADR 0051 (three files split along their seams),
ADR 0052 (the readback gate counts instead of timing), and one clause defect: a blended
stroke inside a knockout group was blended where §11.4.6 replaces.

**Local is ahead of that by three rounds**: the function paint (ADR 0053); the 2026-08-15
second round — ADR 0054's parallel geometry phase, the `device.rs` split (2 283 lines → 235
over eleven submodules), and type 4's knockout and retained-frame tests; and the debt round
below. Whether the first two reached the remote could not be checked from the `AI` user,
which has no key for `git@github.com:close2/quorra.git`; `origin/main` in this checkout
points at the second round's tip, which is evidence and not proof.

**The 2026-08-15 debt round** closed the small debts this file listed, in three worktrees at
once: a generated function shader's compile cost measured here (`examples/function_compile.rs`),
the workspace's eleven rustdoc warnings fixed *and gated*, a uniform's byte offsets checked
against the WGSL struct they mirror (`src/shaders/layout.rs`), `device.rs`'s three leftovers
closed, six clause-derived unit tests on `device/ramp.rs`, and `tests/` reorganised —
`retained_frame.rs`'s 1 139 lines into five files along its own seams, plus `tests/common/`.
It moved no pixel and needed no corpus run. **It also found one clause defect that does move
pixels** — item 3 below.

**One thing the release round owes the caller and they have not done yet.** Their corpus
ratchet lists the pages that differ from their oracle by name, and ADR 0049 moved
`issue2177.pdf` onto the oracle — so their `every_corpus_page_agrees_with_the_cpu_oracle`
**fails on the pushed commit** until they re-baseline that list. The verdict counts are
right (931/23/2/18) and every other page line is identical; the ratchet is doing its job.
`pdf-viewer/doc/QUORRA_API_2026_08_15.md` §0 tells them to expect it.

**The round is corpus-clean and the bump is ready.** Base against merged, one copy of their
tree, one hour, both coverage lanes: scale 1 goes 930/24 → 931/23 on the CPU lane and
928/26 → 929/25 on the GPU lane, scale 4 is unmoved at 936/10/5/23, and exactly two page
lines change out of 956 with every other line identical to the character. `PLAN.md` carries
the matrix.

**And they are already blocked on it.** Their working tree uses `RetainedScene` and
`EncodeSource`, which do not exist at the `87898c6` their own `Cargo.lock` pins — the corpus
gate cannot even be built against their pin. Their adoption of ADR 0048 is done and waiting
on a push from this side, which reverses who is holding the round up.

**What the caller must do to take it** is written for them in
`/home/cl/projects/pdf-viewer/doc/QUORRA_UPGRADE.md` and, for the retained encode,
`QUORRA_RETAINED_FRAME.md`. Their `doc/QUORRA_FEEDBACK.md` is the other half of that
conversation and is worth reading before answering anything in it: a heading there can be
stale in either direction, and §13 sat marked *open* for eleven of their rounds after
ADR 0023 had answered it.

## What to do next, in this order

Three items, and the ordering reason is one sentence: **the caller conversation is the older
problem and costs an afternoon, the tiling seam is the only work left with milliseconds in
it and wants its own measurement before its own design, and the ramp's boundary is a clause
we read wrong — cheap to fix, and it moves bytes, so it costs a corpus round rather than an
edit.**

### 1. The adoption round with the caller — cheap, and overdue

Three sections of their `QUORRA_FEEDBACK.md` wait on this side, and **all three are
drafted** in `doc/feedback-answers-draft.md` — a draft for the owner to carry across, since
we never edit their tree:

- **§15** asks whether the coverage lane bounds a clipped fill by its clip's extent, and
  offers to close itself if yes. It is yes: every coverage tile is `shape ∩ clip ∩ target`
  (`encode.rs` `visible_tile` and `coverage_tile`, `encode/clips.rs` `residue_intersection`,
  `encode/rare.rs` for the image lane). Its secondary ask — can the scene say "this fill is
  bounded by this rectangle" — is the same fact: a rectangular clip collapses to a device
  rectangle and never becomes a mask, so the clip already *is* the bound. The draft
  cross-references item 2, because §15 and the tiling ceiling are one seam seen from two
  sides.
- **§19** is measured (`examples/rect_lane.rs`). Median page: microseconds, so nothing. A
  page of thousands of rectangles: 0.6–2.1 ms. Which of those their corpus actually has is
  a row their profile does not carry, and the draft asks for it before either side writes a
  recogniser. ADR 0047 has since made the ask worth a third of what it was.
- **§22.5** (a process note): the recommendation is *document*, not rename — `layer_textures`
  always meant what it still means, and what was wrong was a derived claim in our own
  rustdoc that theirs inherited. The rule the draft states: rename when the *unit* changes,
  date a correction when a derived claim does, name the counter in the ADR that moves its
  value.

Deliver the answers and let their corpus re-baseline absorb the twenty commits in one round.
The draft's release-note section doubles as what the bump delivers.

### 1b. The function paint — **built**; what is left of it

ADR 0053 is accepted and the lane draws (`src/function/`, `src/pipeline/function.rs`,
`src/encode/function.rs`, `src/compose/function.rs`). Read its **amendment** before quoting
its §3: `Agreement::Exact` was renamed `Bounded` because WGSL permits reassociation and
fusion, so bit-exactness was never ours to promise. The caller's answer document carries the
same correction.

Three of the four things this item listed are done. `tests/function_knockout.rs` draws
§11.4.6's replacement over a function paint and measures it in the pixels against an ordinary
group as control, with both of its gates verified able to fail by forcing the defect each
names; `tests/function_retained.rs` replays a function op byte-identically and holds the
release path, which is where principle 6 has teeth in this lane; and **the compile now has a
number of ours** — `examples/function_compile.rs`, 8.25 ms on RADV and 6.88 on llvmpipe for a
482-instruction program at the witness's length, minima of twelve round-robin rounds over
three alternating runs per adapter at load 5.1–8.6. The spike's 6.3 ms holds. The finding
worth carrying: **a one-instruction program still costs 2.0–2.7 ms**, which is
`function_lane.wgsl` parsed and built rather than anything generated — so the fixed half is
where to look if this ever needs to be cheaper, and the cost is mildly superlinear above it
(~10 → ~17 µs per instruction, the same shape on two independent compilers).
`doc/notes-function-wiring.md` §4.6–§4.8 carry all three.

What is left is `gt`/`ge`/`lt`/`le` comparing a boolean numerically where PLRM3 raises
`typecheck`, which is ADR 0053 §3.2's open question with the caller and theirs to answer;
and three gaps §4.5 now names, each found by reading rather than by a failure: **a clip and a
soft mask over this paint are never anything but 1** anywhere in the tree (the other two
factors of `base_weight` are unobserved), **no function test runs `Coverage::Gpu`**, and
ADR 0025's `DestOut`/`Plus` stages are compiled and selected but never drawn.

Do not build the interpreter shape. It is 133 ms against 0.060, it costs 596 ms–4.5 s of
cold compile against 6.3 ms, and at 4× it lost the device.

### 2. A page-sized coverage tile per clipped shape — **not** multi-sheet passes

*(This was **item 5** while three finished items still stood above it, and ADR 0048 and
`doc/feedback-answers-draft.md` both cite it under that number.)*

Two pages at 4× refuse with `ScratchExhausted` — `bug1703683_page2_reduced.pdf` and
`issue1905.pdf`, the coverage sheet against the adapter's 16 384 limit, a different ceiling
from the frame budget. **It is the only *budget* that refuses a frame we could otherwise
draw.** The other three refusals of their corpus are each something else: 548 MB of resident
images against `max_resource_bytes` (`22060_A1_01_Plans.pdf`, at upload rather than at the
frame), and two clause refusals that are correct — a four-component blending colour space
and a non-isolated knockout group. Counted on 2026-08-15; the earlier "three at 4× and one
at scale 1, the only reason any frame is refused" was their tree at an earlier revision,
and was too strong in both halves.

This item used to read "a frame would have to use more than one sheet". **That was measured
and refused**: the pages have placed 194, 240 and 253 MB of tiles against a 268 MiB budget
when they run out of *height*, and the tile that does not fit is 2 448 × 3 168 or
4 763 × 7 204. A second sheet takes `bug1721218_reduced` to 287 013 092 bytes — it refuses
again, on bytes — and lets the other two draw at a quarter of a gigabyte of per-frame
coverage upload each. That is a page "drawn" at a cost §6.2 would call a failure.

**ADR 0049 took the other half of this item and left this one untouched, deliberately.**
The residue is no longer re-rasterised per command — that was 17.3 ms of artwork's 65.6 ms
of geometry, *not* the whole of the "35 ms of a 43 ms frame" this item used to claim — and
artwork's encode geometry went 37.8 → 28.9 ms. But a *region* is host memory that never
reaches the sheet, and `Counters::tiles` is unchanged on every archetype, which is the
evidence that no refusal moved.

So the ceiling that bites is not the sheet's dimension: it is that a clipped shape becomes
one coverage tile of its own device bounds, and at 4× a full-page clipped shape is a
full-page tile. The work is on the *tiling* side — ADR 0028's panes are the nearest existing
mechanism.

*(A paragraph filed here since ADR 0048 — a page whose glyph tiles overflow the atlas
re-encodes on every frame — has been **removed rather than moved**. ADR 0050 did it, and
it was never this seam: the glyph atlas has nothing to do with residue-clip tiling. The
claim was also too broad, which is its own lesson and is now a trap below.)*

### 3. A ramp's subdomain boundary belongs to the subfunction that starts there

Found on 2026-08-15 by the first unit tests `device/ramp.rs` has ever had, which is the
argument for writing them: the function had been exercised only through a rendered frame, and
a rendered frame could not see this.

§7.10.4's subdomains are **closed on the left**, so a bound belongs to the subfunction
starting there, and a coincident stop pair should take the *later* stop's colour at exactly
that point. `ramp_color_at` reaches the earlier stop's interval first (`t <= stop.offset`
with `u == 1`) and returns the earlier colour; the `span <= 0.0` branch that was meant to
implement the clause is **unreachable for a validated ramp**. It is reachable in output: a
boundary whose offset lands on the sampling grid puts one texel of 4 096 on the wrong side
(verified — offset `2048/4095` gives texel 2048 red where the clause says blue), and a
coincident pair at 0 or 1 always does.

**The fix is a `<` for a `<=`.** It was deliberately not taken in the debt round, because it
moves bytes and "the corpus is part of a change, not a check after it" — so it wants a round
with the corpus in it, which is an hour rather than a minute. `device/ramp.rs`'s comments
carry the reasoning and the test asserts the two sides of the boundary while saying in its
own comment why the bound itself is not asserted.

### Small debts, none blocking

- **`encode.rs` is 2 421 lines**, and it is now the only source file far past the ~500-line
  smell: `device.rs` is 235 lines over eleven submodules, and the debt round below closed the
  rest. Splitting it is the next structural round, and it wants to happen *before* the
  recording and tiling work rather than under it.
- **`SceneBuilder::image` refuses a bad image alpha with `SceneError::InvalidGroupAlpha`**
  — a shared variant. Public API, so it is a bump's business rather than a refactor's.
- **`tests/shader_copies.rs` keeps its own `include_str!` list of the shader files**, which
  since the layout gate is the second such list beside `src/shaders.rs`. An integration test
  cannot reach a private module, so closing it means deciding whether that list is public.
- **What the test reorganisation deliberately did not unify.** The two-argument `render`,
  `alpha`, `pixel` and `deviation_from_the_clause` each index a raster through their own
  file's `SIZE`, so one home for them is one home for `SIZE` — and `SIZE` means 64 in six
  files and something else in four others, which is a decision about what those probes are
  rather than a refactor. **`alpha` is the reason to be careful**: its text is identical in
  `coverage_lanes.rs` and `mask_regions.rs` and the `SIZE` it reads is *not*.
  `deviation_from_the_clause` is §11.4.6's arithmetic written out three times, two identical
  and the third in `function_knockout.rs`; giving it one home is a round that must touch that
  file too. Two smaller ones left where they were: `m1.rs`'s `max_byte_diff` doc credits the
  function with PNG artefacts its *caller* writes, and `m1.rs` and `m3.rs` each state their
  own derivation of `UNORM_TOLERANCE = 2`, which is why the constant was not merged.
- **Three gaps in the function lane**, named in `doc/notes-function-wiring.md` §4.5 and found
  by reading rather than by a failure: a clip and a soft mask over a function paint are never
  anything but 1 anywhere in the tree, no function test runs `Coverage::Gpu`, and ADR 0025's
  `DestOut`/`Plus` stages are compiled and selected but never drawn.

**Closed by the 2026-08-15 debt round**, listed once so nobody re-proposes them: the rustdoc
warnings (all eleven in the workspace, and `cargo doc --workspace --no-deps` under
`RUSTDOCFLAGS=-D warnings` is a CI step now, which is why seven could accumulate unseen);
`take_pass_query`'s stale opening line; the duplicate accessors (`pipeline_store()` and
`gpu()`/`queue()` deleted, with the `#[allow(dead_code)]`); the uniform byte-offset check,
which is `src/shaders/layout.rs` deriving each field's offset from the shader source by
WGSL §14.4.4 and §14.4.6 and covering all nine host writers, verified able to fail from both
sides (a host struct's `vec2f` pair exchanged, and two fields exchanged in `blit.wgsl`);
`device/ramp.rs`'s missing unit tests, six of them, each expectation derived from its clause;
and both test-suite debts — `retained_frame.rs` is five files along its own comment's seams
(`retained_replay`, `retained_atlas`, `retained_invalidation`, `retained_refusals`,
`retained_handle` — the fifth responsibility was in the file and *not* in its comment), with
`tests/common/` holding the fixtures that had copies. 403 tests before and after, the same
names.

## Instruments — how to measure without re-deriving it

- **A presenting frame on the real GPU**: `examples/surface_measure.rs`. The owner left a
  trigger loop — `touch tmp/start-measurement` here and the loop rebuilds the working tree,
  runs it on the real display and writes `tmp/output.stdout.txt`, `stderr` beside it. It
  turned a round in about ninety seconds on 2026-08-14; confirm it is still alive before
  relying on it.
- **An encode, exactly**: `perf` is not installed for this user and wall clocks here are
  worthless at the load averages this machine runs at — 4.49 ms for an encode the owner
  clocked at 1.96–2.35. Use **callgrind**: it counts instructions and load cannot touch it.
  `encode` needs no adapter, so the harness is a `#[cfg(test)]` module inside `quorra-gpu`
  that builds a `ResourceStore` and an `AtlasStore` directly, copies the archetype out of
  `examples/surface_measure.rs`, encodes twice to fill the atlas and then N times from an
  `#[inline(never)]` wrapper. Build with `CARGO_PROFILE_RELEASE_DEBUG=1` into its own target
  dir, then `valgrind --tool=callgrind --collect-atstart=no --toggle-collect='*steady_run*'
  --read-inline-info=yes --cache-sim=no`, and read it with `callgrind_annotate
  --inclusive=yes` (the lines carrying the binary's path are the per-function inclusive
  totals) and `--tree=caller` for call sites and counts. **Check the counter row against
  `tests/archetypes.rs` before believing any of it** — that is what says the harness encoded
  §6.2's page. Delete the harness with the round; `Cargo.toml`'s note on `criterion` is the
  standing decision that a benchmark harness does not live in this tree.
- **What a generated shader costs to compile**: `examples/function_compile.rs` — a §7.10.5
  program's shader, timed by the program's length. It reads a **direct span** that
  `PipelineStore::function_pipeline` already brackets rather than a subtraction, its own
  `CARGO_TARGET_DIR`, round 0 reported apart, minima of round-robin rounds with the load
  average beside them. Every sample must be a program **no process has compiled**, because
  RADV's on-disk cache keys on SPIR-V — the trap that cost the spike a round.
- **A page of curve-clipped marks**: `examples/residue_clip.rs` — the artwork archetype,
  headless into a texture, `instrument_encode` on, minima of twenty steady frames with the
  first reported apart and the load average printed beside them. It prints
  `clip_residue_regions` and `clip_residue_tiles` with the clocks, which is what makes two
  runs on a loaded machine comparable at all: the counters are exact functions of the
  scene. Two builds of it cannot be round-robined inside one process, so the A/B is a
  `git checkout` between the base and the change, three rounds each, alternating.
- **Whether the atlas settles**: `examples/retained.rs`'s second section — twelve retained
  frames of a page whose tiles overflow a stated atlas budget, printed as a string of `E`
  (encoded) and `.` (replayed). A **property, not a clock**, so it reads the same at load
  average 90 as on an idle machine. `E...........` is a settled atlas; `EEEEEEEEEEEE` was
  the pathology ADR 0050 removed. The section asserts its page is still inside the band —
  a fixture that drifts out of it would go on passing, because a page that never overflows
  replays trivially.
- **What a code path allocates, exactly**: `tests/counting_allocator/` — a
  `#[global_allocator]` for one test binary that counts allocations of a megabyte or more
  on the calling thread. Deterministic where a wall clock here is not, and it is how
  ADR 0052 gates the readback. Thread-local on purpose: llvmpipe's worker threads allocate
  too, and a global counter would make the number a property of the adapter's core count.
- **Memory inside a frame**: six lines in `Device::render` — a `Region::of(root.bounds)` and
  an `eprintln!` — in a `git worktree`. That is what turned ADR 0039 from a paragraph saying
  "not worth it" into a 41 % reduction. The path is finished for now: every layered frame of
  the corpus at 4× prices 1 325.5 MB in total, the heaviest single one is 93.0 MB and is a
  page whose root marks its whole target, and nine frames in ten are flat and allocate no
  layer at all. Price before opening it again.

## Running the caller's corpus

Never build in `/home/cl/projects/pdf-viewer` and never edit it: the owner works there, and
its `[patch]` and lock are often their work in progress. Copy it instead —

```
rsync -a --exclude=target --exclude=corpus-cache --exclude=fuzz --exclude=tmp \
      --exclude=.git --exclude='doc/pdf.js/.git' --exclude=.claude \
      /home/cl/projects/pdf-viewer/ <dir>/viewer/
```

— the excludes are not optional (that tree is 100 GB). `--exclude=.claude` because that
tree's `.claude/worktrees/` holds other agents' build dirs, which reached 15 GB and was
still growing when it was learned; `<dir>` under `/home/AI` rather than the `/tmp`
scratchpad, because that tmpfs has run out mid-copy and the copy is about 537 MB. Then
append a `[patch."https://github.com/close2/quorra"]` block pointing `quorra`, `quorra-gpu`
and `quorra-scene` at `crates/*` here, and run

```
CARGO_TARGET_DIR=<scratch>/target cargo test --release -p render-quorra --test corpus \
  -- --ignored --nocapture
```

`PDFVIEWER_QUORRA_ONLY=a.pdf,b.pdf` narrows it (the ratchets are then *not* checked),
`PDFVIEWER_QUORRA_COVERAGE=cpu|gpu` picks the lane and `PDFVIEWER_QUORRA_SCALE=n` the
magnification.

## Traps

**The corpus is part of a change, not a check after it.** Layers sized to their plans passed
208 unit tests and moved 31 corpus pages off *agree*, then 12 more, before it was right:
884, 903, 915. Two defects, neither reachable by any test in this tree.

**Always run the baseline in the same copy, on the same day.** Verdict counts are stable
across copies of *one* viewer revision, and that tree changes under you: ADR 0036 recorded
915 / 37 / 5 at scale 1, and a copy taken a day later read 919 / 37 / 1 for the same quorra
commit. Nothing regressed — their tree moved. So a count quoted in an older ADR is not a
baseline; a `git worktree` at the base commit, patched into a second copy of the viewer and
run the same hour, is. Compare the **per-page lines**, not only the totals: ADR 0037's
evidence that it moved memory and not pixels is that all 37 differing pages matched to the
last digit of every mean, worst tile and SSIM.

**Wall clocks lie under load, and this machine is somebody's desktop.** A first-frame
improvement measured at 24.7 ms → 10.3 on a quiet machine re-measured as 19.9 → 20.0 an hour
later with Firefox and a slicer running. Check `uptime` before believing a timing, prefer
minima over means, and make the *test* assert a property — a device warmed for one size,
another, or none draws the same bytes — rather than a duration. **That improvement was not
real** (ADR 0040): 40 round-robin rounds could not find it at either page size, and the
allocation it was credited to takes 0.06 ms. When a difference is a difference of wall
clocks, run the configurations round-robin so drift falls on all of them, and look for a
*direct* span before believing the subtraction.

**A first-use pipeline compile is invisible to "wait a while and try again".** Two of them
were 2.6 ms of a layered first frame for three ADRs, and `Timings::phases` had been
reporting them by name the whole time. When a first frame is slow, read its phases before
theorising about memory.

**Stage "every stage learns an offset" changes at zero first.** Panes (ADR 0028) shipped with
one of three subtractions missing and drew nothing at all for every band after the first. The
same change done as *plumbing at zero, verified by equality, then the value* caught a
vertex-only uniform binding immediately and cost one extra commit.

**A refusal is arithmetic, a fidelity difference is not.** Which pages refuse is
machine-independent and can be reasoned about; which lane is faster is a property of the
processor *and* the adapter together, so never publish a crossover as a constant — the two
ADRs that tried (0027, then 0028) both had to delete one.

**A tile is not a window on a wider region unless the rasteriser cuts at its border.**
`fill_mask` clamped the endpoints of an edge piece that left the region and interpolated
between them: the row's total winding survived, so every column past the crossing read the
right value and no test could see it, while the columns *at* the border took the height the
piece spent outside. 2 684 pixels of a 2.9-million-pixel probe, the worst by 185 of 255.
Any change that wants to compute coverage once and cut it up afterwards has to check this
first — the probe is `a_tile_is_the_crop_of_the_region_that_contains_it`, and it took
ninety seconds to write and settled the design of a whole round (ADR 0049).

**A determinism fixture that does not overlap is not a determinism fixture.** ADR 0054's
first thread-count gate used a 15-pixel lattice where no two marks touched, and it **passed
with an ordering drain removed**. At 6 pixels for a 44-pixel mark it fails. The same round's
atlas defect — a duplicate insert, `bytes_uploaded` off by exactly 64 — was found by the
gate rather than by reading, which is what a fixture that actually contends buys you.

**A cache's "would this help?" test must be asked in the units the cache allocates in.**
ADR 0024 gated the atlas repack on `bytes requested <= bytes available`, and the packer
allocates *shelves*: a page at 63 % of the atlas by area did not fit it by packing, so the
repack fired, changed nothing, and fired again on the next frame — for ever, invalidating
its own retained encode each time (ADR 0050). Sixteen of seventeen swept configurations
settled after one encode and looked like proof the design was fine. **Sweep the parameter,
and read the sequence rather than the first two frames.**

**Read a gate's threshold against the number it names before trusting it.** The readback
gate asserted `< 6 ms` against a regression measured at 3.84 — so the shape it existed to
catch would have passed it, and the only thing it ever failed for was this desktop's load
(two runs in five). It had been that way for three ADRs. ADR 0052 replaced it with an
allocation count and recorded the general form: **a claim about "how many" is a count and
is exact; a claim about "how fast" is a duration and this machine cannot measure one.**
Split a gate along that seam rather than picking a threshold between them, and verify a
new gate *in both directions* — a test that passes proves only that a test exists.

**A fixture that names a lane should say which lane it means.** ADR 0047 found three tests in
`m45.rs` using a rectangle as a stand-in glyph: one failed, and two would have gone on
passing while comparing one lane with itself.

**A WGSL compile error now fails the suite by name** (ADR 0042). It used to hang it: a
reserved keyword in `blit.wgsl` panicked the warm-up thread inside wgpu, and every test file
that calls `wait_until_warm` then waited on a `Condvar` with no notifier left alive — an
infinite hang with no output at all. If you ever see a silent hang again, suspect the same
shape: a `Condvar` whose only notifier can leave without notifying.

**A clippy run that says "Finished" may not have looked at your file.** Eight edited test
files in the shared `/home/AI/cargo-target/quorra`, and `RUSTFLAGS=-D warnings cargo clippy
--workspace --all-targets` reported clean while five of them held an unused import — which
failed the moment the files were `touch`ed. Before believing a green gate on a fresh edit,
check that cargo printed `Checking <crate>` and not only `Finished`. Clippy *does* lint
integration tests; that was verified by planting a lint and watching it fail, which is the
right way to settle "did the gate look at this?" in either direction.

**A pipeline's exit status is the last command's.** `cargo test --workspace 2>&1 | tail -80`
exits 0 when cargo fails, because `tail` succeeded, and the tail of a passing run and of a
failing one both end in doctests. Redirect to a file and read cargo's own status, or set
`pipefail`; a status read through a pipe is not evidence that a suite passed.

**The shared cargo target dir is not yours alone.** Sibling agents in other worktrees build
the same crate and example names into `/home/AI/cargo-target/quorra`, and
`release/examples/<name>` is whoever linked last. A measurement round was published from
another tree's binary and only caught because its counters were impossible. Anything you read
numbers from gets its own `CARGO_TARGET_DIR`; `cargo build` reporting "Compiling" is not
proof the binary on disk is yours.

**A fresh `CARGO_TARGET_DIR` is a cold cache, and a per-checkout remap flag is a colder one.**
`sccache`'s key includes the paths cargo derives from the target directory, so inventing a new
target dir per purpose reads as 0 % hits and a 27 s build where the same build against a
stable dir is 100 % and 4 s. Across worktrees the cache already shares at ~97 %, and
`--remap-path-prefix` with a per-checkout value makes it *worse* — differing values took the
same cross-worktree build from 2 misses to 121. One stable target dir per user (the untracked
`.cargo/config.toml` here pins `/home/AI/cargo-target/quorra`), shared across worktrees when
builds are sequential.

**`pgrep -f "<your own command>"` in an `until` loop never exits**, because the loop's own
shell matches its own pattern. Two rounds were lost to a `cargo test` that never started.

## Recorded and deliberately not taken

Each has an ADR stating the measurement and why it was left. Do not re-propose one without
reading it:

- the census cannot see how often a shape is placed, at phase granularity (0029);
- a pane is cut in sheet order rather than by what packs tightest (0028);
- tiles are packed in encounter order; sorting them needs positions assigned after the walk,
  which is a two-pass encode (0034);
- the layer pool matches on **exact** extent — serving a smaller plan from a larger texture
  would buy 0.06 ms and would put a viewport in every pass (0040);
- `warm_for` does not draw a warm frame: 4.7–22.3 ms on the calling thread to save 0.5–2, and
  what was attributable in it is in the warm set instead (0040);
- the mask's transparent value is computed on the CPU as well as in `reduce.wgsl`, because an
  independent implementation a test compares is stronger evidence than agreement by
  construction (0037);
- a non-isolated group still takes its parent's region rather than its own: §11.4.4's
  interpolation is stated over the whole of the group's buffer, so shrinking it is a clause
  question and not a plumbing one (0038);
- a culled child's plan stays in `Encoded::layers`, unreferenced, because that list is indexed
  by `ChildOp::layer` *and* by `MaskPlan::root`; the cost is that `is_flat` asks whether
  `layers` is empty rather than whether any plan is reachable, so a page whose only group is
  clipped away renders through a root accumulator instead of straight into the target (0041).
  The same question is open for a culled group's soft mask, which still realises;
- the hand-off gained a branch per pixel of the target; nothing surfaced above the run-to-run
  spread on the corpus, and that is the honest statement rather than a claim either way (0039);
- the atlas has no recency, so two pages that alternate and do not fit beside each other
  still repack once per frame — the same cost that shape had before ADR 0050, now *visible*
  as `Counters::atlas_repacked` true on every frame rather than inferred. A single page too
  large for its atlas is stable (0050); genuine thrash is what recency answers, and ADR 0024
  has been waiting for its measurement since;
- the *divide* half of ADR 0022 is no longer gated at all: it is a throughput claim, no wall
  clock on this machine can hold one, and its instrument is the callgrind round above rather
  than anything in CI (0052);
- a residue region is the chain's own bounds and not the union of the tiles that ask for it;
  the union needs a second pass over the commands before the first ask, which is the
  two-pass encode ADR 0034 declined for the tiling (0049);
- residue regions are not pooled across frames: the retained encode already answers "the
  same page again", and a cache that outlived its scene is ADR 0029 §3's rejected memory in
  a second place (0049).

## This machine

Arch, AMD Ryzen AI 9 HX 370 with a Radeon 890M. Two adapters and the difference is a feature:
RADV is the default and llvmpipe is pinned by name in most tests, so CI can run on a software
rasteriser. Claude Code runs as user `AI` through the `coders` group and has no X cookie —
headless is fine, a window on the owner's display is not.
