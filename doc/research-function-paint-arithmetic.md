# The arithmetic of a device-evaluated PDF function

Status: **research record**, written 2026-08-15. It answers one question and only that
one: `/home/cl/projects/pdf-viewer/doc/QUORRA_FUNCTION_PAINT.md` §5.1, "which of these
three can you live with". It contains no code and no measurement — a sibling spike owns
the performance half of the same subject, and nothing here depends on its answer.

Throughout, three kinds of statement are kept apart on purpose:

- **Quotation.** Inside quotation marks or a blockquote, under its clause number,
  verbatim. Every ISO 32000-2 and PLRM3 quotation below was read from the PDF page named
  beside it, not from an extracted text form. Every WGSL quotation was read from the
  published specification, cited in §2.0.
- **Paraphrase.** Prose without quotation marks that restates a clause. It says which
  clause.
- **Our inference.** Marked **[inference]**. These are ours, and a reader may disagree
  with them without disagreeing with the clause above them.

## 0. The short answer

**Their option 2** — the processor keeps evaluating for the oracle, the device evaluates
for the screen — is the one we can live with, and it is the only one of the three that is
*true* in the sense principle 6 uses the word. Option 3 ("the shared operators are
specified to the bit") is not purchasable: §2 shows that WGSL declines to specify, by
design, exactly the operations that contract would have to cover. Option 1 (a gate that
tolerates some differing pixels along discontinuities) rests on a bound the
specification does not supply and a §7.10.5 program can violate arbitrarily; §3.4 shows
why the differing set is not a boundary.

§4 states the recommendation with four amendments, one of which — classifying a program
as continuous or not, and gating the two differently — is the part we would ask them to
take seriously, because for a continuous program a bounded gate *is* meaningful and is in
the currency ISO 32000-2 §10.7.3 already uses.

## 1. What ISO 32000-2 requires of a function's arithmetic

### 1.1 What a function is: an idealisation, stated as one

ISO 32000-2:2020 §7.10.1 (PDF page 123):

> Functions in PDF represent static, self-contained numerical transformations.

and, in the same clause:

> In PDF functions, all the input values and all the output values shall be numbers, and
> functions shall have no side effects.

> Each function definition includes a domain, the set of valid values for the input. Some
> types of functions also define a range, the set of valid values for the output. Input
> values passed to the function shall be clipped to the domain, and output values produced
> by the function shall be clipped to the range.

Paraphrase: the clause defines a function mathematically — a map from numbers to numbers
— and normatively requires two clamps around it. It says nothing about how the numbers
between those clamps are represented.

**[inference]** The two clamps are the only part of a function's evaluation that
ISO 32000-2 pins exactly, and they are cheap to agree on: both are `min`/`max` against
constants carried in the paint, and §2's accuracy table gives `min`, `max` and the
comparison operators "correctly rounded" and "correct result" respectively. Whatever else
disagrees, the domain and range clamps will not.

### 1.2 The one clause about number representation, and it is a deferral

ISO 32000-2:2020 §7.3.3, "Numeric objects" (PDF page 24), the third sentence:

> The range and precision of numbers may be limited by the internal representations used
> in the computer on which the PDF processor is running; Annex C, "Advice on maximising
> portability", gives these limits for typical implementations.

Annex C is titled, on its own first page, **"Annex C (informative) Advice on maximising
portability"**. Its Table C.1, row "Real numbers" (PDF page 851), reads in full:

> Modern computers often represent and process real numbers using IEEE Standard for
> Floating-Point Arithmetic (IEEE 754) single or double precision.

Paraphrase: an informative annex observes what computers often do. "Often" is not a
requirement, "single or double" is not a choice the annex makes, and the annex containing
it is not normative.

**IEEE 754 appears exactly twice in ISO 32000-2:2020.** Once in that informative
Annex C row, and once in the Bibliography, as `IEEE 754-2019`. It is **not** in clause 2,
"Normative references". We checked the whole of clause 2.

### 1.3 Type 4's semantics are normatively delegated, and to a document that defers again

ISO 32000-2:2020 §7.10.5.1 (PDF page 130), complete:

> A Type 4 function (*PDF 1.3*), also called a PostScript calculator function, shall be
> represented as a stream containing code written in a small subset of the PostScript
> language. This subset is comprised of the following PostScript language features:
>
> - Expressions involving only integers, real numbers, and boolean values
> - Comments
> - No composite data structures (such as strings or arrays)
> - No procedures
> - No variables or names

That is the whole of §7.10.5.1's restriction on the language, as the task asked us to
check: five bullets about *what may be written*, none about *how it is computed*.

ISO 32000-2:2020 §7.10.5.2 (PDF page 131), first paragraph:

> "Table 42 — Operators in Type 4 functions" lists the operators that can be used in this
> type of function. The PostScript Language Reference, Third Edition shall define the
> semantics of these operators and all other syntax rules of the PostScript language.
> Although the semantics are those of the corresponding PostScript language operators, a
> full PostScript language compatible interpreter is not required.

Two things follow, and they pull in opposite directions.

- `shall define` makes PLRM3 normative for operator semantics, and clause 2 carries it:
  the entry reads "PostScript Language Third Edition , (February, 1999), Adobe Systems
  Incorporated". (The entry's title is set without the word "Reference"; the referenced
  document's own title page reads "PostScript Language Reference, third edition", Adobe
  Systems Incorporated, Addison-Wesley, 1999. We take them to be the same document.)
