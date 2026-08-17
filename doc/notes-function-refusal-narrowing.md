# Narrowing the §7.10.5 agreement refusal: what WGSL allows, and how many pages it is worth

Status: **research record with a census**, written 2026-08-18. Nothing in this round changed
what is drawn or what is refused; the decision it argues for is `doc/adr/0067`.

It answers one question the caller asked — *can `pi.pdf`'s type 4 program run on the device?*
— and it answers it in three parts, because the question turned out to contain a false
premise, a true premise about the wrong machine, and a measurement nobody had taken.

Three kinds of statement are kept apart, as in
`doc/research-function-paint-arithmetic.md`:

- **Quotation.** Inside quotation marks or a blockquote, under its clause number, verbatim.
- **Paraphrase.** Prose without quotation marks that restates a clause, naming the clause.
- **Our inference.** Marked **[inference]**.

---

## 0. The ask, and the short answer

The caller's argument, quoted from the round's brief:

> Most arithmetic is already bit-exact on a GPU. IEEE 754 requires + − × ÷ sqrt to be
> correctly rounded … GPUs diverge from CPUs not because the hardware is sloppy but because
> shader compilers fuse multiply-add and relax precision by default. Vulkan exposes the knobs
> to stop that: `VK_KHR_shader_float_controls` (denorm preserve, round-to-nearest-even) plus
> SPIR-V's `NoContraction`. With those set, a GPU evaluation of that operator set is
> bit-identical to the CPU's. So refusing on `div` reaching `truncate` is refusing on the
> wrong property — `div` is exactly reproducible.

> What genuinely isn't reproducible is the transcendentals … `pi.pdf` uses exactly one of
> those, `exp`, and only ever as 16ⁿ with an integer exponent, which is a power of two and
> therefore exact if computed by repeated multiplication instead of `exp2(y·log2(x))`.

The short answer:

1. **The headline claim does not reach us.** We do not write SPIR-V and we do not talk to
   Vulkan; we write WGSL and hand it to `wgpu`. WGSL's own accuracy table gives `x / y`
   **2.5 ULP** (§1), and `wgpu` 30 exposes neither `VK_KHR_shader_float_controls` nor
   `NoContraction` — `naga`'s SPIR-V writer emits neither, and WGSL has no spelling for
   either (§2). `Div` is classified correctly today.
2. **The second claim is true, and about the wrong machine.** IEEE 754 does pin `÷` — on the
   *host*. The way to use that fact is to move a division that has no fragment in it off the
   device entirely, by folding it at shader-generation time; and the way to make `16ⁿ` exact
   *on* the device is `ldexp`, whose WGSL row is "Correctly rounded" (§3). Both are real
   narrowings and both are specification-backed.
3. **Neither is worth building.** A census over **67 464 documents** found **four** with a
   `/ShadingType 1` at all, of which **one** is a real-world document — and that one, plus
   the pdf.js test file, is already drawn on the device today. The agreement refusal fires on
   **5 of 7 139** type 4 programs, and the only two that could ever reach this lane are the
   caller's own two hand-written witnesses (§4, §5).

---

## 1. What WGSL promises

### 1.0 The source

W3C Candidate Recommendation Draft, *WebGPU Shading Language*,
<https://www.w3.org/TR/WGSL/>, dated **17 August 2026**, retrieved 2026-08-18. Sections
§15.7.4 "Floating Point Accuracy", §15.7.4.1 "Accuracy of Concrete Floating Point
Expressions", §15.7.5 "Reassociation and Fusion", and §17.5.37 `ldexp`. Rows below were read
out of the published HTML, not out of a summary.

### 1.1 "Correctly rounded" in WGSL is not IEEE 754's correct rounding

§15.7.4, verbatim, complete on the point:

> Let x be the exact real-valued or infinite result of an operation when computed with
> unbounded precision. The correctly rounded result of the operation for floating point type
> T is:
>
> - x, when x is in T,
> - Otherwise:
>   - the smallest value in T greater than x, or
>   - the largest value in T less than x.
>
> That is, the result may be rounded up or down: WGSL does not specify a rounding mode.

**[inference]** Two consequences, and they pull opposite ways. When the exact result **is**
representable, "correctly rounded" pins it to that one value — this is the sentence
narrowing 3.2 rests on. When it is not, either neighbour is permitted, so even `+`, `−` and
`×` cannot be promised to tie the way a host does.

