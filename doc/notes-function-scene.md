# The scene half of a function paint — what was built, and the decision that was withdrawn

Written 2026-08-15 against ADR 0053, `doc/spike-function-paint.md`,
`doc/research-function-paint-arithmetic.md` and the integrator's pinned vocabulary,
**revision 2**. This is the `quorra-scene` half only: the vocabulary, the boundary
refusals, and the cost. Nothing here evaluates a program, generates a shader, or reaches a
device — it cannot (ADR 0001).

§5 records the one thing worth a reader's time even if they skip the rest: **revision 1's
decision 3b said the generated shader clamps its position into the domain, and that is
wrong for a type 1 shading.** It was withdrawn on the clause, before anything was built on
it. §6 records what the two revisions changed and what it cost.

---

## 1. What was implemented

| file | what it holds |
|---|---|
| `crates/quorra-scene/src/function.rs` | `FnOp` and `FnRange`, with rustdoc and clause citations |
| `crates/quorra-scene/src/function/validate.rs` | `check_program`, `MAX_PROGRAM_LENGTH`, one test per refusal — the **upload** boundary's half |
| `crates/quorra-scene/src/ids.rs` | `FunctionId`, and its `ResourceId` variant |
| `crates/quorra-scene/src/paint.rs` | `Paint::Function { program, domain, matrix, range, background }`; `Paint::is_valid` extended |
| `crates/quorra-scene/src/scene/validate/function.rs` | `check_function_paint` — the **scene** boundary's half — and one test per refusal |
| `crates/quorra-scene/src/error.rs` | seven new named `SceneError` variants and their `Display` |
| `crates/quorra-scene/src/scene/cost.rs` | a count of distinct `FunctionId`s in `Cost` |
| `crates/quorra-scene/src/geom.rs` | `Affine::max_coefficient`, so one bound is written once |

### The two boundaries, and why the checks are split across them

A program is an uploaded resource, so the questions divide by *when they can be answered*
rather than by what type they are about:

- **`Device::upload_function`** (wave 2) asks whether the program can be executed.
  `check_program` is the structural half — non-empty, within `MAX_PROGRAM_LENGTH`, every
  jump strictly forward and in range — and the generator's analyser is the semantic half.
  It is public API of `quorra-scene` rather than `pub(crate)`, because the device that
  calls it is in another crate and because a caller may want the answer without a device
  in hand.
- **`SceneBuilder`** asks whether the *paint* is well-formed: a domain rectangle, a
  matrix, a range and a background. Those are the scene's numbers.

That split is strictly better than revision 1's, and the reason is §5's: a caller learns
its program is unsupported **before it has built a scene at all**, not mid-page.

## 2. The refusal grounds, and the clause behind each

### At the scene boundary — `scene/validate/function.rs`

| ground | `SceneError` variant | why, and from where |
|---|---|---|
| a non-finite, inverted or oversized `domain` | `NonFiniteRect`, `UnorderedRect`, `RectTooLarge` | a domain **is** a rectangle in the shading's own space (Table 78), so §4.7's rectangle rule is the rule. Reusing the names is deliberate: a second vocabulary for one rule makes "how often does this happen?" harder to answer, not easier — the error still carries the rectangle that failed. |
| a non-finite or oversized `matrix` | `NonFiniteTransform`, `TransformTooLarge` | the same argument again. |
| a **singular** `matrix` | `SingularFunctionMatrix` | Table 78's `Matrix` maps *the domain into* the target space; a fragment shader has to go the other way. A singular matrix collapses the domain onto a line, so no fragment names a point to evaluate. `Affine::invert` returning `None` is what says so, and there is no identity fallback — a substituted identity is §4.7's plausible-looking wrong answer. |
| a non-finite `range` bound | `NonFiniteFunctionRange` | §7.10.1: "output values produced by the function shall be clipped to the range". A clip against NaN admits nothing and reports nothing. |
| `range` min above max | `UnorderedFunctionRange` | Table 38 states the range as `[min, max]` pairs. Clamping into `[max, min]` returns the upper bound for *every* input — a flat colour that looks drawn. Refused rather than swapped. |
| a `background` outside `0..=1` | `InvalidColor` | §8.7.4.5.2's `Background`, resolved upstream, is a colour like any other. |

### At the upload boundary — `function/validate.rs`

