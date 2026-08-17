# Where a page lives, and what runs the assertions about it

Round notes for 2026-08-17. The decision is ADR 0060; this file is the working — what was
found by reading, what was measured, what compiles and what does not, and the three things
left undone with their reasons.

**The short version.**

- Two defects, one cause: **there was no place a page could live that both a test and an
  example can reach**, so the archetype generator was written five times, and **nothing
  executed four of the five**. `cargo test` neither builds nor runs an example.
- The answer is a `publish = false` workspace member, `quorra-pages`, reached as a
  **dev-dependency** (which is the only edge that serves `tests/` *and* `examples/`), plus
  a `--check` mode on every example and one CI step that runs all twelve.
- The list of twelve is itself gated against the `examples/` directory, which is ADR 0059's
  rule used a second time.
- **No page's content changed** except one ink, which is argued in §4 and which moves no
  number the affected section reports. Every archetype counter row is identical.

---

## 1. What was actually copied, counted

Before this round, by file:

| page | defined in | copies |
|---|---|---|
| archetype generator (7 pages) | `tests/archetypes.rs` | `residue_clip.rs`, `encode_threads.rs`, `retained.rs`, `surface_measure.rs` |
| glyph page (107 × 5 933) | `floor.rs` | `zoom.rs`, `retained.rs` |
| `zoomed` / `magnified` | `zoom.rs` | `retained.rs` |

**Two of the copies had drifted, and neither had ever failed anything.**

- `encode_threads.rs`'s `DENSE_TEXT` says it is "`tests/archetypes.rs`'s dense-text row"
  and has `clips: 0, clipped: 0` where the archetype has `2` and `40`. It has been a
  different page since ADR 0054 measured the thread sweep on it. **It is not changed
  here** — re-cutting a page in the round that moves it is exactly what
  `doc/notes-clipped-instrument.md` §3.4 warns about — it is *named*:
  `quorra_pages::DENSE_TEXT_UNCLIPPED`, with a doc comment saying what it is not and
  leaving the question of which page the sweep should run on to the round that measures it.
- `retained.rs`'s overflow page says it is "`examples/zoom.rs`'s dense page, **verbatim**"
  and drew in a different ink. §4 below.

The drift was invisible because comparing five files is something nobody does. Putting the
definitions in one file made both visible in the first ten minutes, which is the round's
cheapest finding and the argument for the register in one sentence.

---

## 2. What compiles — established before designing around it

Three arrangements were checked rather than reasoned about, because the round's brief said
to.

**A dev-dependency cycle works.** A scratch workspace with `a` dev-depending on `b`, and
`b` depending on `a`: `cargo run --example` (in `a`, using `b`, which uses `a`), `cargo
test --workspace`, `cargo clippy --workspace --all-targets` and `cargo doc --workspace
--no-deps` all succeed. So the one-line-per-call-site version of this round *was*
available. It was refused on principle 4 rather than on feasibility, and the ADR says so —
"cargo tolerating a particular kind of cycle is not the same as the architecture reading in
one direction".

**A `#[path]`-included module works too**, and is the version that needs no new crate. It
was refused on a concrete cost rather than a taste: every consumer uses a *subset* of the
generator, so the file needs `#![allow(dead_code)]` to build under `-D warnings` in six
targets. A crate's `pub` items are never dead code.

**`tests/common/` is not reachable from `examples/`**, and neither is a `#[cfg(test)]`
module. That was the constraint each of the four copies' comments already stated correctly;
what none of them had noticed is that a **dev-dependency is reachable from both**.

---

## 3. What the crate holds, and the rule that bounds it

`crates/quorra-pages`, four modules and a `lib.rs`:

- `archetype.rs` — what an archetype *is*, and the pure arithmetic that places its marks
  and cuts its clips. No device, no clock, no randomness.
- `build.rs` — `outlines`, `image_spec`, `scene`: resources out, scene in.
- `page.rs` — the named pages and **each one's recorded counter row**.
- `glyph.rs` — the glyph page, both phase variants, and `zoomed`.

The rule, stated in `lib.rs`: **a page drawn by more than one target is defined here; a
page with exactly one reader stays with its reader.** `floor.rs`'s `one_rect`, its
`dense_page` of 5 933 rectangles and its `figure_page` did not move — each has one reader
and each is that instrument's subject rather than a fixture.