### 1.2 The rows, verbatim, for the operations a §7.10.5 program reaches

§15.7.4.1, f32 column, verbatim:

| WGSL expression or built-in | Accuracy for f32 (verbatim) |
|---|---|
| `x + y`, `x - y`, `x * y`, `-x` | "Correctly rounded" |
| **`x / y`** | **"2.5 ULP for `\|y\|` in the range [2⁻¹²⁶, 2¹²⁶]"** |
| `x % y` | "Inherited from `x - y * trunc(x/y)`" |
| `x == y`, `!=`, `<`, `<=`, `>`, `>=` | "Correct result" |
| `abs(x)`, `floor(x)`, `ceil(x)`, `trunc(x)`, `round(x)`, `sign(x)`, `step` | "Correctly rounded" |
| **`ldexp(x, y)`** | **"Correctly rounded"** |
| `frexp(x)` | "Correctly rounded, when x is zero or normal." |
| `inverseSqrt(x)` | "2 ULP" |
| `sqrt(x)` | "Inherited from `1.0 / inverseSqrt(x)`" |
| `exp(x)`, `exp2(x)` | "3 + 2 * `\|x\|` ULP" |
| `log(x)`, `log2(x)` | "Absolute error at most 2⁻²¹ when x is in the interval [0.5, 2.0]. 3 ULP when x is outside the interval [0.5, 2.0]." |
| `sin(x)`, `cos(x)` | "Absolute error at most 2⁻¹¹ when x is in the interval [-π, π]" |
| `atan(x)` | "4096 ULP" |
| `atan2(y, x)` | "4096 ULP for `\|x\|` in the range [2⁻¹²⁶, 2¹²⁶], and y is finite and normal" |
| `pow(x, y)` | "Inherited from `exp2(y * log2(x))`" |

and, from §15.7.4:

> When the accuracy for an operation is specified over an input range, the accuracy is
> undefined for input values outside that range.

**The `x / y` row is the whole of the answer to the caller's first claim.** A conformant WGSL
implementation may return a value 2.5 ULP from the exact quotient. IEEE 754 requires a
conformant host to return the correctly rounded one. Those are different requirements, and
`Binary::is_inexact` naming `Div` is a reading of the first, not a guess about hardware.

### 1.3 Reassociation and fusion are licensed, and cannot be switched off

§15.7.5, verbatim:

> An implementation may reassociate operations.

> An implementation may fuse operations if the transformed expression is at least as accurate
> as the original formulation. For example, some fused multiply-add implementations can be
> more accurate than performing a multiply followed by an addition.

**The word "contraction" does not appear in the WGSL specification at all**, in any section;
the only occurrence of the substring is the word "contract" in §1's prose about behavioural
requirements. Checked by searching the full published document. So WGSL has no `NoContraction`
and no pragma that would introduce one.

---

## 2. Whether `wgpu` 30 can reach the knobs the caller names

**No, on three independent counts.** Checked against the vendored sources at
`~/.cargo/registry/src/index.crates.io-*/`:

- **`naga` 30.0.0 never emits `NoContraction`.** Its SPIR-V writer emits 22 decorations —
  `ArrayStride`, `Binding`, `Block`, `BuiltIn`, `Centroid`, `Coherent`, `ColMajor`,
  `DescriptorSet`, `Flat`, `Index`, `Invariant`, `Location`, `MatrixStride`, `NonReadable`,
  `NonUniform`, `NonWritable`, `NoPerspective`, `Offset`, `PerPrimitiveEXT`, `PerVertexKHR`,
  `Sample`, `Volatile` — and `NoContraction` is not among them. The absence is a choice, not a
  missing constant: the `spirv` crate `naga` depends on defines
  `NoContraction = 42u32`.
- **`naga` 30.0.0 emits no float-controls execution mode.** The twelve `ExecutionMode`s it can
  emit are the depth, `EarlyFragmentTests`, `LocalSize`, `OriginUpperLeft` and the mesh/geometry
  output ones. `RoundingModeRTE`, `DenormPreserve` and `SignedZeroInfNanPreserve` appear nowhere
  in the crate.