| ground | `SceneError` variant | why |
|---|---|---|
| an empty program | `EmptyFunctionProgram` | §7.10.1 makes a function a transformation that produces output values. An empty program produces none — and ADR 0053's empty-stack-yields-zero rule would silently turn that into a plausible black. |
| a program past `MAX_PROGRAM_LENGTH` | `FunctionProgramTooLong` | our bound, not a clause's; §3 below. |
| a jump target past the program's length | `FunctionJumpOutOfRange` | a target *equal to* the length is legitimate and means "stop" — it is how a trailing `if` lowers. Anything beyond it names no instruction. |
| a backward or self jump | `BackwardFunctionJump` | the property the whole design rests on. Forward-only jumping is what makes a program's length a bound on its own execution, which is what a fragment shader without a loop needs; a backward jump is not slow, it is **unbounded**, and no device budget can be stated over it. ADR 0053 §1 has the measurement of what unbounded costs: the interpreter shape took the device down. |

What is deliberately **not** checked in this crate, and is the generator's analyser: stack
depth, the static classification of the operators our two adapters disagree on, the static
resolution of `copy`/`index`/`roll` counts, and a `JumpUnless` whose condition is not a
boolean. All four need a walk that models the stack and its types.

### Two refusals considered and rejected

- **`Range` outside `[0, 1]`.** Table 78's `Function` row applies a second adjustment after
  the range clip — "If the value returned by the function for a given colour component is
  out of range, it shall be adjusted to the nearest valid value" — so a conforming document
  may declare a range wider than the colour space and rely on that. Refusing it would refuse
  a conforming file. Tested: `a_range_outside_the_unit_interval_is_accepted`.
- **A degenerate `domain`.** A degenerate rectangle is not a malformed one, and what it
  covers is §10.7.4's question — Table 77's `BBox` note makes the same point for a
  zero-height bounding box. Tested: `a_degenerate_domain_is_accepted`.

## 3. `MAX_PROGRAM_LENGTH = 8 192`, and what it costs

ISO 32000-2 bounds a type 4 program's length nowhere, so this is a deliberate choice of
ours and it is written down as one.

