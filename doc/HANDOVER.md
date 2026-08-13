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
like every other plan (0039), and the compositor's two pipelines moved into the warm set
after ADR 0035's first-frame number failed to reproduce (0040).

**What the caller must do to take it** is written for them in
`/home/cl/projects/pdf-viewer/doc/QUORRA_UPGRADE.md`: one line for `GroupSpec::compose`, a
test of theirs that fails by design, and the `Command::Shaped` translation that their four
refused pages need. Their `doc/QUORRA_FEEDBACK.md` is the other half of that conversation
and is worth reading before answering anything in it: a heading there can be stale in
either direction, and §13 sat marked *open* for eleven of their rounds after ADR 0023 had
answered it.

## What to do next, in this order

### 1. Multi-sheet passes

Three pages at 4× and one at scale 1 refuse with `ScratchExhausted` — the coverage sheet
against the adapter's 16 384 limit, which is a different ceiling from the frame budget and
one the pane work cannot reach. **It is now the only reason any frame of the corpus is
refused.** (The fourth refusal at 4×, `22060_A1_01_Plans.pdf`, is a third budget again: 548
MB of resident images against `max_resource_bytes`, refused at upload rather than at the
frame.) A frame would have to use more than one sheet, which means batches carrying a sheet
index and touching the encoder, the compositor and the device together: the largest and
least certain item on this list.

### 2. The warm set is compiled for one format, and a surface has another

Every pipeline is keyed by `(kind, target format)`. The warm set compiles `Rgba8Unorm`;
`SurfaceState::new` negotiates **`Bgra8Unorm`** where the adapter offers it. So a presenting
host's first frame compiles the lane it draws with inside that frame — and the blit too, if
the page has a group — which is the same cost ADR 0040 just took off a headless first frame,
still sitting on the caller's launch path. The device knows the surface's format one line
above `spawn_warm_up`, so the change is small; **what is missing is the measurement**, and
it needs a window. This account has no X authority for the owner's display and `lavapipe`
under `Xvfb` would time a software compiler rather than the one a viewer waits for, so this
is the owner's number to take — after which the fix is a line.

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
- **a child whose region misses its parent's is still rendered** before the composite
  discovers there is nothing to write; culling it belongs in the encoder, which is where
  the clip that emptied it is known (0038);
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

**A WGSL compile error hangs the test binary; it does not fail it.** A reserved keyword
(`from`) in `blit.wgsl` made `create_shader_module` a validation error, which panicked the
warm-up thread inside wgpu, and the process then sat forever instead of reporting anything.
`cargo test` looks like an infinite hang with no output. Run the test binary directly with
`--test-threads=1 --nocapture` and read the **last** lines: the panic is there.

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
