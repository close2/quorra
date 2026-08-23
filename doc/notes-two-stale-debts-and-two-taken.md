# Four recorded debts: two were already closed, one is taken, one is declined

Round notes written 2026-08-23 against this worktree, over four entries of
`doc/HANDOVER.md`'s "Small debts, none blocking".

**The short version.**

| # | debt | outcome |
|---|---|---|
| 1 | `fill_solid` repeats `encode_fill`'s `HashMap` lookup | **already closed**, 2026-08-16. The bullet is stale. |
| 2 | `tests/shader_copies.rs` keeps its own `include_str!` list | **already closed**, ADR 0059. That file does not exist. |
| 3 | `m3.rs`'s `UNORM_TOLERANCE` needs a derivation of its own | **taken** — ADR 0077, and the bound is now read at the pixel |
| 4 | should the thread sweep run the dense-text archetype? | **declined**, with the number: 1.84 % against 89 % noise |

Two of the four were entries describing a tree that had moved. That is the round's own
lesson and it is the same one `doc/notes-fill-solid-lookup.md` §2 recorded about the
sentence it disproved: **a debt list is a claim, and it decays.** The two that were live
are §3 and §4 below.

**No pixel moves.** Nothing outside `crates/quorra-gpu/tests/`, one archetype doc comment
and one example doc comment is touched, and the prediction for the caller's corpus gate is
**no change on any page** for all four items — items 1 and 2 change nothing at all, item 3
is test-only, item 4 is a comment. The gate was not run, per the round's constraints.

---

## 1. `fill_solid`'s second lookup — closed on 2026-08-16, and the bullet is stale

`crates/quorra-gpu/src/encode/fill.rs` carries `SolidFill<'a> { stored: &'a StoredOutline,
… }` with the benchmark in its field comment, and `fill_solid` reads `fill.stored`.
`grep -n '\.outline(' crates/quorra-gpu/src/encode/fill.rs` returns **one** line — 96, in
`encode_fill`. There is no second probe to remove.

