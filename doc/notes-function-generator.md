# The §7.10.5 analyser and generator: what was built, and what it decides

Written 2026-08-15, against `doc/adr/0053`, `doc/spike-function-paint.md`,
`doc/research-function-paint-arithmetic.md` and the integrator's pinned vocabulary
(revision 2).

Two units, both pure, both testable without a device:

- **`quorra_gpu::function::analyse(&[FnOp])`** — one walk over a flat instruction list,
  producing an `Analysis` or a named `FunctionRefusal`. It is what
  `Device::upload_function` runs, which is why it takes the program and nothing else.
- **`quorra_gpu::function::generate(&Analysis, FnRange)`** — WGSL and a content hash. It
  resolves nothing about the program; the one thing it can refuse is a `Range` the program
  cannot fill.

`crates/quorra-gpu/src/shaders/function_ops.wgsl` is the Table 42 operator library both
share, with one clause citation per operator.

---

## 1. The analysis rules

### 1.1 What one walk produces

A compile-time operand stack of *what is known about a value* rather than of values: its
§7.10.5.1 type, the literal it was pushed as where it was pushed as one, and whether an
inexact operator is upstream of it. From that, in one pass:

| fact | why it is computed rather than supplied |
|---|---|
| **maximum depth** | it is the shader's `var` count; a supplied depth is a claim a wrong page rests on |
| **the type of every slot** | Table 42's `not` is two operators wearing one name, and only the operand's type says which |
| **`copy`/`index`/`roll` counts** | a generated shader cannot name a slot it cannot compute |
| **the agreement classification** | ADR 0053 §3, below |
| **empty-stack pops, counted** | a decision rather than a reading, so it is reported |
| **the lowered `Step` list** | so the generator only prints |

The two inputs occupy slots 0 and 1. A slot index *is* an operand-stack position, so the
shader declares `max_depth` variables and no more. The single-assignment alternative — a
fresh slot per computed value, making `dup`, `exch` and `roll` free renames that emit no
code — is cleaner and is deliberately **not** taken: it would declare 482 `var`s for the
caller's seven-segment witness against 8, on a shader whose measured cold compile is 6.3 ms
and whose compile sits on their first-frame path. Principle 2 forbids trading a measured
number for an unmeasured elegance.

### 1.2 Types, and where a type is enforced

`SlotType` is `Real | Integer | Boolean | Undecided`, with `Integer ⊔ Real = Real` and
anything joined with `Boolean` becoming `Undecided` — because `not` is the operator whose
two meanings genuinely differ.

The line, stated once in `typing.rs`: **a type is enforced where lowering a wrongly-typed
operand would silently compute a different function, and inferred but not enforced
everywhere else.**

- Enforced: `not`, `and`, `or`, `xor` (two booleans or two integers), `idiv`, `mod`,
  `bitshift` (two integers), and an `if`/`ifelse` condition (a boolean).
- Not enforced: `abs` of a boolean is a `typecheck` in PostScript too, and there is nothing
  for us to get wrong about it. Policing it would refuse programs for no gain.

The enforced set is exactly the set whose lowering truncates or reinterprets its operand.
`7.5 2 idiv` under a truncating lowering is `3`; no reading of PLRM3 produces that number,
so it is a plausible-looking wrong colour and principle 6 prices it above a refusal.

### 1.3 The classification, and why the dangerous-operator set is what it is

ADR 0053 §3 asks: does any of `atan`, `sin`, `cos`, `exp`, `ln`, `log`, `sqrt`, `div` reach
a comparison or a jump condition on any path?

**The inexact set is taken exactly as the ADR states it.** Each row is
`doc/research-function-paint-arithmetic.md` §2.2's, and the reasons are WGSL §15.7.4.1's own
words: `atan` at 4096 ULP, `sin`/`cos` with *no* stated accuracy outside `[-π, π]`, `exp`
composed from two loose rows because PLRM3's `-9 -1 exp ⇒ -0.111111` forbids `pow`, `sqrt`
"inherited from `1.0 / inverseSqrt(x)`", `div` at 2.5 ULP where IEEE 754 requires correct
rounding.

