# The function paint, wired end to end: what draws, what is verified, and two clause defects the corpus found

Written 2026-08-15, against `doc/adr/0053` **including its amendment**, the three wave-1
notes (`notes-function-scene.md`, `notes-function-generator.md`,
`notes-function-conformance.md`), and `doc/spike-function-paint.md`.

**Amended later the same day**, after `tests/function_knockout.rs`, `tests/function_retained.rs`
and `examples/function_compile.rs`: §4.6, §4.7 and §4.8 are new, and §4.5 and §6 carry
struck-through corrections where they now say something false. Nothing else moved, and
nothing was deleted — a claim two test files cite by section number is corrected in place
rather than quietly removed.

**Amended again on 2026-08-16**: §4.5's remaining three bullets are closed, in the same
style. `doc/notes-function-gaps.md` is that round — what each gate observes, the clause each
expectation comes from, and the forced defect that proved each able to fail.

Wave 1 left a vocabulary, an analyser, a generator and a conformance corpus, and no lane:
`Paint::Function` reached the encoder and was refused by name. **It draws now.** This
records what was wired, what was measured and where, the bound I justify for the corpus
comparison, and the two places where running the corpus on the device found our reading of
ISO 32000-2 to be wrong.

---

## 1. The lane, module by module

| what | where | one sentence |
|---|---|---|
| admission | `quorra-gpu/src/function/admit.rs` | `check_program`, then `analyse`, then ADR 0053 §3's classification — the three questions an upload asks, in the order that makes a refusal cheapest |
| the resource | `quorra-gpu/src/resources.rs` | a fifth id space; what is stored is the **analysis**, not the instruction list, because every static question was answered once at upload |
| the API | `Device::upload_function` / `Device::release` | one method each, and the release drops the program's compiled pipelines when the last id naming it goes |
| the pipeline cache | `quorra-gpu/src/pipeline/function.rs` | one module and one pipeline per `(shader hash, style, target format)`, compiled on first use, never on the launch path |
| the shader's fixed half | `quorra-gpu/src/shaders/function_lane.wgsl` | the quad, the coverage, the clip and the soft mask, appended to what `generate` emits |
| the placement | `quorra-gpu/src/encode/function.rs` | `FunctionOp`: the program, the `Range`, the inverse `Matrix`, the `Domain`, the `Background`, and a `QuadPlacement` |
| the pass | `quorra-gpu/src/compose/function.rs` | 208 bytes of uniform, three bindings, and the two pipelines a style needs |
| the report | `encode::function::empty_stack_reports` | one `ReportKind::FunctionEmptyStackRead` per distinct program per frame |

### 1.1 The one refactor in existing code, and why it is not duplication

`encode/rare.rs` gained `QuadPlacement` — *where* a quad goes and what weights it — and
`RarePaint`, the enum of what colours it. The shading lane and the function lane now share
`rect_placement` and `coverage_placement` and differ only in the op they build from the
result. That matters for one line in particular: "the quad is exactly the tile", which both
shaders' texel arithmetic (`coverage.xy + p − dest.xy`) depends on, is now stated once.

`encode.rs`'s two call sites changed by name only (`shaded_geometry` → `rare_paint`,
`push_shaded_*` → `push_rare_*`): the fill and stroke walks do not know which of the two
paints they have, which is the property that kept `encode.rs` from growing.

### 1.2 What the pass does per fragment, and the clause for each step