`doc/notes-fill-solid-lookup.md` is the round that took it, and it carries the callgrind
numbers `HANDOVER.md`'s bullet asks for: 9 340 817 Ir on the caller's 58 009-mark page
(2.26 % of that page's `recording`), 729 118 on dense text (**5.93 %**), 80 460 on artwork.
It also disproves the obstacle the bullet still quotes — "needs a lifetime that fights
`&mut self`" — in one edit and one `cargo build`, because the encoder holds
`resources: &'a ResourceStore` and a reborrow through it keeps no loan on `self`.

**Nothing was measured again here and nothing should be.** A before/after of a change that
is already in `main` has no "before" to run, and re-deriving numbers that a deleted
callgrind harness produced would be a worse measurement of the same thing.

**What `HANDOVER.md` should say.** The whole "Four things the `encode.rs` split found and
left" bullet is closed, not just the third clause: `push_op`'s two openings, `CULL_MARGIN`'s
dead link to `Encoder::push_glyph`, the `fill_solid` lookup and `command`'s
`#[allow(clippy::only_used_in_recursion)]` are all done — `grep -rn only_used_in_recursion
crates/` returns nothing, and `device_space.rs`'s `CULL_MARGIN` comment no longer names
`push_glyph`. `doc/notes-fill-solid-lookup.md` §5 records all four with their outcomes.

## 2. The second shader list — closed by ADR 0059, and re-verified able to fail today

`crates/quorra-gpu/tests/shader_copies.rs` **does not exist**. The gate is
`crates/quorra-gpu/src/shaders/copies.rs`, a `#[cfg(test)]` module reading `super::ALL`,
exactly as ADR 0059 decided; the recorded obstacle ("an integration test cannot reach a
private module, so closing it means deciding whether that list is public") was resolved by
moving the gate rather than by publishing the list, which is what that ADR argues at length.

**Re-verified rather than taken on the ADR's word**, because the failure this replaced was a
gate that passed for months. `("function_lane.wgsl", FUNCTION_LANE)` deleted from
`shaders::ALL` — the exact shader the old integration test was blind to — and
`cargo test -p quorra-gpu --lib shaders`:

| test that went red | what it said |
|---|---|
| `shaders::tests::every_wgsl_file_is_named_here` | `the shader directory and shaders::ALL disagree: ["function_lane.wgsl"] exist unnamed, [] are named and absent` |
| `shaders::copies::promised_helpers_are_textually_identical` | ``  `soft_mask_value` is defined in 5 shaders …, not the 6 that are supposed to carry it `` |
| `shaders::copies::every_sameness_promise_is_guarded` | `5 shaders make the sameness promise, 6 functions' worth of copies are guarded` |
| `shaders::shape_inputs::a_shape_pass_cannot_reach_the_soft_mask` | `4 shaders define `fs_shape`, not the 5 lanes that are supposed to` |

**Four** tests, where the arrangement ADR 0059 replaced reported `2 passed` under the
equivalent drift. Restored, `cargo test -p quorra-gpu --lib shaders` is `12 passed`.

**What `HANDOVER.md` should say.** Delete the bullet.

### One thing seen on the way, and it is not ours

The first attempt at that run failed to compile:

```
error[E0063]: missing field `alpha_is_shape` in initializer of `GroupSpec`
   --> crates/quorra-gpu/src/census.rs:197:17
```

`alpha_is_shape` appears **nowhere** in this worktree — `grep -rn alpha_is_shape crates/`
is empty, and `quorra-scene`'s `GroupSpec` has no such field at this revision. A sibling
agent's `quorra_scene` rlib was linked from the shared `/home/AI/cargo-target/quorra`. The
identical command succeeded on retry with no source change. This is the third and fourth
sighting of the shared-target-dir collision `doc/notes-fill-solid-lookup.md` §6 recorded
("extern location for quorra_gpu does not exist", "found possibly newer version of crate
quorra_gpu"), and it is the first that produced a *type error in our own source*, which is
the dangerous shape: the other two look like build noise and this one looks like a bug in
the tree. Recorded rather than dismissed. It does not argue for per-checkout target dirs —
the memory note on sccache measured those at 0 % cache hits — but it does argue that a
compile error naming a symbol `grep` cannot find should be retried before it is believed.

## 3. `m3.rs`'s bound — taken, and it is ADR 0077

ADR 0072 left `m3.rs`'s `const UNORM_TOLERANCE: i32 = 2` standing with the note that it
"now needs its own derivation rather than a citation of this one". Its comment had already
been corrected once, on 2026-08-17, to say honestly that `m1.rs`'s derivation *does not
reach this page* — which left a number with no derivation at all, and forwarded `m1.rs`'s
disproven "minimum alpha is 128" as a third copy of a claim ADR 0072 had killed. (The
second copy was `tests/common/probe.rs`'s doc for `max_byte_diff`. Both are gone.)

### What m3's reference actually produces

Checked before assuming, since m3's reference is a different function with clips in it.
Instrumented print over the fixture, llvmpipe, 2026-08-23:

| | |
|---|---|
| store histogram over 2 005 644 pixels | `{0: 1 071 244, 1: 934 400}` — **no pixel stored twice** |
| distinct alphas | `{0, 29, 57, 86, 115, 172, 230}` |
| per-pixel bound `⌈255/α⌉` where inked | `{2, 3, 5, 9}` |
| largest raw byte difference, whole raster | **0** |

The no-overlap result is a property of the fixture and not a coincidence: the 4 000 marks
are 10.25 × 24.5 on a 14.75 × 32.5 pitch. So the bound is `⌈255/α⌉` where there is ink and
**0** on the 53 % of the raster where there is none.

### The decision, and where the arithmetic went

`m1.rs`'s per-pixel derivation is reused, and the arithmetic — `Reference`, `bound_at`,
`disagreement` — moved verbatim to `crates/quorra-gpu/tests/common/bound.rs`. ADR 0077
argues the seam: `tests/common/mod.rs`'s standing rule is *"a measurement is shared; a claim
about a fixture is not"*, and `bound_at` reads both of its inputs off the pixel it is called
for, so it is the same function for every fixture. What was a claim about a fixture was
`m3.rs`'s **constant**, and the way to stop a file making an unsupportable claim is not to
give it a better one. `m1.rs` loses 109 lines; `max_byte_diff` loses its last caller and is
deleted.

This overturns a sentence ADR 0072 wrote into `bound_at`'s own doc comment — *"`m3.rs`
states its own bound and should keep doing so"* — which is why it is an ADR and not a
refactor.

### Verified able to fail, and the first attempt did not verify anything

Forced defect: `rect_link_box` in `crates/quorra-gpu/src/encode/clips.rs` outset so the clip
rectangle is slightly too large in every direction. **The size of the outset decides
whether the experiment says anything**, and that is the finding worth carrying:

| outset | `max_byte_diff` over the raster | old gate (≤ 2) | new gate |
|---:|---:|---|---|
| 0.01 px | **128** | red | red |
| **0.004 px** | **1** | **green** | **red** |

At 0.01 the leaked pixels carry α = 2, and the straight-alpha conversion divides the leaked
colour by that α and amplifies it to 128 — so *both* gates catch it and the run proves
nothing about either. Only a leak small enough for the alpha to round to 1 and the colour to
round to 0 separates them. At 0.004, **3 696 pixels are inked where nothing stored** and the
old constant passes the lot; the new gate reddens
`three_hundred_three_identical_clips_collapse_to_one_region` at the first of them:

```
at (60, 79) channel 3: got [0, 0, 0, 1], expected [0, 0, 0, 0]
    — 1 unorm steps past a bound of 0 (0 stores at α 0)
```

The 0.01 run also reddened `empty_clip_admits_nothing_and_differs_from_absent`, which is a
different gate seeing a different consequence of the same defect and is not evidence about
this one.

Defect reverted; `cargo test -p quorra-gpu --test m1 --test m3` is 13 + 6 green.

### Where the new bound is weaker, stated rather than discovered

At the α = 29 slivers it is **9** where the constant was 2. That is loose, and it is the
honest direction: 2 was never derivable there, and ADR 0077's alternatives section refuses
both "raise the constant to 9 everywhere" (nine steps of slack at every pixel that needs
none) and "tighten it to 0 since the page agrees exactly" (curve-fitting to one adapter's
runs). The gate is tighter on 53 % of the raster, equal at α = 230, looser on the slivers.

### Left undone, deliberately

**`m3.rs` still pins llvmpipe** through `common::headless::device` where `m1.rs` asks every
Vulkan adapter. The per-pixel bound is exactly what would make an every-adapter comparison
of this page meaningful, so this is now a smaller decision than it was — but it is a
1191 × 1684 page of 4 000 clipped marks per adapter, and it is a different question with its
own cost. ADR 0077 records it under "what this does not decide".

## 4. The thread sweep's dense-text row — declined, and the question is closed

The recorded question: should `examples/encode_threads.rs` sweep `DENSE_TEXT` rather than
`DENSE_TEXT_UNCLIPPED`, putting the archetype's 40 residue-clipped marks into the "does not
divide" column where `ARTWORK` already is?

### First, the premise — and it holds, by construction rather than by clock

`Encoder::deferrable_bounds` (`encode/parallel/commit.rs`) is
`(resolved.residues.is_none() && self.coverage == Coverage::Cpu).then(…)`, and the glyph
lane's guard in `encode/fill.rs` is `if let (None, Some(_)) = (resolved.residues.as_ref(),
cache.admission())`. A residue-clipped mark satisfies neither, falls through to
`raster::flatten` + `push_coverage_styled`, and is rasterised **on the walk**. So the 40
marks genuinely do not divide, and that can be read rather than timed.

(Checked and found *not* to be a problem on the way: the serial path's work is inside
`encode: geometry` after all — `coverage.rs` opens a clock span around `raster::fill_mask`
and, since 2026-08-17, a second one around `residue_product`. The metric is not blind to
the work the switch would add.)

### Then, the size of it — three counters, no clock

| | dense text archetype | artwork |
|---|---:|---:|
| serial: residue coverage texels | **8 956** | **3 542 360** |
| parallel: atlas working set, bytes | 476 892 | — |
| serial share of the page's coverage work | **1.84 %** | — |

The atlas figure is measured (llvmpipe, `Counters::atlas_working_set_bytes`) and is
**476 892 on both pages** — the two curve clips change no glyph tile, which is why the
serial residue is the whole of the difference between them. Carried through Amdahl at the
~2.8× this page actually reaches at 24 threads, 1.84 % serial moves the scaling ratio from
about 2.80× to 2.71×: a **3 %** effect.

### And the instrument's resolution, which is what decides it

Two sweeps of the same two pages on the same day, minima of round-robin rounds, load average
printed either side as the example does:

| run | rounds | load before → after | unclipped, 1 thread | archetype, 1 thread |
|---|---:|---|---:|---:|
| A | 9 | 19.0 → 17.0 | 7.687 ms | **6.170 ms** |
| B | 15 | 101.3 → 64.7 | 8.620 ms | **11.680 ms** |

An **89 % spread** on one configuration — and in run A the archetype came out *faster* than
the unclipped page, which the dispatch above says is impossible, since it does strictly more
work on identical geometry. The clock could not order the two pages correctly, let alone
resolve 3 % between them.

### The decision

**Declined.** The sweep keeps `DENSE_TEXT_UNCLIPPED`. A shape earns its place in a sweep by
being something the sweep can resolve, and the effect this one exists to expose is thirty
times smaller than the noise the instrument had. Adding it — or switching to it — would also
cost ADR 0054's published series its comparability, for a row nobody could read.

Adding the archetype **as a fifth shape** was considered and refused on the same ground: it
is ADR 0059's test of machinery — *what does it catch that the simpler thing cannot?* — and
the answer is a 3 % perturbation under 89 % noise.

**What the question was really about is closed, and by naming.** The hazard behind it was
that "dense text" in this example and "dense text" in `tests/archetypes.rs` were two pages
under one name. The register (ADR 0060) fixed that on 2026-08-17: the row this example
prints reads `dense text, unclipped`.

The decision and its numbers are in `SHAPES`' doc comment in the example, and
`quorra_pages::DENSE_TEXT_UNCLIPPED`'s doc comment — which carried the open question — now
carries the answer.

**What `HANDOVER.md` should say.** Replace the bullet with the decline and the two numbers
(1.84 % serial share; 89 % single-configuration spread on 2026-08-23).

---

## 5. Verification

- `cargo fmt --all --check`: clean.
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`: clean, with
  `Checking quorra-gpu` printed rather than only `Finished`.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: clean.
- `cargo test --workspace --no-fail-fast`: **586 passed, 0 failed, 3 ignored** over 80
  suites. Checked against the source rather than believed:

  | | |
  |---|---:|
  | `grep -rn '#\[test\]' crates --include=*.rs` | 585 |
  | of those, `#[ignore]`d (`archetypes.rs`, `lane_crossover.rs` — both wall clocks) | 2 |
  | doctests: three ```` ```no_run ```` (pass) plus one ```` ```ignore ```` in `quorra-pages/src/lib.rs` | 4 |

  583 + 3 = **586 passed**; 2 + 1 = **3 ignored**. Exact, which is the arithmetic
  `doc/HANDOVER.md`'s trap asks for and the reason a green run is not evidence on its own.

  **Run in an isolated `CARGO_TARGET_DIR`**, because the shared one was serving a foreign
  `quorra_scene` (see §2's aside): the same command failed to compile twice in
  `/home/AI/cargo-target/quorra` and passed cleanly in a fresh directory with no source
  change. The isolated dir was removed afterwards; the standing arrangement in the
  sccache memory note is unchanged and this is not an argument against it.
- **Test names verified able to fail**: §2's four (shader list) and §3's one (m3's bound),
  each forced, each named above with the message it produced, each reverted.
- **The caller's corpus gate was not run**, per the round's constraints. Predicted movement:
  **none, on every page, for all four items.** Items 1 and 2 changed no file; item 3 is
  confined to `tests/`; item 4 is two doc comments. Nothing in `crates/*/src/**` changed
  behaviour this round — the only non-test source edit is
  `quorra-pages/src/page.rs`'s doc comment on `DENSE_TEXT_UNCLIPPED`.
