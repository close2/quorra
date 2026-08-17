# 0067 — A narrowing the specification allows, and the corpus does not ask for

Date: 2026-08-18. Status: **accepted — measured, and left.** No code changed; no pixel moved.

The evidence is `doc/notes-function-refusal-narrowing.md`; the classification this is about is
ADR 0053 §3 and its amendment.

## Context

The caller asked whether `pi.pdf`'s §7.10.5 program — refused today because a `div` reaches a
`truncate` — could run on the device. Their argument had two halves: that IEEE 754 pins `÷`
and that Vulkan's `VK_KHR_shader_float_controls` plus SPIR-V's `NoContraction` make a GPU
evaluation bit-identical to a CPU's; and that the one transcendental in the program, `exp`, is
only ever `16ⁿ` for integral *n*, which is a power of two and therefore exact.

ADR 0053's amendment already warned that this is the crux: `Agreement::Exact` was renamed
`Bounded` because "WGSL permits reassociation and fusion, so bit-exactness was never ours to
promise". The question is whether the *classification of `Div`* survives the same scrutiny.

## Decision

Three parts. The first two are readings; the third is the decision proper.

### 1. `Div` is classified correctly, and the argument for narrowing it does not reach us

We do not write SPIR-V and we do not talk to Vulkan. We write WGSL and hand it to `wgpu`, and
WGSL specifies its own accuracy. §15.7.4.1, f32 column, verbatim:

> 2.5 ULP for `|y|` in the range [2⁻¹²⁶, 2¹²⁶]

against IEEE 754's requirement that a host's `/` be correctly rounded. The two are different
requirements and the gap between them is exactly what `Binary::is_inexact` names.

WGSL is weaker than IEEE 754 one step earlier, too. §15.7.4, on what its own "correctly
rounded" means, verbatim:

> That is, the result may be rounded up or down: WGSL does not specify a rounding mode.

**And the knobs are unreachable from here.** Checked against the vendored sources: `naga`
30.0.0's SPIR-V writer emits 22 decorations and `NoContraction` is not among them, though the
`spirv` crate it depends on defines it; it emits no `RoundingModeRTE`, `DenormPreserve` or
`SignedZeroInfNanPreserve` execution mode; `wgpu-hal` 30.0.0 never requests
`VK_KHR_shader_float_controls`; and the word "contraction" does not occur in the WGSL
specification at all. **This is a cost of ADR 0002 ("wgpu rather than Vulkan") and is recorded
here as one**, because it bounds every future claim in this project about pinning a rounding
mode or forbidding a fusion.

### 2. The second half of their argument is true, about the host, and it makes two real narrowings available

IEEE 754 does pin `÷` — on a *processor*. Two narrowings follow, and both are backed by a row
rather than by an intuition:

- **Constant folding.** An operator all of whose operands are known at shader-generation time
  can be evaluated then, in `f32`, and carried into the shader as a literal; the device then
  performs no WGSL `/` for it, so the 2.5 ULP row never applies. §15.7.6 carries the value
  across intact — "If X is exactly representable in the destination type T, then XOut is the
  value in T equal to X" — and IEEE 754 makes the folded value the same on any host, which
  includes the caller's `f32` evaluator. `sin`, `cos`, `ln`, `log`, `atan` and general `exp`
  are **excluded**: no standard pins them for a host either, so folding one would pin our
  libm's answer and call it the host's, which principle 5 forbids outright.
- **`ldexp`.** WGSL gives `ldexp(e1, e2)` — "Returns e1 * 2^e2" — the accuracy row
  "Correctly rounded", and §15.7.4's first bullet makes that *the exact value* whenever the
  exact value is representable. So a division by a literal power of two is an exponent
  adjustment and is exact, and `16ⁿ = 2⁴ⁿ` is `ldexp(1.0, 4n)` and is exact — which is the
  caller's own insight, reached by the mechanism WGSL actually supplies rather than by
  repeated multiplication.

**The provenance refinement is not available, and the reason is sharper than "no bound
exists".** Deciding that an amplification cannot occur needs a bound on the accumulated error.
For `type4_pi.pdf` one looks computable: the value reaching its `truncate` is 3141.5873, an
integer boundary is 0.41 away, and nine divisions at 2.5 ULP each accumulate under 0.01. But
accumulating per-operation ULPs presumes the operations happen in the order the program wrote
them, and §15.7.5 grants an implementation permission to reassociate **with no bound
attached** — its own third example, `(a * b) / c` becoming `(a / c) * b`, moves a division.
A generated shader hands the compiler exactly that expression tree. So the 0.41 is evidence
that the refusal is conservative on that one document and is not a bound about the class,
which is the same distinction ADR 0053's amendment drew over the spike's zero differing
pixels.

