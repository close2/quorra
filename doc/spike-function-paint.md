# Spike — a paint the device evaluates: what §7.10.5 actually costs

Written 2026-08-15 against `/home/cl/projects/pdf-viewer/doc/QUORRA_FUNCTION_PAINT.md`,
which asks for a paint whose colour is a small program and says its own §3 — "a
function-based shading is a fragment shader written in another language" — is an
intuition. This is the number.

**It is a spike, not a feature.** `crates/quorra-gpu/examples/function_paint/` is the
whole of it; nothing under `crates/quorra-gpu/src` changed, no lane was opened in
`encode.rs`, and no `todo!()` was left anywhere. Run it with
`cargo run --release -p quorra-gpu --example function_paint`.

**The recommendation, first: worth building, for type 4 only, and only as a generated
shader.** The evidence is below, and it includes one measurement that says *no* very
loudly — about the other shape.

---

## 1. What was measured, and on what

The caller's two witnesses, read straight out of `doc/corpora-own/` in their tree
(nothing was written there, and nothing was vendored here — `fixture.rs` reads the
type 4 stream out of the PDF and asserts the `/Length` §1 promises):

| program | bytes | instructions | max depth | branches | forward-only jumps | instructions whose result depends on the fragment |
|---|---|---|---|---|---|---|
| `pi_seven_segment.pdf` | 2 580 | 482 | 8 | 23 | yes | **275 of 482** |
| `type4_pi.pdf` (BBP π) | 1 605 | 311 | 8 | 1 | yes | **174 of 311** |

Two shapes, both drawing a full-viewport pass whose fragment shader evaluates the
program at its own coordinate:

- **(i) an interpreter** — one shader, a `switch` over the instruction list, the
  program uploaded as a storage buffer of `(opcode, operand)` pairs. Nothing compiles
  on the frame path.
- **(ii) a shader generated per distinct program** — the same operators, but the
  operand stack is `var s0 … sN` with every index resolved at compile time, so nothing
  is dynamically indexed.

Both call **the same WGSL operator functions** (`ops.wgsl`), so a difference between
them cannot be a difference of arithmetic. A third, independent implementation in Rust
(`eval.rs`) is the oracle and the processor-side cost anchor.

Measurement discipline is `doc/HANDOVER.md`'s: every variant round-robin, minima
quoted, device column from timestamp queries. This machine's load average ran between
24 and 57 across the runs, which is why the wall-clock columns are reported but not
argued from.

## 2. The answer to §1's comparison

The caller's number for one page whose whole content is one function-based shading:
**1 142.8 ms of scene building and 30.8 ms of device time**, against `mutool draw -r 96`
at 15–16 ms for the whole page.

Device time for the same arithmetic, at a full page of 1191×1684 (2 005 644 fragments),
minimum over five runs:

| | seven-segment | BBP π |
|---|---|---|
| **(ii) generated, RADV** | **0.062 ms** | **0.059 ms** |
| (i) interpreter, RADV | 143.1 ms | 121.0 ms |
| **(ii) generated, llvmpipe** | **2.22 ms** | 1.89 ms |
| (i) interpreter, llvmpipe | 936.6 ms | 823.3 ms |
| the processor, one thread, no allocation | 4 988 ms | 4 083 ms |

At 4× (4764×6736, 32 090 304 fragments):

| | seven-segment | BBP π |
|---|---|---|
| **(ii) generated, RADV** | **1.37 ms** | **1.37 ms** |
| **(ii) generated, llvmpipe** | **31.3 ms** | 19.9 ms |
| (i) interpreter | *not run* — see §4 | *not run* |

So the generated shader draws a whole page of the discontinuous seven-segment function
in **62 microseconds on the hardware adapter and 2.2 ms on the software one**, against
1 142.8 ms of processor time for the grid that reaches the device today. That is not a
close call in either direction, and it holds on the adapter the caller's CI uses.