- **`wgpu-hal` 30.0.0's Vulkan backend never requests `VK_KHR_shader_float_controls`.** The
  string does not occur in the crate, and no `wgpu-types` feature corresponds to it.

**[inference]** This is a cost of ADR 0002 ("wgpu rather than Vulkan"), and it should be
recorded as one. The caller's argument is not wrong about Vulkan; it is inapplicable to a
renderer whose shader language is WGSL and whose only route to the driver is `wgpu`. Any
future claim in this project about pinning rounding modes or forbidding fusion has to start by
overturning this paragraph.

---

## 3. What survives, and the two narrowings it makes available

### 3.1 Constant folding — the caller's IEEE argument, applied to the host

**[inference]** The premise "IEEE 754 requires `÷` to be correctly rounded" is true. It is a
requirement on a *host*, and the way to spend it is to make the host do the division.

If every operand of an operator is known at shader-generation time, the operator can be
evaluated then, in `f32`, and the shader can carry the result as a literal. The device then
performs no WGSL `/` for it at all, so §15.7.4.1's 2.5 ULP row never applies — and neither
does §15.7.5's reassociation licence, which is a permission about expressions a shader
*contains*. **That is what makes folding the only one of the three candidate narrowings that
escapes §15.7.5 rather than reasoning around it** (§3.3). Two specification rows carry the
value across the boundary intact:

- **§15.7.6, numeric scalar conversion to floating point**, verbatim: "If X is exactly
  representable in the destination type T, then XOut is the value in T equal to X." The
  generator already prints every literal as Rust's shortest round-tripping form with an `f`
  suffix (`generate::literal`), so X is exactly the `f32` that was folded.
- **IEEE 754** requires `+ − × ÷ √` and the rounding operations to be correctly rounded, which
  under the round-to-nearest-even default Rust does not allow changing makes each of them
  *uniquely* determined. So a second host — the caller's `pdf-model` evaluator, which carries
  the operand stack in `f32` (`Value::Real(f32)`, `fn number(self) -> f32`) — computes the same
  bits from the same program.

**What may not be folded, and why the exclusion is the interesting half.** `sin`, `cos`, `ln`,
`log`, `atan` and `exp` are *not* pinned for a host either. IEEE 754 recommends but does not
require correct rounding for them, and the standard library that would do our folding says so
itself — `f32::powf`, "Unspecified precision", verbatim:

> The precision of this function is non-deterministic. This means it varies by platform, Rust
> version, and can even differ within the same execution from one invocation to the next.

Folding one of those would pin *our* libm's answer into a shader and call it the host's, which
is curve-fitting to an implementation and CLAUDE.md principle 5 forbids it outright. The same
sentence is why §3.2's `exp` narrowing is a question for the caller rather than a decision for
us.

### 3.2 `ldexp` — how `16ⁿ` becomes exact *on the device*

The caller's second claim is right about the mathematics and wrong about the mechanism.
"Repeated multiplication" is not what makes `16ⁿ` exact in WGSL — §15.7.4's "correctly
rounded" is, and WGSL has a built-in whose whole purpose is the exponent adjustment.

§17.5.37, `ldexp`, verbatim:

> Returns e1 * 2^e2, except:
>
> - The result may be zero if e2 + bias ≤ 0.
> - If e2 > bias + 1
>   - It is a shader-creation error if e2 is a const-expression.
>   - It is a pipeline-creation error if e2 is an override-expression.
>   - Otherwise the result is an indeterminate value for T.
>
> Here, bias is the exponent bias of the floating point format: … 127 for f32

with the accuracy row "Correctly rounded" (§1.2).

**[inference]** Put the two together. When `e1 × 2^e2` is representable in `f32`, §15.7.4's
first bullet — "x, when x is in T" — makes the correctly rounded result *that value and no
other*. So:

- `x` divided by a literal power of two 2ᵏ is `ldexp(x, -k)`, **exact**, where `x / 2ᵏ` gets
  2.5 ULP.
- `16ⁿ = 2⁴ⁿ` is `ldexp(1.0, 4n)`, **exact**, where the shader's present composition
  `exp2(n · log2(16))` gets "3 + 2·|x| ULP" twice over.

Both are bounded by the clause's own range conditions, which a lowering would have to check
rather than assume: the result must stay in the normal range, and `e2 > 128` is a
shader-creation error for a constant exponent.

