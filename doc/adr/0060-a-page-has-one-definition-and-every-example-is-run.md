# 0060 — A page has one definition, and every example is run

Date: 2026-08-17. Status: **accepted**, and built.

## Context — two defects with one cause

**1. Nothing ran the examples.** `cargo test` neither builds nor runs an example. Five
examples carry `assert!`s that are their own signature gates, and `examples/retained.rs`
**panicked at one of them on `main` for two days** — ADR 0057 changed what a clipped
mark costs, that example's copy of the dense-text row still said 40 tiles for a page that
had stopped drawing any, and nothing anywhere reported it. An assertion nothing executes
is not a gate; it is a comment that can rot into a panic.

**2. The pages were copied.** One page under one name lived in five files. The archetype
generator — the `Archetype` struct, `outline_of`, `position`, `outline_side`, `clip_of`,
`marks_box`, `curve_clip`, `emit`, `build` — was in `tests/archetypes.rs` and in private
copies in `residue_clip.rs`, `encode_threads.rs`, `retained.rs` and `surface_measure.rs`.
Each copy carried a comment saying, correctly, that an example cannot reach a test's
module. The glyph page (107 letterforms, 5 933 fills) was copied a third time over, into
`floor.rs`, `zoom.rs` and `retained.rs`, the last under the word "verbatim".

The two are the same cause seen twice: **there was no place a page could live that both a
test and an example can reach**, so the page was written five times, and nothing executed
four of the five.

Two things the copies had already drifted, neither found by a failure and both found by
putting the definitions side by side:

| copy | what it claimed | what it was |
|---|---|---|
| `encode_threads.rs`'s `DENSE_TEXT` | "`tests/archetypes.rs`'s dense-text row" | the archetype **without its two curve clips** — a different page since ADR 0054 |
| `retained.rs`'s overflow page | "`examples/zoom.rs`'s dense page, **verbatim**" | the same geometry in the **archetypes' ink** rather than the glyph page's |

## The decision, in two halves

### 1. A page drawn by more than one target is defined in `quorra-pages`

A new workspace member: `publish = false`, a dev-dependency of `quorra-gpu`, holding the
seven archetypes, the two instrument-only pages, the glyph page, the generator that
realises them, and **each page's recorded counter row**.

A **dev-dependency is the only arrangement that reaches a test and an example**, which is
the property nothing inside `quorra-gpu` has. The precedent is `quorra-function-conformance`,
whose manifest already argues the crate-not-module half of this for a corpus.

The rule is one sentence, and it is in the crate's own documentation: *a page drawn by
more than one target is defined here; a page with exactly one reader stays with its
reader, where its reasons are.* `floor.rs`'s single rectangle and its figure page are not
fixtures — they are that instrument's subject — and they did not move.

### 2. Every example accepts `--check`, and CI runs it

`--check` is **the smallest configuration that executes every assertion the example
makes**, printing no statistics. One round instead of forty, two thread counts instead of
five, one adapter instead of two, two frames instead of forty-one. Three examples are
already their own smallest run (`window_smoke`, `present_thread`, `startup`) and take the
flag as an accepted no-op, so that one CI loop can invoke all twelve the same way.

The step runs under `xvfb-run` because two of the twelve need a window, and in `--release`
because several draw tens of thousands of marks through a software rasteriser.

**And the list is gated.** `tests/example_checks.rs` reads `.github/workflows/ci.yml` and
fails if an example exists that no line of the step names, or if a named example does not
read the flag. That is ADR 0059's rule applied a second time: *the directory is the source
of truth for what exists, the list is the source of truth for what is run, and a test is
what stops those from being different questions.*

## Why not the alternatives

**A `#[path]`-included shared module** under `crates/quorra-gpu/fixtures/`, pulled into
the test and each example by `#[path = "…"] mod pages;`. It compiles — that was checked —
and it needs no new crate. Refused because **every consumer uses a subset**, so the module
needs `#![allow(dead_code)]` to compile under `-D warnings` in six targets, and an
`allow` on a whole file is exactly the enforcement this tree does not give up for
convenience. A crate's `pub` items are never dead code, so the same content in a crate
costs no `allow` at all.

**The pages inside `quorra-gpu` behind a feature or `#[doc(hidden)] pub`.** Refused on
ADR 0051's and ADR 0059's grounds, unchanged and if anything stronger here. A public path
is a promise; `#[doc(hidden)]` is the same promise with a note asking people not to rely
on it, and it buys an invisible public surface, which is worse than a visible one because
the review that would catch a change to it no longer happens. A cargo feature adds a flag
CI can forget to set, which is what ADR 0052 is about. And a *fixture* is the least
defensible thing to promise a caller: the viewer pins us by git revision and re-baselines
a 974-page corpus on every bump, and re-cutting a page — which this project does — would
become a breaking change.