The processor anchor is worth one sentence, because it is what makes the comparison
fair rather than flattering: `eval.rs` is single-threaded and allocates nothing, where
the caller's evaluator allocates three times per device pixel. It costs 2 439–3 111
ns/px here against their measured ~4 019 ns/px, so it is the *floor* of what removing
allocations can buy them, and the generated shader is still 80 000× under it on RADV.

**The upload disappears too.** Their §3 prices the round trip: at a 1000×1000 placement,
four megabytes of grid per frame per shading, recomputed at every zoom step. The
program is 482 instructions × 8 bytes = **3.9 kB**, uploaded once and invariant under
zoom.

## 3. Shape (i) is hopeless, and its own advantage is where it loses worst

The interpreter was the shape with the property the caller's §5.2 cares about: no
shader compilation on the frame path. It fails on both counts.

**Per fragment it is 2 300× slower than the generated shader on RADV** (143.1 ms
against 0.062) and 422× slower on llvmpipe. Two reasons, and both are structural
rather than a matter of tuning:

- **The operand stack is dynamically indexed.** `stack[sp]` is an index no compiler can
  resolve, so the array lands in scratch memory and every push and pop is a memory
  operation. The generated shader's `s0 … s7` are registers.
- **The interpreter cannot fold anything.** 207 of the seven-segment's 482 instructions
  and 137 of the BBP program's 311 compute values that do not depend on the fragment's
  coordinate — the BBP series itself is literals all the way down — and a shader
  compiler evaluates those once. An interpreter re-executes them at every one of two
  million fragments.

**And its compile is the expensive one.** With a cold driver shader cache
(`XDG_CACHE_HOME` pointed at an empty directory), RADV's pipeline creation:

| | interpreter | generated |
|---|---|---|
| cold, first pipeline of the process | 595.7 / 617.6 / 2 956.3 ms | 64.5 / 84.4 ms |
| cold, not first | 4 527 ms (one sample) | 6.3 / 7.8 / 16.3 / 16.6 ms |
| warm driver cache | 1.07–2.28 ms | 1.4–4.2 ms |

llvmpipe: interpreter 13.6–69.7 ms, generated 4.6–18.2 ms.

The order was switched deliberately (`QUORRA_FUNCTION_PAINT_ORDER=generated-first`)
because the first pipeline of a process pays what the driver defers until then, and
attributing that to whichever shader happened to be first is exactly the error
`HANDOVER.md` warns about. With the order reversed the interpreter still cost seconds,
so the cost is the shader and not the position: **a `switch` of 44 arms inside a loop
is a hard shader to compile**, and one that quorra would have to compile once at a
cost `PLAN.md` §1.8 has no room for.

One methodological note, because it cost a round: RADV's on-disk cache keys on the
compiled SPIR-V, not on the WGSL text, so appending a comment to force a recompile does
not. The first process to compile the interpreter took 1 500 ms and every process after
it took 1.1 ms, which looked like a measurement error until the cache was the
explanation. Every cold number above comes from a fresh `XDG_CACHE_HOME`.

## 4. The interpreter's loop bound: legal, bounded, and it still loses the device

The caller's load-bearing claim holds. Every jump the compiler emits is forward
(`Program::verify_forward_only` asserts it), so the WGSL loop is a `for` of exactly
`op_count` iterations and the instruction count *is* the execution bound. Naga and both
drivers accept it without complaint. There is no unbounded loop and no way for a
program to express one.

What the bound does not do is make the cost acceptable. 482 instructions per fragment
at 2 005 644 fragments is 967 million instruction-executions per page, and the
measurement of that is 143 ms.

At 4× it is worse than slow. The first run of this spike attempted the interpreter at
4764×6736 and the driver killed the context:

```
radv/amdgpu: The CS has been cancelled because the context is lost.
This context is guilty of a hard recovery.
```

