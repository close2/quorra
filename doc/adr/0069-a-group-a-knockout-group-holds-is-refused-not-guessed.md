# ADR 0069 — A group a knockout group holds is refused, not guessed

Status: accepted, 2026-08-18. Closes the defect ADR 0066 recorded and did not fix, and
`doc/notes-mask-shape-or-opacity.md` §8 carried the reproduction for. The evidence is
`doc/notes-nested-knockout.md`. **It moves no pixel and adds one public error variant.**

## Context

ISO 32000-2 §11.4.6 names one element kind and puts a normative obligation on it:

> The existence of the knockout feature is the main reason for maintaining a separate
> shape value rather than only a single alpha that combines shape and opacity. The
> separate shape value shall be computed in any group that is subsequently used as an
> element of a knockout group.

§11.3.7.2 says what that value is, under **Object shape**:

> The shape of a group object shall be the union (as defined in 11.3.7.3, "Result shape
> and opacity") of the shapes of the objects it contains.

**A finished group cannot supply it.** It reaches the compositor as one premultiplied
RGBA texture whose alpha is §11.3.7.3's *result alpha* — the union of each element's shape
**times its opacity** — and the union of shapes alone is not a function of that. So the
two quantities §11.4.6 weights apart arrive as one number, which is precisely what the
sentence above exists to forbid.

`encode_group` pushed `Op::Child` without consulting `self.style`, so such a group was
composited by §11.3.6 instead. Confirmed independently at 16 × 16, an opaque cover then a
half-opaque isolated group, both inside a knockout group:

```
knockout group  : [128, 76, 128, 255]
ordinary group  : [128, 76, 128, 255]   <- byte-identical
clause requires : [26, 102, 229, 128]
```

The clause value is NOTE 5's first extreme — "a shape value of 1.0 (inside) yields the
colour and opacity that result from compositing the object with the initial backdrop" —
and the initial backdrop of an isolated group is transparent (§11.4.5). Nothing refused
it and nothing reported it: principle 6's third state, and a plausible-looking wrong page
rather than a hole.

**Five constructions were wrong, one was already refused, and three were already right.**
`doc/notes-nested-knockout.md` §3 has the table. The one that was right by algebra rather
than by luck is the nested group whose opacity is 1 everywhere: §11.4.6 weights the
backdrop by `(1 − f)` and §11.3.6 by `(1 − f·q)`, so the two readings are equal exactly
where `q = 1`. Every wrong row puts opacity below 1 by one of §11.3.7.2's three routes.

## Decision

**`SceneError::KnockoutElementGroupUnsupported`.** A group that is an element of a
knockout group and names `Compose::SrcOver` is refused at the builder, by name, before
anything is encoded.

**And the construction is not lost, because it was never the one that arrives.**
ADR 0033's `Compose::DestOut` then `Compose::Plus` on two groups *is* §11.4.6's own two
stages, and it is not refused here at any depth. Measured on a 64 × 64 wedge, worst
premultiplied deviation from `P' = (1 − f) × P + S`:

| | deviation |
|---|---|
| the two groups, `DestOut` then `Plus` | **1.27 of 255** (unorm rounding) |
| §11.3.6's composite of the same content | **114.95 of 255** |

### Why refused rather than drawn, with the price of the alternative

Two correct routes exist and both are real; a third does not and the reason is a clause.

- **A shape channel per layer.** Well-defined in every lane, and ADR 0066 is why: all five
  `fs_shape` entry points already compute exactly `f`. **Cost:** 13 of `Kind`'s 18 variants
  — the twelve lane pipelines that draw into a layer, plus `Composite` — gain a second
  colour target and blend state, as do the function lane's per-program pipelines; every
  drawing shader gains a `@location(1)`; and every layer gains an R8 attachment, +25 % of
  ADR 0063's measured 1 325.5 MB of layer allocation across the corpus at 4× and +23 MB on
  the heaviest single frame.
- **A second pass instead of a second attachment**, the cheapest correct route: a
  `Style::Shape` variant per lane with the union blend, rendered into a shape layer that
  the existing `compose: 1` erase reads. **No shader change at all.** Cost: ~5 pipeline
  kinds, a `shape_pass` flag through every encode arm, and a doubled encode, layer and
  composite per affected group. About ADR 0033's own size.
- **Re-encoding the subtree with opacity forced to 1 is not available.** For a non-opaque
  image the alpha is §11.6.5.2's `/SMask` (opacity) or §8.9.6.3's explicit mask (shape) and
  one RGBA buffer cannot say which — ADR 0066 §8a records this for us and the caller's
  `stated_shape` answers `None` for the same reason. For a non-opaque shading the caller
  has folded §11.6.4.4's constant into the ramp's colours. Forcing alpha to 1 would be
  right for solids and silently wrong for two lanes: the forbidden third state one level
  down.

**The population decided it.** A walk over the caller's 974 page-one display lists, by
ADR 0067's instrument:

> **Zero** ordinary `Compose::SrcOver` group elements of a knockout group. **Ten** nested
> groups on four pages, and **all ten are halves of a `Command::Shaped`** — the staged
> pair. Six of the ten, on three pages, reach quorra's encoder and are drawn.

Zero by the caller's design rather than by accident: `pdf-model`'s `knockout_elements`
wraps every element of a knockout group in a `Command::Shaped` unless
`element_shape_is_coverage` says otherwise, and that predicate excludes a group with the
comment "A group's result reaches the backends as a raster, so its shape would be its alpha
by construction — the same conflation one level down." **They read the clause the way we
just did, one round earlier.** So building either correct route would be a second way to
say what the caller already says, moving zero pixels; ADR 0067 declined a narrowing on the
same evidence one round ago.

The refusal is one-way and cheap to reverse: the day a document turns up whose knockout
group holds a group nobody can state a shape for, the second route above is a build and not
a re-derivation, and the note specifies it.

## What it costs

**One public API item, additive: `SceneError::KnockoutElementGroupUnsupported`.** It
belongs in the bump's list. The caller already maps every `SceneError` to
`QuorraRasterError::Scene`, so it arrives on a path they have tested.

**A predicate narrower than the one already there, and the corpus can only charge for one
of the three that were available.** `OpenFrame` carried a single *transitive* "inside a
knockout group" boolean, which `check_isolation` reads on purpose; this refusal must not,
because §11.4.6 governs a knockout group's **elements** and a group two levels down is an
element of its own parent. `OpenFrame` now carries `Knockout { element, inside }`, and the
two questions have two names.

| predicate | refuses beyond the clause | corpus cost |
|---|---|---|
| **`element_of_knockout() && compose == SrcOver`** — built | nothing | **0 pages** |
| `inside_knockout() && compose == SrcOver` | a `SrcOver` group strictly inside a `DestOut`/`Plus` half | **0 pages** |
| `inside_knockout()` alone, no compose exemption | the halves themselves | **3 pages** |

The third row is the measured price of exempting §11.4.6's own two stages: the ten halves
sit on four pages, `issue18032.pdf` is refused whole for an unrelated reason, and the other
three are rendered and agree today. The second row costs nothing, and that is a measurement
rather than an assumption — the walk was re-run and found **0 groups inside a `Shaped`
half's body at any depth**, over **12 commands walked inside those bodies**. An earlier
draft of `doc/notes-nested-knockout.md` claimed the transitive predicate would have cost
those same three pages; it would not, and the note now says so with the count that settles
it.

**So the narrow predicate is chosen on the clause, not on the corpus.** `pdf-model`'s
`stated_shape` maps a group's shape to *the group of its elements' shapes*, so a knockout
group holding a group that itself holds a group yields a half with a group inside it — a
document the corpus does not currently contain. That is the case a corpus cannot rule on
and a fixture can, which is why
`the_clauses_own_two_stages_are_not_refused_at_any_depth` builds three levels.

Nothing escapes by nesting, because there is nothing to escape: §11.4.6 reaches exactly one
level, and a group deeper than that is not the construction the clause is about — its own
parent composites it by §11.3.6, correctly.

**The fuzzer learned the question, and turned out to be the gate the corpus could not be.**
`nest_chain` asserted that its chain is refused at the depth bound "and *only* at the
bound", which is now two conditions; `random_ops` carries `element_of_knockout` so each
refusal is pinned to its own condition rather than either being admitted anywhere. Verified
reached rather than assumed: inverting that arm fails the fuzz gate at depth 1. And forcing
the *transitive* predicate fails it at **depth 2** — the construction 974 real documents
never produce. The scene boundary's fuzzer is covering the population the corpus is silent
about, which is the argument for having one.

## The gates, and that each was verified able to fail

`crates/quorra-gpu/tests/nested_knockout.rs`, three tests.

- **`a_group_that_is_an_element_of_a_knockout_group_is_refused`** — the five wrong
  constructions, each refused by name **and each accepted one flag away** in an ordinary
  group, so a case refused for some other reason cannot pass by accident. That control
  caught a real defect as the file was written: the soft-mask case allocated its `MaskId` in
  a different builder and was passing on `UnknownMask`.
- **`the_clauses_own_two_stages_are_not_refused_at_any_depth`** — `DestOut`/`Plus` inside a
  knockout group with each half a group holding a group holding a group.
- **`the_refused_composite_misses_the_clause_that_the_stages_hit`** — §11.4.6's line
  measured **both ways**, 1.27 against 114.95. The rejected reading is measured through an
  *ordinary* group, and that is not an analogy: `encode_group` does not read `self.style`
  and `Compose::SrcOver` is `compose: 0` in either position, so it is the same encode — the
  byte identity above is the proof.

**Forced, each in the direction it claims and with the right split:** the check deleted
fails the first alone; the predicate made transitive fails the **second** alone while the
first goes on passing; `composite.wgsl`'s `compose == 1u` branch deleted fails the third at
140.05 against its bound of 3.0.

## The corpus

**Nothing moved**, over one copy of the caller's tree at `736e01f3`, RADV, both lanes, both
scales, taken in one sitting: `931 / 23 / 2 / 18`, `929 / 25 / 2 / 18`, `937 / 11 / 3 / 23`
and `938 / 10 / 3 / 23`, identical in both columns. All 79 printed page lines across the
four rows — 37 distinct documents — match in every field, and the two columns are two
distinct binaries (`corpus-3ef401d9b2d4df05` against `corpus-288ad9df85abf799`), which is
the check a predicted-null range actually needs.

Two things make that null a result rather than an absence of one. **The refusal never fires
anywhere in the corpus** — no run contains its message, which is §4's population of zero
measured through the renderer instead of through the walk. And **the three pages that must
not move were rendered and compared, not skipped**: run alone, both columns print
`3 agree, 0 differ, 1 refused, 0 not comparable`, with `issue18032.pdf`'s pre-existing
non-isolated-knockout refusal unchanged. `0 not comparable` is the load-bearing figure
there.

> **Confirmed 2026-08-18** against a later round's contrary aside, which is retracted in ADR
> 0070. Re-measured at `3f6df72` (the mainline commit before this one's merge) and at `main`
> `c443bc2`, in one copy of the caller's tree at `829d7faa`: the CPU scale-4 lane reads
> `938 / 11 / 3 / 22` in both columns with all 14 page lines identical, and
> `issue18032.pdf`'s refusal line is byte-identical — it is `render-quorra`'s own §11.4.6
> check, raised before a scene is built, and `KnockoutElementGroupUnsupported`'s message
> appears in neither column. `doc/notes-release-matrix.md`, "A refusal that did not move".

`doc/notes-nested-knockout.md` §9 has the matrix, the method and the one ratchet — the
caller's `REFUSED_AT_FOUR`, failing in both columns with character-identical lists because
both are patched past ADR 0057 while their lock is not. Their re-baseline, three matrices
running.

## Revisit when

A document appears whose knockout group holds a group the caller cannot state a shape for —
that is, one where `stated_shape` returns `None` on a group's contents, so
`knockout_elements` gives up and the group loses its knockout rule with a report. Today
that costs the page a report rather than a refusal, on their side; if it becomes common,
the second route in the Decision is what buys the page back, and it is priced above rather
than left to be re-derived.
