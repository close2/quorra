# 0061 — A split module's tests keep the parent's path

Date: 2026-08-16, rebased 2026-08-17. Status: accepted, and built.

ADR 0051 decided **where the parts of a split module go** and answered it for the parts
a caller can name. It says nothing about the `#[cfg(test)] mod tests` those files carry,
and three rounds have now had to decide that on their own — `encode.rs` (which took this
answer without stating it), `raster.rs` and `pipeline.rs`.

## The decision

**When a module is split, its test module moves to `<module>/tests.rs` and keeps the
path `<module>::tests`.** It is not divided along the seams the source was divided
along, and not in the same round.

```rust
// in the parent, exactly as before:
#[cfg(test)]
mod tests;          // now the file `raster/tests.rs`
```

## Why

**A test's name is its module path, and a rename is a change to the gate.** `cargo test
-- --list` reports `raster::tests::aligned_rectangle_is_exact`; move that test into
`raster/fill.rs` and it becomes `raster::fill::tests::aligned_rectangle_is_exact`.

That matters because of what a split round can offer as evidence. A refactor of a
rasteriser cannot be proved correct by review — the file it touches is the hottest and
most safety-critical in the tree, and three arithmetic defects have been found in it. What
it *can* offer is that **the sorted list of test names is identical before and after, and
every one of them passes**. Divide the tests in the same round and that instrument is
gone: the list differs on every line, and nothing distinguishes "the same tests, renamed"
from "a test lost and another added" without reading all of them.

The second reason is weaker but real: **the tests often do not divide along the source's
seams.** `raster`'s split is three clauses — flattening, filling, stroking — and
`each_cap_deposits_the_area_table_53_gives_it` is a statement about §8.4.3 *read out of*
§8.5.3.3's coverage bytes. Most of that file's cases go through all three parts, so a file
per source module would put most of them in the wrong one. `pipeline`'s tests divide more
cleanly (five warm-up, three store) and are still one file, because the first reason binds
either way.

## What it costs, stated rather than discovered

1. **A test file may sit past the ~500-line smell.** `raster/tests.rs` is 704 lines. That
   is a real cost and it is why this ADR says "not in the same round" rather than "never":
   dividing them is legitimate work, with the renames as the visible change and **nothing
   else moving**, so the diff is exactly a list of names and a reviewer can check it.
2. **A test can end up in a different file from the code it exercises.** `pipeline`'s five
   warm-up tests are a sibling of `warm.rs`, not inside it. The mitigation is the one this
   project already uses everywhere: the test file's module comment says what it holds, and
   names the follow-on.
3. **The imports change even though the tests do not.** `use super::{…}` reaches the
   parent's re-exports; anything the parent does not re-export is imported from the module
   that owns it (`use super::flatten::cubic_tolerance;`). That is a change to the test
   file, and it is the only one — it shows up in review as an import block and nothing
   else.

## Where this applies

Every split of a module that carries a `#[cfg(test)] mod tests` in this workspace. It
does not bind integration tests under `tests/`, which have no module path to preserve —
the 2026-08-15 debt round divided `retained_frame.rs` into five files and renamed nothing,
because a file under `tests/` is its own binary.
