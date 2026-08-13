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
hint (0035), and layers sized to their plans (0036, in two commits plus a specification of
the next piece).

**What the caller must do to take it** is written for them in
`/home/cl/projects/pdf-viewer/doc/QUORRA_UPGRADE.md`: one line for `GroupSpec::compose`, a
test of theirs that fails by design, and the `Command::Shaped` translation that their four
refused pages need. Their `doc/QUORRA_FEEDBACK.md` is the other half of that conversation
and is worth reading before answering anything in it: a heading there can be stale in
either direction, and §13 sat marked *open* for eleven of their rounds after ADR 0023 had
answered it.

## What to do next, in this order

### 1. Soft masks sized to their plan

`issue16287.pdf` is the last corpus page refused for bytes at 4×, and 93 MB of its 291 is
four soft masks realised at the whole target. Any page that soft-masks at zoom pays the
same on every frame.

**The design is already written**, in ADR 0036's "The masks, which are the next piece and
are specified here": the reduce needs no origin, five sampling sites move, the value
*outside* a mask's rectangle is what the reduce writes for a transparent pixel rather than
zero, an absent mask becomes "size (0, 0), outside 1.0", and the parameters belong in the
lane's group 1 rather than in the sixteen-byte globals. Stage it the way 0036 was staged.

### 2. The root's pair

The same page's other 186 MB is the *root* plan's ping-pong pair, at the target's size
because the root is the target. Worth asking whether it needs two full-target textures, or
whether it can ping-pong against the target the frame is drawing into. Unmeasured, and the
answer decides whether page 1 above is enough for that page.

### 3. Multi-sheet passes

Three pages at 4× refuse with `ScratchExhausted` — the coverage sheet against the adapter's
16 384 limit, which is a different ceiling from the byte budget and one the pane work
cannot reach. A frame would have to use more than one sheet, which means batches carrying a
sheet index and touching the encoder, the compositor and the device together. The largest
and least certain of the three; `bug1721218_reduced.pdf` would still refuse on bytes.

### Recorded and deliberately not taken

Each of these has an ADR that states the measurement and why it was left:

- **the census cannot see how often a shape is placed** at phase granularity (0029);
- **a pane is cut in sheet order** rather than by what packs tightest (0028);
- **tiles are packed in encounter order**, and sorting them needs positions assigned after
  the walk — a two-pass encode (0034);
- **`warm_for` predicts the target's size**, which stops being the right size to warm once
  layers are their plans' (0035, and 0036 made it true).

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
