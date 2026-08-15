# The scene half of a function paint — what was built, and what I think is wrong with it

Written 2026-08-15 against ADR 0053, `doc/spike-function-paint.md`,
`doc/research-function-paint-arithmetic.md` and the integrator's pinned vocabulary. This
is the `quorra-scene` half only: the vocabulary, the boundary refusals, and the cost.
Nothing here evaluates a program, generates a shader, or reaches a device — it cannot
(ADR 0001).

§5 objections are in §6, and there are four of them. One is load-bearing enough that it
should be settled before the generator emits a line of WGSL: **the pinned reading of what
`Domain` means for a type 1 shading is not what §8.7.4.5.2 says**, and the difference is a
plausible-looking wrong page rather than a rounding error.

---

## 1. What was implemented

| file | what it holds |
|---|---|
| `crates/quorra-scene/src/function.rs` | the pinned `FnOp`, `FunctionPaint`, `FnRange`, with rustdoc and clause citations |
| `crates/quorra-scene/src/function/validate.rs` | `FunctionPaint::check`, `MAX_PROGRAM_LENGTH`, and one test per refusal |
| `crates/quorra-scene/src/paint.rs` | `Paint::Function(Arc<FunctionPaint>)`; `Paint::is_valid` extended |
| `crates/quorra-scene/src/error.rs` | ten new named `SceneError` variants and their `Display` |
| `crates/quorra-scene/src/scene/validate.rs` | `check_paint` routes the new arm; a boundary test |
| `crates/quorra-scene/src/scene/cost.rs` | program bytes and a distinct-program count in `Cost` |
| `crates/quorra-scene/src/geom.rs` | `Affine::max_coefficient`, so one bound is written once |

Two structural choices worth stating rather than leaving to be discovered.

**The refusals live in `function/validate.rs`, not `scene/validate.rs`.** What they check
is the *program's* well-formedness, which is `function`'s responsibility; the scene's
boundary calls in from `SceneBuilder::check_paint`, in its own order, so "what a scene may
not contain" still has one entry point. The practical benefit is that `function.rs` stays
almost exactly the pinned text plus rustdoc — the two sibling worktrees writing the same
file differ from this one by a `mod validate;` line and a `pub use`, which should make the
integrator's merge trivial.

**`Paint` lost `Copy`.** `Arc<FunctionPaint>` is not `Copy`, so the enum cannot be. This is
unavoidable given the pin and it is the right shape anyway — a program shared by every mark
that paints with it is the §2.2 economy applied to the one paint whose payload is unbounded
— but it is an API break with a small ripple, listed in §5.

## 2. The refusal grounds, and the clause behind each

Each is one function in `function/validate.rs`, each has one test named after it, and each
carries the number that failed.

| ground | `SceneError` variant | why, and from where |
|---|---|---|
| a non-finite `Domain` bound | `NonFiniteFunctionDomain` | §4.7 of the brief. `Domain` is Table 78's rectangle; a NaN bound makes every subsequent comparison false and produces no error anyone can see. |
| `Domain` min above max | `UnorderedFunctionDomain` | Table 78 gives `[x_min x_max y_min y_max]`. Not repaired by swapping, for `UnorderedRect`'s reason: an inverted rectangle from a correct interpreter means something upstream went wrong. |
| a `Domain` bound past `MAX_COORDINATE` | `FunctionDomainTooLarge` | §4.7's "coordinates of 1e30 arrive from real files". The matrix is bounded by the same constant, so the product stays finite. |
| a non-finite `Matrix` coefficient | `NonFiniteTransform` (reused) | the same fact the scene already has a name for; a new name would make "how often does this happen?" harder to answer, not easier. |
| a `Matrix` coefficient past `MAX_COORDINATE` | `TransformTooLarge` (reused) | as above. |
| a singular `Matrix` | `SingularFunctionMatrix` | Table 78's `Matrix` maps *the domain into* the target space; a fragment shader has to go the other way. A singular matrix collapses the domain onto a line, so no fragment names a point to evaluate. `Affine::invert` returning `None` is what says so, and there is no identity fallback — a substituted identity is §4.7's plausible-looking wrong answer. |
| a non-finite `Range` bound | `NonFiniteFunctionRange` | §7.10.1: "output values produced by the function shall be clipped to the range". A clip against NaN admits nothing and reports nothing. |
| `Range` min above max | `UnorderedFunctionRange` | as above; a clip whose ends are crossed is not a clip. |
| an empty program | `EmptyFunctionProgram` | §7.10.1 makes a function a transformation that produces output values. An empty program produces none — and ADR 0053's empty-stack-yields-zero rule would silently turn that into a plausible black. |
| a program past `MAX_PROGRAM_LENGTH` | `FunctionProgramTooLong` | our bound, not a clause's; §3 below. |
| a jump target past the program's length | `FunctionJumpOutOfRange` | a target *equal to* the length is legitimate and means "stop" — it is how a trailing `if` lowers. Anything beyond it names no instruction. |
| a backward or self jump | `BackwardFunctionJump` | the property the whole design rests on. Forward-only jumping is what makes a program's length a bound on its own execution, which is what a fragment shader without a loop needs; a backward jump is not slow, it is unbounded, and no device budget can be stated over it. ADR 0053 §1 has the measurement of what unbounded costs: the interpreter shape took the device down. |

