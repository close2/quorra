# 0053 — A paint the device evaluates, and the operators that forbid it

Date: 2026-08-15. Status: **proposed** — the answer is sent, nothing is built.

The caller asked for it in `pdf-viewer/doc/QUORRA_FUNCTION_PAINT.md`; our answer is
`pdf-viewer/doc/QUORRA_FUNCTION_PAINT_ANSWER.md`. This ADR is the decision behind that
answer and the evidence under it. The measurements live in `doc/spike-function-paint.md`
and the clause work in `doc/research-function-paint-arithmetic.md`; the spike itself is
`crates/quorra-gpu/examples/function_paint/`.

## Context

A §8.7.4.5.2 type 1 shading is a function of two variables over a domain, and the function
may be a §7.10.5 PostScript calculator program. The caller evaluates it **once per device
pixel on the processor** — deliberately, since their ADR 0339 replaced a fixed 128×128
sampled grid with the device's own grid because the fixed grid blurred every discontinuity
across three pixels at 4×, and §10.7.4's centre rule is what says which value a pixel
carries. That decision is right and the ask is how to make the same answer cheap.

Their measurement: a one-page document whose whole content is one such shading spends
**1 142.8 ms building the scene** and 30.8 ms on the device, against `mutool draw -r 96` at
15–16 ms for the page.

## Decision

**Yes — worth building, for type 4 only, and only as a generated shader cached by the
program's hash.** Not built yet; this records the commitment and its conditions.

Three parts, and the third is the one that is ours rather than theirs.

### 1. The generated shader, not the interpreter

Their §4 left the choice to us and expected the interpreter to win the startup property
(`PLAN.md` §1.8: nothing on the launch path waits for warmth). **It loses on both axes.**
Full page at 1191×1684, device time, minimum of five, re-run independently at load 29:

| | seven-segment (482 instructions) | BBP π (311) |
|---|---:|---:|
| generated, RADV | **0.060 ms** | 0.059 ms |
| interpreter, RADV | 133.7 ms | 105.9 ms |
| generated, llvmpipe | **1.97 ms** | 1.41 ms |
| interpreter, llvmpipe | 825.0 ms | 714.9 ms |
| their processor path, allocation-free | 4 988 ms | 4 083 ms |

Cold-cache pipeline compile: generated **6.3 ms**, interpreter **596 ms to 4.5 s**. So the
shape that existed to avoid a frame-path compile is the one that must never be near a frame
path.

And the interpreter is not merely slow: at 4× its pass **lost the device** — `radv/amdgpu:
The CS has been cancelled … guilty of a hard recovery`, then `Parent device is lost`. That
is principle 6 with teeth. A paint that can take the device down cannot be refused *after*
the frame, so any such lowering would need a fragment-count budget checked before it.

### 2. The specification supplies no precision contract, so we do not pretend to one

ISO 32000-2 **§7.3.3**, verbatim:

> The range and precision of numbers **may be limited by the internal representations used
> in the computer on which the PDF processor is running**; Annex C, "Advice on maximising
> portability", gives these limits for typical implementations.

Annex C is informative. **"IEEE 754" appears exactly twice in ISO 32000-2** — that
informative row and the Bibliography — and is absent from clause 2, Normative references;
verified by extraction rather than by search. §7.10.5.2 makes PLRM3 normative for Table 42's
semantics and PLRM3 defers again to the hardware's native representation.

This is the §5 kind of silence that must be *stated as a silence* rather than filled: there
is no clause either side can be measured against, and §8.7.4.4/§10.7.3's accuracy language
is about not evaluating at every point, which §8.7.4.5.2 makes unavailable at a
discontinuity by saying the function "need not be smooth or continuous".

### 3. Classify the program; be **exact** for what we accept, and refuse the rest

The caller offered three answers to the agreement problem and asked us to pick one. **We
pick none of them**, because the measurement makes a fourth available.

On both of their real witnesses, the device and an independent processor evaluation of the
same instruction list agreed with **zero differing pixels** over four million device pixels
of deliberately discontinuous function, on both adapters. The 246 044 off-by-one texels on
one of them are ADR 0006's fixed-function store rounding, not the program — llvmpipe reports
none of them and the other witness has none on either adapter.

They are exact because **neither witness calls a transcendental**. The danger is real but is
carried by a statically identifiable subset of Table 42, not by function evaluation as such.
Measured on our two adapters, over 4 096 inputs:

| op | bitwise RADV vs llvmpipe |
|---|---|
| `sin`, `cos` | 3 201 and 3 334 of 4 096 differ |
| `exp` | 2 660 |
| `sqrt` | 618 |
| `div` | 398 |
| `atan` | 375 |

— and the comparison flip their §5.1 predicts is reproducible: `sin 0 ge` and `cos 0 ge`
disagree between the two adapters on 2 of 4 096 inputs, before any CPU oracle is involved.

So: **a program that reaches only the exactly-agreeing operators is accepted and the oracle
relationship stays exact; a program that can reach a transcendental on any path into a
comparison is refused by name**, and the caller falls back to the raster they build today.
The classification is a dataflow walk over the flat list, on the pass that already computes
stack depth and slot types.

## Consequences

- **Their option 3 is refused on evidence**, not opinion: our two adapters do not agree
  bitwise on any of these operators, including division and square root, so a contract
  "specified to the bit" would have to be honoured by a driver that never agreed to it.
- **Cross-adapter identity is not promised for this paint.** ADR 0006's shape carries but
  its bound does not: 0006 could offer ±1 unorm because the diverging quantity was
  continuous, and a discontinuous function amplifies instead. A function-shading page under
  lavapipe is not evidence about the same page on RADV, and the caller's CI needs to know
  that before it finds out.
- **The refusal path is the gate**, not an afterthought — five grounds, each demonstrated on
  a program that reaches it (`examples/function_paint/refusal.rs`).
- **Nothing is built.** The spike measured a bare full-viewport pass, not a `Paint::Shading`
  clipped, grouped and composited; the classification needs a conformance test per dangerous
  and per safe operator before it is a contract; and two contract questions are back with the
  caller (a type tag on a literal, and what a pop from an empty stack means).

## What would overturn this

A witness whose program calls a transcendental *and* whose corpus presence matters. Every
number here rests on two documents, and their §5.3 already says function shadings are rare
and catastrophic rather than common. If the accepted set turns out to be small enough that
the refusal path is the common path, the win is smaller than 0.060 ms suggests and this is
worth re-costing before it is built.