- "a full PostScript language compatible interpreter is not required" is a licence about
  *coverage*, not about *arithmetic*. **[inference]** It excuses a processor from
  implementing the rest of PostScript; nothing in the sentence excuses it from the
  semantics of the 42 operators, which the sentence before makes normative.

Table 42 itself (PDF page 131) lists 42 operators, verified by counting the table's
cells: 21 arithmetic (`abs add atan ceiling cos cvi cvr div exp floor idiv ln log mod mul
neg round sin sqrt sub truncate`), 13 relational/boolean/bitwise (`and bitshift eq false
ge gt le lt ne not or true xor`), 2 conditional (`if ifelse`), 6 stack (`copy dup exch
index pop roll`). The caller's count of 42 is right.

### 1.4 What PLRM3 says about precision: the same deferral, one level down

PLRM3 §3.3.3, "Integer and Real Objects" (PLRM PDF page 38):

> The range and precision of numbers is limited by the internal representations used in
> the machine on which the PostScript interpreter is running. Appendix B gives these
> limits for typical implementations of the PostScript interpreter.

PLRM3 Appendix B, §B.1 "Typical Limits" (PLRM PDF page 738), third paragraph, complete:

> Table B.1 shows the typical architectural limits for most PostScript interpreters
> running on 32-bit machines. Although these limits are likely to remain constant across a
> wide variety of platforms, they do not necessarily apply to all PostScript
> implementations. In particular, the limits for real numbers in any implementation are
> those imposed by the native floating-point representation of the underlying hardware
> platform. The real-number limits shown in the table are based on the IEEE 754 standard
> for normalized single-precision floating-point arithmetic. (See the Bibliography for a
> reference to this document.) Not all implementations adhere to this standard, however;
> see product documentation for the exact limits in a particular implementation.

Table B.1's `real` rows give ±10³⁸, ±10⁻³⁸ and "8 — Significant decimal digits of
precision (approximate)".

Paraphrase: the normatively referenced document, asked what precision an operator has,
answers "whatever the hardware has", says its own table is typical rather than required,
and says explicitly that not all implementations adhere to IEEE 754.

**This is the answer to the task's first question.** ISO 32000-2 states no precision, no
rounding rule and no accuracy requirement for the evaluation of a PDF function; it defers
to the machine, and the document it defers to for Type 4's semantics defers to the machine
again. Two documents, two deferrals, no number.

### 1.5 The only accuracy requirement in the whole subject, and its currency is not the last bit

There *is* an accuracy requirement in this neighbourhood — the task's instruction to read
around the subject before recording a silence is what found it — and it is about sampling,
not about arithmetic.

ISO 32000-2:2020 §8.7.4.4 (PDF page 231), opening paragraph:

> Conceptually, a shading determines a colour value for each individual point within the
> area to be painted. In practice, however, PDF processors may actually compute colour
> values only for some subset of the points in the target area, with the colours of the
> intervening points determined by interpolation between the ones computed. PDF processors
> are free to use this strategy as long as the interpolated colour values approximate
> those defined by the shading to within the smoothness tolerance specified in the graphics
> state (see 10.7.3, "Smoothness tolerance").

ISO 32000-2:2020 §10.7.3 (PDF page 383):

> Smoothness is the allowable colour error between a shading approximated by piecewise
> linear interpolation and the true value of a (possibly nonlinear) shading function. The
> error shall be measured for each colour component, and the maximum independent error
> shall be used. The allowable error (or tolerance) shall be expressed as a fraction of the
> range of the colour component, from 0.0 to 1.0.

and, in the same clause:

> Each output device may have internal limits on the maximum and minimum tolerances
> attainable.

**[inference]** Three things follow.

1. The standard's unit of accuracy for a shading is *a fraction of a colour component's
   range*, not a ULP of an intermediate. A design that argues about last bits is arguing
   in a currency the standard never uses.
2. §8.7.4.4's licence is about **not evaluating at every point**. It permits us to
   evaluate on a coarser grid and interpolate; it does not, on its face, permit us to
   evaluate *differently*. But it establishes that the standard already contemplates a
   processor's picture differing from the ideal by a stated tolerance — which is the
   shape of the answer §4 gives.
