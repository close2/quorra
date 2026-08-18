# The thin-mark condition, measured — ADR 0070's matrix and its forced defects

Round notes, 2026-08-18. These are the notes ADR 0070's own round promised and never wrote,
taken again from scratch rather than recovered from anywhere: that ADR reached `main` with
`<<MATRIX>>` and `<<DEFECTS>>` sitting in it as literal text, and
`doc/notes-release-matrix.md` §5 is the round that found them.

The rule this file exists to obey is `doc/HANDOVER.md`'s, and it is the reason nothing here is
copied from the earlier round's report:

> A claim that lives only in a round's report is a claim no later round can check. A number in
> a report is not a number in the tree.

**The headline.** All four rows re-measured, both lanes at both scales, one copy of the
caller's tree, one sitting. `doc/PLAN.md`'s quoted scale-1 device row — `930/25/2/17 →
932/23/2/17` — **is confirmed exactly**. Three per-page lines move and no others; no refusal
moves in any row. Six defects were forced against the gates ADR 0070 lists; five went red where
they should, and **one did not fail at all**, which turned out to be a defect in the gate
rather than in the condition.

---

## 1. The matrix

### 1.1 What was compared, and how the columns were kept apart

- **Base** `b5a09d7` — the mainline commit immediately before ADR 0070's merge `c443bc2`,
  reached as that merge's first parent. It already contains ADR 0069's knockout refusal, so
  the two columns differ by ADR 0070 and by nothing else.
- **Change** `ada5e3f` — `main` at the time of writing.
- `git diff --stat b5a09d7 ada5e3f -- crates` is nine files, **all of them in
  `quorra-gpu`**, and `quorra-scene` is untouched. That is what makes "no refusal moved" a
  prediction rather than an observation: refusals are raised in the scene builder.
- **The caller's tree** at `829d7faa`, copied once under `/home/AI` by
  `doc/HANDOVER.md`'s rsync recipe — the same revision the settlement round used, so its CPU
  scale-4 row is directly comparable with this one.
- Both revisions extracted to **two separate paths**, and only the `[patch]` block's paths
  flipped between columns.

### 1.2 The rows

| lane, scale | base `b5a09d7` | change `ada5e3f` | cargo exit |
|---|---|---|---|
| `Coverage::Gpu`, scale 1 | 930 / 25 / 2 / 17 | **932 / 23 / 2 / 17** | 0 / 0 |
| `Coverage::Cpu`, scale 1 | 932 / 23 / 2 / 17 | 932 / 23 / 2 / 17 | 0 / 0 |
| `Coverage::Gpu`, scale 4 | 939 / 10 / 3 / 22 | 939 / 10 / 3 / 22 | 0 / 0 |
| `Coverage::Cpu`, scale 4 | 938 / 11 / 3 / 22 | 938 / 11 / 3 / 22 | 101 / 101 |

*agree / differ / refused / not comparable.* 974 documents at page one; 957 pages compared at
scale 1 and 952 at 4×. Adapter: `AMD Radeon 890M Graphics (RADV STRIX1)`, 24 encode threads,
release build.

**The scale-4 exit 101 is in both columns and is not ours.** The caller's ratchets are checked
only on `Coverage::Cpu` (`ratchets()` requires `coverage == Coverage::Cpu`), which is why the
two device rows exit 0 while carrying the same three refusals. The assertion's two lists
differ in exactly one element, `bug1703683_page2_reduced.pdf`, which ADR 0057 moved from
refused to **drawn** — their outstanding re-baseline. `issue18032.pdf` is on **both** sides.
`doc/notes-release-matrix.md`, "A refusal that did not move", settles this at length and it is
not re-argued here.

### 1.3 Every page line that moved

Three, over four rows. Everything else — every `differs` line and every `refused` string — is
identical to the character between the columns, checked by `diff` on the extracted lines
rather than by eye.

| page | row | base | change |
|---|---|---|---|
| `bug1883609.pdf` | Gpu, 1 | `mean 0.4926 worst tile 2.99 at (160, 672) differing 0.0311 ssim 0.98615` | *agrees; no line* |
| `vertical.pdf` | Gpu, 1 | `mean 0.1526 worst tile 9.38 at (0, 320) differing 0.0092 ssim 0.98572` | *agrees; no line* |
| `issue12295.pdf` | Gpu, 4 | `mean 0.9517 … differing 0.0490 ssim 0.95585` | `mean 0.9201 … differing 0.0473 ssim 0.95881` |

`issue12295.pdf` keeps its `worst tile 16.31 at (1792, 2208)` and does not reach agreement, so
**the 4× totals do not move**. A matrix of totals alone would have called that row null; it is
the per-page comparison that finds it, which is `doc/HANDOVER.md`'s corpus trap paying for
itself in the direction it was written for.

