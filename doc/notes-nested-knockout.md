# A group used as an element of a knockout group

The defect ADR 0066 found while enumerating something else, measured, costed and closed.
`doc/notes-mask-shape-or-opacity.md` §8 is where it was written down; this is the round it
asked for.

Everything below is measured in this worktree on llvmpipe unless a line says otherwise.
Base: `fa2747c`, confirmed against `main` before a line was written — the worktree opened
**twelve commits stale** and was reset, which is `doc/HANDOVER.md`'s "a stale worktree
argues that your brief is wrong" arriving for the ninth time.

## 1. The clause, and the sentence that decides it

ISO 32000-2 §11.4.6's last paragraph but two names this element kind and puts a
**normative obligation** on it. Verbatim, both sentences, because the second is the whole
round:

> The existence of the knockout feature is the main reason for maintaining a separate
> shape value rather than only a single alpha that combines shape and opacity. The
> separate shape value shall be computed in any group that is subsequently used as an
> element of a knockout group.

What that separate value *is* for a group, §11.3.7.2, "Source shape and opacity", under
**Object shape**:

> The shape of a group object shall be the union (as defined in 11.3.7.3, "Result shape
> and opacity") of the shapes of the objects it contains.

And what §11.4.6 does with it — the two stages, verbatim:

> - a) Composite the source object with the group's initial backdrop, disregarding the
>   object's shape and using a source shape value of 1.0 everywhere. This produces
>   unnormalised temporary alpha and colour results, α t and C t .
> - b) Compute a weighted average of this result with the object's immediate backdrop,
>   using the source shape as the weighting factor. Then normalise the result colour by
>   the result alpha:

with NOTE 5 giving the extreme the fixture below sits on:

> NOTE 5 The extreme values of the source shape produce the straightforward knockout
> effect. That is, a shape value of 1.0 (inside) yields the colour and opacity that result
> from compositing the object with the initial backdrop. A shape value of 0.0 (outside)
> leaves the previous group results unchanged.

`tests/common/clause.rs` writes the pair as one premultiplied line, `P' = (1 − f) × P + S`,
from §11.4.6's own recurrence `𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t` against the transparent
initial backdrop §11.4.5 gives an isolated group.

**A finished group cannot supply `f`.** It reaches the compositor as one premultiplied
RGBA texture, and that texture's alpha is §11.3.7.3's *result alpha* — the union of each
element's shape **times its opacity**. The union of shapes alone is not a function of it.
So the two quantities §11.4.6 weights apart arrive as one number, which is what the
sentence "the separate shape value shall be computed" exists to forbid.

The caller reached the same reading independently, and their comment says so in the same
words — `pdf-model/src/content/transparency.rs`, on why a nested group disqualifies a
knockout group from the bare route:

> **No nested group.** A group's result reaches the backends as a raster, so its shape
> would be its alpha by construction — the same conflation one level down.

## 2. The defect, confirmed independently

Rebuilt from §8's reproduction rather than trusted: 16 × 16, an opaque cover
`(0.9, 0.2, 0.1, 1)` filling the target, then a nested group over it, both inside an
isolated knockout group. The nested group holds one opaque `(0.1, 0.4, 0.9)` rectangle,
also full-target, so its shape is 1 everywhere and NOTE 5's first extreme applies: the
required result is the group composited with the transparent initial backdrop, which for
a group at constant alpha ½ is `[26, 102, 229, 128]`.

```
knockout group  : [128, 76, 128, 255]
ordinary group  : [128, 76, 128, 255]   <- byte-identical
clause requires : [26, 102, 229, 128]
```

Confirmed. The alpha is 255 where the clause requires 128, and the frame is byte-identical
to the same content inside an ordinary group — which is the mechanism as well as the
symptom: `encode_group` pushes `Op::Child` without reading `self.style`, and
`Compose::SrcOver` maps to `compose: 0` in either position, so the two are *the same
encode*.

Nothing refused it and nothing reported it. That is principle 6's third state.

## 3. The reach, measured

Same fixture, one field varied at a time. "Clause" is `[26, 102, 229, 128]` in every row
where the nested group's shape is 1 — which is every row here, since the body is a
full-target opaque rectangle.