**The `exp` half is not ours to take alone.** ISO 32000-2 §7.10.5.2 makes PLRM3 normative for
Table 42's semantics, and PLRM3's `exp` entry says only that it "raises base to the exponent
power". For `16 2 exp` that value is 256, which is representable, so **[inference]** a
conformant host returns 256 exactly and the device's `ldexp` agrees with it. But the caller's
evaluator computes `a.powf(b)` (`pdf-model/src/function.rs`), and `powf`'s own documentation
disclaims its precision "by platform, Rust version, and … from one invocation to the next"
(§3.1). Whether the two agree is therefore a property of their evaluator rather than of the
clause — CLAUDE.md's third consequence, "a decision either side can make alone is a decision
neither side has made". Measured on this machine, for what it is worth as evidence and not as
truth: `16f32.powf(n)` and the shader's own `exp2(n * log2(16))` both give exactly 1, 16 and
256 for n = 0, 1, 2 — so the refusal that names `exp` is, on this witness, refusing a
divergence that does not occur.

### 3.3 The provenance refinement the round asked about, and why it is not available

The round asked whether there are chains where an inexact result reaches an amplifier but the
*amplification cannot occur* — a comparison whose operands are separated by more than the
accumulated bound.

It is a good question, and on `type4_pi.pdf` it looks answerable. Folded in `f32`, the BBP sum
that program computes is **3.1415873**; the amplifier is `1000.0 mul truncate`, so the value
reaching the truncation is **3141.5873** and the nearest integer boundary is **0.41** away.
Nine divisions sit upstream of that truncation. At the full 2.5 ULP each — a relative error
under 3 × 10⁻⁷ — plus the correctly-rounded sums between them, the accumulated error at that
magnitude is **under 0.01 against a boundary 0.41 away**: more than an order of magnitude of
margin. In *this instance* the amplification demonstrably cannot occur.

**[inference] And the refinement is still not available, for a reason with a clause behind
it.** Accumulating "2.5 ULP per division" presumes the divisions happen in the order the
program wrote them. §15.7.5 says they need not, verbatim:

> Reassociation is the reordering of operations in an expression such that the answer is the
> same if computed exactly. … However, the result may not be the same when computed in
> floating point. The reassociated result may be inaccurate due to approximation, or may
> trigger an overflow or NaN when computing intermediate results.

> An implementation may reassociate operations.

**No bound accompanies that permission**, and §15.7.5's own third example is `(a * b) / c`
reassociating to `(a / c) * b` — a rewrite that moves a division. A generated shader hands the
compiler exactly the straight-line expression tree this licence applies to. So a forward error
bound over such a tree is not derivable in WGSL even for the operators whose *own* rows carry
a number, and `doc/research-function-paint-arithmetic.md` §1.7 supplies the rest: `atan` is
4096 ULP, `sin` and `cos` are unbounded outside [−π, π], and an "inherited from" row may be
implemented differently again.

So the 0.41 above is **evidence that the refusal is conservative on this document**, and it is
not a bound anyone can state about the class. Recording it and refusing anyway is the same
discipline ADR 0053's amendment applied to the spike's zero differing pixels.

The refinement that *is* available is the one §3.1 describes, and it is not about bounding an
error — it is about removing the operation.

---

## 4. The census

### 4.1 The instrument, and what it does not do

Three throwaway pieces, described here rather than vendored, because a census instrument that
lives in the tree is a test nobody runs. They were built under `/home/AI/census-0067/` and are
short enough to rewrite from this paragraph:

- `extract.py` scans a PDF's raw bytes for `N G obj << … /FunctionType 4 … >> stream`, inflates
  `/FlateDecode` bodies, and yields the program text. **A raw scan is sound for this one
  question**, and the reason is a clause: a type 4 function is a *stream* object, and
  ISO 32000-2 §7.5.7 says of an object stream that "the following objects shall not be stored
  in an object stream: — Stream objects". So every type 4 function's dictionary is plaintext in
  the file body, whatever the file's compression. A shading *dictionary* is not a stream and
  may hide in one, so the instrument also inflates every `/Type /ObjStm` before looking for
  `/ShadingType 1`.