**The amplifier set is extended, and the argument is the research document's §3.2 rather
than the ADR's §3.** The ADR names only the comparisons and a jump condition. To those are
added:

- `ceiling`, `floor`, `truncate`, `cvi`, `round` — step functions. Two evaluations that
  straddle an integer by a last bit land a whole unit apart, which is the same order-one
  divergence a comparison produces. Refusing to call these amplifiers would classify
  `x sqrt truncate` as `Exact` while it is the same shape as `x sqrt 0.5 ge`.
- `idiv`, `mod`, `bitshift`, `and`, `or`, `xor`, and `not` over an integer — all truncate
  their operands to integers first, so all amplify for the same reason.
- `not` over a *boolean* is deliberately **not** an amplifier: its operand is already
  exactly 0 or 1 and has no last bit to disagree about.

Two narrowings, both deliberate:

- **The research document's §4.3(a) also lists "stack-count operand" as an amplifier.** It
  is unreachable here, because a computed count is already a refusal
  (`DynamicStackCount`). The refusal subsumes the amplifier.
- **`add`, `sub` and `mul` are not on the inexact list**, and that is a narrower claim than
  it looks. WGSL §15.7.5 permits an implementation to reassociate and fuse them, and a
  generated shader is exactly the shape that hands it a whole expression tree to do it over.
  They are excluded because the difference that licence permits is a rounding of the same
  magnitude as the operation's own, where the rows above are bounded by thousands of ULP or
  by nothing. See §5's objection about the name `Exact`.

The classification names the *earliest* inexact operator in the chain and the *first*
amplifier that reaches it, so it is a function of the program rather than of the walk's
visiting order. Tested.

### 1.4 An arm that is correct and unreachable, kept on purpose

`Agreement::Approximate { amplifier: "if", .. }` cannot be produced today. A `JumpUnless`
condition must be a boolean (§1.2), every boolean-producing operator is itself an amplifier,
and amplifiers are recorded in program order — so the comparison that produced the boolean
is always recorded first.

The check is kept anyway, with the reason beside it: it is what states that *a branch
amplifies*, independently of who else happens to be checking types. If the type refusal is
ever relaxed — a caller with an untyped condition — the classification is still right. A
classification that depended on a refusal for its correctness would be two rules pretending
to be one.

---

## 2. The refusal grounds, with the program that reaches each

Every one is a test in `crates/quorra-gpu/tests/function_refusals.rs`, except the two
`Range` grounds which live with `Analysis::admits` in `function_analysis.rs`. A ground
nobody can reach is not a ground.

| ground | the program that reaches it |
|---|---|
| `ProgramTooLong` | `MAX_PROGRAM_LENGTH + 1` instructions |
| `BackwardJump` | `pop pop` then `Jump { target: 0 }` |
| `JumpOutOfRange` | `true` then `JumpUnless { target: 9 }` in a two-instruction program |
| `UnstructuredControlFlow` | a bare `Jump` reached in sequence — a `goto` |
| `BranchesTooDeep` | `MAX_BRANCH_NESTING + 1` nested `if`s |
| `StackTooDeep` | `MAX_OPERAND_SLOTS` consecutive `dup`s |
| `DynamicStackCount` | `1 1 add copy` (and the same for `index` and `roll`) |
| `StackCountOutOfRange` | `9 copy` on a stack of two |
| `NonFiniteLiteral` | `PushReal(NaN)`, `PushReal(inf)` |
| `UnbalancedBranches` | `x 0.5 gt { 1 1 } { 1 } ifelse` |
| `OperandType` | `7.5 2 idiv`, `1.5 not`, `true 1 and`, and a non-boolean `if` condition |
| `UndecidableOperandType` | `x 0.5 gt { true } { 1 } ifelse not`, and the same before an `if` |
| `InsufficientOutputs` | `x sqrt` under `FnRange::Rgb` |
| `RangeNotFinite` | `FnRange::Gray([0.0, NaN])` |
| `RangeNotOrdered` | `FnRange::Gray([1.0, 0.0])` |

### 2.1 Three of the spike's six grounds are gone, and their absence is the finding