| # | the nested group | drawn | verdict |
|---|---|---|---|
| 1 | plain isolated, constant alpha ½ | `[128, 76, 128, 255]` | **wrong** — and byte-identical to the same group in an ordinary group |
| 2 | plain isolated, **opacity 1 throughout** | `[26, 102, 230, 255]` | **correct**, and correct by algebra rather than by luck — see below |
| 3 | constant alpha 1, its **body** at alpha ½ | `[128, 76, 128, 255]` | **wrong** |
| 4 | **knockout** group inside a knockout group, alpha ½ | `[128, 76, 128, 255]` | **wrong** |
| 5 | blend `Multiply`, alpha ½ | `[127, 36, 25, 255]` | **wrong**, and wrong differently: §11.3.5 is applied against the accumulated content where §11.4.6 leaves no blend at all |
| 6 | soft mask of ½, constant alpha 1 | `[128, 77, 128, 255]` | **wrong** |
| 7 | **non-isolated**, alpha ½ | — | **already refused**: `NonIsolatedGroupUnsupported { reason: InsideKnockoutGroup }` |
| 8 | `Compose::DestOut` then `Compose::Plus` (ADR 0033) | `[26, 102, 229, 128]` | **correct**, exactly |
| 8b | the same pair, each half a group **containing a group** | `[26, 102, 229, 128]` | **correct**, exactly |

Row 2 is the row that explains the corpus. Premultiplied, §11.4.6 weights the backdrop by
`(1 − f)` and §11.3.6 weights it by `(1 − f·q)`, and the deposit `S` is the same term in
both. **The two readings are equal exactly where `q = 1`** — where the nested group's
opacity is 1 everywhere — or where the backdrop under it is empty. Every wrong row above
is a row that puts opacity below 1, by one of the three routes §11.3.7.2 lists: the
group's constant opacity (row 1), an element's own (row 3), the mask's (row 6).

Row 5 needs one sentence of derivation, because "the clause requires `[26, 102, 229, 128]`"
is less obvious with a blend mode in play: §11.4.6 stage (a) composites the element with
the group's *initial* backdrop, which §11.4.5 makes transparent, and §11.3.6 against
`ab = 0` leaves `co = as·Cs` — both terms carrying `B(Cb, Cs)` vanish. Every blend mode
degenerates to Normal there, which `tests/knockout_blend.rs` holds directly.

Row 8b is the row the refusal's *predicate* turns on — it is drawn correctly today and must
go on being accepted — and §7 is where that is paid off.

## 4. The population

Reusing ADR 0067's instrument, per `doc/HANDOVER.md`'s "Counting a feature's population
without the corpus test": one walk over the caller's 974 page-one display lists, rendering
nothing. Copy at `/home/AI/nested-knockout/viewer`, caller revision `736e01f3`, submodule
`doc/pdf.js` at `2ea8820d9`; the walk is
`crates/render-quorra/tests/nested_knockout_reach.rs` **in the copy**, which is deleted
with it and is recorded here because the number is load-bearing.

**The previous walk's triple was reproduced first, and then it moved — and the movement is
the finding.** `doc/notes-release-matrix.md` §3 recorded "16 pages emit a knockout group
(29 in total), 142 groups overall". Descending into a `Command::Shaped`'s two halves reads
**16 / 30 / 152** instead; not descending reads **16 / 29 / 142** exactly, with the sixteen
document names matching character for character. So the enumeration is the same one, and
the difference — 10 groups and 1 knockout group — is *entirely* the nested groups this
round is about. The definition the earlier walk used is precisely the one that cannot see
this population.

| | count |
|---|---|
| documents walked | 974 (958 producing a page-one display list) |
| pages nesting a group inside a knockout group | **4** |
| such nested groups | **10** |
| of them, `isolated: true, knockout: false` | 9 |
| of them, `knockout: true` (isolated) | 1 |
| of them, `isolated: false` (§11.4.4) | **0** |
| carrying a soft mask | 0 |
| carrying a constant alpha ≠ 1 | 2 (0.0 and 0.5) |
| carrying a blend mode ≠ Normal | 1 (`Color`) |
| carrying a blending colour space | 0 |
| **of them, halves of a `Command::Shaped`** | **10 of 10** (5 object, 5 shape) |
| **of them, ordinary `Compose::SrcOver` elements** | **0 of 10** |