**Nine pages are named, and naming them is the deliverable.** Seven archetypes,
`CALLERS_DRAWING` (the caller's file at its own 58 009 commands, which only the thread
sweep draws) and `DENSE_TEXT_UNCLIPPED`. The two instrument-only pages carry
`recorded: None`, so "a page no gate has priced" is a state the type can hold and not a
gap in a table.

`Recorded` is a struct of ten **named** fields rather than a `[u64; 10]`. Each consumer
builds one from its own `Counters` — three of them do — and a field name is a mapping that
cannot be written wrongly in silence. The recorded *numbers* are the crate's and exist
once, which is the half that rotted.

---

## 4. The one page-content change, and why it is not a re-cut

`retained.rs`'s overflow page drew `Color::new(0.12, 0.13, 0.16, 1.0)` — the archetypes'
ink — where `floor.rs` and `zoom.rs` both drew `Color::new(0.1, 0.1, 0.1, 1.0)`. Its own
comment said "verbatim". It is now the glyph page's ink, i.e. what the comment claimed.

**Nothing that section reads is a function of a solid fill's colour.** It reads the encode
source per frame, `atlas_working_set_bytes`, `atlas_distinct_keys`, `atlas_entries`,
`tiles` and `atlas_repacked`. Run after the change:

```
a page the atlas cannot hold — 107 letterforms at 4×, atlas 262144 bytes
  working set 165596 bytes over 107 distinct keys; 102 resident, 21 tiles on the scratch sheet
  encode sources [E], repacks 0
```

`107 distinct keys, 102 of them resident` is the pair the section's own doc comment
records, which is what says the page is still the page the band was measured on.

The alternative was to keep both inks as two named pages. Refused: that is keeping the
drift the round exists to remove, in a place where a reader would have to work out that the
difference is deliberate.

---

## 5. The assertions, and what now runs them

`--check` on every example: **the smallest configuration that executes every assertion it
makes**, and no statistics.

| example | full run | `--check` |
|---|---|---|
| `encode_threads` | 4 shapes × 5 thread counts × 3 rounds | × 2 counts × 1 round |
| `retained` | 40 round-robin rounds + 12 overflow frames | 1 round + 1 overflow frame |
| `residue_clip` | 21 frames | 2 |
| `surface_measure` | 8 rounds × 41 frames | 2 × 3 |
| `floor` | 2 adapters × 3 sizes × 11 frames | 1 adapter, 1 size, 2 frames |
| `rect_lane` | 2 adapters × (400, 60) rounds | 1 adapter, 1 round |
| `zoom` | 9 magnifications × 6 + a 24-frame sweep | 2 magnifications × 1 |
| `function_compile` | 12 round-robin rounds | 1 |
| `function_paint` | 2 adapters | 1 |
| `startup`, `window_smoke`, `present_thread` | — | the run itself; the flag is an accepted no-op |

**`--check` may only shrink a sweep, never substitute a page.** An example that checked a
page it does not measure would be this round's defect wearing a different hat, and that
rule is written into the ADR rather than left as an intention.

**What the loop costs, measured here rather than guessed.** All twelve, under `xvfb-run`,
release, llvmpipe and RADV both present, on this desktop at load average 17–45: **114 s**
with the binaries already linked, and **~8 min** when every example relinks first (fat
LTO, one codegen unit — that is the profile's cost, not the step's). The two longest are
`function_paint` (54 s: it runs both witnesses through an analysis, a generated shader and
a CPU reference) and `surface_measure` (11 s: two surface devices, each warmed). Nothing
else exceeds ten seconds of run time.

**Three examples gained an assertion they did not have.** `residue_clip` and
`surface_measure` printed counters and checked nothing — `surface_measure` is the instrument
`doc/PLAN.md`'s real-display row is read from, and nothing said the page it drew was the
page the row is attributed to. Both now compare the frame's counters against the page's
recorded row. `surface_measure`'s viewport is `WIDTH × HEIGHT` whatever the window
negotiates, so the comparison is the same exact function of the scene everywhere, which is
what makes it safe to assert on the owner's display.

**The list is gated.** `tests/example_checks.rs`:

- `every_example_is_run_by_ci` — reads `.github/workflows/ci.yml`, parses the loop's list,
  and compares it against the `examples/` directory in **both** directions.