**The cause, and why it is a deduction and not a guess.** The only behavioural difference
between the columns is the fifth lane condition. So any page that moved moved because a mark
on it whose thin axis is below `1/√16 = 0.25` device pixels stopped taking the device lane and
was rasterised by the producer that computes exact area. There is no second candidate in the
diff.

Re-run in isolation with `PDFVIEWER_QUORRA_ONLY`, the four scale-1 device-lane pages in
question read:

```
base   4 pages compared: 0 agree, 4 differ, 0 refused, 0 not comparable
change 4 pages compared: 2 agree, 2 differ, 0 refused, 0 not comparable
```

— the two that move are `bug1883609.pdf` and `vertical.pdf`, and the two that do not
(`bug1863910.pdf`, `issue16500.pdf`) keep byte-identical metrics across both columns.

### 1.4 The claim that did not survive

ADR 0070's §4 says, quoting `doc/notes-thin-mark-options.md` §2.4, that "after the change no
page outside the processor lane's own differing set remains". **That is false**, and the
totals are precisely what hide it: at scale 1 after the change both lanes name **23** pages,
and they are not the same 23.

| | pages |
|---|---|
| device lane only | `bug1863910.pdf`, `issue16500.pdf` |
| processor lane only | `bug1743245.pdf`, `issue21068.pdf` |

The processor lane's set is the caller's own baseline — the `Coverage::Cpu` scale-1 run passes
`Ratchets::All`, which asserts `differing == differing_pages()` — so "outside the processor
lane's own differing set" is exactly "outside their baseline", and two pages are.

Neither device-only page is touched by ADR 0070: both carry byte-identical lines in both
columns. So they are a **residual of the device lane that the thin-mark condition does not
reach**, not a regression it caused, and the correct sentence is that *the count converges and
the set does not*. The four names above are where a round that wants the sets to converge
should start.

### 1.5 The run is reproducible

`Coverage::Gpu` at scale 1 on the base column was run twice, an hour apart, and the second run
is identical to the first in totals **and** in all 27 per-page lines. Verdict counts on this
gate are arithmetic, not a wall clock, which is the property that makes a matrix like this
worth writing down at all (`doc/HANDOVER.md`, "A refusal is arithmetic").

---

## 2. Method, and the two traps it had to clear

### 2.1 `git archive` mtimes, and how the swap was proven instead of assumed

`git archive` stamps every extracted file with the **commit's** timestamp. Extracting the
older base revision over a stable `[patch]` path therefore leaves its sources with mtimes
*behind* the artefacts the change column has already produced, cargo declares the crate fresh,
and the base column silently measures the change column's library. That is not a hypothetical:
`doc/notes-release-matrix.md` §4 records a run discarded for it.

Three things were done about it, and the third is the one that actually proves anything:

1. **Two separate paths**, one per revision, rather than one path re-extracted. A cargo
   package identity includes its path, so the two columns cannot share a fingerprint.
2. `find <dir> -type f -exec touch {} +` after every extraction, belt to the braces.
3. **The swap was read out of the build log and out of the binary name.** Every column showed
   `Compiling quorra-gpu v0.1.0 (/home/AI/adr0070-remeasure/<column>/crates/quorra-gpu)`, and
   the test binary's hash tracked the column:

   | column | test binary |
   |---|---|
   | base | `corpus-90ad6864e4e23f91` |
   | change | `corpus-7120653dfd42c0bf` |
   | base again | `corpus-90ad6864e4e23f91` |

   Returning to the base rebuilt the *same* hash. A stale-freshness run would have kept the
   change column's hash while the log said base, so the two readings disagree under exactly
   the failure they are there to catch. (The settlement round could not use a hash comparison
   because it held one `[patch]` path across both columns; two paths make the hash a witness
   again, at the cost of one extra compile.)

### 2.2 Reading which element of an assertion differs

The 4× CPU assertion prints two sorted lists and fails in both columns of every matrix this
week. The element that differs is `bug1703683_page2_reduced.pdf`. `issue18032.pdf` appears on
**both** sides and is refused by `render-quorra`'s own §11.4.6 check before a
`quorra_scene::Scene` exists. Reading the failure as naming the newest thing in the tree is
what cost ADR 0070's round its aside; the fix is to diff the two lists rather than scan them.

---

## 3. The forced defects

Each defect was applied alone to `ada5e3f`, the suite run with it in place, and the defect
reverted before the next. **The column that matters is what actually went red.**

```
cargo test --workspace --release --no-fail-fast
```

**`--no-fail-fast` is load-bearing.** Without it cargo stops after the first failing test
binary. The first attempt at defect 1 reported *one* red test; the same defect with
`--no-fail-fast` reports three. A blast radius measured with fail-fast on is a lower bound
wearing the clothes of a measurement.