`doc/spike-function-paint.md` §6 lists "an operator outside Table 42", "unbalanced braces"
and "a procedure that is not an `if`/`ifelse` operand". All three were grounds because the
spike compiled PostScript *text*. `FnOp` is a closed enum with no procedure in it, so **none
of the three is expressible** — the caller's compiler owns them. That is the pinned
vocabulary paying for itself.

### 2.2 Two grounds that were mine in revision 1 and are not mine now

`DomainNotFinite`, `DomainNotOrdered` and `MatrixNotFinite` were in the enum until the
domain and matrix moved onto the shading. They belong to the scene boundary now, with every
other rectangle and transform: `Rect::is_ordered`, `Rect::is_finite` and `Affine::is_finite`
already exist and are already applied there. A second copy here would be a second definition
of a valid rectangle.

**The generated shader depends on the domain being ordered and finite** — a `Rect` with
`min > max` makes the emitted membership test admit nothing rather than everything, which is
safe but wrong, and a NaN corner makes it admit nothing at all. That dependency is stated in
`function::domain_bounds`'s doc comment and it is wave 2's to honour when `Paint::Function`
reaches `SceneBuilder`'s validation.

---

## 3. What the generated shader does, clause by clause

```wgsl
fn quorra_function_evaluate(
    x: f32, y: f32,
    domain: vec4<f32>, range_low: vec3<f32>, range_high: vec3<f32>,
    background: vec4<f32>,
) -> vec4<f32>
```

`x` and `y` arrive in the *shading's own space*; mapping the device point through the
inverse `Matrix` is the composing lane's job, and doing it here would bake a placement into
a shader whose hash claims to be the program's. Nothing else is baked in either — the four
`vec` parameters are runtime values, so two placements of one shading compile one pipeline.

1. **§8.7.4.5.2's domain test, and it discards.**

   > Points within the shading's bounding box (BBox) that fall outside this transformed
   > domain rectangle shall be painted with the shading's background colour (Background); if
   > the shading dictionary has no Background entry, such points shall be left unpainted.

   Two things the code says that the clause does not. The test is written inverted —
   `!(x >= lo && x <= hi && …)` — so that a NaN coordinate lands on the *unpainted* side;
   every NaN comparison is false, and the direct spelling would have called it inside and
   run the program on it. And there is no branch on whether a `Background` exists: an absent
   one arrives as `vec4<f32>(0.0)`, so "paint the background" and "paint nothing" are one
   instruction.

2. **The program**, one `var` per slot, every operator carrying its Table 42 citation, the
   stack operators as read-every-source-then-write blocks.

3. **§7.10.1's output clip**, per component, against the `Range` — and *only* the output
   clip. The clause's first half ("input values … clipped to the domain") governs a
   *function* asked about a point outside its domain, which a type 1 shading never does,
   because step 1 has already returned.

The return is a `vec4<f32>`: a colour and a coverage, `1.0` inside the domain and
`background` unchanged outside it.

### 3.1 Three places the operator library departs from the spike

- **`ps_round` is PLRM3's half-toward-greater**, written as floor-then-step rather than
  `floor(a + 0.5)` — the addition would round for operands near 2²³ and hand back the wrong
  integer. The spike's `sign(a) * floor(abs(a) + 0.5)` is half-away-from-zero, which is
  Rust's rule and not PostScript's. All three answers differ at −6.5, and the caller's own
  evaluator has the same defect.
- **`ps_exp` is not `pow`.** PLRM3 gives `-9 -1 exp ⇒ -0.111111`; WGSL's `pow` is undefined
  for a negative base. It is composed from `exp2`/`log2` with a sign-and-parity case split.
- **`ps_bitshift`'s right shift is logical.** PLRM3: "Bits shifted out are lost; bits
  shifted in are 0." An `i32 >>` sign-extends and would shift ones in for a negative
  operand. `-16 -28 bitshift` is 15 under this reading and −1 under the other; the spike and
  the caller's evaluator both take the other. **This is a reading, marked as one** — the
  research document quotes PLRM3's sentence but the sentence's second half is about the left
  shift, and whether "bits shifted in are 0" governs the right shift too is our inference.