3. The tolerance is worthless at a discontinuity, and the standard knows the
   discontinuity is there. §8.7.4.5.2 (PDF page 233): "In Type 1 (function-based)
   shadings, the colour at every point in the domain is defined by a specified
   mathematical function. **The function need not be smooth or continuous.**" A
   piecewise-linear approximation across a jump of size *J* has error *J*/2 at the
   midpoint, so no tolerance below *J*/2 can be met by interpolation at all. At a
   discontinuity, §8.7.4.4's licence is not available and §10.7.3's tolerance is not a
   bound anyone can honour — which is precisely the situation the caller's ADR 0339 walked
   into and the reason they moved to the device grid.

### 1.6 Where the standard says the opposite — that a processor may choose

Collected, because a reader deciding this question deserves them in one place. All are
quotations, cited above except the last two:

- §7.3.3: "The range and precision of numbers **may be limited by the internal
  representations used in the computer on which the PDF processor is running**".
- §10.7.3: "Each output device **may have internal limits** on the maximum and minimum
  tolerances attainable."
- §8.7.4.4: "PDF processors **are free to use this strategy** as long as …".
- §8.7.4.4 (PDF page 232), on device colour spaces: "Thus, shadings defined with device
  colour spaces may have colour gradient fills that are less accurate and somewhat
  **device-dependent**." (NOTE 3 immediately after excludes shadings with a `Function`
  entry from that particular rule.)
- §7.10.5.2: "a full PostScript language compatible interpreter **is not required**".

### 1.7 The silence, stated with its scope

We read, in the PDF: §7.3.3 in full; §7.10.1 through §7.10.5.3 in full, including
Tables 38–42; §8.7.4.1 through §8.7.4.5.2 in full, including Tables 75–78; §10.7.3 and
§10.7.4 in full; Annex B (informative) in full; Annex C (informative) in full; and
clause 2, "Normative references", in full.

**ISO 32000-2:2020 nowhere states a precision, a rounding rule, or an accuracy
requirement for the evaluation of a PDF function of any type.** It states the opposite in
five places (§1.6). The one accuracy requirement in the neighbourhood (§10.7.3) is about
approximating a shading by interpolation, is measured in colour-component fractions, and
is explicitly subject to a device's own internal limits.

Two corroborations, neither of them a source of truth:

- **Errata.** `/home/cl/projects/pdf-viewer/doc/errata-read.md` records a verdict for
  every one of the 120 distinct passages that tree's `spec-errata check` names in Errata
  Collection 3. It has **no row for §7.3.3, §7.10.x or §8.7.4.x**. The sponsored PDF
  prints a "Goto errata" badge beside §7.3.3's heading; we did not follow that link (it
  leaves this tree), so we cannot say what it points at — only that nothing in that tree's
  completed errata reading bears on function arithmetic.
- **Arlington.** `/home/cl/projects/pdf-viewer/doc/arlington-pdf-model/tsv/latest/`
  contains `FunctionType0/2/3/4.tsv` and `ShadingType1.tsv`. They are a structural model:
  key, type, version, required, default, links. Nothing in the model is about arithmetic,
  and nothing could be — it describes dictionaries, not evaluation.

### 1.8 What the standard *does* require that bears on this design

Three requirements survive the silence, and all three are exactly implementable on both
sides:

- **Domain and range clipping** (§7.10.1 and Table 38, quoted in §1.1). Exact `min`/`max`.
- **Range adjustment at the shading** — §8.7.4.5.2, Table 78, `Function` row (PDF page
  233): "If the value returned by the function for a given colour component is out of
  range, it shall be adjusted to the nearest valid value."
- **A domain exit is an error the standard contemplates.** §8.7.4.5.2 (PDF page 233):
  "If the function is undefined at any point within the declared domain rectangle, an
  error may occur, even if the corresponding transformed point falls outside the shading's
  bounding box."

**[inference]** That last sentence is the hook principle 6 needs. When a program divides
by zero or takes the square root of a negative, the standard's own word is *error* — not
"substitute something plausible". §2.4 shows that a device that does nothing produces an
*indeterminate value* instead, which is the plausible lie in its purest form.

## 2. What WGSL guarantees

### 2.0 The source

W3C Candidate Recommendation, *WebGPU Shading Language*, <https://www.w3.org/TR/WGSL/>,
retrieved 2026-08-15; sections §15.7 "Floating Point Evaluation", §15.7.4.1 "Accuracy of
Concrete Floating Point Expressions", §15.7.5 "Reassociation and Fusion", §15.7.6
"Floating Point Conversion", and the built-in definitions in §17.5.

### 2.1 The base: IEEE 754 minus the parts a GPU declines to pay for

WGSL §15.7:

> WGSL floating point features are based on the IEEE-754 standard for floating point, but
> with reduced functionality reflecting the compromises made by GPUs, and with some
> additional guardrails for portability.

