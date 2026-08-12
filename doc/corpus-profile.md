# The corpus profile: what a page of a real document is shaped like

Status: **record** — a measurement, its date, and how to redo it. The numbers it produced
live in `crates/quorra-gpu/tests/archetypes.rs`, which is the only place they are used.

## What this is, and what it is not

**It is a set of counts.** Nothing else from the measurement is in this repository: no
document, no display list, no recorded scene, no path or dependency on the project the
counts came from. Delete that project from the machine and `archetypes.rs` still
compiles, runs and means the same thing — which is the test this design was held to.

**It is not a corpus.** Recording the intermediate form of those pages was considered and
rejected: the scene halves alone are 60.5 MB across 995 pages and the resources are
unbounded (one page holds 94 MB of decoded image), and — the reason that settles it — the
pages are third-party PDFs whose terms we cannot establish. The project that owns the
corpus does not redistribute 459 of the 1 434 files itself, which is that project
declining to ship them. A display list is a mechanical transformation of a page's
expression, so it inherits the question; a count of its commands does not.

## The measurement

**2026-08-12**, over the 995 first pages the caller's `render-quorra` corpus gate draws,
by walking each `quorra_scene::Scene` that gate built and counting. Per page:

| quantity | median | p90 | p99 | max | pages with any |
|---|---|---|---|---|---|
| drawing commands | 12 | 588 | 4320 | 66309 | 819 |
| solid fills | 10 | 460 | 4295 | 66304 | 731 |
| strokes | 0 | 6 | 405 | 34970 | 259 |
| `Command::Rect` | 0 | 0 | 0 | 0 | 0 |
| image placements | 0 | 1 | 32 | 2156 | 106 |
| shading/mesh fills | 0 | 0 | 11 | 3511 | 50 |
| groups | 0 | 0 | 8 | 29 | 52 |
| knockout groups | 0 | 0 | 1 | 10 | 13 |
| non-isolated groups | 0 | 0 | 0 | 3 | 8 |
| non-Normal blends | 0 | 0 | 4 | 22 | 53 |
| commands under a clip | 0 | 44 | 3498 | 15004 | 295 |
| commands under a soft mask | 0 | 0 | 4 | 25 | 32 |
| group nesting depth | 0 | 0 | 1 | 2 | 52 |
| distinct outlines | 9 | 117 | 818 | 65978 | 786 |
| clip regions defined | 0 | 4 | 185 | 15004 | 302 |
| soft masks defined | 0 | 0 | 4 | 25 | 32 |
| **placements per distinct outline** | 1.33 | 8.28 | 22.9 | 512 | 786 |

Nothing here is a rate or a ratio of ours; every column is a count of a thing on a page.

## The four findings that changed our fixtures

**Not one page emits a `Command::Rect`.** The lane is real, reachable and documented,
and every rectangle a document draws arrives as a `Fill` whose outline happens to be
one. Our flagship performance fixture draws 5 933 of them. It is not wrong, but it is a
floor measurement rather than a page measurement, and it says so now.

**Glyph reuse is 1.33 at the median, not 55.** The brief's dense page — 5 933 fills over
107 outlines — is the p99.9 of reuse, not the typical case. Half of all pages place each
outline once or twice. That is why a cold atlas is the normal state of a page turn and
why every archetype renders on a fresh device.

**The median page is twelve commands.** The mass of the corpus is trivially small pages,
and everything interesting is in the tail: 89 pages of 995 exceed 100 KB of scene, one
holds 66 309 commands and another 15 004 clips. A profile of averages would have
described nothing.

**Groups are rare and shallow.** Fifty-two pages use one at all, nesting reaches depth 2,
thirteen pages knock out, eight are non-isolated. The clause-11 machinery that dominates
this library's design is exercised by 5% of documents — which is an argument for
correctness, not for optimisation.

## How to redo it

The measurement needs a checkout of the consuming project, a scratch copy of it, and a
temporary probe in this one; none of that is committed here. In outline: copy their tree
to a scratchpad, point its `[patch]` at a worktree of this repository carrying a probe
that walks `Scene::commands()` and prints one line per page, run their corpus gate, and
aggregate. The memory note `corpus-gate-from-render-lib` carries the invocation.

Re-measure when their document mix changes enough to matter, and when you do, update
`archetypes.rs` and this file in the same commit — an archetype that no longer matches
a measured shape is a fixture pretending to be evidence.