- `probe/` is a small crate that compiles each program to `quorra_scene::FnOp` and calls the
  real `quorra_gpu::function::admit`. **The verdicts below are the analyser's, not a mirror
  of it.**
- `narrow.py` simulates the candidate narrowings. Its `today` mode reproduces the analyser's
  **operator pair** on all five programs the analyser refuses on agreement grounds — not the
  instruction indices, which differ because the simulation does not materialise the jump
  instructions the compiled form carries. That agreement is what licenses reading its other
  modes; the indices in §4.4 are the analyser's, the ones in §5 are the simulation's and are
  not quoted.

**Two limits, stated.** 79 files carry a filter the extractor does not implement and are
counted as unread rather than guessed at; 9 extracted programs did not compile (garbled or
encrypted bytes). And every program was compiled as a **2-input §8.7.4.5.2 shading function**,
which is what inflates the `index`/`roll` refusals below — a DeviceN tint transform takes more
inputs than a shading does, and refusing it here says nothing about that document.

### 4.2 The population

Scanned: **67 464 PDFs** — `corpus-cache/openpreserve` and `corpus-cache/safedocs` (66 211),
`doc/corpora` (275), `doc/corpora-own` (2), `doc/pdf.js/test/pdfs` (976).

| | count |
|---|---:|
| documents carrying `/FunctionType 4` | 2 100 |
| documents yielding at least one readable program | 2 025 |
| **type 4 programs extracted** | **7 139** |
| **documents carrying a `/ShadingType 1` *and* a type 4 function** | **4** |

The `/ShadingType 1` count needs its scope stated, because a shading dictionary — unlike a
function stream — *may* hide in an object stream. Plaintext `/ShadingType 1` was searched for
across all 67 464; object streams were inflated and searched in the 2 100 that carry a type 4
function. **That is exactly the set where a hidden one could matter**, because a shading whose
function is type 0, 2 or 3 never reaches this lane at all.

The four, in full — this is the entire population that can reach the function-paint lane:

| document | origin | type 4 programs |
|---|---|---:|
| `doc/corpora-own/pi_seven_segment.pdf` | the caller's own, hand-written | 1 |
| `doc/corpora-own/type4_pi.pdf` | the caller's own, hand-written | 1 |
| `doc/pdf.js/test/pdfs/function_based_shading.pdf` | pdf.js's test suite | 9 |
| `corpus-cache/safedocs/cc-main-2021-31/2514/2514229.pdf` | **a real document** | 2 |

`PLAN.md`'s earlier sentence — a census over the gate's 974 files found one page with a
§8.7.4.5.2 program — is confirmed and extended by two orders of magnitude of denominator.

**`pi.pdf` is `pi_seven_segment.pdf`.** The caller's `tmp/pi.pdf` and their tracked
`doc/corpora-own/pi_seven_segment.pdf` have the same MD5 (`1ec9476…`); the chain the brief
quotes, `exch pop exch 16 exch exp div`, occurs three times in it and nowhere else in the
corpus. It is a witness, not a population.

### 4.3 What the analyser says about all 7 139

| verdict | programs |
|---|---:|
| admitted | 6 254 |
| refused | 876 |
| did not compile | 9 |

Refusals, by ground:

| ground | programs | note |
|---|---:|---|
| `index` names *n* operands with *m* on the stack | 642 | census artefact — a >2-input tint transform compiled as a 2-input shading function |
| `roll` names *n* operands with *m* on the stack | 208 | same |
| the program is empty | 19 | |
| **`sin` reaches `lt`** | **3** | agreement; none is a `/ShadingType 1` |
| **`div` reaches `truncate`** | **2** | agreement; both are the caller's witnesses |
| the `ifelse` arms leave different depths | 1 | |
| `mod` was given a real | 1 | `function_based_shading.pdf` |

**The agreement refusal fires on 5 of 7 139 programs — 0.07 %.**

### 4.4 The thirteen programs that can reach the lane

| document | verdict |
|---|---|
| `pi_seven_segment.pdf` | refused: `div` at 234 reaches `truncate` at 354 |
| `type4_pi.pdf` | refused: `div` at 2 reaches `truncate` at 35 |
| `function_based_shading.pdf` × 8 | admitted (`Bounded`) |
| `function_based_shading.pdf` × 1 | refused: `mod` was given a real, and requires two integers |
| `2514229.pdf` × 2 | admitted (`Bounded`) |