WGSL §15.7.1 confirms `f32` is IEEE-754 binary32 ("Recall that the f32 WGSL type
corresponds to the IEEE-754 binary32 format"). WGSL §15.7.2, "Differences from IEEE-754",
lists what is given up. The load-bearing ones here:

> No rounding mode is specified. An implementation may round an intermediate result up or
> down.

> To flush to zero is to replace a subnormal value for a floating point type with a zero
> value of that type. Any inputs or outputs of operations listed in § 15.7.4 Floating Point
> Accuracy may be flushed to zero.

> Implementations may ignore the sign field of a floating point zero value. That is, a
> zero with a positive sign may behave like a zero a with a negative sign, and vice versa.

And WGSL §15.7.4, on what "correctly rounded" means there:

> That is, the result may be rounded up or down: WGSL does not specify a rounding mode.

**[inference]** "Correctly rounded" in WGSL is therefore weaker than IEEE 754's
`roundTiesToEven`: it permits either neighbour when the exact result is not representable.
For `+`, `-`, `*` this is a distinction without a difference in practice — every shipping
GPU rounds to nearest-even — but it is a distinction the specification insists on, and a
contract written on WGSL's guarantees cannot promise the tie.

### 2.2 The accuracy table, for the operations a PDF function needs

Verbatim rows from WGSL §15.7.4.1, f32 column:

| WGSL operation | required accuracy for f32 (verbatim) |
|---|---|
| `x + y`, `x - y`, `x * y`, `-x` | "Correctly rounded" |
| `x / y` | "2.5 ULP for \|y\| in the range [2⁻¹²⁶, 2¹²⁶]" |
| `x % y` | "Inherited from x - y * trunc(x/y)" |
| `x == y`, `!=`, `<`, `<=`, `>`, `>=` | "Correct result" |
| `abs(x)` | "Correctly rounded" |
| `floor(x)`, `ceil(x)`, `trunc(x)`, `round(x)`, `sign(x)`, `step`, `saturate` | "Correctly rounded" |
| `min(x, y)`, `max(x, y)` | "Correctly rounded. / If both x and y are subnormal, the result may be either input." |
| `sqrt(x)` | "Inherited from 1.0 / inverseSqrt(x)" |
| `inverseSqrt(x)` | "2 ULP" |
| `sin(x)`, `cos(x)` | "Absolute error at most 2⁻¹¹ when x is in the interval [-π, π]" |
| `atan(x)` | "4096 ULP" |
| `atan2(y, x)` | "4096 ULP for \|x\| in the range [2⁻¹²⁶, 2¹²⁶], and y is finite and normal" |
| `exp(x)`, `exp2(x)` | "3 + 2 * \|x\| ULP" |
| `log(x)`, `log2(x)` | "Absolute error at most 2⁻²¹ when x is in the interval [0.5, 2.0]. / 3 ULP when x is outside the interval [0.5, 2.0]." |
| `pow(x, y)` | "Inherited from exp2(y * log2(x))" |
| `radians(x)` | "Inherited from x * 0.017453292519943295474" |
| `degrees(x)` | "Inherited from x * 57.295779513082322865" |
| `mix(x, y, z)` | "Inherited from x * (1.0 - z) + y * z" |

Two sentences from §15.7.4 that decide how much those rows are worth:

> When the accuracy for an operation is specified over an input range, the accuracy is
> undefined for input values outside that range.

> An expression that the accuracy is inherited from. That is, the accuracy of the
> operation is defined as the accuracy of evaluating the given WGSL expression. The given
> expression is only one valid implementation of the function. […] A WebGPU implementation
> may implement the operation differently, with better accuracy or with greater tolerance
> for extreme inputs.

Four consequences, all **[inference]** from the rows above:

1. **`/` and `sqrt` are not correctly rounded in WGSL, and are in IEEE 754.** Division is
   2.5 ULP; `sqrt` is inherited from `1.0 / inverseSqrt(x)`, i.e. a 2 ULP reciprocal square
   root followed by a 2.5 ULP division. A host `f32` `/` and `f32::sqrt` are each correctly
   rounded because IEEE 754 requires it. So two operators the caller did not list are
   already outside bit agreement, before a single transcendental is reached.
2. **`sin` and `cos` have *no* specified accuracy for a PDF program's typical argument.**
   PDF's `sin`/`cos` take degrees (PLRM3, §3.2 below); WGSL's take radians. The guarantee
   covers [−π, π] radians, i.e. ±180°. The standard's own worked example — §7.10.5.3's
   `DoubleDot` spot function, `{360 mul sin 2 div exch 360 mul sin 2 div add}` — evaluates
   `sin` at arguments up to ±360°, i.e. ±2π radians, entirely outside the guaranteed range.
   Argument reduction is ours to do, and doing it costs accuracy of its own.
3. **`atan` is the worst row in the table by three orders of magnitude.** 4096 ULP on a
   result of magnitude ≈ 1 is ≈ 4096 × 2⁻²³ ≈ 4.9 × 10⁻⁴ radians ≈ 0.028°. A host `atan2`
   is typically within 1 ULP. The caller was right to name `atan`, and it is not a last-bit
   problem: it is a third of a hundredth of a degree, which a `ge` against a
   document-supplied threshold notices.
4. **`exp` cannot be `pow`.** PostScript's `exp` is `base exponent exp`; PLRM3 gives
   `−9 −1 exp ⇒ −0.111111`, a negative base. WGSL's `pow` is inherited from
   `exp2(y * log2(x))`, undefined for negative `x`. So `exp` must be built by hand from a
   sign/parity case split over `exp2` and `log2`, and its error is the composition of two
   loose rows.

### 2.3 Reassociation and fusion: the correctly-rounded rows are conditional too

WGSL §15.7.5, complete on the point:

> An implementation may reassociate operations.

> An implementation may fuse operations if the transformed expression is at least as
> accurate as the original formulation. For example, some fused multiply-add
> implementations can be more accurate than performing a multiply followed by an addition.

**[inference]** This is the single most important paragraph in §2 for the caller's §4
sketch, and it points the opposite way to intuition. The caller offered us two lowerings
and said the choice was ours: a switch inside a loop over the instruction list, or a
generated shader per distinct program. The generated shader is the one the compiler can
see through — a straight-line expression tree it may reassociate and fuse at will — so
`add`/`sub`/`mul`, the three operators that *are* correctly rounded, stop being
reproducible exactly in the lowering that is otherwise faster. An interpreter loop, whose
operands come out of a mutable value stack under data-dependent indices, denies the
compiler most of that. **The arithmetic argument and the startup argument point the same
way**, which is worth knowing before the performance spike reports.

### 2.4 What happens when a program leaves a domain

WGSL §15.7.2, the "Finite Math Assumption":

> Implementations may assume that overflow, infinities, and NaNs are not present during
> shader execution.

> In such an implementation, if the intermediate result of evaluating a runtime expression
> overflows, or yields an infinity or a NaN, the final result will be an indeterminate
> value of the target type.

And, from §15.7.2's preamble:

> No floating point exceptions are generated.

**[inference]** An unguarded `1.0 / 0.0`, `sqrt(-1.0)`, `log(0.0)` or an overflowing
`exp2` in a lowered PDF function does not produce a NaN we can detect and refuse. It
produces *an indeterminate value of type f32* — an arbitrary colour, chosen by the driver,
that looks exactly like a colour. Against principle 6 and §1.8's "an error may occur",
this settles the design: every domain exit in a lowered program is guarded explicitly
before the operation, or the paint is refused.

Integer operations, by contrast, are pinned. WGSL §8.7 makes signed integer division by a
runtime zero evaluate to the numerator, and shifts take the shift amount modulo the bit
width. Float-to-integer conversion is exactly specified in §15.7.6:

> If X is a NaN, the result is an indeterminate value in T. If X is exactly representable
> in the target type T, then the result is that value. Otherwise, the result is the value
> in T closest to truncate(X) and also exactly representable in the original floating point
> type.

with the note "for non-NaN cases, floating point to integer conversion clamps the value to
be within the range of the target type, then rounds toward zero" and the worked example
"1e20f converted to i32 is the maximum i32 value, 2147483520i".

**[inference]** That clamp differs from Rust's. We ran it: `1e20f32 as i32` in Rust is
`2147483647`, WGSL's is `2147483520`. The two agree for every value below 2³¹ and disagree
above it. A `cvi` in a float-only evaluator never reaches that (it is a `trunc`), but a
`bitshift`, `idiv`, `mod` or `and` whose operand came from a wild coordinate does.

### 2.5 The answer to "is option 3 purchasable"

**No.** "Specified to the bit" would require, at minimum, that `add`, `sub`, `mul`, `div`
and `sqrt` be correctly rounded with a fixed tie rule and that every transcendental have a
stated exact reference. WGSL specifies no rounding mode at all, gives `div` 2.5 ULP,
defines `sqrt` by composition, gives `atan` 4096 ULP, gives `sin`/`cos` an absolute error
bound over a range that PDF programs routinely exceed, permits subnormal flushing on every
one of them, permits reassociation and fusion of the exact ones, and permits an
implementation to substitute a different implementation entirely. Every one of those is a
deliberate concession to what GPUs do, not an oversight that a stricter caller can opt out
of.

We could *emulate* correctly-rounded arithmetic in WGSL — software `sqrt`, a polynomial
`atan` with a proved bound, Payne–Hanek reduction for `sin` — and the result would be a
software floating-point library running per fragment, which is the thing this whole ask
exists to stop doing on the processor. **[inference]** A contract we can only honour by
being slower than the alternative is not a contract; it is a refusal with extra steps.

## 3. Which of Table 42's operators are dangerous

### 3.1 The three categories

- **Exact** — both sides are *required* by their own specifications to produce the same
  bits from the same inputs, subject to the caveats named.
- **Inexact** — the two sides may differ, by a bounded amount, on the same inputs.
- **Amplifier** — the operator is itself exact, but is discontinuous, so an *upstream*
  inexactness of one ULP becomes an output difference of order 1.

**[inference]** No operator is dangerous on its own. `ge` is exact; so is `truncate`. The
danger is always a composition: an inexact operator upstream of an amplifier. This is the
structural point §3.4 turns into the recommendation.

### 3.2 The table

PLRM3 semantics below are quotations from the operator's own entry in PLRM3 chapter 8,
read from the PDF. Accuracy claims are from WGSL §15.7.4.1 (§2.2 above).

| Operator | PLRM3 semantics (verbatim where quoted) | Category | Why |
|---|---|---|---|
| `add` `sub` `mul` `neg` | sum / difference / product / negative | **Exact**, conditionally | "Correctly rounded" on both sides — *unless* the lowering lets WGSL §15.7.5 reassociate or fuse. See §2.3. |
| `abs` | absolute value | **Exact** | "Correctly rounded"; sign clearing. |
| `div` | "divides num1 by num2, producing a result that is always a real number even if both operands are integers"; error `undefinedresult` on zero divisor | **Inexact** | WGSL 2.5 ULP vs IEEE 754 correctly rounded. **Not on the caller's list and should be.** Zero divisor is a guarded domain exit. |
| `sqrt` | "returns the square root of num, which must be a nonnegative number"; error `rangecheck` | **Inexact** | WGSL "inherited from 1.0 / inverseSqrt(x)" ≈ 4.5 ULP vs IEEE 754 correctly rounded. **Not on the caller's list and should be.** Negative operand is a guarded domain exit. |
| `atan` | "returns the angle (in degrees between 0 and 360) whose tangent is num divided by den. Either num or den may be 0, but not both." | **Inexact, worst** | WGSL `atan2` is 4096 ULP ≈ 0.028° after conversion to degrees. `0 0 atan` is `undefinedresult` — a guarded exit that Rust's `atan2(0.0, 0.0)` silently returns 0.0 for. |
| `sin` `cos` | "returns the sine of angle, which is interpreted as an angle in degrees" | **Inexact, unbounded outside ±180°** | Absolute error 2⁻¹¹ inside [−π, π] radians; **accuracy undefined outside**, and §7.10.5.3's own `DoubleDot` example evaluates at ±360°. Argument reduction is ours and costs more error. |
| `exp` | "raises base to the exponent power … If the exponent has a fractional part, the result is meaningful only if the base is nonnegative." | **Inexact** | Must be composed from `exp2`/`log2` with a sign/parity case split; WGSL's `pow` is undefined for negative base and PLRM3 defines `−9 −1 exp ⇒ −0.111111`. |
| `ln` | "returns the natural logarithm (base e) of num"; error `rangecheck` | **Inexact** | Absolute 2⁻²¹ in [0.5, 2], 3 ULP outside. Non-positive operand is a guarded exit. |
| `log` | "returns the common logarithm (base 10) of num"; error `rangecheck` | **Inexact** | As `ln`, plus one multiply by 1/ln 10 — one more rounding than the host's `log10`, which many libms compute directly. |
| `ceiling` `floor` `truncate` | "truncates num1 toward 0 by removing its fractional part" | **Amplifier** | "Correctly rounded" in WGSL, exact in Rust. The operator agrees; it turns a 1-ULP upstream disagreement at an integer boundary into a difference of 1. The caller's instinct to list `truncate` is right, for this reason rather than the stated one. |
| `cvi` | "If the operand is a real number, it truncates any fractional part (that is, rounds it toward 0) and converts it to an integer. […] A rangecheck error occurs if a real number is too large to convert to an integer." | **Amplifier** | Same as `truncate` in a float-only evaluator. If a lowering materialises an `i32`, WGSL's clamp differs from Rust's above 2³¹ (§2.4). |
| `round` | "returns the integer value nearest to num1. **If num1 is equally close to its two nearest integers, round returns the greater of the two.**" Examples include `−6.5 round ⇒ −6.0`. | **Amplifier, and a three-way disagreement** | PLRM3 requires half-toward-greater. WGSL's `round` is half-to-even ("the result is k when k is even, and k + 1 when k is odd"). Rust's `f32::round` is half-away-from-zero — we ran it: `-6.5f32.round()` is `-7`. All three differ on `−6.5`. Both sides must spell PLRM3's rule out; neither built-in is it. |
| `cvr` | "(convert to real)" | **Exact** | A no-op in a float-only representation. |
| `idiv` `mod` | "Both operands of idiv must be integers"; `mod`'s "sign of the result is the same as the sign of the dividend"; both `undefinedresult` on zero | **Exact** | Integer arithmetic, exact on both sides. Zero divisor is a guarded exit — note that WGSL's runtime integer division by zero yields the numerator, which is *not* an error and *not* what PLRM3 says. |
| `and` `or` `xor` `not` `bitshift` | logical on booleans, bitwise on integers; `bitshift` "shifts the binary representation of int1 left by shift bits and returns the result. Bits shifted out are lost; bits shifted in are 0. If shift is negative, a right shift by –shift bits is performed. […] Both int1 and shift must be integers." | **Exact** | Integer/boolean, exact on both sides once the float→int conversion is pinned (§2.4). WGSL takes the shift amount modulo the bit width; PLRM3's entry does not state what happens past the operand's width. |
| `eq` `ne` | "Simple objects are equal if their types and values are the same." […] "an integer and a real number representing the same mathematical value are considered equal by **eq**" | **Exact, and an amplifier** | Exact comparisons on both sides ("Correct result"). They amplify. **Separately: the oracle deviates here** — see §3.3. |
| `ge` `gt` `le` `lt` | test greater/less | **Exact, and the principal amplifier** | "Correct result" in WGSL, exact in Rust. Every one of them turns an upstream ULP into a whole colour. The caller named `ge`; all six are the same case. |
| `true` `false` | push a boolean | **Exact** | — |
| `if` `ifelse` | "Execute expr if bool is true" | **Exact, and the amplifier that matters** | The branch itself is exact. It converts an upstream disagreement into a different program path. |
| `copy` `dup` `exch` `index` `pop` `roll` | stack manipulation | **Exact**, with one catch | No arithmetic. But `copy`, `index` and `roll` take integer counts *off the value stack*, so a count computed by arithmetic makes them control-flow amplifiers too. |

### 3.3 Two observations about the oracle, offered rather than asserted

Both are consequences of §7.10.5.2 making PLRM3 normative. Neither is about the GPU, and
we raise them because principle 5 says a disagreement sends us to the clause:

- **`round`.** `pdf-model`'s evaluator uses Rust's `f32::round` (half-away-from-zero).
  PLRM3's entry requires half-toward-greater and prints `−6.5 round ⇒ −6.0` as an example.
  They disagree for every negative half-integer.
- **`eq` / `ne`.** The evaluator compares with `(a - b).abs() < f32::EPSILON`. PLRM3's
  `eq` entry describes exact equality of values, with numeric type coercion between
  integer and real; it does not describe a tolerance. An epsilon-equality also is not
  transitive, and `f32::EPSILON` is an absolute constant, so the tolerance is meaningless
  for operands of large magnitude and enormous for operands near zero.

We have not filed these; they belong to the caller's tree and to a reading of PLRM3 they
should make themselves. They matter here because a device lowering must match *something*,
and matching the oracle's current `round` would be curve-fitting to another
implementation, which principle 5 forbids outright.

### 3.4 The finding that decides §4

**[inference]** Put §3.1's categories together with the fact that a §7.10.5 program is an
unrestricted stack machine over 42 operators:

> Any inexact operator anywhere upstream of any comparison, branch or truncation makes the
> whole program's output arbitrary at the pixels where the two evaluations land on
> different sides.

There is no locality in that statement. The set of pixels where a lowered program
disagrees with the oracle is the preimage, under the program, of a set whose measure the
program itself decides. For `{ x atan 45 ge { … } { … } ifelse }` it is a thin curve. For a
program that thresholds `sin` of a high-frequency argument — and the caller's own witness
document is a seven-segment display driven by `ifelse` branches — it is whatever the
document's author drew, and it can be half the page.

That is why option 1's "a bounded number of differing pixels along discontinuity
boundaries" is not a bound. It is an observation about the programs someone happened to
test.

## 4. Recommendation

### 4.1 The answer

**Option 2**: the processor keeps evaluating for the oracle, the device evaluates for the
screen, and the two are stated as different answers to different questions.

Argued from §1–§3:

1. **Option 3 is not for sale** (§2.5). Not "expensive" — unavailable. WGSL declines to
   specify what it would have to specify, and the emulation that would recover it costs
   more than the CPU evaluation the ask exists to remove.
2. **Option 1 asserts a bound nobody can supply** (§3.4). Neither ISO 32000-2 nor PLRM3
   states an accuracy requirement (§1.7), so there is nothing to derive the bound *from*;
   and the program decides the size of the differing set, so there is nothing to bound it
   *by*. A tolerance-shaped gate would also weaken the corpus comparison for every scene
   unless it can be scoped to this paint — and a gate scoped to this paint, exempting it
   from the pixel comparison, *is* option 2 under a name that hides what it does.
3. **Option 2 is the only one that is true.** Principle 6's "whatever a `Frame` says about
   itself must be true" and the `Report`-not-approximation rule both come down on the side
   of two admitted answers over one pretended agreement. It also costs nothing extra: the
   caller says so themselves — "it doubles nothing because the oracle runs offline".

### 4.2 What ADR 0006 implies here, and where it stops

ADR 0006 measured cross-adapter byte identity for the fixed-function store and answered
**no**, replacing identity with a stated bound of ±1 unorm step per blend stage. Two things
carry over and one does not.

- **The shape carries.** We again tell this caller "no, not for this path", early, from
  our own reading, rather than at a golden mismatch in a late milestone. That is what
  `RENDER_LIBRARY.md` §11.4 asked for and what 0006 did.
- **The consequence for their CI carries, and gets worse.** 0006's finding was that RADV
  and lavapipe genuinely round differently. Here the divergence is licensed on a far larger
  scale: two conformant WGSL implementations may differ by 4096 ULP on `atan` and are
  unconstrained on `sin` outside ±π. **[inference]** So cross-adapter identity for a
  `FunctionPaint` should not be promised at all, and their CI's reliance on lavapipe
  agreeing with RADV does not extend to this paint. A function-shading page rendered under
  `Xvfb`/lavapipe is not evidence about the same page on the user's RADV.
- **The bound does not carry.** 0006 could offer ±1 unorm step because the diverging
  quantity was *continuous*: a rounding difference in a store conversion moves a colour by
  one step and no further. A discontinuous function has no such property (§3.4). Our answer
  here is therefore *stricter* than 0006's, not looser: we offer no per-pixel bound for a
  discontinuous program, because any bound we offered would be false.

### 4.3 Four amendments we would attach

**(a) Two classes of program, not one policy.** The distinction §3.1 draws is decidable
statically. A program in which no comparison, branch, `truncate`, `cvi`, `round`, `floor`,
`ceiling`, `idiv`, `mod` or stack-count operand is reachable, in dataflow, from an inexact
operator is **continuous**: its output is a Lipschitz-bounded function of the accumulated
arithmetic error, so the device and the oracle differ by a *bounded colour error* — which
is exactly §10.7.3's currency (§1.5), and exactly the thing a tolerance gate can measure.
Only a **discontinuous** program needs option 2's full retreat. The classification is an
abstract interpretation over the flat, forward-jump-only instruction list the caller
already builds, it is linear in the program's length, and it costs nothing at frame time.
**[inference]** We would rather offer them "continuous programs get a bounded gate,
discontinuous ones get two answers" than tell them the whole feature is ungatable, because
the first is true and the second is only mostly true.

**(b) The lowering choice is an arithmetic decision as well as a startup one** (§2.3).
Generating a shader per program hands WGSL §15.7.5's reassociation and fusion licence the
whole expression tree; an interpreter loop over a value stack denies it most of it. That
argues the same way as the caller's startup requirement ("no shader compilation on the
first-frame path"), and it should be recorded as a second reason rather than left as a
coincidence — and it is an *input* to the performance spike, not a conclusion from it.

**(c) Every domain exit is guarded, and the guard value is a joint decision written down**
(§1.8, §2.4). WGSL's Finite Math Assumption turns an unguarded overflow into an
indeterminate value — an arbitrary colour that looks like a colour, which is principle 6's
worst outcome by name. PLRM3 makes each of these an error (`undefinedresult`,
`rangecheck`) and ISO 32000-2 §8.7.4.5.2 says "an error may occur"; neither defines a
substitute value. So the substitutes are a decision neither side can take alone — which
CLAUDE.md's third consequence says is a decision neither side has made — and they belong
in the caller's document, cited from ours, not inferred by us from their source.

**(d) The refusal path is also the gate mechanism.** Their §5.2 already asks for a
refusal by name for any program the device declines. **[inference]** That same refusal is
what makes option 2 cheap to operate: a corpus gate run can ask for the host evaluation
deterministically by taking the refusal path, so "the oracle evaluates on the processor"
needs no second code path and no test-only switch. It is the fallback that has to exist
anyway, exercised on purpose.

### 4.4 What we are not saying

We are not saying the paint is a bad idea; §1.5's reading of §8.7.4.4 and §10.7.3 says the
standard already expects a processor's shading to differ from the ideal by a stated amount,
and §3's table says most of Table 42 agrees to the bit. We are saying that *agreement with
the oracle at every pixel* is a property the specification never promised anybody, that
WGSL forecloses buying it back, and that for a discontinuous program it cannot be
approximated by a tolerance either — so it should be given up explicitly rather than
approximated quietly.

## 5. What we could not verify

- **PLRM3's status as a document in this tree.** ISO 32000-2 clause 2 makes it normative,
  and it is not in either project's checkout. The copy read for §1.4 and §3.2 was
  downloaded from Adobe's public site during this session (title page: *PostScript Language
  Reference, third edition*, Adobe Systems Incorporated / Addison-Wesley, © 1985–1999;
  912 pages; matching the February 1999 edition clause 2 cites). It is **not** committed
  here. Whoever relies on these quotations should obtain the document rather than trust
  this record.
- **The "Goto errata" badge beside §7.3.3.** We did not follow it (§1.7).
- **`bitshift` past the operand width.** PLRM3's `bitshift` entry, as we read it, does not
  state the behaviour when the shift exceeds the integer width; WGSL takes it modulo the
  bit width. We could not verify what PLRM3 requires there.
- **Anything measured.** No timing, no shader, no adapter comparison. A sibling spike owns
  that half, and the numbers in §2 are the specification's ULP figures arithmetically
  converted to degrees and radians, not measurements of any driver.