And one addition: **`ps_sin`/`ps_cos` reduce their argument to `[-180, 180)` degrees first**,
because WGSL §15.7.4.1 bounds `sin` and `cos` only over `[-π, π]` radians and states the
accuracy is *undefined* outside — while §7.10.5.3's own `DoubleDot` example evaluates `sin`
at ±360°. The reduction costs accuracy of its own for a large operand, which is stated
beside it.

### 3.2 The guard value is an open contract question, and this is the record of it

PLRM3 makes a zero divisor `undefinedresult`, a negative `sqrt` operand `rangecheck`, and
several more; ISO 32000-2 §8.7.4.5.2 says "an error may occur". Neither defines a substitute
value, and a fragment shader has nowhere to raise to.

Every one is guarded, and **the guard must exist**: WGSL §15.7.2's Finite Math Assumption
turns an unguarded overflow into "an indeterminate value of the target type" — an arbitrary
colour that looks like a colour. The guard *value* is 0.0, matching the caller's evaluator.
That number is a decision neither side has written into a contract, and it belongs in the
caller's document beside the empty-stack pop. It is recorded here rather than inferred
silently.

---

## 4. What is device-verified, and what is not

`crates/quorra-gpu/tests/function_device.rs` compiles the generated module, runs it in a
compute pass over a storage buffer of `vec4<f32>`, and compares against an independent host
evaluation. A buffer rather than a raster on purpose: the spike measured 246 044 texels off
by one from ADR 0006's 8-bit store conversion alone, and a raster would put that between the
shader and the assertion.

**Verified on `AMD Radeon 890M Graphics (RADV STRIX1)`, this machine's real adapter:**

- Every one of the sixteen witness programs, at 49 sample points, on all four channels.
  Programs with no inexact operator in them are asserted **bitwise equal** to the host
  evaluation; the rest against an explicit 1e-4 relative tolerance, because WGSL licenses
  the disagreement and a tighter bound would be a promise about a driver.
- `-6.5 round → -6.0` and `2.5 round → 3.0` — PLRM3's tie rule, surviving the compiler.
- `-16 -28 bitshift → 15` — the logical right shift.
- `63 not → -64` — Table 42's one's complement.
- §8.7.4.5.2's domain rule, both halves: with a domain a quarter the size of the sampled
  area, the outside is `vec4(0)` with no background and exactly the background colour with
  one — and in both cases *not* the `[0.5, 0.5, 0.25]` a clamping shader would have painted.

**Not verified:**

- **Anything on lavapipe.** The test takes whichever adapter `request_adapter` returns and
  names it in every assertion message; it was run on RADV. ADR 0053's consequence stands
  unmeasured by me: cross-adapter identity is not promised for this paint, and a
  function-shading page under lavapipe is not evidence about the same page on RADV.
- **Compile time.** No number of my own; the spike's 6.3 ms cold for a 482-instruction
  program is the only figure, and my emitter's output differs from the spike's (the domain
  test, the citations, the clamps). The comment budget alone adds roughly 50 bytes per
  instruction.
- **Anything inside a frame.** No lane is wired, by instruction. Every device number here is
  a compute pass over 49 points.
- **The caller's two real witnesses.** They are PDF streams in the viewer's tree and the
  compiled form is theirs to produce; my corpus is sixteen small programs written to reach
  named properties, not a conformance corpus. A sibling owns that.

---

## 5. Objections to the pinned vocabulary and to ADR 0053

Revision 2 fixed the two I would otherwise have raised (the domain clamp, and `Paint`
losing `Copy`). Four remain.

### 5.1 `Agreement::Exact` claims more than it can deliver — **the important one**

The name says bitwise identity. The property is "no inexact operator's value reaches an
amplifier". Three things sit between them, and all three are in documents this project
already treats as authoritative:

1. **WGSL §15.7.5**: "An implementation may reassociate operations", and may fuse them. A
   generated shader is a straight-line expression tree, which is exactly what a compiler
   reassociates. So `add`, `sub` and `mul` — the three operators the accuracy table calls
   "correctly rounded" — are not reproducible bit for bit *in this lowering*. The research
   document's §2.3 says so explicitly and calls it the most important paragraph in its §2.
2. **§15.7.4's own definition of "correctly rounded"** is weaker than IEEE 754's: "the
   result may be rounded up or down: WGSL does not specify a rounding mode."
