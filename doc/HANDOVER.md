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

**`a35dc70` is what the owner pushed, and `git log --oneline a35dc70..HEAD` counts 41
commits since.** Everything after it is local, and the ADRs say what each is: 0032 and 0033
answer the caller's §14.2, then the sheet packer (0034), the size
hint (0035), layers sized to their plans (0036, in two commits), a scissor fix that 0036
made necessary (no ADR — a defect), masks sized to their plans (0037, in two commits), and
one accumulator per plan instead of a ping-pong pair (0038, in two commits), the root sized
like every other plan (0039), the compositor's two pipelines moved into the warm set after
ADR 0035's first-frame number failed to reproduce (0040), a child the encoder drops when its
clip leaves it nothing to contribute (0041), the round cap that was drawing the
half-disc *inside* the stroke — the caller's `QUORRA_FEEDBACK.md` §21.1 — and a WGSL
compile failure turned from a silent test-suite hang into a refusal that names its span
(0042). Then the ten rounds of 2026-08-14, whose three ADRs are the warm set keyed by the
surface's negotiated format (0043), a flatness bound that is relative to the shape when the
shape is smaller than the tolerance — the caller's §21.2 (0044) — and what an unchanged
frame need not pay again, priced whole and landed by half (0045); the other half is now
built as the retained `RetainedScene`/`render_retained` API (0048), under the owner's
same-day authorisation to change the API. The rest of those rounds
carry no ADR because none decided anything: `examples/rect_lane.rs` and the §19 numbers,
the fuzz vocabulary, the shader-sameness guard, `SceneError`'s own module, and
`encode.rs` split along the seams its own comments named.

On 2026-08-13 the tree had a full review — code, plan, and both sides of the caller
conversation — and the "What to do next" list below is its result. The review's one
structural finding: the work itself is disciplined, but the brief's own success metric
(§6.2, surface-tier) had never been measured, and the caller sync was lagging the local
work. `examples/surface_measure.rs` was built for the first of those; it is the only
code the review added. The metric has since been measured and met (item 1); the sync is
still lagging (item 2), and is now the older of the two problems.

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

**As of the close of 2026-08-14, items 1, 3 and 4 are done — item 1's second half
included, as ADR 0048 — and only 2 and 5 are open.** The numbers are kept rather than
deleted because three later items lean on them; the two that remain are **the sync
round** — now the largest thing either side is holding, and since ADR 0048 carrying a
fourth document (`QUORRA_RETAINED_FRAME.md`, an API the caller must *adopt* rather than
merely take) — and **the residue-clip tiling seam**, which is the only work left with
milliseconds in it.

### 1. The two surface-tier numbers — **done** (2026-08-14), and what they left behind

The owner ran `examples/surface_measure.rs` on RADV at the real display, twice, and the
warm-set fix is landed and verified: ADR 0043, the numbers in `PLAN.md`'s entry for the
date, **compiles: none on eight presenting first frames of eight** on the re-run. The
owner also left a trigger loop for this instrument — `touch tmp/start-measurement` in
this repository and the loop rebuilds the working tree and runs `surface_measure` on
the real GPU, writing `tmp/output.stdout.txt` — so a measurement no longer waits for a
hand-off (confirm the loop still runs before relying on it; it was alive and turned a
trigger round in about ninety seconds on 2026-08-14, `stderr` beside the output showing
what it rebuilt).

What the numbers re-rank, recorded here rather than jumped on:

- **The §6.2 gap is CPU recording, not the device.** A steady presenting dense-text
  frame is 2.84–3.38 ms against the 2.0 ms success bar, and the device's share is
  0.11–0.13 ms; recording (hash lookups, instance writes — 4 320 commands) is
  1.90–2.32 ms of it and submit/device-wait another 0.55–0.71.

  **And the gap is closed** (2026-08-14, closing re-run on the same instrument, the
  numbers in `PLAN.md`'s top entry): **1.816 ms `wall − acquire`**, encode 1.126 of it,
  recording 1.130 — under §6.2's 2.0 ms success bar, by the two landed encode changes
  below and nothing else. It is a minimum from one run on a desktop, not a margin; the
  clear-win figure is 0.6 ms and ADR 0045's unbuilt half is what points at it.

  **Recording is now attributed** (2026-08-14; the table and the instrument are in
  `PLAN.md`'s entry for the date). Three rows of it were free and are landed —
  `distinct_outlines` off `SipHash`, the instance buffers reserved from phase 1's
  count, and `CacheProspect` carrying the entry so the glyph key is probed once
  instead of three times (ADR 0024 said twice; the profile found a third) — worth
  6.4 % of the dense-text encode by instruction count and
  **0.940 → 0.627 ms on `examples/zoom`** at 1×, 30 round-robin rounds. The
  headline for whoever takes the second half: recording is 78 % of the encode, and
  **`outline_device_bounds` alone is a third of recording** — 4 320 fills × 37 control
  points transformed and min/maxed, recomputed identically on every frame of a page
  nobody is touching. Adding the atlas probes, the key construction and the instance
  bytes, **over 40 % of recording is a pure function of (scene, viewport)**. So
  **encode reuse across identical frames** is the item, it is sized rather than
  guessed, and two cheaper experiments should be priced before it: a per-`(outline,
  linear)` bound cache (a placement's device box is its neighbour's translated), and
  whether the reuse can be a whole retained `Encoded` rather than a set of caches.

  **Both were priced, and the verdict is ADR 0045** (2026-08-14; the numbers and the
  survival table are in `PLAN.md`'s entry for the date). The bound cache is **landed** —
  `encode/hull.rs`, **−21.2 % of a dense-text encode** by instruction count and −0.21 %
  on artwork, counters identical, no public surface touched. The whole retained
  `Encoded` is **priced and proposed, not built**: a throwaway prototype puts an
  unchanged dense-text frame at **0.154 ms against 1.538**, and the reason it is not
  built is that it is an API question the caller has to answer — their `doc/todo/44` §3
  asks for it, their frame's scene is rebuilt with fresh `Arc`s every frame so a cache
  keyed on scene identity would miss every time, and the answer we want from them first
  is in `doc/feedback-answers-draft.md`: **can the page and the overlays be two `render`
  calls into one target?** If yes, nothing new is needed in `quorra_scene`; if no, the
  reason why is the specification for scene-fragment composition. **This item is now
  blocked on the sync round below, not on more measurement.**

  **And the second half is built** (2026-08-14, ADR 0048): the owner authorised API
  changes the same day — "document the necessary change in `pdf-viewer/doc` and I will
  inform the pdf-viewer team" — which unblocked it without the sync round. The retained
  `Encoded` lives behind a caller-held `RetainedScene` that owns its `Scene`, and
  `Device::render_retained` replays when nothing an encode reads has moved. An
  unchanged dense-text frame is **0.174 ms against 1.107**, measured through the real
  API with both variants in one binary (`examples/retained.rs`); 0 of 8 022 576 bytes
  differ on either adapter. `Device::render` is untouched. The two-render-calls
  question above is answered in the process — **no**, both compose paths
  `LoadOp::Clear` the target — and the fallback if their overlays change on frames the
  page does not is ADR 0045's candidate (B). What the sync round carries for this is
  `pdf-viewer/doc/QUORRA_RETAINED_FRAME.md`: an API the caller must *adopt*, and it
  names three things their `present` does every frame that would defeat it.

  **How to measure it, so the next round does not re-derive this.** `perf` is not
  installed for this user, and wall clocks on this machine are worthless at the load
  averages it actually runs at — 4.49 ms for an encode the owner clocked at 1.96–2.35.
  Use **callgrind**: it is installed, it counts instructions, and load cannot touch it.
  `encode` needs no adapter, so the harness is a `#[cfg(test)]` module inside
  `quorra-gpu` that builds a `ResourceStore` and an `AtlasStore` directly, copies the
  archetype out of `examples/surface_measure.rs`, encodes twice to fill the atlas and
  then N times from an `#[inline(never)]` wrapper. Build with
  `CARGO_PROFILE_RELEASE_DEBUG=1` into its own target dir, then
  `valgrind --tool=callgrind --collect-atstart=no --toggle-collect='*steady_run*'
  --read-inline-info=yes --cache-sim=no`, and read it with `callgrind_annotate
  --inclusive=yes` (the lines carrying the binary's path are the per-function
  inclusive totals) and `--tree=caller` for call sites and counts. **Check the counter
  row against `tests/archetypes.rs` before believing any of it** — that is what says
  the harness encoded §6.2's page. Delete the harness with the round; `Cargo.toml`'s
  note on `criterion` is the standing decision that a benchmark harness does not live
  in this tree.
- **The artwork shape's steady frame is geometry: 35–47 ms of residue-clip
  re-rasterisation** every frame, on the corpus's p99 clip shape. That is item 5's
  seam with a steady-state cost beside its refusal count, and §15's question measured
  from our side — the three answers belong together in the sync round.

### 2. A sync round with the caller — cheap, and overdue

The viewer pins `a7babab`, twenty-four commits behind, and those include the §21.1
round-cap fix their own §22.7 predicted and wants to re-run first thing after a bump, and
ADR 0044, which moved sixteen of their corpus pages onto their own oracle. Forty-one
commits are local. Their `QUORRA_FEEDBACK.md` has three sections waiting on this side,
and two of them cost an afternoon:

**All three are drafted** in `doc/feedback-answers-draft.md` (2026-08-14) — a draft for the
owner to carry across, since we never edit their tree. What each one says:

- **§15** asks whether the coverage lane bounds a clipped fill by its clip's extent, and
  offers to close itself if yes. It is yes: every coverage tile is `shape ∩ clip ∩
  target` (`encode.rs:1470` `visible_tile`, `:1415` `coverage_tile`,
  `encode/clips.rs:146` `residue_intersection`, `encode/rare.rs:42` for the image lane).
  Its secondary ask — can the scene say "this fill is bounded by this rectangle" — is
  answered by the same fact: a rectangular clip collapses to a device rectangle at
  `encode/clips.rs:96` and never becomes a mask, so the clip already *is* the bound. The
  draft cross-references item 5, because §15 and the tiling ceiling are the same seam seen
  from two sides.
- **§19** is measured: `examples/rect_lane.rs`, numbers and interpretation in `PLAN.md`'s
  2026-08-14 entry. Median page: microseconds, so nothing. A page of thousands of
  rectangles: 0.6–2.1 ms. Which of those the corpus actually has is a row their profile
  does not carry, and the draft asks for it before either side writes a recogniser.
- **§22.5** (a process note, not code): the recommendation is *document*, because the name
  `layer_textures` always meant what it still means and what was wrong was a derived claim
  in our own rustdoc that theirs inherited. The rustdoc is corrected in the same commit,
  and the draft states the rule — rename when the *unit* changes, date a correction when a
  derived claim does, and name the counter in the ADR that moves its value.

Push (or hand the owner the push), deliver the answers, and let their corpus re-baseline
absorb ADRs 0036–0045 in one round instead of ten. The draft's release-note section carries
ADR 0044 as a fourth answer — their §21.2 was not on the waiting list because it landed
after that list was written, and it is the one of the four they can see in pixels.

### 3. §21.2 — a tiny outline flattens to its inscribed polygon — **done** (2026-08-14)

ADR 0044: a cubic's flatness bound is now the tighter of `FLATTEN_TOLERANCE` and 1/32 of
the cubic's own device extent, which is a floor of 16 chords a full turn. Two things it
left behind for whoever answers the caller next:

- **Their citation is to the wrong clause, and the right one is stronger.** §10.7.3 is
  *smoothness* (a shading's colour error); flatness is §10.7.2, which both licenses a
  device tolerance outright ("PDF processors may choose to ignore any flatness tolerance
  specified within a PDF file") and says where it stops — NOTE 2's "not to draw inscribed
  polygons". Worth carrying across in the sync round: their §21.3 defers a gate on the
  strength of that reading.
- **The corpus moved 16 pages onto the oracle** and none off, at both scales, which was
  not the expectation — the population a chord floor reaches is glyph outlines, not just
  the sub-pixel dots the report is about. `doc/PLAN.md`'s entry has the tables.

The cost was still open here and **is now answered**: the closing surface re-run
(`PLAN.md`'s top entry, 2026-08-14) puts `geometry` on the dense-text page at **0.161 ms**,
inside the 0.16–0.18 ms it read before the chords were added. That is the ADR's revisit
condition discharged — and it is *not* a licence to raise the floor to 32, which the ADR
refuses on accuracy grounds rather than on cost.

### 4. Split `encode.rs` along its stated seams — **done** (2026-08-14)

Six commits over two rounds, on the `pipeline.rs`/`pipeline/` layout ADR 0042 used:
`encode.rs` 2 852 → 2 060, with `encode/scratch.rs` (307, the coverage sheet and its
packer), `encode/clips.rs` (187, a chain resolved to a rectangle and a residue) and
`encode/rare.rs` (401, ADR 0011's image and shading quads) beside it. Each is a pure
move — same bodies, same signatures, same comments, same test names — and what is left
in `encode.rs` is the walk and the two lanes the brief is about. The thrice-repeated
verbatim `ChildOp` construction is now `ChildOp::implicit_blend_group`; the three sites
were compared field by field first and were identical in all nine. Item 5 still lands
here, which is why this went before it.

ADR 0045 then added a fifth file — `encode/hull.rs` (365, the `(outline, linear)` bound
memo and its proof) — and its call site, so `encode.rs` reads 2 077 today rather than
2 060. **The comparison this split made possible also found a clause question, and it is
now fixed** (2026-08-14, `A blended stroke inside a knockout group is replaced, not
blended`, no ADR — a defect): `encode_stroke` wrapped a non-Normal blend in §11.3.5's
implicit group on the blend mode alone, where inside a knockout group §11.4.6 governs.
`tests/knockout_blend.rs` measures it at 112.95 of 255 before and 0.87 after.

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
existing mechanism — and it is worth its own measurement before its own design. One more
shape belongs to this seam since ADR 0048: a page whose glyph tiles overflow the atlas
re-encodes on every frame, because the repack that follows bumps the atlas generation and
invalidates its own retained encode — magnified text is that shape, separately from the
three pages the coverage sheet refuses outright.

### Small debts, none blocking — fill-in work between the numbered items

The 2026-08-13 review's code-health pass, plus what the `encode.rs` split turned up, so a
fresh session can pick one without re-auditing. Each is one sitting:

- **A solid fill of a rectangular outline does not take the rectangle lane**, and a
  shaded one does. `StoredOutline::rect_hint` is computed for every outline at upload
  (`resources.rs:160`) and read at `encode.rs:1086` — but only on the shading arm; a solid
  paint returns into `fill_solid` at `encode.rs:1071` first and takes the atlas or the
  coverage path. Output is byte-identical either way (`examples/rect_lane.rs` checks it),
  so this is symmetry rather than correctness — but it is worth **0.13–0.49 µs a
  rectangle**, it is where most of the caller's §19 lives, and it is four lines against a
  recogniser they would otherwise write. Take it with a corpus run: it moves which lane a
  page takes, and `HANDOVER`'s "the corpus is part of a change" trap applies exactly.
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

**The corpus copy recipe now pulls in other agents' worktrees.** `rsync` with the
excludes below reached **15 GB and was still growing** on 2026-08-14, because
`/home/cl/projects/pdf-viewer/.claude/worktrees/*/tmp-target` is inside it. Add
`--exclude=.claude`, and copy to `/home/AI` rather than the `/tmp` scratchpad — that
tmpfs had 4.9 GB free and the corpus copy alone is about 6.

**`pgrep -f "<your own command>"` in an `until` loop never exits**, because the loop's
own shell matches its own pattern. Two rounds were lost to a `cargo test` that never
started.

**A fresh `CARGO_TARGET_DIR` is a cold cache, and a per-checkout remap flag is a
colder one.** The `AI` user's builds run through `sccache`, and its key includes the
paths cargo derives from the target directory — so inventing a new target dir per
purpose reads as 0 % hits and a 27 s build where the same build against a stable dir
is 100 % and 4 s (measured 2026-08-14, `quorra-gpu` dev lib, five controlled runs).
Across git worktrees the cache already shares at ~97 % — only the workspace's own
crates miss, since their source paths differ — and `--remap-path-prefix` with a
per-checkout value makes it *worse*, not better: the flag lands in every unit's key,
so differing values took the same cross-worktree build from 2 misses to 121. Use one
stable target dir per user (the untracked `.cargo/config.toml` here pins
`/home/AI/cargo-target/quorra`), share it across worktrees when builds are
sequential, and leave path remapping alone until `sccache` learns to normalise it.

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
      --exclude=.git --exclude='doc/pdf.js/.git' --exclude=.claude \
      /home/cl/projects/pdf-viewer/ <dir>/viewer/
```

— `--exclude=.claude` because that tree's `.claude/worktrees/` holds other agents' build
dirs (15 GB and growing when it was learned), and `<dir>` under `/home/AI` rather than
the `/tmp` scratchpad tmpfs: the copy is about 537 MB and the tmpfs has run out mid-copy.

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