The four pages are `issue18032.pdf` (4), `knockout_inner_backdrop.pdf` (2),
`knockout_nested.pdf` (2) and `knockout_nested_group_alpha.pdf` (2). All ten are at
knockout depth 1; nothing in the corpus nests two knockout groups deep.

**Six of the ten reach quorra's encoder and are drawn**, on three pages —
`issue18032.pdf`'s four are behind that page's own refusal, an enclosing non-isolated
knockout group `render-quorra` will not translate. Checked by running the corpus gate on
those pages on RADV: `3 agree, 0 differ, 1 refused, 0 not comparable`.

**So the population of the defect is zero, and the population of the construction that
must keep working is six.** That is not a coincidence and it is not luck: `pdf-model`'s
`knockout_elements` wraps every element of a knockout group in a `Command::Shaped` unless
`element_shape_is_coverage` says its shape *is* its coverage, and that predicate's `_ =>
false` arm covers `Command::Group` — with the comment quoted in §1 saying exactly why. A
bare group inside a knockout group is a construction this translator cannot emit, by its
own design, for our reason.

## 5. The two answers, costed

### Draw it correctly

Three routes, and the first two are the ones ADR 0025 and ADR 0033 each priced and each
declined, now arriving a third time at group scale.

- **A shape channel per layer.** Every layer gains a second attachment carrying
  §11.3.7.2's union of shapes; the composite reads the child's shape where the parent
  knocks out. Well-defined in every lane, and ADR 0066 is why: all five `fs_shape` entry
  points already compute exactly `f` — §11.6.4.2's geometry met with §8.5.4's clip — so
  nothing has to be invented, only routed. **Cost:** 13 of `Kind`'s 18 variants — the
  twelve lane pipelines that draw into a layer (`Rect`/`Cover`/`Image`/`Shaded` × the three
  styles), plus `Composite` — gain a second colour target and a second blend state, and so
  do the function lane's per-program pipelines, which are built outside that enum;
  `Winding`, `WindingResolve`, `Reduce`, `Blit` and `Present` do not, because none writes
  into a layer. Every drawing shader gains a `@location(1)`; the shape channel takes the
  union blend, plus the erase/add pair for a knockout group's *own* shape accumulation; and
  every layer gains an R8 attachment, +25 % of a layer's bytes — against ADR 0063's measured
  1 325.5 MB of layer allocation across the corpus at 4× and a heaviest single frame of
  93.0 MB, that is +331 MB and +23 MB. For a quantity one clause reads.

- **A second pass instead of a second attachment** (the cheapest correct route). A
  `Style::Shape` variant of each lane — entry `fs_shape`, blend `(One, OneMinusSrc)`, which
  is §11.3.7.3's union — renders the same subtree into a shape layer; `encode_group` then
  emits the existing `compose: 1` erase against that layer and the existing `compose: 2`
  deposit against the ordinary one. **No shader changes at all**, because the stages and
  the entry points both exist. **Cost:** ~5 new pipeline kinds, a `shape_pass` flag
  threaded through every encode arm, and a second encode plus a second layer plus a second
  composite for each affected group. Roughly ADR 0033's own size.

- **Re-encode the subtree with opacity forced to 1**, and composite the result with the
  existing stages. **This one is not available, and the reason is a clause rather than a
  cost.** For a non-opaque image the alpha is §11.6.5.2's `/SMask`, which is opacity, or
  §8.9.6.3's explicit mask, which is shape, and one RGBA buffer cannot say which
  (ADR 0066 §8a records this for us; the caller's `stated_shape` returns `None` for the
  same reason, in the same words). For a non-opaque shading the caller has already folded
  §11.6.4.4's constant into the ramp's colours, so a translucent colour and an unpainted
  region are one number by the time a `Paint::Shading` holds them. Forcing alpha to 1
  would be right for solids and silently wrong for two lanes — the forbidden third state,
  one level down.

### Refuse it by name

One predicate in `SceneBuilder::group`, one `SceneError` variant, and a sentence naming
the construction that replaces it.

### Which, and why the evidence chose it

**Refuse.** Three things decide it, in this order.

1. **The population of the defect is zero over 974 documents**, and zero by the caller's
   design rather than by accident (§4). ADR 0067 set the precedent one round ago and it is
   the same rule pointed at a correctness gap rather than a narrowing: neither route above
   is built, because nothing asks for it.
2. **The construction is already expressible, and it is already what arrives.** All ten
   nested groups in the corpus come through `Compose::DestOut`/`Plus`, which is ADR 0033's
   vocabulary, which draws §11.4.6's line to `1.27 of 255` (§6). Building route A or A′
   would be a second way to say what the caller already says — and would move zero pixels,
   because the caller would go on saying it the first way.
3. **The refusal is one-way and cheap to reverse.** It is a `SceneError` variant and a
   two-line predicate; the day a document turns up whose knockout group holds a group the
   caller cannot state a shape for, route A′ is a build and not a re-derivation, and this
   note specifies it.

What the other answer would have cost is written above rather than waved at: route A′ is
about the size of ADR 0033, route A about twice that plus 331 MB across the corpus, and
route C is not correct at all.

## 6. What changed

**`quorra-scene`**, four files:

- `error.rs` — `SceneError::KnockoutElementGroupUnsupported`, carrying no payload because
  the position is the whole story, with §11.4.6's and §11.3.7.2's sentences quoted on it
  and the measured bytes beside them. **This is public API and it is additive**; it belongs
  in the bump's list.
- `scene/frames.rs` — `OpenFrame` carries a `Knockout { element, inside }` pair instead of
  one boolean. §7 is why.
- `scene/validate.rs` — `check_knockout_element_group`: refuse when the commands landing
  here are elements of a knockout group *and* the group names `Compose::SrcOver`.
- `scene/builder.rs` — the call, in `group`, after `check_isolation` so that a
  non-isolated group keeps the refusal it already had.

**`quorra-gpu`**: no code. `encode/layer.rs`'s `encode_group` gains the paragraph saying
why `self.style` is deliberately not consulted there, so the next reader of that function
meets the argument at the site of the defect rather than in an ADR.

## 7. One boolean was two questions

`OpenFrame::inside_knockout` was transitive — `self.inside_knockout() || spec.knockout` —
and `check_isolation` reads it that way on purpose. **This refusal must not.**

§11.4.6 governs the *elements* of a knockout group. A group two levels down is an element
of its own parent, and if that parent is an ordinary group it is composited by §11.3.6
whatever encloses it. So:

**Three predicates were available, and they have three different prices.** They are set out
together because two of them cost nothing in the corpus and one costs three drawn pages,
and only naming all three says which fact is doing the work.

| predicate | what it refuses beyond the clause | corpus cost |
|---|---|---|
| **`element_of_knockout() && compose == SrcOver`** — what was built | nothing | **0 pages** |
| `inside_knockout() && compose == SrcOver` — transitive | a `SrcOver` group strictly inside a `DestOut`/`Plus` half's body, at any depth | **0 pages** |
| `inside_knockout()` alone, without the compose exemption | the halves themselves | **3 pages** |

The third row is the price of the `DestOut`/`Plus` exemption, and it is the only one the
corpus can charge: the ten halves sit on four pages, `issue18032.pdf` is refused whole for
an unrelated reason, and the other three — `knockout_inner_backdrop.pdf`,
`knockout_nested.pdf`, `knockout_nested_group_alpha.pdf` — are rendered and agree today
(`3 agree, 0 differ, 1 refused, 0 not comparable` on RADV). A predicate that swallowed the
exemption would turn three drawn pages into refusals, and that would be a regression rather
than a refusal.

**The second row's cost is zero, and an earlier draft of this note claimed it was those same
three pages. That was wrong and it was measured wrong.** The walk was re-run to settle it:

```
is itself a Command::Shaped half:            10
is INSIDE a Shaped half that is a group:      0    <- at every depth
depths below the half:                       {}
group levels below the knockout group:       {1: 10}
```

with **12 commands walked inside those ten half bodies and not one of them a group** — the
denominator is what makes that a measurement rather than an unexecuted branch. The
corpus's `Shaped` half groups hold about one mark each, so the exposure is thin, and thin
is the point.

So the honest sentence for the narrow predicate is **"the corpus does not exercise this;
the gate does"** — and it is the honest sentence for the refusal itself too, since the
construction it refuses also has population zero (§4). The reason to write the narrow
predicate anyway is the clause rather than the corpus: `pdf-model`'s `stated_shape` maps a
group's shape to *the group of its elements' shapes*, so a knockout group holding a group
that itself holds a group produces a half with a group inside it, and today's corpus simply
has no such document. `the_clauses_own_two_stages_are_not_refused_at_any_depth` builds three
levels for exactly that reason: the corpus cannot rule on this and a fixture can.

**Nothing escapes by nesting, and the reason is that there is nothing to escape.** §11.4.6
reaches exactly one level: a group deeper than one level below a knockout group is not the
construction the clause is about, because its own parent composites it by §11.3.6 and that
is correct. So the narrow predicate is not a weaker version of the transitive one — it is
the one that matches the clause.

The one shape that could look like an escape is a `DestOut` or `Plus` half holding a
`SrcOver` group. That group is an element of an *ordinary* group — the half — and is drawn
by §11.3.6 correctly; the half itself is the element of the knockout group, and the caller
has stated its shape by drawing it opaque. Row 8b of §3 measures that whole construction at
the clause's value, exactly.

## 8. The gates, and that each was verified able to fail

`crates/quorra-gpu/tests/nested_knockout.rs`, three tests, filed by the clause they state
(ADR 0062).

| test | what it holds |
|---|---|
| `a_group_that_is_an_element_of_a_knockout_group_is_refused` | the five wrong constructions of §3, each refused by name — **and each accepted one flag away**, in an ordinary group, so a case refused for some other reason cannot pass by accident |
| `the_clauses_own_two_stages_are_not_refused_at_any_depth` | `DestOut`/`Plus` inside a knockout group, with each half a group holding a group holding a group |
| `the_refused_composite_misses_the_clause_that_the_stages_hit` | §11.4.6's line measured **both ways** on a 64 × 64 wedge: the stages at **1.27 of 255**, §11.3.6's composite of the same content at **114.95 of 255** |

The third test needs its stand-in defended, and the file's module comment does it: the
refused construction and the same content inside an *ordinary* group are the same encode —
`encode_group` does not read `self.style`, and `Compose::SrcOver` is `compose: 0` in either
position — which §2 measured as byte identity before the refusal existed. So the 114.95 is
not an analogy for what the caller was being handed; it is the number itself.

The second test's control is the round's answer to "a gate whose assertion is an absence
needs a control", and the first test's per-case positive is the answer to it a second time:
`built(false, …)` must return `None` for every case, or the refusal is being read off the
wrong variant. That control caught a real defect while this file was being written — the
soft-mask case allocated its `MaskId` in a different builder and was failing with
`UnknownMask`, passing the "is refused" half for the wrong reason.

**Verified able to fail**, each in the direction it claims and with the right split:

1. The `check_knockout_element_group` call deleted → test 1 fails, tests 2 and 3 pass.
2. The predicate made transitive (`inside_knockout` for `element_of_knockout`) → **test 2
   fails alone**, and test 1 goes on passing — which is the split §7 is about.
3. `composite.wgsl`'s `compose == 1u` branch deleted → test 3 fails at **140.05 of 255**
   against its bound of 3.0, and the other two pass.

**And a fourth gate the round did not set out to build.** `tests/fuzz_scene.rs` catches the
transitive predicate too — forced, it fails with "at depth 2 the builder applied §11.4.6's
element rule outside a knockout group". Depth 2 is exactly the construction the corpus has
**zero** instances of (§7), so the fuzzer is generating what 974 real documents do not.
That is worth stating in its own right: the scene boundary's fuzzer is not only a
crash-finder here, it is the instrument that covers the population the corpus is silent
about. Its `nest_chain` now pins each of the two refusals it can meet to its own condition
rather than admitting either anywhere, and **the assertion was verified reached rather than
assumed** — inverting the arm fails the gate at depth 1.

## 9. The corpus

**Expected to move nothing**, and for a reason that is measured rather than assumed: the
refused construction has a population of zero (§4), and the construction that is *not*
refused is the one all ten corpus instances use. So the claim a run checks here is the
**negative** one — that the refusal is narrow enough not to take the six nested groups that
do reach the encoder, on three pages that agree today. That is not a formality: §7's third
predicate would have taken exactly those three pages, and a run is what tells the two apart
after the fact rather than by reading the diff.

One copy of the caller's tree at `736e01f3`, RADV, both lanes, both scales, taken
2026-08-18 in one sitting. Verdicts are `agree / differ / refused / not comparable`.

| lane, scale | base `fa2747c` | change `e45ab44` |
|---|---|---|
| CPU, scale 1 | 931 / 23 / 2 / 18 | **931 / 23 / 2 / 18** |
| GPU, scale 1 | 929 / 25 / 2 / 18 | **929 / 25 / 2 / 18** |
| CPU, scale 4 | 937 / 11 / 3 / 23 | **937 / 11 / 3 / 23** |
| GPU, scale 4 | 938 / 10 / 3 / 23 | **938 / 10 / 3 / 23** |

**Nothing moved, and the per-page lines say so rather than the totals.** The corpus test
prints a line only for a page that differs or is refused: **79 lines across the four rows**
(25 at CPU 1, 27 at GPU 1, 14 at CPU 4, 13 at GPU 4), naming **37 distinct documents**, and
the change column's set is identical to the base column's in every field — page name,
verdict, mean, worst tile and its coordinates, differing fraction, SSIM, and the full text
of every refusal. `diff` over the sorted line sets is empty for all four rows.

Normalising each run whole — stripping wall clocks, cargo's progress and `Compiling` lines,
the over-60-seconds notice, the expected unused-`quorra`-patch warning and the panic's
thread id — the two columns' outputs diff clean in all four rows (43, 42, 46 and 32 lines).

**And the refusal never fires.** No run in either column contains
`KnockoutElementGroupUnsupported`'s message anywhere. The refusals present are the
pre-existing three, character-identical across columns: `bug1721218_reduced.pdf` (a
four-component blending space, §11.6.6/§11.7.2), `issue18032.pdf` (a non-isolated knockout
group, §11.4.6) and, at scale 4 only, `issue1905.pdf` (a 4763 × 7103 tile against the
adapter's coverage sheet). That is the direct check on §4's population of zero, taken
through the renderer rather than through the walk.

**The `[patch]` demonstrably took**, which is the check that matters most for a range
predicted to change nothing: all five base runs executed
`target/release/deps/corpus-3ef401d9b2d4df05` and all five change runs executed
`corpus-288ad9df85abf799`. Cargo's metadata hash moves with the patched source, so two
hashes is evidence and one would have been the failure mode. Corroborated further down the
graph — both columns' `libquorra_scene`, `libquorra_gpu` and `librender_quorra` artefacts
coexist in the target directory under distinct hashes.

**And the negative claim was checked by name.** Re-run in both columns with
`PDFVIEWER_QUORRA_ONLY` set to the four pages §4 names, both print, character for
character:

```
  refused: issue18032.pdf: this backend cannot draw a non-isolated knockout group: each
  element composites with the group's own initial backdrop, which a scene cannot retain
  beside the accumulation (ISO 32000-2 §11.4.6)
