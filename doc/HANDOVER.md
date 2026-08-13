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
made necessary (no ADR — a defect), and masks sized to their plans (0037, in two commits).

**What the caller must do to take it** is written for them in
`/home/cl/projects/pdf-viewer/doc/QUORRA_UPGRADE.md`: one line for `GroupSpec::compose`, a
test of theirs that fails by design, and the `Command::Shaped` translation that their four
refused pages need. Their `doc/QUORRA_FEEDBACK.md` is the other half of that conversation
and is worth reading before answering anything in it: a heading there can be stale in
either direction, and §13 sat marked *open* for eleven of their rounds after ADR 0023 had
answered it.

## What to do next, in this order

### 1. The root's pair

**No corpus page is refused for *frame* bytes any more** (ADR 0037 took the last one), so
this is no longer about refusals — it is about what every page with a group pays on every
frame. `issue16287.pdf` at 4× is now 203 MB, of which **186 MB, 91.6 %, is the root plan's
ping-pong pair**, at the target's size because the root *is* the target.

Worth asking whether it needs two full-target textures, or whether it can ping-pong against
the target the frame is drawing into. Unmeasured. Note what makes it harder than 0036 and
0037 were: the target is the caller's texture or a swapchain image, its format is not
always `WARM_FORMAT`, and a damage patch must not touch a pixel outside its rectangles —
so "draw into the target and read it back" is three contracts at once, not one.

### 2. Multi-sheet passes

Three pages at 4× and one at scale 1 refuse with `ScratchExhausted` — the coverage sheet
against the adapter's 16 384 limit, which is a different ceiling from the frame budget and
one the pane work cannot reach. **It is now the only reason any frame of the corpus is
refused.** (The fourth refusal at 4×, `22060_A1_01_Plans.pdf`, is a third budget again: 548
MB of resident images against `max_resource_bytes`, refused at upload rather than at the
frame.) A frame would have to use more than one sheet, which means batches carrying a sheet
index and touching the encoder, the compositor and the device together: the largest and
least certain item on this list.

### Recorded and deliberately not taken

Each of these has an ADR that states the measurement and why it was left:

- **the census cannot see how often a shape is placed** at phase granularity (0029);
- **a pane is cut in sheet order** rather than by what packs tightest (0028);
- **tiles are packed in encounter order**, and sorting them needs positions assigned after
  the walk — a two-pass encode (0034);
- **`warm_for` predicts the target's size**, which stops being the right size to warm once
  layers are their plans' (0035; 0036 made it true and 0037 made it true of masks too);
- **the mask's transparent value is computed on the CPU as well as in `reduce.wgsl`**,
  rather than fed into the reduce so the two cannot disagree — an independent
  implementation a test compares is stronger evidence than an agreement by construction
  (0037).

## Traps

**Wall clocks lie under load, and this machine is somebody's desktop.** A first-frame
improvement measured at 24.7 ms → 10.3 on a quiet machine re-measured as 19.9 → 20.0 an
hour later with Firefox and a slicer running; the load average was 12. Check `uptime`
before believing a timing, prefer minima over means, and make the *test* assert a property
— a device warmed for one size, another, or none draws the same bytes — rather than a
duration.

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
magnification. Verdict counts are stable and comparable across copies; **timings are not** —
compare before and after inside one copy, flipping only the `[patch]` between a
`git worktree` at the base commit and the working tree.

## This machine

Arch, AMD Ryzen AI 9 HX 370 with a Radeon 890M. Two adapters and the difference is a
feature: RADV is the default and llvmpipe is pinned by name in most tests, so CI can run on
a software rasteriser. Claude Code runs as user `AI` through the `coders` group and has no
X cookie — headless is fine, a window on the owner's display is not.