The eight-and-one split is exactly what `PLAN.md` records from the gate run, reached here from
a different direction. **The only real-world document in the population is already drawn on the
device**, and the ninth `function_based_shading.pdf` program is refused for a type reason that
no narrowing in this note touches.

### 4.5 Operators, bucketed

Over the 7 139 programs. "class" is what `function::operators` says about the row:
**inexact** is `is_inexact`, **amp** is `amplifies`, blank is neither.

| operator | class | programs | uses |
|---|---|---:|---:|
| `exch` | | 6 600 | 25 876 |
| `sub` | | 6 396 | 22 295 |
| `pop` | | 3 387 | 8 108 |
| `roll` | | 3 313 | 36 649 |
| `index` | | 3 129 | 6 484 |
| `cvr` | | 2 849 | 16 682 |
| `dup` | | 742 | 3 546 |
| `mul` | | 609 | 4 302 |
| `add` | | 366 | 1 523 |
| `if` | amp | 317 | 1 068 |
| `gt` | amp | 315 | 1 087 |
| **`div`** | **inexact** | **18** | **87** |
| `neg` | | 7 | 7 |
| `copy` | | 6 | 37 |
| `ifelse` | amp | 6 | 55 |
| **`sin`** | **inexact** | **5** | **7** |
| **`sqrt`** | **inexact** | **5** | **6** |
| `abs` | | 4 | 4 |
| `le` | amp | 4 | 44 |
| `lt` | amp | 4 | 4 |
| **`exp`** | **inexact** | **3** | **5** |
| `ge` | amp | 3 | 38 |
| `and` | amp | 2 | 56 |
| `mod` | amp | 2 | 2 |
| `truncate` | amp | 2 | 4 |
| **`atan`** | **inexact** | **1** | **1** |
| **`cos`** | **inexact** | **1** | **1** |
| `cvi` | amp | 1 | 2 |
| `eq` | amp | 1 | 14 |
| `floor` | amp | 1 | 2 |
| `idiv` | amp | 1 | 1 |
| `or` | amp | 1 | 10 |
| `false` | | 1 | 1 |

Thirty-three of Table 42's forty-two operators appear. **Nine appear nowhere in the
population**: `ceiling`, `ln`, `log`, `round`, `bitshift`, `ne`, `not`, `true`, `xor` — which
is worth knowing next to the fact that `round`'s three-way disagreement and `not`'s two
readings each cost this project a written decision.

**22 of 7 139 programs (0.31 %) use any inexact operator at all**, and `div` accounts for 18 of
them. The shape of the population is a Separation/DeviceN tint transform —
`exch sub cvr roll index pop mul add dup gt if` — with no arithmetic anyone could disagree
about.

---

## 5. The narrowing matrix

Simulated over the five programs the agreement rule refuses. "ldexp" is §3.2's division
lowering alone; "fold" is §3.1's constant folding over the IEEE-pinned operators alone;
"fold + ldexp" is both, with `exp` over a power-of-two base and an integral exponent folded
through `ldexp`.

| program | reaches the lane? | today | ldexp | fold | fold + ldexp |
|---|---|---|---|---|---|
| `type4_pi.pdf` | **yes** | refused `div`→`truncate` | refused | **admitted** | **admitted** |
| `pi_seven_segment.pdf` (= `pi.pdf`) | **yes** | refused `div`→`truncate` | refused | refused `exp`→`truncate` | **admitted** |
| `colorspace_sin.pdf` | no | refused `sin`→`lt` | refused | refused | refused |
| `colorspace_cos.pdf` | no | refused `sin`→`lt` | refused | refused | refused |
| `colorspace_atan.pdf` | no | refused `sin`→`lt` | refused | refused | refused |

Read it in one line: **`ldexp` alone recovers nothing; folding recovers one hand-made witness;
folding plus `ldexp` recovers both hand-made witnesses; nothing recovers a real-world
document, because no real-world document is refused.**

Why the `ldexp` column is empty is worth keeping. The nine divisors in `type4_pi.pdf`'s BBP
block are 1, 4, 5, 6, 9, 12, 13, 14 and 16 — **three** powers of two — so an exponent
adjustment removes three divisions and leaves six, and the two divisions after the block are by
1000.0 and by π itself. It is the *constants*, not the *powers of two*, that carry these
programs.