4 pages compared in …: 3 agree, 0 differ, 1 refused, 0 not comparable
```

**0 not comparable** is the load-bearing figure: the three pages whose knockout groups hold
groups by the `DestOut`/`Plus` route were actually rendered and compared in both columns, so
"the refusal did not take them" is a measurement. `issue18032.pdf`'s refusal is the
pre-existing non-isolated-knockout one and its text is unchanged.

**One ratchet fails, in both columns, identically.** The caller's `REFUSED_AT_FOUR` — checked
only on the CPU lane at scale 4 — expects `bug1703683_page2_reduced.pdf` in the refused list
and both columns now draw the page, because both are patched past ADR 0057 while their
`Cargo.lock` pins `eada81ec`, which is earlier. Their re-baseline to take, not a regression
from this range; the same failure ADR 0066's matrix recorded, with character-identical lists
here. The other six runs exit 0.

**No timing is published from this run.** The load average was above 80 when the base column
started. Which pages refuse is arithmetic and machine-independent; how long a row took is
not.

The corpus columns were built from `e45ab44`. The only code difference between that and the
recorded `966bdc1` is a local variable rename inside `tests/nested_knockout.rs`, so no
library behaviour differs between the revision measured and the revision this note sits on.

## 10. Recommended edits to files this round may not touch

`doc/PLAN.md`, appended to the settled-questions list beside ADR 0066's entry:

> - **A group used as an element of a knockout group is refused, not composited — settled,
>   ADR 0069** (2026-08-18). ISO 32000-2 §11.4.6: "The separate shape value shall be
>   computed in any group that is subsequently used as an element of a knockout group",
>   and §11.3.7.2 makes that value "the union […] of the shapes of the objects it
>   contains" — which a premultiplied layer, whose alpha is shape *times* opacity, does
>   not carry. `encode_group` composited such a group by §11.3.6 instead, drawing
>   `[128, 76, 128, 255]` where the clause requires `[26, 102, 229, 128]` and
>   byte-identically to the same group in an ordinary group: principle 6's third state.
>   Now `SceneError::KnockoutElementGroupUnsupported` — **additive public API, for the
>   bump's list**. The construction is available by name: ADR 0033's `Compose::DestOut`
>   then `Compose::Plus` on two groups draws the clause's line at 1.27 of 255 where the
>   composite misses it by 114.95. A census over the caller's 974 page-one display lists
>   found **zero** ordinary group elements of a knockout group and **ten** staged halves
>   on four pages, so the refusal takes nothing and the vocabulary that replaces it is
>   already what arrives.

`doc/PLAN.md`, in the **caller's adoption round** bullet, replacing the sentence that ends
"whose transfer document is `doc/api-change-image-alpha.md`":

> The bump now also carries `CoverageSheet` and `Counters::coverage` with
> `RenderError::ScratchExhausted`'s three new fields (ADR 0057),
> `RenderError::ViewportTransformTooLarge`, and **two** `SceneError` additions —
> `InvalidImageAlpha`, whose transfer document is `doc/api-change-image-alpha.md`, and
> `KnockoutElementGroupUnsupported` (ADR 0069), which refuses a group used as an ordinary
> element of a knockout group. The second needs no transfer document: a census over their
> own 974 page-one display lists found the construction **zero** times, because
> `pdf-model`'s `knockout_elements` already wraps every group element of a knockout group
> in a `Command::Shaped`, and the `DestOut`/`Plus` pair that arrives instead is untouched.

`doc/HANDOVER.md`, a new Traps entry:

> **A rule about "inside a knockout group" is three rules with three different prices, and
> the corpus can only charge for one of them.** §11.4.6 governs a knockout group's
> *elements*; a group two levels down is an element of its own parent and is composited by
> §11.3.6 whatever encloses it. `OpenFrame` carried one transitive boolean, and ADR 0069
> had to pick: keyed on the element, it costs nothing; keyed transitively, it also refuses
> a group inside a `DestOut`/`Plus` half, which costs nothing **today** and is not
> therefore harmless — the caller's expansion of that very clause makes a group's stated
> shape *the group of its elements' stated shapes*, so the day a knockout group holds a
> group that holds a group, a half holds a group; keyed transitively **without** exempting
> the staged pair, it refuses the halves themselves and turns
> `knockout_inner_backdrop.pdf`, `knockout_nested.pdf` and `knockout_nested_group_alpha.pdf`
> from drawn pages into refusals. Only the third has a corpus cost, so the corpus cannot
> choose between the first two and the clause has to. **A null population is a reason to
> read the clause harder, not a licence to pick the wider predicate** — and a draft of that
> ADR asserted the three-page cost for the middle rule before the walk was re-run and
> returned 0 groups inside a half at any depth, over 12 commands walked in those bodies.
> Same family as "a fixture that names a lane should say which lane it means": **when a
> predicate says "inside", ask inside *what*, and at what depth.**

`doc/HANDOVER.md`, a second entry, on the instrument:

> **A population count is a claim about the walk, not only about the corpus.** ADR 0069
> re-took `doc/notes-release-matrix.md` §3's "16 pages / 29 knockout groups / 142 groups"
> and got 16 / 30 / 152 — because the earlier walk did not descend into a
> `Command::Shaped`'s two halves, and **every** nested group in the corpus is one. The
> definition that produced the recorded triple is exactly the definition that cannot see
> the population the next round needs. Reproduce a quoted count under the old definition
> *first*, then vary it: the difference is the answer, and quoting the old number forward
> would have read as "the population is zero" for the wrong reason.
