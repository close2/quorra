# Splitting `error.rs`: whether it should be, and where each part went

Written 2026-08-17, against CLAUDE.md principle 1's file-scale rule and `doc/adr/0051`,
and in the shape of `doc/notes-device-split.md` and `doc/notes-encode-split.md`.
`error.rs` was **583 lines** — since the `encode.rs` round, the largest file in
`quorra-gpu` past the ~500-line smell that nobody had opened, and the one
`doc/HANDOVER.md` names as the next split candidate.

No ADR accompanies it. ADR 0051 decided the only question a split of a *public* module
raises — private submodules, re-exported from the parent, no new public path — and
nothing here needed a decision that is not the application of it. The two judgements
worth recording are §1 (whether to split at all, since declining was a legitimate
outcome and would itself have needed an ADR) and §5 (what was left alone).

## 1. The first question: is this one irreducible thing?

CLAUDE.md is explicit that the line count is a smell and not a verdict — "a file that is
genuinely one irreducible thing at 600 lines is fine *when the module comment says what
that one thing is*" — and an error enum serving a whole crate is a plausible candidate
for exactly that. The file even had a one-sentence answer ready: *every way this crate
refuses*. So the split was argued against before it was taken.

**It is seven things, and three pieces of evidence say so.**

*It holds seven items, not one.* An enum is one item and cannot be split; a file of seven
of them can. Read whole, in order:

| # | Item | Lines | Raised in |
|---|---|---|---|
| 1 | `DeviceError` | 84 | `device/construct.rs`, `device/resident.rs`, `resources.rs`, `startup.rs`, `surface.rs` |
| 2 | `ResourceProblem` | 47 | `resources.rs`, and nowhere else |
| 3 | `FunctionProblem` | 41 | `function/admit.rs`, and nowhere else |
| 4 | `PipelineProblem` | 60 | `pipeline.rs`, `pipeline/function.rs` |
| 5 | `LayerProblem` | 69 | `present/layer.rs`, `present/pass.rs` |
| 6 | `SurfaceProblem` | 19 | `surface.rs`, and nowhere else |
| 7 | `RenderError` | 232 | eighteen files across `device/`, `encode/`, `compose/`, `present/` |

Line counts are the items themselves, so they exclude the 25 lines of module comment and
imports above them. **Five of the seven are raised inside one subsystem each**, which is
the seam: a round that changes what a presenter accepts has business in exactly 69 of
those 583 lines.

*Its module comment was already a map.* Eight of its twenty lines existed to tell a
reader which of the types below were the refusals and which were the vocabularies they
carry. A comment that has to enumerate a file's parts is describing a directory.

*It grows by whole new types, one per round.* Eleven commits have touched it. Three of
them added an entire vocabulary at a stroke — `PipelineProblem` (+44, ADR 0042),
`FunctionProblem` (+42, ADR 0053), `LayerProblem` (+70, ADR 0056) — and each of those
rounds owned a subsystem, not the error file. The honest other half: `RenderError` is
touched by eight of the ten commits after creation, because a frame is where everything
lands. That is an argument for its own module, not against the split.

**What declining would have cost**, since that was the alternative: the next round to add
a vocabulary — the tiling seam and the caller's §15 both have one in view — would have
opened a 650-line file to add sixty lines it alone owns, and the *only* place its
relationship to the rest was written would still be a prose paragraph at the top.

## 2. The seams

Two commits: the split, which moves text verbatim, and the four doc corrections §4
records separately so that the move can be reviewed as a move.

| Module | Lines | Its one thing |
|---|---|---|
| `error.rs` | 68 | the map: which module holds what, and which enum carries which vocabulary |
| `error/device.rs` | 105 | what a device refuses when no frame is in flight |
| `error/render.rs` | 258 | why one frame — or one present — was refused |
| `error/resource.rs` | 61 | what an upload's content violated |
| `error/function.rs` | 56 | why a program the device would have evaluated is not admitted |
| `error/pipeline.rs` | 73 | what this adapter would not build |
| `error/layer.rs` | 80 | what a layer handed to a presenter did not satisfy |
| `error/surface.rs` | 34 | what the swapchain answered instead of handing over a texture |

`error/surface.rs` at 34 lines is small on purpose. The rule taken was **one vocabulary,
one module, named for the subsystem that raises it**, and a nineteen-line enum that
mirrors `wgpu`'s five non-success arms is not improved by being a lodger in a file about
something else.

### Visibility: nothing widened, and no public path was added

Every one of the seven types was already `pub` in a `pub mod`, so nothing widened at all;
the modules are `mod`, and the parent re-exports the seven names. `quorra_gpu::error::
RenderError`, `quorra_gpu::RenderError` and `quorra::RenderError` all resolve exactly as
before, and `quorra_gpu::error::render::RenderError` is not a path anybody can write — so
it is not a path we have to keep (ADR 0051).

The caller names `quorra_gpu::DeviceError` and `quorra_gpu::RenderError` (their
`render-quorra/src/lib.rs`, `#[from]` on both) and mentions `LayerProblem` in two doc
comments. All four spellings are unchanged.

### One doc link was rewritten with its path

`PipelineProblem`'s comment says it is "its own type rather than a [`RenderError`]
variant's fields". That link resolved through the file the two types shared; it is now
`[`RenderError`](crate::error::RenderError)` — same rendered text, and no import that
exists only for rustdoc's benefit (ADR 0051 §2). It is on a **public** item, so
`RUSTDOCFLAGS="-D warnings" cargo doc` catches it; the private-item warnings were checked
too, and all 34 of them are elsewhere in the crate.

## 3. Behaviour: nothing moved, and here is the diff that says so