- `every_example_reads_the_check_flag` — every example named there contains the literal
  `"--check"`, so a name added to the list whose example takes a positional adapter
  substring first cannot silently treat the flag as an adapter name.

Both were run against a forced defect; §7 has the output.

---

## 6. Every archetype row is unchanged

The whole point of moving a definition is that nothing else moves with it. The counter gate
compares by equality and passed on the first run after the rewire:

```
median page    Recorded { commands: 12, … }
dense text     [4320, 0, 818, 2164, 1, 40, 0, 0, 40, 8956]
artwork        [684, 0, 300, 300, 1, 600, 3, 66, 384, 3542360]
image page     [232, 0, 60, 158, 4, 0, 0, 0, 0, 0]
clip mountain  [1200, 0, 200, 800, 1200, 0, 0, 0, 0, 0]
giant          [1500, 0, 1500, 1500, 0, 0, 0, 0, 0, 0]
drawing        [1200, 0, 1200, 1194, 0, 6, 0, 0, 0, 245]
```

Two of those rows are hard to get right by accident and are the evidence the generator is
the same generator: **image page's 232 and 158**, which depend on the unit-rectangle clip
outline being uploaded *after* the sixty mark outlines and **not** being folded into the
`index % outlines.len()` that picks a command's outline. Folding it in was the one real
hazard in the rewrite — it silently renumbers every command on a rect-clipped page — and
the code separates `marks` from `rectangle` with a comment saying so.

---

## 7. Verified able to fail

**The round's whole point**, forced by moving `DENSE_TEXT`'s recorded `tiles` from 40 to
41 — the shape of the defect ADR 0057 caused:

```
(before this round) examples/retained.rs panics; cargo test --workspace is green;
                    CI is green; nobody knows for two days
(after)             cargo test --workspace fails:
                      the_archetypes_cost_what_they_are_recorded_to_cost
                        dense text: got … tiles: 40 …  recorded … tiles: 41 …
                    and the CI --check step fails at `retained`
```

One edit, two red gates, where the same edit produced no signal at all before.

**The list gate**, with a `probe_unlisted.rs` added to `examples/` and named nowhere:

```
["probe_unlisted"] exist under examples/ and are not run by …/.github/workflows/ci.yml.
Nothing else runs an example — `cargo test` does not build one — so every assertion in
them is a comment (ADR 0060). Add each to the `--check` step.
```

**The flag gate**, run before `window_smoke` and `present_thread` were taught to consume
the flag rather than merely document it:

```
["present_thread", "window_smoke"] do not read `--check`.
```

That one is worth recording for its own sake: the first version of both examples carried a
doc comment *saying* `--check` was accepted and no code that read it. A gate over a claim
in prose is what this whole round is about, so the gate is over the code.

---

## 8. What was found and deliberately not done

- **`tests/perf_gate.rs` still builds its own 5 933-rectangle page.** It is the page
  `tests/archetypes.rs`'s own module comment calls invented — no corpus document emits a
  `Command::Rect` — so bringing it into the register would be recording a page rather than
  sharing one. It has one reader, which is the rule's answer.
- **`floor.rs`'s `figure_page` and `dense_page` stayed put.** One reader each.
- **The `Counters` → `Recorded` mapping is written three times**, and no arrangement was
  found that removes it without `quorra-pages` depending on `quorra-gpu`. Named fields are
  the mitigation, not the fix. If a fourth consumer appears, the question is worth
  re-opening — with the cycle on the table, since by then the five-lines-per-site argument
  will have changed shape.
- **`DENSE_TEXT_UNCLIPPED` was not reconciled with `DENSE_TEXT`.** ADR 0054's numbers were
  taken on it. Whether the thread sweep should run on the archetype — which would put its
  40 residue-clipped marks into the "does not divide" column where artwork already is — is
  a question with a measurement attached, and it belongs to the round that takes it.
- **`--check` runtimes were not tuned.** §5's table is what each configuration *is*, not a
  budget. If the step becomes the long pole in CI, the two to shorten first are
  `encode_threads` (whose caller's-page shape is 58 009 commands on a software rasteriser,
  twice) and `function_paint`.
- **No corpus run.** Nothing this round changes what is drawn: `Device::render` is
  byte-for-byte the same function, the archetype rows are identical, and the one ink that
  changed is in an example's own page.