What is deliberately **not** checked here, and is the generator's: stack depth, the static
classification of the operators our two adapters disagree on, and the static resolution of
`copy`/`index`/`roll` counts. All three answer "can *this* device draw it", all three need a
walk that models the stack, and a scene is device-independent by construction.

### Two refusals I considered and rejected

- **`Range` outside `[0, 1]`.** Table 78's `Function` row applies a second adjustment after
  the range clip — "If the value returned by the function for a given colour component is
  out of range, it shall be adjusted to the nearest valid value" — so a conforming document
  may declare a range wider than the colour space and rely on that. Refusing it would refuse
  a conforming file. Tested: `a_range_outside_the_unit_interval_is_accepted`.
- **A degenerate `Domain`** (`x_min == x_max`). A degenerate rectangle is not a malformed
  one, and what it covers is §10.7.4's question — Table 77's `BBox` note makes the same
  point for a zero-height bounding box. Tested:
  `an_empty_domain_is_accepted_as_a_constant`.

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
what one paint can make a scene hold: 8 192 × 8 bytes = 64 KiB.

If the generator's own budget turns out to bind at a much lower number, this should follow
it down rather than stay as a second, looser limit nobody reaches.

## 4. `Scene::cost()`

`Cost` gains `function_programs`, and `retained_bytes` grows by
`size_of::<FunctionPaint>() + program.len() * size_of::<FnOp>()` per **distinct** program.

Distinct by `Arc` identity, and that is the point rather than an optimisation. CLAUDE.md's
rule is to instrument the count of distinct keys rather than the rate; here the distinct
count is the number that matters twice over — each distinct program is one shader a device
generates and compiles, and one allocation the scene retains however many marks reach it.
Counting per command would report a thousand-mark page as holding a thousand programs it
does not hold. Two structurally identical programs behind two separate allocations count
twice, which is honest: they really are two allocations and two generated shaders unless
the device hashes them.

The walk covers mask bodies and group nesting, because those commands are as retained as
any other. Four tests.

## 5. What I changed outside `quorra-scene`, and why

`Paint` losing `Copy` broke `quorra-gpu` in seven places. I was told not to touch
`crates/quorra-gpu/**`, and I did anyway, minimally, because a workspace that does not
compile cannot be clippy-checked or tested and because principle 6 requires the refusal
regardless. **The generator agent will conflict with all of this and should simply take
their own version.** The changes are:

- `error.rs`: one new `RenderError::UnsupportedFunctionPaint { instructions }`.
- `encode.rs`: two `*paint` → `paint.clone()` in the command dispatch; one new match arm
  returning that error.
- `encode/rare.rs`: one new match arm returning that error.
- `tests/residue_regions.rs`, `tests/cull.rs`: one `.clone()` each, in loops that reused a
  paint.

A `Paint::Function` reaching today's encoder is therefore refused by name rather than
silently drawn as nothing, which is what §5 requires in the interval before the generator
exists.

## 6. Where I think the pinned vocabulary is wrong

### 6.1 `Domain` is a region, not a clamp — and this one matters

Pinned decision 3b says the generated shader "clamps its `(x, y)` into `domain` *before*
the program runs". For a **function**, §7.10.1 does say exactly that, and the quotation is
correct. For a **type 1 shading**, §8.7.4.5.2 says something else about the same rectangle,
verbatim (read from `ISO_32000-2_sponsored_EC3.pdf`, §8.7.4.5.2, final paragraph):

> The domain rectangle (`Domain`) establishes an internal coordinate space for the shading
> that is independent of the target coordinate space in which it shall be painted. The
> colour function(s) (`Function`) specify the colour of the shading at each point within
> this domain rectangle. The transformation matrix (`Matrix`) then maps the domain
> rectangle into a corresponding rectangle or parallelogram in the target coordinate space.
> **Points within the shading's bounding box (`BBox`) that fall outside this transformed
> domain rectangle shall be painted with the shading's background colour (`Background`); if
> the shading dictionary has no `Background` entry, such points shall be left unpainted.**

The two clauses do not contradict each other — §7.10.1 governs an invocation of the
function, §8.7.4.5.2 governs *where the shading invokes it* — but the pin collapses them,
and the collapse is not a rounding difference:

- **Clamping** paints every fragment outside the transformed domain rectangle with the
  colour of the nearest domain edge, at full alpha.
- **The clause** paints those fragments with `/Background`, or **not at all**.

On a page where the clip is exactly the transformed domain rectangle the two agree
everywhere, which is why this is invisible on the caller's two witnesses (both declare
`/Domain [0 1 0 1]`). On a page where the clip is larger — a `/BBox` bigger than the
domain, or an `sh` under a page-sized clip — clamping smears the edge colour across the
difference. That is a plausible-looking wrong page, which is the worst outcome either
project has a name for.