The anchor is the spike: the largest program in either of the caller's witnesses is **482
instructions**, and its generated shader took **6.3 ms** to compile cold. 8 192 is
seventeen times that; extrapolating the compile cost linearly — **an extrapolation, not a
measurement** — puts a program at the bound near 107 ms of pipeline compilation, eighteen
times the whole 5.9 ms CPU-rasteriser frame principle 2 measures against. So the bound is a
ceiling that keeps a refusal cheap, not a target: anything near it will be refused by a
*device* budget (ADR 0053's fragments × instructions) long before it is drawn. It also caps
what one upload can hold: 8 192 × 8 bytes = 64 KiB.

If the generator's own budget binds at a much lower number, this should follow it down
rather than stay as a second, looser limit nobody reaches.

## 4. `Scene::cost()`

`Cost` gains `function_programs`: the number of **distinct** `FunctionId`s the scene's
paints reference, counted through group nesting and mask bodies.

Distinct, and that is the point rather than an optimisation. CLAUDE.md's rule is to
instrument the count of distinct keys rather than the rate; here the distinct count is
exactly what a device pays — **one generated shader per distinct program**, 6.3 ms of cold
compile each. A thousand fills sharing one identifier compile one shader and count one; a
hundred fills naming a hundred identifiers is a hundred, and that is the page a caller
wants to hear about before the frame rather than during it.

The programs are **not** in `retained_bytes` any more: under revision 2 they live on a
device (§2.2), and what a scene holds is four bytes of handle per reference, already
counted inside `Command`. Four tests.

## 5. The decision that was withdrawn, and why it is worth a reader's time

**Revision 1's decision 3b:** the generated shader "clamps its `(x, y)` into `domain`
*before* the program runs", justified by §7.10.1's

> Input values passed to the function shall be clipped to the domain, and output values
> produced by the function shall be clipped to the range.

That quotation is correct, and it is about **a function**. It is not what §8.7.4.5.2 says
about the same rectangle when the function is a **type 1 shading** (read from
`ISO_32000-2_sponsored_EC3.pdf`, §8.7.4.5.2, final paragraph):

> The domain rectangle (`Domain`) establishes an internal coordinate space for the shading
> that is independent of the target coordinate space in which it shall be painted. The
> colour function(s) (`Function`) specify the colour of the shading at each point within
> this domain rectangle. The transformation matrix (`Matrix`) then maps the domain
> rectangle into a corresponding rectangle or parallelogram in the target coordinate space.
> **Points within the shading's bounding box (`BBox`) that fall outside this transformed
> domain rectangle shall be painted with the shading's background colour (`Background`); if
> the shading dictionary has no `Background` entry, such points shall be left unpainted.**

The two clauses do not contradict each other — §7.10.1 governs an invocation of the
function, §8.7.4.5.2 governs *where a type 1 shading invokes it* — but decision 3b
collapsed them, and the collapse is not a rounding difference:

- **Clamping** paints every fragment outside the transformed domain rectangle with the
  colour of the nearest domain edge, at full alpha.
- **The clause** paints those fragments with `/Background`, or **not at all**.

Three things make this the kind of defect the project has a name for.

1. **It is invisible on the corpus.** Both of the caller's witnesses declare
   `/Domain [0 1 0 1]` mapped onto the shape they paint, so clamping and discarding agree
   at every pixel. A corpus gate would have passed it. It needs a unit test with a domain
   *smaller* than the shape, asserting the outside is untouched rather than edge-coloured —
   which is the generator's to write, and which revision 2 asks for.
2. **It is a plausible wrong page, not a visible failure.** A smear of the edge colour
   across a clip looks like a gradient someone drew.
3. **Nothing in the data could have said otherwise.** Revision 1's `FunctionPaint` had no
   `background` field, so even a device that read the clause correctly had no way to
   express "left unpainted" versus "painted with this". Revision 2 adds
   `background: Option<Color>`, and `None` **is** the clause's "left unpainted".

Two further notes the correction carries with it:

- **`/Background` is not always applicable.** Table 77: "The background colour shall be
  applied only when the shading is used as part of a shading pattern, not when painted
  directly with the `sh` operator." That distinction is one only the caller can see, so the
  field arrives already resolved — `None` from an `sh`, whatever the pattern declared
  otherwise. `Paint::Function`'s rustdoc says so.
- **`/BBox` is still resolved upstream to a clip**, consistently with `Paint::Shading`,
  which also has no bounding box. Table 77 calls it "a temporary clipping boundary", which
  is exactly what the caller's display list can already express.

The citation `§7.10` for the clip-at-both-ends sentence was also corrected to **§7.10.1**;
our own research record read it from the PDF and places it there, and the markdown
extraction agrees (§7.10 is the clause heading, §7.10.1 "General" holds the sentence, and
§7.10.2 follows it).

## 6. What the two revisions cost, recorded so the next pin is cheaper

Revision 1 put the program inside the paint as an `Arc<FunctionPaint>`. That cost `Paint`
its `Copy`, which broke seven call sites in `quorra-gpu`; I fixed them and then unfixed
them when revision 2 made the program a `FunctionId`. Two observations worth carrying
forward:

- **The `Copy` break was the visible symptom of a design mismatch, not a Rust annoyance.**
  Every other heavy paint in this crate is already a resource identifier plus the geometry
  that places it, and `Paint::Function` is now the same shape as `Paint::Shading`. The
  conformance agent's counter-proposal is better on three separate counts: `Paint` keeps
  `Copy`; two shadings can share one program under different matrices, which revision 1's
  shape made impossible; and the upload is where ADR 0053's shader-cache hash is computed
  anyway, so the refusal lands where identity is already being decided.
- **A pinned vocabulary is worth reviewing against the clause before it is written down in
  three worktrees.** Both defects were found by reading §8.7.4.5.2 and by counting existing
  call sites — neither needed a build.

`quorra-gpu` still carries the minimum this crate's change forces, and it is one commit so
the generator agent can take their own version whole: `RenderError::UnsupportedFunctionPaint
{ program }`, the two match arms that return it (a `Paint::Function` reaching today's
encoder is refused by name rather than silently drawn as nothing, which §5 requires in the
interval before the generator exists), and the two `ResourceId::Function` arms that
`release` needs — where `Resources::release` returns `UnknownResource`, which is the truth
until `upload_function` exists.

## 7. Gaps a later session should close

- **The domain-smaller-than-the-shape test.** §5's defect is invisible without it. It
  belongs to the generator, and it is the single most valuable test in this feature.
- **The fuzzer does not generate function paints.** Principle 3 says fuzz the scene
  boundary from the first commit, and `quorra-gpu/tests/fuzz_scene/generator.rs` builds
  every other paint. It is in the generator agent's tree, so it is theirs to extend, but a
  jump-heavy random program is exactly the input `check_program` exists for.
- **`MAX_PROGRAM_LENGTH` has no measured anchor of its own** — §3 says which number is
  measured and which is extrapolated, and the extrapolation should be replaced by a
  measurement once the generator can compile a program of that size.
- **`SceneError`'s `Display` carries an `#[allow(clippy::too_many_lines)]`** with its
  reason written above it. It is a table with one arm per variant, and every way of
  splitting it costs the exhaustiveness that makes a new refusal fail to compile until it
  has a message. If the enum grows much further, the honest fix is to split `SceneError`
  itself by what was refused — and the natural seam is already visible: the four program
  refusals are raised at `Device::upload_function`, not at a scene boundary at all.