**`quorra-pages` depending on `quorra-gpu`.** It would make each call site one line
instead of five, and cargo permits the resulting cycle because it closes through a
dev-dependency — verified end to end in a scratch workspace: `cargo run --example`,
`cargo test`, `cargo clippy --all-targets` and `cargo doc --workspace` all succeed.
Refused anyway, on principle 4's "no circular dependencies". The graph would have a cycle
in it, and the fact that cargo tolerates a particular kind of cycle is not the same as the
architecture reading in one direction. The cost is stated below.

**Moving the examples into `quorra-pages`**, so the instruments sit above the library and
nothing is a cycle. Architecturally the cleanest of all of them, and refused on the
disruption: every invocation in `doc/`, in six ADRs and in **the owner's trigger loop**
(`tmp/start-measurement` → `examples/surface_measure.rs`) names `-p quorra-gpu --example`.
Breaking a person's measurement loop to tidy a dependency edge is the wrong trade.

**Making CI run each example in full.** Refused with the reason the round was given:
`surface_measure` is the owner's instrument on the real display and its forty Fifo frames
measure a display's refresh rate; `floor` and `rect_lane` sweep hundreds of rounds for
minima; `function_compile` needs twelve round-robin rounds on a quiet machine and every
sample must be a program no process has compiled. **What must be checked is that each
example's assertions still hold, and that is not the same run as the measurement.**
Separating the two is what `--check` is.

## What it costs, stated rather than discovered

1. **Five lines of plumbing at each call site.** `quorra-pages` cannot upload, so a caller
   maps `outlines(shape)` through its own `Device` and passes the identifiers back to
   `scene(shape, &ids, image)`. That is duplicated at six sites, and it is the price of
   the acyclic graph. It is *inert* duplication: it carries no page content, and getting
   it wrong fails immediately and loudly rather than drifting.
2. **The `Counters` → `Recorded` mapping is written three times** — in
   `tests/archetypes.rs`, `examples/retained.rs` and `examples/surface_measure.rs` —
   because `Counters` lives in `quorra-gpu` and this crate must not depend on it.
   Mitigated by making `Recorded` a struct of **named fields** rather than an array: a
   positional row can be written wrongly in silence, a field name cannot. And the thing
   that rotted was never the mapping — it was the recorded *number*, and that number now
   exists once.
3. **`--check` is a second code path through every instrument**, and a second code path
   is a thing that can be wrong. Bounded deliberately: `--check` may only *shrink* a
   sweep — fewer rounds, fewer adapters, fewer sizes — never substitute a different page,
   because an example that checked a page it does not measure would be the defect this
   ADR exists for, wearing a different hat.
4. **A test reads a file under `.github/`.** `tests/example_checks.rs` is coupled to the
   workflow's text, and it parses the loop by its exact opening line. That is stated in
   the test and it fails loudly — "a gate that cannot read its own list is worse than no
   gate" — rather than matching nothing and passing.
5. **One page's content changed.** `retained.rs`'s overflow page now draws in the glyph
   page's ink (`0.1, 0.1, 0.1`) rather than the archetypes' (`0.12, 0.13, 0.16`), which is
   what its own comment claimed and what the other two copies had. Nothing that section
   reads is a function of a solid fill's colour — it reads encode sources, atlas working
   set, distinct keys, residency and tiles — and the run before and after is identical:
   `working set 165596 bytes over 107 distinct keys; 102 resident`, which is the pair its
   doc comment records. This is the one exception to the round's own rule against changing
   a page, taken because keeping two inks is keeping the drift the round exists to remove.

## Verified able to fail

- **The round's whole point.** `examples/retained.rs`'s signature gate, broken
  deliberately by moving `DENSE_TEXT`'s recorded `tiles` from 40 to 41: today that is a
  silent panic nobody sees until someone runs the example by hand; now
  `cargo test --workspace` fails at `the_archetypes_cost_what_they_are_recorded_to_cost`
  **and** the CI `--check` step fails at `retained`. One edit, two red gates, where before
  it was one edit and no signal for two days.
- `every_example_is_run_by_ci` — a `probe_unlisted.rs` added to `examples/` and named
  nowhere: `["probe_unlisted"] exist under examples/ and are not run by …`.
- `every_example_reads_the_check_flag` — run before `window_smoke` and `present_thread`
  were taught to consume the flag: `["present_thread", "window_smoke"] do not read
  `--check``.

## What this does not do

It does not re-cut any page. Every archetype counter row is unchanged, character for
character, which is the evidence that moving the definition moved no scene — and it is
checked by the same equality gate that would have caught it.