1. Map the device pixel's **centre** back through the inverse of `Matrix ∘ viewport`
   (§10.7.4 is why the centre; the caller's ADR 0339 replaced a sampled grid with the
   device's own grid for the same reason).
2. The generated function's own first instruction is §8.7.4.5.2's domain test, and it
   **discards**: outside the transformed domain rectangle it returns `Background`, which is
   `vec4(0)` when there is none. Never the nearest edge's colour.
3. Inside, the program runs and each output is clamped into its `FnRange` pair (§7.10.1's
   second half; the first half governs a *function* asked about a point outside its domain,
   which a type 1 shading never does because step 2 has already returned).
4. The result is premultiplied by its own alpha and weighted by coverage × clip × soft mask,
   exactly as the ramp lane weights its own.

`fs_shape` (knockout's erase, ADR 0010) marks shape where the paint marks at all. The one
consequence worth stating: a `Background` whose alpha is zero is indistinguishable from an
absent one, because `background_rgba` encodes both as `vec4(0)`. They paint the same pixels,
and §11.4.7.2's shape/opacity distinction survives everywhere it is observable.

### 1.3 The startup rule, honoured by construction

`PLAN.md` §1.8: nothing on the launch path waits for warmth. A generated pipeline **cannot**
be in the warm set — the program does not exist when the device is constructed — so it
compiles on the first frame that draws it and names itself in that frame's
`Timings::phases` as `"function shader compile (first use)"`, distinct from the fixed
table's phase because its cost is a function of the program's length. A caller that wants
the compile off its first frame uploads the program early.

A compile that fails is a refusal that names its span: `PipelineProblem::GeneratedShader`
and `::GeneratedPipeline` carry the program's content hash where the fixed table carries a
`&'static str`, plus `wgpu`'s message with its span — which for generated text is the only
way to say *where*, since the source is not in this tree. Verified in both directions by
`pipeline::function::tests::a_refused_generated_pipeline_names_its_program`, which asks for
a format WebGPU gives no `RENDER_ATTACHMENT` usage.

---

## 2. Two clause defects the corpus found, and what they cost

Both were found by running `quorra-function-conformance` against the device, which is
exactly what that crate was built for. Both are changes to **wave-1 code**, and both are
recorded here because a reader of the wave-1 notes will otherwise find those notes stale.

### 2.1 §7.10.5.3's output count is an equality, and `Analysis::admits` had it as a floor

ISO 32000-2 §7.10.5.3, verbatim:

> It shall be an error for the number of remaining operands to **differ** from the number of
> output variables specified by **Range** or for any of them to be objects other than
> numbers.

Wave 1 read this as "at least": `values_left`'s own doc said "a `Range` may take its
components from the top of that; anything below is the program's own scratch", and
`admits` refused only `produced < required`. The corpus's
`refusal/output-count-cannot-match-the-range` case — three values left, one component
declared — was therefore **drawn**, and `Refusal::OutputCountMismatch` was a ground nothing
enforced.

Changed: `admits` refuses `produced != required`, and
`FunctionRefusal::InsufficientOutputs` is renamed `FunctionRefusal::OutputCount` because the
name was half the reading. The consequence worth knowing:

- **A program now admits exactly one component count.** Two shadings can still share one
  program under different matrices, domains, backgrounds and range *bounds* — but not under
  a `Gray` range and an `Rgb` one, because a conforming program cannot supply both.
  `Analysis::shader_hash`'s component mixing is therefore redundant with the program today;
  it stays, because it is what makes the key correct independently of who enforces the
  count, and `function_generate.rs` now asserts it on the hash rather than on two shaders.
- Six test fixtures that leaned on the loose reading were corrected (a witness has to
  consume the two inputs a shading pushes if it wants a one-component range). None of them
  was asserting the count; all of them were choosing a range casually.

### 2.2 `true 1 eq` was `true`, and PLRM3 says it is `false`

PLRM3's `eq`, which §7.10.5.2 makes normative:

> Simple objects are equal if their types and values are the same.

A boolean and a number are of different types, so they are never equal. On our operand stack
`true` *is* the `f32` `1.0`, so the emitted comparison answered the opposite of the clause —
a wrong colour, not an error, and invisible to every test in the tree until the corpus's
`eq/boolean-is-never-equal-to-a-number` ran on a device.

Fixed in the walk, where the types are already known: an `eq`/`ne` whose operands are a
boolean and a number lowers to a **literal** rather than to a comparison
(`typing::comparison_is_decided_by_type`), and an `eq`/`ne` over a type two branches
disagreed about is now `UndecidableOperandType` — because down one path the answer is the
numeric comparison and down the other it is the constant, and there is no third reading.

Nothing is amplified or tainted by the constant: it does not read either operand's value, so
an inexact operator upstream of one cannot reach it.

**The gate was verified able to fail**: inverting the constant makes exactly that corpus case
fail, with the reference's value and the device's printed side by side.

### 2.3 A third thing I found and deliberately did **not** change

PLRM3 makes `gt`, `ge`, `lt` and `le` a `typecheck` on a boolean operand, and we compare it
numerically instead — producing a value where the entry has none. It is left alone because
it is the same shape as every other guarded error in `function_ops.wgsl` (a zero divisor, a
negative `sqrt`), and ADR 0053 §3.2 already records the guard value as an open contract
question with the caller. Changing one of the family without the others would be a third
reading. The corpus's expectation for those cases is `Error`, so nothing asserts a value
either way.

---

## 3. ADR 0053 §3's refusal, taken at the upload

> a program that can reach a transcendental on any path into a comparison is refused by
> name, and the caller falls back to the raster they build today

`Device::upload_function` refuses an `Agreement::Unbounded` program with
`FunctionProblem::NoAgreementBound`, naming both operators and both positions. The analyser
is untouched: it still *classifies*, and the policy is in `admit.rs` where a reader can find
it beside the sentence it implements.

The consequence is that `Analysis::agreement()` is `Bounded` for every program a device
holds, and the classification's value to a caller is entirely in the refusal. That is what
the ADR asked for; the amendment renamed the classification and did not move this line.

---

## 4. What is device-verified, and on which adapter

Everything below was run on **both** adapters of this machine, by name:
`AMD Radeon 890M Graphics (RADV STRIX1)` and `llvmpipe (LLVM 22.1.8, 256 bits)`.
`QUORRA_ADAPTER` selects one in `tests/function_lane.rs`, `tests/function_conformance.rs`,
`tests/function_knockout.rs`, `tests/function_retained.rs` and the shared compute harness;
the suite's default is the software rasteriser, as everywhere else in this tree.

### 4.1 In a real frame — `tests/function_lane.rs`, 12 tests

- **A program of both inputs colours every pixel by its own position**, through the
  rect-hinted lane and through the rasterised-coverage lane, to ±1 of the byte ADR 0006's
  store leaves. The two lanes place the quad differently and only one of them is a
  rectangle, which is why both are tested.
- **§8.7.4.5.2's domain rule, with a domain a quarter of the shape.** With no `Background`
  the outside is `[0, 0, 0, 0]`; with one it is exactly the background's bytes; and in
  neither case is it the colour a clamping shader would have smeared there. *No corpus run
  can see this* — both of the caller's witnesses declare `/Domain [0 1 0 1]` — which is why
  `notes-function-scene.md` §7 called it the single most valuable test in this feature.
- **§7.10.1's output clip against a `Range` that is not the unit interval**: red clipped
  into `[0.25, 0.75]` while green and blue keep `[0, 1]`, so the clip is per component and
  not one rule applied to a colour. Invisible to the corpus for the same reason.
- Two placements of one program draw their own geometry; a released program is refused by
  name; a `Range` the program cannot fill refuses the frame before a quad is placed; two
  frames of one scene are byte-identical.
- **The empty-stack `Report`**: a program that reads a value the stack has not got draws,
  and the frame carries one report. A program that does not carries none.
- **Counters**: one command, no coverage tile for a rect-hinted fill, exactly one for a
  curve-bounded one, no layer texture. Exact functions of the scene, so they compare by
  equality on any machine.
- **One shader compile, then none**: the first frame's `phases` names the compile and the
  second's does not.

### 4.2 The corpus on the device — `tests/function_conformance.rs`, 3 tests

Every one of the corpus's **125** cases lands in exactly one bucket, and the buckets are
printed and asserted so that a run comparing nothing cannot pass:

| bucket | count | why |
|---|---:|---|
| compared against the reference evaluator | **91** (73 of them bitwise) | the case fixes outputs and the device admits it |
| carrying no value | 30 | PLRM3 names an error, or neither document defines anything |
| domain-clip cases a shading cannot observe | 4 | §8.7.4.5.2 discards a point outside the domain instead of clipping it, so a *shading* can never ask its function about one |
| refused before the frame | 0 of the value-carrying cases; 7 of the refusal family | — |

Identical on both adapters. The **seven** refusal grounds `Refusal::ALL` names are each
reached through the device's own path — at `upload_function` for the five that are a
property of the program, at `Analysis::admits` for the output count — and the ground the
device names is matched against the ground the corpus named, variant by variant, rather than
by string.

### 4.3 The bound I justify, and what it is not

Two comparisons, chosen by a property of the program rather than by what passed:

- **A program that calls none of `atan`, `sin`, `cos`, `exp`, `ln`, `log`, `sqrt`, `div` is
  compared bit for bit** — 73 of the 91. That is stricter than `Agreement::Bounded`, which
  only says no inexact operator reaches an *amplifier*.
- **Everything else to 1e-3, relative or absolute, whichever is larger.** The loosest row of
  WGSL §15.7.4.1 is `atan` at **4 096 ULP**, which at a result of magnitude *m* is
  `m × 4096 × 2⁻²³ ≈ m × 4.9e-4` — inside a relative 1e-3 at every magnitude and inside an
  absolute 1e-3 below 1. `sin`/`cos` are stated as an absolute 2⁻¹¹ ≈ 4.9e-4 over `[-π, π]`;
  `div` is 2.5 ULP; `sqrt` is inherited from `inverseSqrt`. Every one is inside the bound
  with room, and the bound is *derived from the accuracy table* rather than from a run.

**The bound is the test's instrument and not a claim about ISO 32000-2**, which states no
precision anywhere (§7.3.3; ADR 0053 §2). It is also not a claim about a second adapter:
ADR 0053's consequence stands, and the two adapters agreeing here is an observation about
125 small programs, not a promise.

### 4.4 The scene boundary, fuzzed

`tests/fuzz_scene` now uploads random §7.10.5 programs and paints with them: **161 programs
were admitted** across its 2 000 seeds (one seed in three attempts an upload), and every
other attempt exercised a refusal. The run's wall clock did not move out of its own spread —
1.40 s before and 1.46 s after, on a machine whose load average was 1.9 at the time, which
is context rather than evidence. This closes `notes-function-scene.md` §7's second gap — principle 3
asks for the scene boundary to be fuzzed from the first commit, and a compiled program is
the newest thing to arrive across it.

### 4.5 Not verified

**Three of the four items below have closed since this section was written, and they are
kept here with their corrections rather than deleted** — `tests/function_knockout.rs` and
`tests/function_retained.rs` both cite this section by number for what they were written
against, and a reader following the citation should find the sentence *and* what happened
to it. Corrected 2026-08-15.

- ~~**A duration of my own for a generated compile.**~~ **Closed by §4.8**, which measures
  it on both adapters. What the bullet said was that the spike's 6.3 ms is the only figure
  and that a wall clock on this machine is worthless — the first half was true, and the
  second was too strong for *this* quantity. The compile is a direct span the frame already
  reports by name, not a difference between two frames, and it is milliseconds rather than
  microseconds; minima of round-robin rounds at a stated load average measure it, and the
  callgrind instrument this bullet asked for is not needed to.
- **The caller's two real witnesses.** They are PDF streams in the viewer's tree and the
  compiled form is theirs to produce.
- **A page of the caller's corpus.** Nothing in their corpus reaches this paint yet — they
  build the raster today — so a corpus run would measure that we changed nothing, which is
  worth doing when the bump lands rather than now.
- ~~**A knockout group over a function paint.**~~ **Closed by §4.6.** The sentence was
  accurate when written — the pair compiled, `Style::of` selected it, `fs_shape` was
  reviewed, and nothing drew one.

And three the two new files did **not** close, found by reading them rather than by
assuming what their names cover. **All three closed on 2026-08-16** —
`doc/notes-function-gaps.md` is the round, ten tests in three new files, and each bullet
below carries its correction rather than being deleted, because the sentence is what a
reader following a citation comes here for:

- ~~**`base_weight`'s clip and soft-mask factors are never anything but 1.**~~ **Closed by
  `tests/function_weights.rs`**, five tests: a rectangular clip whose edge falls inside a
  pixel, a residue clip, §11.5.2's alpha mask, §11.5.3's luminosity mask over a white
  backdrop, and the two together as a product. What the bullet said was right and the
  suspicion it carried was not — no defect was found, and each factor is now observed at a
  value the other two cannot produce. §1.2's step 4 is measured rather than reasoned about.
- ~~**Nothing draws this paint under `Coverage::Gpu`.**~~ **Closed by
  `tests/function_coverage.rs`**, and the bullet's own hedge — "the risk is small and
  stated rather than measured" — understated what could be said. The setting cannot reach
  this paint *at all*: `take_gpu_lane` is asked only in the solid arm, so the two settings
  draw a page of function paint **byte for byte the same**, which is what the file asserts
  instead of a bound. The second test puts a device-drawn tile and a function tile on one
  sheet, which is the interaction the setting does create.
- ~~**The scene's own §11.4.6 stages**~~ (ADR 0025's `DestOut` and `Plus`) **are drawn now**,
  by `tests/function_staged.rs`: the pair against §11.4.6's line at 0.57 of 255 where one
  source-over mark is 114.90, plus the two things peculiar to this paint — a point
  §8.7.4.5.2 leaves unpainted has no shape and so is erased by nothing, and a `Background`'s
  alpha is opacity and changes nothing about what `DestOut` erases.

### 4.6 In a knockout group — `tests/function_knockout.rs`, 3 tests

§11.4.6's replacement is held in the pixels, over the whole 64×64 target, against the line
the clause gives for an isolated group: `P' = (1 − f) × P + S` per channel, premultiplied,
with `f` read from the device as the mark's own shape rather than assumed. The fixture is
ADR 0025's wedge, so 30 or more pixels are partially covered and a weighted average and a
composite cannot agree by accident, and the **same element in an ordinary group is measured
against the same line and must miss it** — 3 of 255 against 16 of 255 — because a fixture
where the two readings agree holds nothing.

Both quad placements are drawn under knockout: the wedge takes the rasterised-coverage
lane, the two full-target fixtures take the rect-hinted one.

The two peculiar to this paint, each of which is a whole test:

- **A point §8.7.4.5.2 leaves unpainted has no shape**, so with a domain a quarter of the
  shape the knockout group keeps exactly what it accumulated outside it, while inside it
  the group carries the function's own value at the pixel's centre.
- **A `Background` of alpha ½ has shape 1 at opacity ½** (§11.4.7.2), which is the only
  construction that can observe the distinction, and therefore the only one of the three
  that holds ADR 0010's erase/add pair.

**Both gates were verified able to fail, in the direction each claims** (2026-08-15).
Forcing the function lane to a single `Over` pass leaves the first two green and fails the
`Background`-alpha test alone, at 0.55 where §11.4.6 requires 0.10. Removing `fs_shape`'s
domain guard so the erase marks its whole quad leaves the other two green and fails the
domain test alone, and it fails it by leaving `[0, 0, 0, 0]` where the page is opaque —
a transparent hole, not a shade.

### 4.7 Through a retained frame — `tests/function_retained.rs`, 6 tests

§6's "a `Paint::Function` op replays like any other" is now checked rather than reasoned
about. A page drawing one program through **both** placements at once replays byte for byte
against an immediate frame, and `Frame::encode_source` says which encode drew each one, so
a replay that had quietly re-encoded could not pass by drawing the right pixels.

`EncodeKey::resource_generation` is asserted in both of its directions on a program the
retained scene never mentions: an unrelated *upload* must replay, an unrelated *release*
must re-encode. And the strongest of the six is principle 6 in one assertion — releasing
the program the encode names re-encodes rather than replaying a stale encode, the fresh
encode refuses by name, and the refusal is identical on all three attempts with the handle
holding nothing. Re-uploading the same instructions does not revive it: the content hash is
the same but the identifier is new, so only a scene naming the new id draws, and it pays
for the shader again — which is the observable half of the release having reached the
pipeline store.

Compiles are counted by name out of `Timings::phases`, never timed: three placements of two
distinct programs are two shaders and a replay is none.

### 4.8 What a generated shader costs to compile

**The one open item of §4.5, measured here.** `examples/function_compile.rs` is the
instrument and its module comment carries the method; this is the result.

Minima of twelve round-robin rounds, three alternating runs per adapter, first round of
each run excluded, **2026-08-15 at a one-minute load average of 5.1–8.6** on a
24-core machine:

| program | RADV | llvmpipe |
|---:|---:|---:|
| 1 instruction | **2.67 ms** | **2.04 ms** |
| 121 | 3.90 ms | 3.03 ms |
| 482 — the witness's length | **8.25 ms** | **6.88 ms** |
| 964 | 16.41 ms | 14.38 ms |

Three things worth taking from it, and one that is not in it:

- **The spike's 6.3 ms holds.** Its RADV row for a generated shader with a cold driver
  cache was 6.3 / 7.8 / 16.3 / 16.6 ms for the 482-instruction witness; ours is 8.25 ms for
  a program of that length out of a different emitter. Same order, inside the spike's own
  spread, and ADR 0053's choice of the generated shape is not disturbed by it.
- **Most of what a short shading pays is not its program.** A *one-instruction* program
  still costs 2.0–2.7 ms, which is the lane's fixed half (`function_lane.wgsl`) parsed and
  built. Anything that ever wants this cheaper should look there before it looks at
  `generate.rs` — and nothing should, until a caller has a page that pays it.
- **The cost is superlinear in the program's length, mildly.** The marginal cost per
  instruction rises from about 10 µs over the first hundred to about 17 µs over the last
  five hundred, on both adapters and by the same shape — two independent compilers agreeing
  is what makes that a finding rather than a wobble. No model is claimed from four points.
- **It is not the witness.** The program measured is our emitter's output at the witness's
  *length* and branch density (482 instructions, 23 branches), built by
  `examples/function_compile.rs`. The witness itself is a PDF stream in the caller's tree,
  which is still §4.5's second bullet.

What this does **not** change is the rule the cost is subject to: a generated pipeline
cannot be in the warm set, because the program does not exist when the device is
constructed (§1.3). 8 ms is four frames of §6.2's 2.0 ms bar, paid once, on the first frame
that draws the shading — which is the sentence a caller needs in order to decide to upload
early, and it is the reason `Device::upload_function` is separate from the frame at all.

---

## 5. The three wave-1 objections this closes

- **Generator §5.2 (the empty-stack report's wording).** Closed as the objection asked:
  `ReportKind::FunctionEmptyStackRead`'s documentation and the report's own text both say
  the count is of the program's *instructions*, not of the pixels that took them, and say
  why making it dynamic (a per-fragment counter for a diagnostic) is the wrong trade. The
  wording follows the count rather than the other way round, which is CLAUDE.md's
  instrumentation rule read in its other direction.
- **Generator §5.3 (an id that can be issued but not released).** Closed: `FunctionId` is a
  `ResourceId` variant with an upload path, a release path, a byte cost counted over the
  lowered tree rather than estimated, and — the part that is easy to miss — a release that
  drops the compiled pipelines the program's hash keys, unless another resident program has
  the same instructions.
- **Generator §5.4 (`INPUTS` is a shading's fact).** Untouched, deliberately: nothing in
  this round needed a second input count, and changing the constant into a parameter with no
  second caller would be a shape nobody measured. Recorded again so it is not lost.

Objection §5.1 (`Exact` claims too much) was already closed by ADR 0053's amendment; the
lane uses the amended names throughout.

---

## 6. Deliberately not done

- **A `Paint::Function` archetype in `tests/archetypes.rs`.** It does not fit that file's
  contract. Every field of its `Archetype` is a number from `doc/corpus-profile.md`, which
  carries a "shading/mesh fills" row and **no function-shading row at all**, and the struct
  has no shading field of any kind. An archetype for this paint would be exactly the
  invented fixture that file's opening paragraph refuses. What it *would* have bought — a
  counter gate that cannot drift — is in `function_lane.rs` instead
  (`a_function_fill_costs_one_command_and_a_tile_only_when_it_is_rasterised`), in the same
  style and with the reason written beside it.
- **A stored golden PNG.** Every expectation in `function_lane.rs` is derived arithmetic —
  the function's value at the pixel's centre, stored per ADR 0006 — which is strictly
  stronger than a blob nobody can re-derive, and is what principle 5 asks for. A PNG would
  be a record of one run on one adapter, which is the thing ADR 0053 says not to promise.
- **A `Scene::cost()` change.** `function_programs` already counts distinct `FunctionId`s,
  which is exactly what a page pays: one generated shader per distinct program.
- ~~**Retained-encode coverage.**~~ **No longer true, and it was the wrong call**:
  `tests/function_retained.rs` now draws one, and §4.7 says what it holds. The bullet's
  reasoning — an op that carries no device handle beyond a raw id replays like any other —
  was sound and was also not the interesting half. What the tests found worth holding is
  the *release* path: an id that stops resolving, a pipeline the encode's frame compiled
  and the release dropped, and a retained encode that must refuse rather than replay. None
  of that follows from "it is an `Op` in a `LayerPlan`". Corrected 2026-08-15.
- **`device.rs` grew by 66 lines** (`upload_function` and its documentation, the release
  arm, an analysis accessor, the report call) and `encode.rs` by 12. Both were already far
  past the ~500-line smell and I was asked not to make them worse; this is the minimum an
  API method and an op variant cost, and every other line of the lane is in one of the four
  new modules.

## 7. Where a reader should start

`crates/quorra-gpu/src/function/admit.rs` for what a device will execute and why,
`src/shaders/function_lane.wgsl` for what a fragment does, and
`tests/function_lane.rs`'s domain and range tests for the two clause readings that no corpus
run can check.

For what it costs, `examples/function_compile.rs` and §4.8. For the two hardest things it
has to get right in a frame, `tests/function_knockout.rs` — §11.4.6's replacement measured
where a partially covered pixel makes the two readings disagree — and
`tests/function_retained.rs`'s last two tests, which are the release path and are the only
place in this lane where principle 6 has teeth.
