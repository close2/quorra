# 0059 — A gate over a private list lives inside the crate

Date: 2026-08-17. Status: **accepted**, and built.

## Context — the second list had already gone one shader behind

`src/shaders.rs` exists because two `include_str!` lists of the same files can drift; its
own module comment says so, and the 2026-08-15 debt round wrote it while adding the
uniform-layout gate. `tests/shader_copies.rs` then kept **its own** `include_str!` list of
eight shaders, because an integration test compiles against the crate's *public* surface
and `shaders` is private.

That second list was not a hypothetical risk. It was already wrong:

| | `src/shaders.rs` | `tests/shader_copies.rs` |
|---|---|---|
| shaders named | 10 (+ `function_ops.wgsl` via `function::OPERATORS`) | 8 |
| missing | — | `present.wgsl`, `function_lane.wgsl` |

`function_lane.wgsl` (ADR 0053) defines a **sixth** `soft_mask_value` and carries the
comment promising the copies are kept textually the same. The gate read five, asserted
there were five, found five, and passed. Both of its two tests — the drift check and the
"nothing promises sameness unguarded" check — were blind to that copy from the day it
landed, and **every run was green.**

Measured rather than argued: with a line of drift planted in `function_lane.wgsl`'s copy,
`cargo test -p quorra-gpu --test shader_copies` reports `2 passed`. The same drift fails
the gate this ADR builds. (The six copies are in fact byte-identical today, so nothing was
drawn wrongly — what was lost was the guarantee, for two milestones.)

## The decision

**A gate that reads a private list is a unit test inside the crate, not an integration
test with a copy of the list.** `tests/shader_copies.rs` moved to
`src/shaders/copies.rs`, a `#[cfg(test)]` module beside `layout.rs`, and reads
`super::ALL` — one list, the one the pipeline store compiles from.

And because a list can be complete against itself while the directory has moved on,
`shaders.rs` gains a test that compares `ALL` against `src/shaders/*.wgsl` **on disk**. The
directory is the source of truth for what exists; the list is the source of truth for what
is compiled and gated; the test is what stops those two from being different questions.
A `.wgsl` file that exists and is not named fails the build.

`ALL` includes `function_ops.wgsl` under `crate::function::OPERATORS`. It is not a
pipeline's shader — it is the operator library a generated program is built from — but it
is WGSL text this crate holds, and a gate over shader *text* has no reason to skip it.

## Why not the alternatives

**Make the list `pub`.** ADR 0051 refused a public path for a *module* on the grounds that
a public path is a promise, and this is a stronger case, not a weaker one: the promise
would be about the crate's file layout — that these ten files exist under these names —
made to a caller that has no use for it. `PRESENT` and `FUNCTION_LANE` were added in the
last two rounds and `winding.wgsl` may yet be split; each of those would be a breaking
change to a surface whose only consumer is a test in the same repository. The viewer pins
us by git revision and re-baselines a 974-page corpus on every bump (ADR 0051), and a path
break costs them a round for nothing gained.

**`#[doc(hidden)] pub`.** The same promise with a note asking people not to rely on it.
`#[doc(hidden)]` hides an item from rustdoc; it does not make it private, does not stop a
downstream crate from importing it, and does not make removing it a non-breaking change.
It buys an invisible public surface, which is worse than a visible one because the review
that would catch a change to it no longer happens.

**A `build.rs`.** It would enforce the same property one phase earlier, and it is the only
option that fails a *build* rather than a test run. Refused on cost: this workspace has no
build script at all, a build script runs on every consumer's machine including the
caller's, it must be kept correct against `cargo`'s rerun-if-changed rules, and it buys
nothing over a unit test that CI already runs. `Cargo.toml`'s standing note that a
benchmark harness does not live in this tree is the same instinct — machinery earns its
place by what it catches that the simpler thing cannot.

**Leave it and add the missing shaders to the second list.** That fixes today's instance
and leaves the mechanism: the next shader added is one edit in `shaders.rs` and one in
`tests/`, and forgetting the second is silent again. This is the same shape as the
duplicated tile arithmetic the previous round closed, and the same answer.

## What it costs, stated rather than discovered

1. **The gate no longer proves the *public* crate compiles its shaders.** An integration
   test links the crate as a consumer does; a unit test does not. Nothing here depended
   on that — the gate reads text and compiles no pipeline — and the property that a shader
   the adapter rejects fails the suite by name is ADR 0042's, which is unaffected.
2. **`shaders.rs` grows a test module and a filesystem read.** The read is
   `env!("CARGO_MANIFEST_DIR")`-rooted, so it is correct wherever `cargo test` is invoked
   from, and it is `#[cfg(test)]`, so nothing ships. A crate consumed as a git dependency
   never runs it.
3. **`ALL` is a second list *in the same file* as the constants.** A constant not named in
   `ALL` is still possible — but it can only be a constant pointing at a file, and the
   directory test would then fail on that file. So the drift class is bounded by
   construction rather than by care, which is what the previous arrangement lacked.
4. **`tests/` is one file smaller, and the crate's test count moved from an integration
   binary to the lib binary.** Anything comparing counts across this commit must add the
   two together.

## Verified able to fail

Three forced defects, each run and each named by the message it produced:

- a `probe.wgsl` added to `src/shaders/` and named nowhere →
  `every_wgsl_file_is_named_here` fails with `["probe.wgsl"] exist unnamed`;
- that file named in `ALL` and carrying the sameness promise above an unguarded function →
  `every_sameness_promise_is_guarded` fails with "`probe` … nothing checks it";
- one line of drift in `function_lane.wgsl`'s `soft_mask_value` →
  `promised_helpers_are_textually_identical` fails naming both shaders, where the
  integration test it replaces passed.