3. **ADR 0006's store rounding** still sits between the shader and the texel, and the spike
   measured 246 044 texels off by one from it on one adapter.

I implemented the classification as ADR 0053 §3 states it and used its names, because the
decision is the ADR's to take. But `Exact` is a word principle 6 holds to a high standard —
"whatever a `Frame` says about itself must be true" — and a caller reading `Agreement::Exact`
will reasonably conclude it may compare pixel for pixel. **I would rename the two variants
`Bounded` and `Unbounded`**, which is what the research document's §4.3(a) actually argues
and what the property actually is: for a program with no amplifier the disagreement stays a
bounded *colour* error, which is ISO 32000-2 §10.7.3's own currency; for one with an
amplifier there is no bound to state. The doc comment on `Agreement::Exact` currently spends
a paragraph walking that back, which is the smell.

My device test works around the naming by asserting bitwise equality only for programs with
**no inexact operator at all**, which is a stricter condition than `Agreement::Exact` — and
the fact that the test needed a second, stricter predicate to say what it meant is itself
evidence about the name.

### 5.2 `Analysis::empty_stack_pops` counts static occurrences, not dynamic ones

A pop from an empty stack inside an `ifelse` arm is counted whether or not that arm runs.
The pinned decision says the frame carries a `Report` "the first time a program relies on
it"; what I can supply is "the program *contains* N such pops". Making it dynamic would need
a per-fragment counter, which is a per-fragment cost for a diagnostic.

I think the static count is the right trade and the `Report`'s wording should follow it —
"this program reads an empty operand stack in N places" rather than "this program relied
on…". Naming it wrongly would be the second kind of instrumentation defect CLAUDE.md
records: a statement about the lookups you made rather than the ones you should have made.

### 5.3 `FunctionId` is not a `ResourceId` variant, and cannot be released

I added `FunctionId` but deliberately left `ResourceId` alone: adding the variant means
adding release arms in `device.rs` and `resources.rs`, which is `Device::upload_function`'s
own work and wave 2's. An identifier a device can issue but not release is a resource leak
with a name, so **the two changes should land together** — the variant, the `From` impl, the
two match arms and `upload_function` in one commit. Stated here so it is a decision rather
than an omission.

For the same reason `Paint::Function` is **not** in this worktree: it would need arms in
`encode.rs`, `encode/rare.rs` and `scene/validate.rs`, and a half-wired lane that draws
something is what principle 6 forbids. `quorra-scene`'s `function.rs` here holds `FnOp` and
`FnRange` only.

### 5.4 A smaller one: `INPUTS` is two, and it is a shading's fact rather than a function's

`analyse` hard-codes two inputs, because §8.7.4.5.2's type 1 shading has two. A §7.10.5
function in general does not — a type 4 used as a `TilingType` or a soft-mask transfer
function has one. If a second caller ever arrives, the input count becomes a parameter and
the shader's signature changes with it. Recorded so the next person does not have to
discover it from the constant.

---

## 6. Where the seams are, for wave 2

- `Device::upload_function(&[FnOp])` runs `function::analyse`. The refusal it returns is a
  `FunctionRefusal`; wrapping it in a `DeviceError` variant is wave 2's.
- `Analysis::program_hash()` keys the upload table (one program, one entry).
  `Analysis::shader_hash(range)` keys the pipeline cache (one program under one component
  count, one shader). They are two hashes because they answer two questions.
- `Analysis::admits(range)` is the question a `Paint::Function` asks at scene-build time,
  before a frame.
- `generate(&Analysis, range)` gives `GeneratedShader::module()` for
  `create_shader_module` and `ENTRY_POINT` for the call site. The lane supplies
  `domain_bounds(rect)`, `range_bounds(range)` and `background_rgba(colour)`.
- `Analysis::agreement()` and `Analysis::empty_stack_pops()` are the two `Report`s ADR 0053
  requires.
- `GENERATOR_REVISION` must be bumped whenever the emitter's output changes without the
  program changing. `tests/function_generate.rs` pins both the emitted text and the hash, so
  the failure that forces the bump is unmissable — but it is discipline, not a type, and it
  is the one place in this design where that is true.
