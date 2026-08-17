# 0062 — A test is filed by the clause it states, not by the code it runs through

Date: 2026-08-17. Status: accepted, and built.

ADR 0061 decided **where a split module's tests go** — `<module>/tests.rs`, keeping the path
`<module>::tests`, so that a split can be proved by an identical list of test names. It wrote
down the cost it was choosing: `raster/tests.rs` at 704 lines, past CLAUDE.md's ~500-line
smell, with dividing it "a legitimate round of its own where the renames are the only
visible change". This ADR is what that round found when it came to divide, and it is
narrower than 0061: **not where the tests go, but which of several files each one goes in.**

## The decision

**A test belongs to the clause it makes a statement about, not to the code path it executes.**

Where a split source module's tests divide, they divide along the parent's own seam, read
that way. The instrument a test observes through does not decide its file.

## Why — ADR 0061 measured this against the wrong question

ADR 0061 gave two reasons for keeping the tests in one file. The first is strong and
unchanged: a test's name *is* the gate, so a split round that also renames every test has
destroyed the only evidence it could have offered. The second is the one this ADR corrects:

> The tests often do not divide along the source's seams. `raster`'s split is three clauses
> — flattening, filling, stroking — and `each_cap_deposits_the_area_table_53_gives_it` is a
> statement about §8.4.3 *read out of* §8.5.3.3's coverage bytes. Most of that file's cases
> go through all three parts, so a file per source module would put most of them in the
> wrong one.

The observation is true and the conclusion does not follow. Almost every case in that file
calls `fill_mask`, because [`stroke`] takes polylines and returns polylines and [`flatten`]
returns polylines too — **filling is the only way to observe either of them.** An instrument
is not a subject. ADR 0061's own sentence contains the answer it did not take: it says
`each_cap_…` *is a statement about §8.4.3*, and the file that test belongs in follows from
that clause, not from the bytes it reads to make the statement.

Asked "which clause is this a statement about?", `raster`'s eighteen cases divide with
**nothing left over**: three about §10.7.2 and ADR 0044's flatness bound, nine about
§8.5.3.3 and ADR 0005/0049's coverage, six about §8.4.3's caps and joins.

## The evidence that the seam is real rather than imposed

`raster.rs`'s module comment already assigns each of this code's three arithmetic defects to
one of the three parts — `stroke::direction`'s, `fill::accumulate_edge`'s and
`fill::deposit_slab`'s — and says why that matters: *"a defect in one of the three is a
defect in one clause, which is what the split is for."*

Filing the tests by clause puts each defect's regression test in its defect's file without
anyone choosing that:

| defect | its part | the test that covers it | where it landed |
|---|---|---|---|
| `stroke::direction` overflowing above `1.9e19` | stroke | `a_stroke_spanning_the_coordinate_range_is_not_drawn_as_nothing` | `tests/stroke.rs` |
| `stroke_polylines` deduping by float equality | stroke | `a_segment_below_the_float_grid_produces_finite_geometry` | `tests/stroke.rs` |
| `fill::accumulate_edge`'s slope leaving `f32` | fill | `an_edge_whose_slope_leaves_f32_deposits_nothing` | `tests/fill.rs` |
| `fill::deposit_slab` smearing a border column | fill | `a_tile_whose_geometry_enters_from_outside_is_exact` | `tests/fill.rs` |

Four independent agreements between a rule stated in prose and a rule applied to files is
what makes this a seam rather than a preference. A test filed by its *call graph* would have
put all four in `fill`, and a reviewer looking for the stroke clause's coverage would find
none of it under stroke.

## What it costs, stated rather than discovered

1. **Every test in the divided module is renamed**, by exactly one inserted path segment.
   That is a change to eighteen gates, and it is why ADR 0061 forbids doing it in the same
   round as the source split. The mitigation is that the mapping is mechanical and is
   published: stripping the one inserted segment from the new sorted list reproduces the old
   one, character for character, over all 554 names in the workspace.
2. **A test still sits in a different file from most of the code it calls.** Six of
   `tests/stroke.rs`'s cases run `fill_mask` and none of `tests/flatten.rs`'s asserts on a
   polyline directly. This is not a defect to be fixed later; it is the decision. Each
   module comment says which clause it holds and names the instrument it reads through.
3. **The imports get one level longer.** These files reach `raster` absolutely
   (`use crate::raster::…`) rather than counting `super`s. ADR 0061 cost 3 predicted the
   import block would be the only other edit, and it was: a multiset comparison of every
   non-comment, non-blank line before and after differs in the three `mod` declarations, the
   import blocks, and one call that lost a `super::` prefix because the name is now imported.
4. **The rule needs a judgement each time.** "What is this a statement about?" is not
   mechanical the way "what does it call?" is. Where a case genuinely states something about
   two clauses, it goes with the one whose *defect* it would catch, and the module comment
   says so. No case in `raster` needed that tie-break, which is a fact about `raster` and
   not a promise about the next module.

## Where this applies

Any module whose `tests.rs` has grown past the smell and whose source is already split along
clause lines. It does **not** license dividing a test module in the same round as its source
— ADR 0061's first reason is untouched and is the stronger of the two. And it does not apply
to integration tests under `tests/`, which have no module path and are already one file per
concern.

The two other test modules ADR 0061 governs both stay one file, and neither is an exception
being tolerated. `pipeline/tests.rs` is 243 lines over eight cases and `encode/tests.rs` is
335 over six: both are well inside the smell, so nothing is asking them to divide.
`pipeline`'s would also divide along a *subsystem* seam (five warm-up, three store) rather
than a clause seam, which this rule has nothing to say about. **The trigger is the line
count; the seam is what this ADR supplies once something has triggered.**