And what makes the two witnesses refusable at all is now visible: **the whole of their
arithmetic is a compile-time constant.** `type4_pi.pdf` computes the BBP series from literals
and truncates it; `pi_seven_segment.pdf` does the same over three terms. Neither division
depends on the fragment. The refusal today is therefore *wider than it needs to be* — it
names a real composition (`div` upstream of `truncate`) over values no pixel influences.

---

## 6. The design, if it is ever taken

Recorded so that the day a witness appears the work is a build rather than a re-derivation.

**6.1 Where it goes.** A new `crates/quorra-gpu/src/function/fold.rs` with one stated
responsibility: *the host value of a Table 42 operator over constant operands.* `walk.rs`
consults it in `unary` and `binary`: when every operand `Cell` carries a `literal` and the
operator is foldable, push `Cell { literal: Some(value), taint: None }` and emit
`Step::Literal` instead of `Step::Unary`/`Step::Binary`.

**6.2 The foldable set, and the row that justifies each.** `abs add sub mul neg div sqrt
ceiling floor truncate round cvi cvr idiv mod and or xor not bitshift eq ne ge gt le lt` —
IEEE 754 pins the first seven for a host, and the rest are exact by construction. `exp` joins
the set **only** for a power-of-two base with an integral exponent whose result is a normal
`f32` (§3.2). `sin cos ln log atan` and general `exp` never join it (§3.1).

**6.3 Two side effects that must not ride along silently.**

- **`Cell::literal` changes meaning.** Today it is `Some` only for a *pushed* literal, and
  `copy`, `index` and `roll` read it for their counts. Folding makes it `Some` for computed
  values too, so a `roll` whose count is `2 1 add` would start resolving where it now refuses
  with `DynamicStackCount`. That is a second change to what is drawn, riding on the first, and
  `HANDOVER.md`'s trap about coupled changes applies: it wants its own round, its own
  refusal-test movement and its own corpus column.
- **A third Table 42 implementation enters the workspace**, after `function_ops.wgsl` and the
  conformance crate's reference evaluator, and it must agree with the WGSL one forever. The
  125-case conformance corpus is the right instrument — it compares device output against an
  independently written evaluator, so a folding defect surfaces there — but the corpus needs a
  case per foldable operator *with constant operands*, which it does not have today, because
  every existing case feeds the fragment coordinate in.

**6.4 What it buys besides the two witnesses, stated small because it is small.** The spike
measured 275 of `pi_seven_segment.pdf`'s 482 instructions as depending on the fragment
(`doc/spike-function-paint.md` §1), so a fold would turn up to **207 operator calls into
literal assignments**. It would not *remove* them: the slot model gives one `var` per
operand-stack position and one statement per instruction, so the shader keeps its shape and
only loses the arithmetic. `doc/spike-function-paint.md` §3 puts a cold pipeline compile of
that shader at 6.3 ms, on the caller's first-frame path — so the saving is real, unmeasured,
and a fraction of a number that is already small. It is not a reason to build this.

---

## 7. What we could not verify

- **Whether `2514229.pdf` renders.** The census establishes that its two programs are admitted
  by `admit()`; it does not render the page. It is in `corpus-cache/safedocs`, which the gate's
  974-file list does not draw from, so no corpus run in either tree has ever drawn it.
- **The 79 files with an unimplemented filter and the 9 uncompilable programs.** If a
  `/ShadingType 1` hides in one of them the count of four is a lower bound. None of the 79 has
  a plaintext `/ShadingType 1`, which is weak evidence and not proof.
- **What the caller's evaluator would do with a folded constant.** §3.2's `exp` question is
  open by design; it is theirs to answer against PLRM3, not ours to assume.
- **§5's `fold` and `fold + ldexp` columns are a simulation, not a run.** Neither narrowing is
  implemented, so no analyser produced those verdicts and no generated shader exists for them.
  What licenses the columns is that the same simulation's `today` mode reproduces the
  analyser's operator pair on all five programs (§4.1) — which is evidence about the
  simulation, not proof about the narrowing.
- **Anything on a device.** No shader was compiled and no pixel was drawn in this round.