— followed by `Parent device is lost` on the next poll. A 143 ms pass at page size
projects to 2.3 s at 4×, which is past the GPU reset watchdog, and what is lost is not
the frame but **the device**. The spike now refuses to run any variant whose projection
exceeds 900 ms, and prints why. For quorra that is a §6 problem with teeth: a paint that
can take the device down is not a paint that can be refused after the fact, so if shape
(i) were ever built, its refusal would have to be a *fragment-count* budget checked
before the frame, not a report after it.

## 5. Arithmetic agreement — their §5.1, as counts

Every raster was compared against the independent Rust evaluation of the same
instruction list, at page size:

| | exact | off by one | differing | worst |
|---|---|---|---|---|
| both shapes, RADV, seven-segment | 1 759 600 | 246 044 | **0** | 1 |
| both shapes, RADV, BBP π | 2 005 644 | 0 | **0** | 0 |
| both shapes, llvmpipe, both programs | 2 005 644 | 0 | **0** | 0 |

**No pixel of either page took a different branch.** The sharp case the caller
describes — "a comparison that lands the other side of `0.8 ge` gives a different colour
for that pixel entirely" — did not occur once in four million device pixels of the two
documents that are made of discontinuities. That is not a proof that it cannot; it is
evidence that f32 evaluated the same way on both sides lands on the same side of these
comparisons, and that the boundaries in these programs fall between representable
coordinates rather than on them.

The 246 044 off-by-one pixels are **not** the program. They are exactly the lit-segment
colour, both shapes agree with each other to the byte, and llvmpipe agrees with the
processor exactly — so the difference is RADV's 8-bit conversion of the attachment
write, one step, on one adapter.

Which produces the finding that matters more than the agreement itself:

> **The two adapters do not draw the same bytes.** RADV against llvmpipe, same program,
> same page: 1 759 600 exact, 246 044 off by one, 0 differing.

`CLAUDE.md`'s environment note and §4.6 of the brief both rest on cross-adapter
byte-equality holding for the current backend. A function paint does not inherit that
promise, and the difference is in the framebuffer conversion rather than in anything
this spike could fix in the program. Any corpus gate over a function paint has to
tolerate one 8-bit step, or run on one adapter.

## 6. What we would refuse, and on what stated ground

§5.2 asks for a refusal by name. These are the grounds the spike found it needs; each is
demonstrated on a constructed program that reaches it, because a ground nobody can reach
is not a ground. The output table is `refusal.rs` plus `report::refusals`:

| ground | shape | reason |
|---|---|---|
| an operator outside Table 42 | both | the compiled form must be closed |
| an operand stack deeper than the shader has slots | both | a WGSL array needs a constant size; the spike uses 64, both witnesses need 8 |
| `copy`/`index`/`roll` whose count is not a literal | both | shape (ii) cannot name a slot it cannot compute, and shape (i) cannot state its own depth |
| an `ifelse` whose arms leave different depths | (ii) | no static slot assignment describes the join |
| `not` on a value two branches disagreed about | both | see §7 |
| a procedure that is not an `if`/`ifelse` operand | both | §7.10.5.1 admits procedures nowhere else |

Neither witness trips any of them. The depth limit is the only one likely to bind in the
wild, and it is discoverable before the frame — which is what §5 of the brief asks of
every limit.

## 7. Two things about the compiled form they would hand us

Both are findings for their side, not requests.

**`Instruction::Push(f32)` cannot implement Table 42's `not`.** Their compiled form
(`pdf-model/src/function.rs`) carries no type on a literal, and Table 42's `not` is two
operators wearing one name: logical negation on a boolean, one's complement on an
integer. Their evaluator implements the logical one only, so `63 not` yields `0.0` where
the standard says `-64`. It is unreachable in either witness. The fix costs nothing at
run time — this spike infers the type of every stack slot statically during the same
walk that computes the depth, and rewrites each `not` into whichever it meant, so
neither shader carries a type tag. `and`, `or` and `xor` need no such rewrite: with
`true` as 1 and `false` as 0 the bitwise reading and the logical one agree, which is why
their implementation gets the right answer everywhere it is used.