- **Text.** The seven modules' bodies, concatenated with their new module comments and
  imports stripped, diff against lines 26–583 of the pre-split file in **exactly five
  hunks** — the one link rewrite above and the four doc corrections in §4. Not one line
  of code, not one `#[error(...)]` format string, not one field, not one derive differs.
- **`tests/archetypes.rs`**, the instrument this round was given: identical before and
  after, and unchanged as a file.

  | archetype | counters |
  |---|---|
  | median page | `[12, 0, 9, 12, 0, 0, 0, 0, 0, 0]` |
  | dense text | `[4320, 0, 818, 2164, 1, 0, 0, 0, 0, 0]` |
  | artwork | `[684, 0, 300, 300, 1, 8, 3, 2, 6, 12284]` |
  | image page | `[232, 0, 60, 158, 4, 0, 0, 0, 0, 0]` |
  | clip mountain | `[1200, 0, 200, 800, 1200, 0, 0, 0, 0, 0]` |
  | giant | `[1500, 0, 1500, 1500, 0, 0, 0, 0, 0, 0]` |
  | drawing | `[1200, 0, 1200, 1194, 0, 6, 0, 0, 0, 245]` |

- **Tests**: `cargo test --workspace`, cargo's own exit status read from a file rather
  than through a pipe — **459 passed, 0 failed, 2 ignored, 57 suites** before and the
  same after, and the sorted list of **461 test names is identical, name for name**. The
  count reconciles: 458 `#[test]` in `crates/` plus 3 doctests.
- **Clippy**: `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` clean,
  with `Checking` printed for all four crates rather than only `Finished`. No new
  `#[allow]` anywhere.
- **Docs**: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` clean.
- **Format**: `cargo fmt --all --check` clean.

It cannot be a performance change: the release profile is `lto = "fat"` with
`codegen-units = 1`, so a module boundary is not a codegen-unit boundary, and nothing
here has a body at all.

## 4. Four doc corrections, in their own commit

Each is a variant or type whose doc did not meet the standard its neighbours set — name
what failed, and where a limit was exceeded, the limit and the number that hit it.

1. **`DeviceError`'s type doc was wrong, not merely thin.** "Why a device could not be
   constructed" has been inaccurate since the upload surface arrived: five of its nine
   variants are residency, not construction. It now says both, and says that being
   *outside a frame* is what makes them one enum.
2. **`SurfaceProblem::Validation` said "reported a validation problem"** and nothing
   about why it carries no detail. `wgpu` 30's `CurrentSurfaceTexture::Validation` arm
   carries none either — the message went to an error scope or the uncaptured-error
   handler when it was raised — so the gap is named rather than left to be discovered.
3. **`ResourceProblem::RampColorInvalid` said "outside its range"** where its own
   `#[error]` string is more precise than its doc. It now names the four components and
   the range, and links `Color::is_valid`, which is the function that decides.
4. **`RenderError`'s type doc said "a frame"** and has been carrying three
   presenter variants since ADR 0056. It now names a present as a frame's last step
   happening somewhere else, and says a refused present acquires nothing.

**No clause citation was added or removed.** The ISO clauses this file cites — §8.5.2 for
a path starting with `m`, §7.10.5 for a type 4 function — were checked against what they
govern and both hold. What was added instead is a convention statement in the parent
module comment: in this module a bare `§n` is the brief and an ISO clause is written in
full, because the file mixed the two with no marker (`(§4.7)` is the brief, `§7.10.5` is
the specification, and only a reader who already knew could tell).

## 5. What was judged irreducible, and left

- **`error/render.rs` at 258 lines.** One enum, and an enum is one item: there is no seam
  inside it that Rust would let us take, and inventing a second "why a frame was refused"
  type to shorten a file would be the arbitrary split CLAUDE.md warns against. Its module
  comment says what the one thing is.
- **`DeviceError` holding construction *and* residency.** They are two phases, but they
  are one phase from the caller's side — everything that is not a frame — and the four
  constructors and six residency calls return the same type today. Splitting the enum is
  a public API change, which is a bump's business and not a refactor's.
- **The five vocabularies stayed five enums.** Merging any two would end the property
  every one of them exists for: "how often does this happen?" must stay answerable.
- **Nothing was moved into or out of the module.** `report::ReportKind` is the deliberate
  neighbour — a frame that *was* drawn — and it stays where it is.

## 6. Findings noticed and deliberately not acted on

1. **`ResourceProblem::OutlineCoordinateTooLarge` names the limit but not the coordinate
   that hit it**, which is the standard its neighbours meet (`RampOffsetOutOfRange`
   carries the offset, `ImageInconsistent` all three numbers). Adding the field is a
   public API change and a message change, so it is not a no-behaviour round's business.
   `RampUnordered` is the same shape: it names neither the stop nor its two offsets.
2. **`RenderError::FrameBudgetExceeded` does not say which lane asked.** Its own doc says
   why — "the budget they share is one number" — and the two raising sites are an encode
   and a compositor texture. Worth revisiting only if a caller ever has to tell them
   apart; recorded so the next reader does not re-derive the question.
3. **`error/function.rs` and `error/surface.rs` share a name with `crate::function` and
   `crate::surface`.** That is deliberate — a vocabulary is named for the subsystem that
   raises it — and the paths are unambiguous, but a reader skimming a stack trace should
   know the two exist.
4. **`SurfaceProblem` is the one type here that is not an `Error`.** It derives `Debug,
   Clone, Copy, PartialEq, Eq` and reaches a message through `{reason:?}` in
   `RenderError::SurfaceUnavailable`, so its five variants print as identifiers rather
   than sentences. Giving it `thiserror::Error` would change the text of a public error
   message, which is why this round did not.