### 3. Neither narrowing is built, because the population is zero

A census over **67 464 documents** — the caller's whole `corpus-cache`, their tracked corpora
and pdf.js's test suite — extracted **7 139** type 4 programs and ran the real
`quorra_gpu::function::admit` over every one.

- **Four documents carry a `/ShadingType 1` *and* a type 4 function** — the whole population
  this lane can serve. Two are the caller's own hand-written witnesses, one is pdf.js's
  `function_based_shading.pdf`, and **one is a real document** (`safedocs/…/2514229.pdf`).
- The agreement refusal fires on **5 of 7 139 programs (0.07 %)**. Three are `sin` reaching
  `lt` in tint transforms that never reach this lane. The other two are the caller's two
  witnesses.
- **Every program in the population that reaches the lane from a real document is already
  drawn on the device.** The one non-agreement refusal there is `mod` given a real, which no
  narrowing here touches.

What each narrowing would recover, simulated over those five:

| | `type4_pi.pdf` | `pi_seven_segment.pdf` (= `pi.pdf`) | the other three | any real document |
|---|---|---|---|---|
| `ldexp` alone | refused | refused | refused | — |
| constant folding alone | **admitted** | refused (`exp`→`truncate`) | refused | **none** |
| folding + `ldexp` | **admitted** | **admitted** | refused | **none** |

So the whole yield is two hand-made demonstration files, and the cost is a **third**
implementation of Table 42 arithmetic in this workspace — after `function_ops.wgsl` and the
conformance crate's reference evaluator — that must agree with the other two forever, plus a
coupled change to what `Cell::literal` means that would quietly start resolving `copy`,
`index` and `roll` counts the analyser refuses today. CLAUDE.md principle 2 forbids
speculative optimisation of code nobody measured; this is the same rule pointed at a
correctness narrowing, and it answers the same way.

**The refusal is nonetheless wider than it needs to be, and that is written down rather than
argued away.** Both witnesses compute their entire arithmetic from literals — no pixel
influences a single one of those divisions — and the classification names a composition over
values no fragment reaches. `doc/notes-function-refusal-narrowing.md` §6 specifies the fix
precisely enough that the day a witness appears it is a build and not a re-derivation.

## Consequences

- **The caller gets a "no" with a number attached.** Not "we cannot", but "we can, it costs a
  third arithmetic implementation, and in 67 464 of your documents it would move two files you
  wrote yourself". Their §5.3's own claim that function shadings are rare is now measured
  rather than believed.
- **`pi.pdf` does not run on the device**, and the reason is stated in two layers: the
  classification is correct as written, and the narrowing that would admit it is real but
  unearned.
- **One question goes back to them.** Folding `16 2 exp` to exactly 256 is what PLRM3's "raises
  base to the exponent power" requires when the value is representable, and it is what WGSL's
  `ldexp` produces; their evaluator computes `a.powf(b)`, and `powf` is not required to be
  correctly rounded by anything. Whether the two agree is a property of their evaluator rather
  than of a clause, so it is CLAUDE.md's third consequence — a decision neither side can take
  alone — and it is theirs to take first.
- **ADR 0002 gains a named cost.** "wgpu rather than Vulkan" means no float controls and no
  `NoContraction`, ever, until that decision is revisited.
- **No corpus run is owed.** Nothing changed what is drawn or what is refused — this round
  adds two documents and not one line of code, so ADR 0066's release matrix, taken in parallel
  with it, covers the same pixels it would have covered anyway.

## What would overturn this

**A real document whose `/ShadingType 1` function has a constant subexpression reaching an
amplifier.** Not a synthetic one, and not a tint transform: this lane is §8.7.4.5.2 only. The
census instrument is described in `doc/notes-function-refusal-narrowing.md` §4.1 — one scan
over the corpus and one extraction over the files that match — so the trigger is cheap to
re-test when the corpus grows.

A second thing would overturn part of it without touching the count: **if the lane widens to
type 4 functions outside §8.7.4.5.2** — tint transforms for Separation and DeviceN, which are
7 139 programs rather than 13 — the denominator changes by three orders of magnitude and this
ADR is void. It would still not be the agreement rule that is under pressure there, because
only 22 of those 7 139 use an inexact operator at all; it would be the `index`/`roll` count
resolution, which is a different subject.