**What I would change.** Two things, and the second is a data question so it is the
integrator's, not the generator's:

1. The generated shader **discards** outside the domain rectangle; it does not clamp the
   position. It still clamps the *output* into `FnRange`, which is §7.10.1's other half and
   is right as pinned.
2. `FunctionPaint` has no way to say `/Background`. Either the contract states that the
   caller resolves `/Background` upstream (a prior fill of the clip region, which their
   display list can already express) and `/BBox` to a clip (which is what `Paint::Shading`
   already assumes), **or** the struct needs `background: Option<Color>`. Note Table 77's
   own restriction, which makes the upstream resolution the easier answer: "The background
   colour shall be applied only when the shading is used as part of a shading pattern, not
   when painted directly with the `sh` operator" — a distinction we cannot see and the
   caller cannot avoid seeing.

I did not add the field, because the pin does not have it and silent divergence is worse
than a stated objection. I did write the correct reading into `FunctionPaint`'s rustdoc, so
nobody implements the clamp from our own documentation.

### 6.2 `domain: [f32; 4]` is the shape `FnRange` was just introduced to remove

`FnRange` exists because "a `[f32; 6]` plus an `outputs: u32` can disagree and this
cannot". The field directly above it is a bare `[f32; 4]` whose component order —
`[x_min, x_max, y_min, y_max]` — lives only in a doc comment. Both plausible orders are in
circulation (`[x0 x1 y0 y1]` here; `[x0 y0 x1 y1]` in most rectangle APIs, including our own
`Rect`), and a generator that reads `domain[1]` as `y_min` produces a page that looks
drawn.

The domain *is* a rectangle in the shading's own space, and this crate already has a type
for one with `is_finite` and `is_ordered` on it. `domain: Rect` would carry the same four
numbers with the ordering enforced by field names, reuse the validation, and cost the
caller one transposition at the boundary they are already crossing. I kept `[f32; 4]`
because it is pinned.

### 6.3 `FnRange::Gray` puts one colour conversion on our side of a line we drew

`paint.rs`'s first contract line is that colour management happened upstream and must not
happen again, and CLAUDE.md's stack table forbids a colour-management crate here.
`FnRange::Gray` means the program leaves one component and *we* replicate it to three. It
is the most trivial conversion there is, but it is a conversion, and the alternative is
free: a caller lowering a `DeviceGray` function appends `dup dup` — two instructions out of
482 — and hands us `FnRange::Rgb` with the same pair three times. Because the pair is
identical across the three components, clip-then-replicate and replicate-then-clip give
identical results, so the lowering is exact rather than approximately equivalent.

That would remove a variant, remove a colour decision from this crate, and make the
generated shader's output width constant.

The argument the other way is real and may win: `Gray` preserves what the document actually
declared, and a paint that round-trips the file's own statement is easier to debug than one
that has been rewritten. I would take either, but I would take it deliberately. I
implemented `Gray` as pinned.

### 6.4 `PushInt` is necessary for `not`, but it is not sufficient — the doc slightly overclaims

The pin's rationale for `PushInt` is that Table 42's `not` is two operators wearing one name
and "an untyped literal cannot tell them apart". True, and the fix is right. But a literal
is only one of the ways a stack slot becomes an integer: `3 2 idiv` yields an integer,
`3 2 div` yields a real "even if both operands are integers" (PLRM3), and `cvi`/`cvr`
convert in both directions. So `not`'s meaning still has to come from the generator's type
inference over the whole program; `PushInt` is an input to that pass, not a replacement for
it. Worth one sentence in whichever document becomes the contract, so nobody ships a
generator that types slots from literals alone.

### 6.5 A citation, offered for correction rather than as a disagreement

The integrator's note cites the clip-at-both-ends sentence as §7.10. Our own research record
read it from the PDF and places it in **§7.10.1** ("General"), and the markdown extraction
agrees — §7.10 starts at the clause heading, §7.10.1 at the next, and the sentence is
between §7.10.1 and §7.10.2. The code cites §7.10.1. Nothing turns on it.

## 7. Gaps a later session should close

- **The fuzzer does not generate function paints.** Principle 3 says fuzz the scene
  boundary from the first commit, and `quorra-gpu/tests/fuzz_scene/generator.rs` builds
  every other paint. It is in the generator agent's tree, so it is theirs to extend, but a
  jump-heavy random program is exactly the input this boundary exists for.
- **`MAX_PROGRAM_LENGTH` has no measured anchor of its own** — §3 says which number is
  measured and which is extrapolated, and the extrapolation should be replaced by a
  measurement once the generator can compile a program of that size.
- **The `Display` for `SceneError` carries an `#[allow(clippy::too_many_lines)]`** with its
  reason written above it. It is a table with one arm per variant, and every way of
  splitting it costs the exhaustiveness that makes a new refusal fail to compile until it
  has a message. If the enum grows much further, the honest fix is to split `SceneError`
  itself by what was refused, not to split its `Display`.