| # | defect | site | went red |
|---|---|---|---|
| 1 | `<=` instead of `<` — a mark **at** the spacing declined | `encode/thin.rs` `can_fall_between_sample_columns` | `encode::thin::tests::a_mark_at_exactly_the_spacing_is_not_declined`; `the_lane_is_declined_exactly_below_the_sample_spacing`; `a_turned_hairline_stroke_is_declined_by_its_own_width` |
| 2 | fifth condition deleted from the chooser | `encode/coverage.rs` `take_gpu_lane` | `a_mark_below_the_sample_spacing_is_drawn_at_every_position_on_both_lanes`; `the_lane_is_declined_exactly_below_the_sample_spacing`; `a_turned_hairline_stroke_is_declined_by_its_own_width` |
| 3 | box only; the stroke width dropped | `encode/thin.rs` `ThinAxis::of` | `encode::thin::tests::a_strokes_width_is_read_where_it_is_thinner_than_the_box`; `a_turned_hairline_stroke_is_declined_by_its_own_width` |
| 4 | spacing written down as `0.25` | `encode/thin.rs` `sample_column_spacing` | `encode::thin::tests::the_spacing_is_one_over_the_grids_side_at_every_admitted_sample_count` **only** |
| 5 | `!(self.0 >= spacing)` — a `NaN` axis declines the lane | `encode/thin.rs` `can_fall_between_sample_columns` | **nothing** |
| 6 | `None` passed for the width at the call site | `encode/stroke.rs` `encode_stroke` | `a_turned_hairline_stroke_is_declined_by_its_own_width` **only** |

Defect 2's message is the clause failing in the clause's own words, which is the one worth
quoting:

```
Gpu at 20, width 0.125: the mark disappeared. §10.7.4: painting any pixel the shape
intersects "ensures that no shape ever disappears as a result of unfavourable placement
relative to the device pixel grid"
```

and its two sibling failures are the lane read from `Counters::bytes_uploaded` rather than
from pixels:

```
a stroke 0.1 device pixels wide is below the grid's column spacing at every angle, and its
box says nothing about that: 29272 bytes against the processor lane's 3664

a mark of 0.225 device pixels is narrower than the grid's column spacing of 0.25, so
§10.7.4 requires the producer that cannot lose it: the frame uploaded 6712 bytes where the
processor lane uploads 832
```

### 3.1 Defect 5 did not fail, and the gate is why

`a_non_finite_thin_axis_declines_nothing` passes for a reason that is not the one its name
gives, and it passes with the defect in place as well as without it.

Its fixture is `ThinAxis::of((0.0, 0.0, f32::NAN, 10.0), None)`. That computes
`(NAN - 0.0).min(10.0 - 0.0)`, and **`f32::min` is IEEE 754's `minNum`, which discards a `NaN`
operand**:

```
x1-x0 = NaN
y1-y0 = 10
fixture across = (NaN).min(10.0) = 10   is_nan=false
  across < 0.25      = false
  !(across >= 0.25)  = false
```

So the thin axis is a finite **ten** device pixels — forty times the spacing — and the test
asserts that a mark ten pixels across is not thin. Every possible implementation of this
condition satisfies that, which is why inverting the comparison leaves it green.

The property the test *means* to assert is nevertheless true of the shipped code: `self.0 <
spacing` is false when `self.0` is `NaN`, which is the documented behaviour. What is missing is
a fixture that can produce a `NaN` thin axis at all, and that needs **both** extents non-finite:

```
both-NaN across = NaN is_nan=true
  both < 0.25        = false     ← the shipped reading, declines nothing
  !(both >= 0.25)    = true      ← defect 5, declines the lane
```

`(0.0, 0.0, f32::NAN, f32::NAN)` separates the two readings and is the one-line change the
gate needs. It is **not made here**: this was a documentation round, and its brief was to write
down what the tree does, not to change it. It is the first thing for the next round.

### 3.2 What the six say together

- **The arithmetic and the wiring have disjoint witnesses.** Defect 2 removes the wiring and
  turns *no* unit test red; defect 6 breaks the wiring at one call site and turns *only* an
  on-device fixture red. Neither kind of test can stand in for the other, which is
  `doc/HANDOVER.md`'s "plumbing at zero" trap seen from the test side.
- **`a_turned_hairline_stroke_is_declined_by_its_own_width` earns its keep four times.** It
  is the only witness for defect 6 and one of several for 1, 2 and 3 — because it is the only
  fixture whose subject is the bound a bounding box cannot supply.
- **Defect 4's single witness is on paper.** Nothing in this tree renders with
  `Options::coverage_samples` away from sixteen, so "the threshold follows the option" rests
  entirely on `encode::thin::tests`. That is ADR 0070's second "Revisit when" restated as a
  measurement: a non-default sample count is not untested, it is untested *on a device*.

---

## 4. What this round did not do

- **No behaviour changed.** The only source edits were the six defects, each reverted; the
  working tree is clean against `ada5e3f` apart from documentation.
- **`doc/PLAN.md`'s scale-1 device row needed no correction.** It reads `930/25/2/17 →
  932/23/2/17` and that is what the run says, to the digit.
- **The gate defect of §3.1 was recorded, not fixed**, per the round's brief.