**`pi_seven_segment.pdf` pops an empty operand stack, three times, and depends on the
result being 0.** In its final `ifelse`, the else arm enters with three operands, pops
two, and then runs `gt` — which their evaluator serves from `stack.pop().unwrap_or(0.0)`.
The consequence is visible in the picture: the comparison is `0.0 > col`, always false,
so the "unlit segment: dark green" branch the program's own comment describes is dead
code and every unlit pixel takes the background colour. ISO 32000-2 defines nothing here
and PostScript would raise `stackunderflow`. **Their choice is the one this spike
adopted**, in all three implementations, because the alternative is refusing their own
witness — but it is a choice, it is theirs, and if a device paint is ever built it has
to be written into the contract rather than inherited from an `unwrap_or`.

## 8. What this spike did not measure

- **The paint inside quorra.** Every number here is a bare full-viewport pass. A real
  `Paint::Shading` would run inside the existing shading lane and the compositor, and
  would carry that lane's own per-fragment cost. Since the paint's own cost is 31 ps per
  fragment on RADV, the lane would dominate it entirely — which is the point, but it is
  an extrapolation and not a measurement.
- **Types 0, 2 and 3.** Their §7 says only type 4 need be interesting, and it is the
  only one measured.
- **A program that is not one of these two.** Two witnesses are two witnesses. The
  shapes of the finding (dynamic indexing, constant folding, compile time) are properties
  of the two mechanisms rather than of these programs, but the *ratios* are theirs.
- **`atan`, `sin`, `cos`, `ln`, `log`, `exp` at the last bit.** Neither witness calls a
  transcendental except `exp` with integer arguments. The agreement in §5 says nothing
  about a program that does, and §5.1's third option — specifying the transcendentals to
  the bit — remains unanswered and would need its own measurement.
- **Anything on a third adapter.** Two is what this machine has.

## 9. Recommendation

**Worth building, for type 4 only, and only as shape (ii) — a generated shader cached by
the program's hash.**

- The win is not marginal: 1 142.8 ms of scene building and a four-megabyte per-frame
  upload become a 3.9 kB buffer and 62 µs of device time, and the zoom case their §3
  calls unfixable becomes free, because the program does not change when the grid does.
- It survives the software rasteriser: 2.2 ms at page size and 31.3 ms at 4× on
  llvmpipe, so their CI is not the objection.
- **Shape (i) should not be built.** It loses on the frame (2 300×), it loses on the
  compile it was supposed to win (596 ms cold against 6.3), and at 4× it takes the
  device down. If the ability to draw a *new* program without a compile is ever needed,
  the answer is not an interpreter — it is that the caller already has a CPU raster for
  that frame, and quorra takes over on the next one.
- The one promise we cannot make is their §5.2's "no shader compilation on the
  first-frame path". A page whose first frame contains a function shading pays 6–8 ms of
  compile with a cold driver cache, 1–4 ms warm, once per distinct program. That is the
  honest cost, it is off the *launch* path (like every non-warm-set pipeline, `PLAN.md`
  §1.8 point 3) but not off the first-frame path for that page, and it is three orders
  of magnitude under the 1 142.8 ms it replaces.
- On §5.1, the answer we can live with is their second option: **the processor keeps
  evaluating for the oracle and the device evaluates for the screen.** The measurement
  supports the stronger claim — zero differing pixels on both witnesses — but §5's own
  finding about the two adapters disagreeing by one 8-bit step is the reason to keep the
  gate's tolerance explicit rather than to promise equality we have only observed.

If this is built, the ADR it needs is not about the shader. It is about the two silences
in §7 — the type tag on a literal, and the meaning of a pop from an empty stack — because
those are contract, and a contract neither side writes down is a contract neither side
has.
