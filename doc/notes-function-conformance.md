# The §7.10.5 conformance corpus: where every expectation came from

Written 2026-08-15, alongside `crates/quorra-function-conformance`. It records three
things a table of numbers cannot: **which document each expectation came from**, **every
place the specification defines nothing**, and **every place the clause, PLRM3, Rust and
WGSL give four different answers**. It closes with the parts of the pinned vocabulary I
think are wrong, with the argument, as the brief asked.

`doc/adr/0053` is the decision this serves, and its last consequence is the reason this
crate exists: *"the classification needs a conformance test per dangerous and per safe
operator before it is a contract"*.

## 0. What was read, and what the corpus is

- **ISO 32000-2:2020**, from the PDF at `/home/cl/projects/pdf-viewer/doc/`: §7.10.1 in
  full including Table 38 and both worked EXAMPLEs (printed pages 123–124), §7.10.5.1–.3
  in full including Table 42 (printed pages 130–132), and **Annex B (informative),
  "Operators in Type 4 Functions"** in full (printed pages 848–849). Table 42 itself
  lists 42 operator *names* and no descriptions; Annex B's four tables are ISO's own
  one-line summary of each, and §7.10.5.3 points at them ("Annex B … contains a summary
  of these operators").
- **PLRM3**, the *PostScript Language Reference, third edition*, which §7.10.5.2 makes
  normative for the semantics. Operator entries were read from the PDF a previous session
  downloaded (`doc/research-function-paint-arithmetic.md` §5 records that it is not
  committed to this tree and that anyone relying on these quotations should obtain the
  document).
- `doc/research-function-paint-arithmetic.md` for the WGSL accuracy rows and the silence
  about precision; `doc/spike-function-paint.md` §6 and §7 for the refusal grounds and
  the two contract questions. **Neither the spike's `eval.rs` nor the caller's evaluator
  was read while writing the reference evaluator**, so a disagreement between the two
  implementations is evidence rather than an echo.

The corpus is **125 cases** across seven families, and the gate
(`tests/corpus.rs`) checks each one against a reference evaluator written separately from
the same clauses. Confirmed to be able to fail: replacing the reference `round` with
Rust's `f32::round` fails exactly one case, `arithmetic/round/tie-negative-goes-up`, with
`expected -6 (Exact), got -7`.

## 1. The operator table

Every one of Table 42's 42 operators has at least one case; a test asserts it
(`the_corpus_covers_every_table_42_operator`). "PLRM3 example" means the expected value
is printed in the operator's own entry; "PLRM3 sentence" means it is derived from a
sentence there, with no worked example to check it against; "ISO Annex B" means ISO's own
summary is the source of the operand/result shape.

| Operator | Cases | Where the expectation comes from |
|---|---|---|
| `abs` | 3 | PLRM3 examples `4.5 abs ⇒ 4.5`, `–3 abs ⇒ 3`; the `i32::MIN` case from the entry's "unless num1 is the smallest (most negative) integer, in which case the result is a real number" |
| `add` | 3 | PLRM3 examples `3 4 add ⇒ 7`, `9.9 1.1 add ⇒ 11.0`; overflow from the integer-or-real sentence |
| `sub` | 2 | PLRM3 sentence (same integer-or-real rule); the mixed-operand case observes the type through `not` |
| `mul` | 2 | PLRM3 sentence; overflow chosen at 2³² so the expectation is not a statement about binary32 |
| `neg` | 2 | PLRM3 example `4.5 neg ⇒ −4.5`; `i32::MIN` from the same sentence as `abs` |
| `div` | 3 | PLRM3 examples `3 2 div ⇒ 1.5`, `4 2 div ⇒ 2.0`; zero divisor from the entry's `undefinedresult` |
| `idiv` | 5 | PLRM3 examples `3 2 idiv ⇒ 1`, `−5 2 idiv ⇒ −2`; type and zero from the entry's sentence and error list; `i32::MIN`/−1 is **undefined** (§2) |
| `mod` | 4 | PLRM3 examples `5 3 mod ⇒ 2`, `−5 3 mod ⇒ −2`; the negative-divisor case from "The sign of the result is the same as the sign of the dividend" |
| `ceiling` | 3 | PLRM3 examples `3.2 ⇒ 4.0`, `−4.8 ⇒ −4.0`, `99 ⇒ 99` |
| `floor` | 2 | PLRM3 examples `3.2 ⇒ 3.0`, `−4.8 ⇒ −5.0` |
| `truncate` | 3 | PLRM3 examples `3.2 ⇒ 3.0`, `−4.8 ⇒ −4.0`, `99 ⇒ 99` |
| `round` | 5 | PLRM3 examples `3.2 ⇒ 3.0`, `6.5 ⇒ 7.0`, `−4.8 ⇒ −5.0`, **`−6.5 ⇒ −6.0`**, `99 ⇒ 99` |
| `cvi` | 3 | PLRM3 examples `–47.8 cvi ⇒ –47`, `520.9 cvi ⇒ 520`; the range check from "A rangecheck error occurs if a real number is too large to convert" |
| `cvr` | 2 | PLRM3 sentence "If the operand is an integer, cvr converts it to a real number"; the second case observes it through `not` |
| `atan` | 5 | PLRM3 examples `0 1 ⇒ 0.0`, `1 0 ⇒ 90.0`, `−100 0 ⇒ 270.0`, `4 4 ⇒ 45.0`; both-zero from "Either num or den may be 0, but not both" |
| `sin` | 3 | PLRM3 sentence (degrees); values are mathematics, bounds are the test's |
| `cos` | 2 | PLRM3 examples `0 cos ⇒ 1.0`, `90 cos ⇒ 0.0` |
| `sqrt` | 3 | PLRM3 sentence; the negative case from "which must be a nonnegative number" + `rangecheck` |
| `exp` | 3 | PLRM3 examples `9 0.5 exp ⇒ 3.0`, `−9 −1 exp ⇒ −0.111111`; the negative-base fractional case is **undefined** (§2) |
| `ln` | 4 | PLRM3 examples `10 ln ⇒ 2.30259`, `100 ln ⇒ 4.60517`; the non-positive cases are **our reading** of the entry's `rangecheck` (§2) |
| `log` | 3 | PLRM3 examples `10 log ⇒ 1.0`, `100 log ⇒ 2.0`; non-positive as `ln` |
| `eq` | 3 | PLRM3 sentences: "Simple objects are equal if their types and values are the same" and the integer/real coercion |
| `ne` | 1 | PLRM3 sentence, by reference to `eq` |
| `gt` | 3 | PLRM3 sentence; the boolean-operand case from "If the operands are of other types … a typecheck error occurs" |
| `ge` | 1 | PLRM3 example `4.2 4 ge ⇒ true` |
| `lt` | 1 | PLRM3 sentence; `3 4 lt` is the entry's own `if` example |
| `le` | 1 | PLRM3 sentence |
| `and` | 3 | PLRM3 truth table and example `52 7 and ⇒ 4`; the mixed-type case is **our reading** of the two operand rows |
| `or` | 2 | PLRM3 truth table and example `17 5 or ⇒ 21` |
| `xor` | 2 | PLRM3 truth table and example `12 3 xor ⇒ 15` |
| `not` | 4 | PLRM3 truth table and example `52 not ⇒ −53`; `63 not ⇒ −64` from "the bitwise complement (ones complement)"; the real-operand case is our reading of the operand rows |
| `bitshift` | 4 | PLRM3 examples `7 3 ⇒ 56`, `142 –3 ⇒ 17`; the negative-operand case from "bits shifted in are 0"; a count of 32 is **undefined** (§2) |
| `true` | 1 | PLRM3 sentence |
| `false` | 1 | PLRM3 sentence |
| `if` | 2 | PLRM3 sentence "executes proc if bool is true", lowered per the pinned decision 1 |
| `ifelse` | 2 | PLRM3's own example `4 3 lt {(TruePart)} {(FalsePart)} ifelse ⇒ (FalsePart)` |
| `copy` | 3 | ISO Annex B.5 diagram; PLRM3 example `(a) (b) (c) 0 copy ⇒ (a) (b) (c)`; negative count is our reading |
| `dup` | 1 | ISO Annex B.5 diagram; PLRM3 sentence |
| `exch` | 1 | ISO Annex B.5 diagram; PLRM3 example `1 2 exch ⇒ 2 1` |
| `index` | 3 | ISO Annex B.5 diagram; PLRM3 examples `… 0 index` and `… 3 index` |
| `pop` | 1 | ISO Annex B.5 diagram; PLRM3 example `1 2 3 pop ⇒ 1 2` |
| `roll` | 3 | ISO Annex B.5's formula `any₍ⱼ₋₁₎ mod n … any₀ anyₙ₋₁ … anyⱼ mod n`; PLRM3 sentences for the two directions — **the entry prints no numeric example**, so both directions are derived |

### The four rules that belong to no operator

An operator-by-operator corpus misses these, and both of the caller's witnesses declare
`/Domain [0 1 0 1]` and `/Range [0 1 0 1 0 1]`, so a corpus *run* misses them too.

| Rule | Cases | Source |
|---|---|---|
| Domain clip | 4 | ISO 32000-2 §7.10.1's EXAMPLE (domain `[-1 1]`, input 6 → 1, f = 3) and Table 38's `Domain` row, per input |
| Range clip | 3 | §7.10.1's second EXAMPLE (range `[0 100]`, output −14 → 0) and Table 38's `Range` row, per component |
| Output count and type | 2 | §7.10.5.3: "It shall be an error for the number of remaining operands to differ from the number of output variables specified by **Range** or for any of them to be objects other than numbers" |
| Pop from an empty stack | 1 | **No clause.** The pinned decision 6, carried as a `Report` (§2) |

### Refusal grounds

Seven, each with a program that reaches it, because "a ground nobody can reach is not a
ground". Two of the spike's six are **retired by the vocabulary**: an operator outside
Table 42 and a procedure that is not an `if`/`ifelse` operand are both unrepresentable in
`FnOp`, so the type system already refuses them and no validator is needed. That leaves
four of the spike's, and three more are added here — a backward jump, a target past the
end, and an output count that cannot match `Range`.

| Ground | Reached by running it? | Source |
|---|---|---|
| operand stack deeper than 100 | yes | §7.10.5.3's normative floor, and "it shall be an error to overflow the stack" |
| backward jump | yes | pinned decision 2; §7.10.5 cannot express a loop, so a backward jump is not a lowering of any legal function |
| jump past the end | yes | as above |
| output count cannot match `Range` | yes | §7.10.5.3, statically |
| `copy`/`index`/`roll` count not a literal | **no** | spike §6 |
| branches join at different depths | **no** | spike §6 |
| type two branches disagree about | **no** | spike §6, and Table 42's two-operators-one-name `not` |

The "no" column is the finding, not a gap: three of the seven are invisible to any single
evaluation — a count that came off the stack is perfectly resolvable *at run time*, a join
is only reached down one arm, a type ambiguity is a property of two paths rather than of
the one taken. The gate asserts that split in both directions, so a ground that moves
between the two columns fails a test rather than quietly weakening the corpus.

## 2. Everywhere the specification defines nothing

Listed explicitly, as principle 5 requires. Four are marked in the corpus as
`Expectation::Undefined` and carry **no expected value at all**; the rest are silences
that a decision or a reading covers, and each says which.

### 2.1 Marked `Undefined` — the corpus supplies no number

| Case | The silence |
|---|---|
| `bitshift/count-at-the-operand-width` | PLRM3's entry does not say what a shift of 32 or more places produces. "Bits shifted out are lost" suggests 0; **WGSL §8.7 takes the count modulo the bit width**, which returns the operand unchanged; Rust's `<<` overflows. Three plausible answers, no clause. `doc/research-function-paint-arithmetic.md` §5 already listed this as unverified |
| `exp/negative-base-fractional-exponent` | PLRM3 says only "If the exponent has a fractional part, the result is meaningful only if the base is nonnegative" — which is neither a value nor an error |
| `idiv/most-negative-over-minus-one` | The quotient 2 147 483 648 is not a 32-bit integer. The entry says "the result is an integer" and lists `undefinedresult` without saying this case raises it |
| any non-finite output | §7.3.3 defers the range of a number to the machine and states no result for an overflow; PLRM3 Appendix B defers again. The evaluator returns `Undefined` **before** the range clip, because clipping an infinity against `[0 100]` yields 100 and clipping a NaN yields 0 — a plausible colour where there is no value at all |

### 2.2 Silences a decision covers, and the decision is recorded rather than assumed

- **Precision, rounding mode and accuracy of every operator.** ISO 32000-2 states none
  anywhere and says the opposite in five places; PLRM3 defers to "the native
  floating-point representation of the underlying hardware platform" and says explicitly
  that "Not all implementations adhere to this standard". Research §1.7 states the scope
  of that reading. The corpus's consequence: every transcendental expectation is a stated
  absolute bound, and the file says in as many words that **the bound is the test's
  instrument and not a claim about the standard**.
- **A pop from an empty operand stack.** PostScript raises `stackunderflow`; ISO 32000-2
  says nothing. The pinned decision 6 takes the caller's reading — 0 — and the corpus
  carries the case with a `Report`, so the choice travels with the frame. **Two sub-gaps
  the pin does not close are in §4 below.**
- **`round`'s tie direction, in ISO's own text.** ISO Annex B says only "Round *num₁* to
  nearest integer". The tie is fixed by PLRM3, which §7.10.5.2 makes normative — so this
  is a silence in the summary and not in the requirement, and it is worth knowing which,
  because Annex B is the table most readers will reach for.
- **`atan`'s endpoint.** "the angle (in degrees between 0 and 360)". Whether 360 itself is
  attainable is not stated; the corpus keeps to the entry's four examples and does not
  test a value that would decide it. The reference evaluator folds into `[0, 360)`, which
  is a choice, and nothing in the corpus depends on it.

### 2.3 Readings that are ours, where the document lists an error but does not join it to an operand

Each of these is marked in the case's own citation. They are not silences about the
*value* — there is no value in any of them — but about *which* named error applies.

- `ln` and `log` of a non-positive number. The entries state the function and list
  `rangecheck`; unlike `sqrt`, whose body says "which must be a nonnegative number", they
  never say which operand raises it. A non-positive operand has no real logarithm and
  `rangecheck` is the only listed error that can describe it — that last step is ours.
- `div`, `idiv` and `mod` with a zero divisor: `undefinedresult` is listed; the entry does
  not name the operand.
- A negative count to `copy`, and an `index` past the bottom of the stack: the entries
  state the restriction ("a nonnegative integer n") and list the errors without joining
  them.
- A mixture of one boolean and one integer to `and`, `or` or `xor`, and a real reaching
  `not`: the operand rows admit neither, and `typecheck` is listed. Joining them is ours.

## 3. Where the clause, PLRM3, Rust and WGSL disagree

Every row is a case in the corpus. This is the list a lowering has to work through, and
the second column is what the corpus asserts.

| Case | PLRM3 (normative via §7.10.5.2) | Rust `f32`/`i32` | WGSL |
|---|---|---|---|
| `−6.5 round` | **−6.0** ("returns the greater of the two") | −7 (half away from zero) | −6 (half to even) |
| `6.5 round` | **7.0** | 7 | **6** (half to even) |
| `−8 −28 bitshift` | **15** ("bits shifted in are 0" — a *logical* right shift) | −1 (`>>` on `i32` is arithmetic) | −1 (same) |
| `1 32 bitshift` | not stated (§2.1) | overflow | operand unchanged (count taken modulo the width) |
| `5 0 idiv` | **`undefinedresult`** | panic | **the numerator** (§8.7: integer division by a runtime zero) |
| `1e20 cvi` | **`rangecheck`** | 2 147 483 647 | 2 147 483 520 (§15.7.6's clamp) |
| `−2147483648 abs` | **the real 2 147 483 648** | panic (debug) / wrap | the operand, unchanged |
| `2147483647 1 add` | **the real 2 147 483 648** | wrap or panic | wrap |
| `0 0 atan` | **`undefinedresult`** | 0.0 (`atan2(0,0)`) | undefined |
| `−1 sqrt` | **`rangecheck`** | NaN | an *indeterminate value of type `f32`* — an arbitrary colour |
| `4 2 div` | **the real 2.0**, never the integer 2 | — | — |
| `atan` result range | **0..360 degrees** | `atan2` gives −π..π radians | same |
| `sin`, `cos` operand | **degrees** | radians | radians |

Two more, about the caller's evaluator rather than about a language. They are in the
corpus because the *clause* says so, not because the other implementation is wrong-by-
comparison; research §3.3 raised both and they belong to that tree.

| Case | PLRM3 | The caller's `pdf-model` evaluator |
|---|---|---|
| `63 not` | **−64** (ones complement on an integer) | 0.0 — its compiled form carries no type on a literal, so every `not` is the boolean one |
| `0.5` vs the next float above it, `eq` | **false** — the entry describes equality of values and names no tolerance | true — it compares with `(a − b).abs() < f32::EPSILON` |

## 4. Objections to the pinned vocabulary

Four, in descending order of how much they cost to fix later.

### 4.1 `Paint::Function(Arc<FunctionPaint>)` takes `Copy` off `Paint`, and the vocabulary already has a better shape for this

`Paint` derives `Copy` today and is passed by value throughout both crates —
`Paint::is_valid(self)`, thirteen `Paint::` sites in `quorra-gpu`, `Command` variants that
are themselves `Copy`. `Arc<FunctionPaint>` is not `Copy`, so the pinned fourth variant
removes the derive and every one of those sites changes. **I did not add the variant in
this worktree for that reason** — it would have broken the workspace build, and the task
that owns the churn is the vocabulary sibling's, not this one's. `crates/quorra-scene/src/function.rs`
is written as pinned, minus the `paint.rs` line.

More than the churn, though: **every other heavy paint in this vocabulary is a resource
id.** `Paint::Shading` carries a `RampId`, `Paint::Mesh` carries a `MeshId`, and
`ids.rs` exists so that a scene names an uploaded resource rather than carrying it. A
`FunctionId` would keep `Paint: Copy`, keep the scene's "cheap to clone" property
trivially true, and match ADR 0053's decision to cache the generated shader *by the
program's hash* — because the upload is exactly where that hash is computed, once, rather
than per frame per command.

The argument for the `Arc` is that a scene stays self-describing and needs no device
round trip before it can be built, which §2.3 cares about. Both are defensible; what is
not defensible is taking the `Arc` without noticing that it changes `Paint`'s shape. This
is a decision for the integrator, and it should be made before either sibling's `paint.rs`
lands.

### 4.2 `FnOp` cannot derive `Hash`, and ADR 0053 caches shaders by the program's hash

`PushReal(f32)` makes `Hash` underivable and `Eq` underivable, and `Arc<[FnOp]>` inherits
both absences. ADR 0053's decision is "a generated shader cached by the program's hash" —
so the generator will need a hand-written hash over the bit patterns (`f32::to_bits`),
with the two NaN and the two zero questions answered deliberately: `PushReal(0.0)` and
`PushReal(-0.0)` are `PartialEq`-equal but have different bits, and a hash that disagrees
with the equality it accompanies is a cache that occasionally misses forever.

This is not an argument against the pin — `PartialEq` is right for the type — but the
consequence should be written down before the generator meets it, because the failure mode
is a silent cache miss rather than a compile error.

### 4.3 Decision 6 does not say what *type* the 0 is, and the type is observable

"A pop from an empty stack yields 0" leaves open whether that is the integer 0 or the real
0.0, and Table 42 can tell them apart: `not`, `and`, `or`, `xor`, `idiv`, `mod` and
`bitshift` all accept an integer and raise `typecheck` on a real. So `1 and` on a
one-element stack is 1 under one reading and an error under the other.

**The reference evaluator chooses the integer**, and the argument is that a bare `0` in a
PostScript program scans as an integer, and an integer coerces into every numeric context
where a real does not — so the integer is the choice that makes the *fewest* programs
fail. It is recorded in `Stack::pop`'s own doc comment as a choice rather than a reading.
It should be in the pin, and it is a question for the caller: their evaluator's
`stack.pop().unwrap_or(0.0)` is float-only and cannot express the distinction, so they have
not answered it either.

### 4.4 Decision 6's scope stops at `pop`, and `copy`/`index`/`roll` are not pops

They read a *region* of the stack rather than one value. The corpus and the evaluator take
the narrow reading — the zero rule applies to popping, and `copy`/`roll` with insufficient
depth raise `stackunderflow`, an `index` past the bottom `rangecheck` — because inventing
operands for a rotation is a second invention on top of the first. The pin should say which
way it goes; nothing in either witness reaches it, which is exactly why it will be decided
by whoever writes the code if it is not decided here.

### 4.5 Two smaller things, offered rather than argued

- **`FnRange` has no validity check.** Table 38 requires `Range₂ⱼ ≤ Range₂ⱼ₊₁`, and
  nothing in the pinned type enforces or tests it, where `Color::is_valid` and
  `ShadingKind::is_valid` set the pattern for exactly this. A range whose bounds are
  inverted makes the clip a function that returns the *upper* bound for every input, which
  is a plausible-looking wrong page. The same applies to `domain`. This belongs to the
  validation sibling; I mention it because the corpus cannot test what the type does not
  express.
- **`JumpUnless` should say what a non-boolean condition does.** The pin says "Pop a
  boolean"; PLRM3's `if` lists `typecheck`, and the evaluator raises it. Worth one sentence
  in the pin so that the analyser and the generator do not have to each decide.

Otherwise the pin is right, and two parts of it are load-bearing in ways worth naming:
separating `PushInt` from `PushReal` is what makes `63 not ⇒ −64` expressible at all, and
`FnRange` carrying the component count is what makes §7.10.5.3's count rule checkable
without a second field that can disagree with it.

## 5. What this corpus does not do

- **It does not run on a device.** Every case carries `Case::two_input_program`, which
  adapts it to the two-input shape a `FunctionPaint` has by prepending `Pop`s *and moving
  every jump target*, so a later wave can iterate the corpus and render each case. Two
  tests cover the adaptor, because an off-by-two jump target still runs and still draws
  something.
- **It does not check the refusals against a validator**, because there is no analyser yet.
  It states each ground, demonstrates a program that reaches it, and asserts which of the
  seven a single evaluation can and cannot see. When the analyser lands, the three "no"
  rows in §1 are its acceptance test.
- **It does not test types 0, 2 or 3.** ADR 0053 is type 4 only.
- **It asserts nothing about agreement between two adapters.** ADR 0053 already answered
  that with measurements, and the answer was no.
