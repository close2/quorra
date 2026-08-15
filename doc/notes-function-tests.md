# Two untested paths in the type 4 lane, closed — and one behaviour pinned

Written 2026-08-15, closing the two entries `doc/notes-function-wiring.md` left open (§4.5's
last bullet and §6's "Retained-encode coverage") and the third thing that note recorded and
deliberately did not change (§2.3).

Three new test files, eleven tests, no change to `src/` — **no defect was found in the lane
itself.** What was found is a *fixture* weakness worth writing down, and it is in §1.4.

| file | tests | what it is about |
|---|---:|---|
| `crates/quorra-gpu/tests/function_knockout.rs` | 3 | a function paint as an element of a §11.4.6 knockout group |
| `crates/quorra-gpu/tests/function_retained.rs` | 6 | a function paint through ADR 0048's `RetainedScene` |
| `crates/quorra-gpu/tests/function_order_typecheck.rs` | 2 | what `gt`/`ge`/`lt`/`le` answer for a boolean, pinned rather than defended |

---

## 1. The knockout group over a function paint

### 1.1 The line every expectation is derived from

ISO 32000-2 §11.4.6 composites **every** element of a knockout group with the group's
*initial* backdrop rather than with the accumulated result, weighting by the element's own
source shape:

> 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t

The group in every fixture is isolated, so §11.4.5 makes the initial backdrop transparent,
and §11.3.6's formula against `ab = 0` collapses to `co = as·Cs`, `ao = as`. So the clause's
line, premultiplied and per channel, is

```text
P' = (1 − f) × P + S
```

— `P` the group before the element, `f` its shape (§11.6.4.2: geometry, so the mark's
coverage under its clip), `S` its own premultiplied deposit. That is the same statement
`tests/knockout_blend.rs` holds the fill and stroke arms to and the same one ADR 0025
measured on a wedge; it is derived again in the new file rather than shared, because the
lane it is about is different.

### 1.2 What each test asserts, and its clause

| test | asserts | from |
|---|---|---|
| `a_function_fill_inside_a_knockout_group_takes_the_replacement` | a `Multiply`-blended function fill of the wedge inside a knockout group is within **0.70 of 255** of `P' = (1 − f)P + S`; the same element in an **ordinary** group is **167 of 255** away from it | §11.4.6 for the replacement, §11.3.6 with `ab = 0` for why the blend function disappears, §11.3.5 for the control |
| `a_point_the_domain_leaves_unpainted_knocks_nothing_out` | outside the transformed domain rectangle with no `Background`, the group holds **exactly** what it held before the element — byte for byte | §8.7.4.5.2 ("such points shall be left unpainted") ∧ §11.6.4.2 (shape is geometry): an unpainted point has no shape, so §11.4.6's weight is 0 |
| `a_background_alpha_knocks_out_at_full_shape` | a `Background` of alpha ½ has shape 1 at opacity ½: the knockout replaces the group with the background's own premultiplied colour **and its alpha**, where an ordinary group leaves the opaque cover under it at alpha 1 | §11.4.6's own sentence on why shape is kept separate from alpha, quoted by ADR 0025; §11.4.7.2 for the distinction |

The fixture is the diagonal-edge wedge for the first test — ADR 0025's own instrument, and
§4.1 of the brief's reason: axis-aligned rectangles would agree while being wrong. The
`partial > 30` guard asserts the fixture still has partially covered pixels, so a shape that
silently stopped producing them cannot leave the test passing for the wrong reason. The
second and third tests use a full-target rectangle instead, which takes the **rect-hinted**
placement where the wedge takes the **rasterised-coverage** one — so both of the lane's two
quad placements are drawn under knockout.

### 1.3 Each gate was verified able to fail

Not by inspection. Each was broken in a working copy, run, and restored:

| break | result |
|---|---|
| `compose::function` forced to `[Some(Style::Over), None]` instead of `Style::of` | `a_background_alpha_knocks_out_at_full_shape` **fails** (expected 0.1, got 0.553); the other two pass |
| `fs_shape`'s `if straight.a <= 0.0 { return vec4f(0.0); }` deleted | `a_point_the_domain_leaves_unpainted_knocks_nothing_out` **fails**, and it fails as predicted — `[0, 0, 0, 0]` where the opaque cover `[230, 51, 26, 255]` belongs, a transparent hole rather than a shade |
| `Style::of` changed for every lane at once | the frame panics in `compose/draw.rs`'s pass table before reaching an assertion, so it is not an instrument for this and was not used as one |

### 1.4 The one thing worth knowing: an opaque element cannot hold the pair

**For a source of alpha 1, §11.4.6's replacement and an ordinary premultiplied over-composite
are the same arithmetic** — `(1 − f)P + f·Cs` either way. A function paint is opaque wherever
it marks inside its domain, so a knockout fixture built only on that paint separates §11.4.6
from §11.3.5 (which is what the first test is for) and would **not** notice a lane drawing one
`Over` pass where ADR 0010's erase/add pair belongs. The break above proves it: two of the
three tests stayed green under exactly that fault.

What makes the pair observable is a source whose alpha is below one at full shape, and for
this paint the only way to state one is §8.7.4.5.2's `Background`. That is why
`a_background_alpha_knocks_out_at_full_shape` exists and why it carries the sentence naming
itself as the test that holds the pair. Any future knockout fixture over an opaque paint
should be read the same way.

---

## 2. The function paint through a retained frame

ADR 0048's `RetainedScene` replays an encode when `EncodeKey` still holds. Two things this
paint has that no other lane does: a `FunctionId` in an op, and a **generated pipeline** keyed
by the program's content hash and dropped with the last resident program naming it.

| test | asserts | why it is not obvious |
|---|---|---|
| `a_replayed_function_page_is_the_page_that_was_encoded` | immediate, retained-and-encoded and retained-and-replayed frames of one page are the same bytes, and the frame after that too | the page draws the rect-hinted **and** the coverage-tile placement, so a replay that dropped the scratch sheet fails half of it |
| `a_replayed_frame_compiles_no_shader` | the first frame names one `"function shader compile (first use)"` phase for two placements of one program; the replay names none | the pipeline belongs to the device, not to the encode |
| `a_second_distinct_program_compiles_a_second_shader` | three placements, two distinct programs and a **third id over the first's instructions**, compile exactly **two** shaders | the cache keys on the program's content, not on the id; a test using only distinct programs could not see a key that was too coarse |
| `an_unrelated_upload_replays_and_an_unrelated_release_re_encodes` | uploading a program the scene never names **replays**; releasing that same program **re-encodes** | this is `EncodeKey::resource_generation` measured in both directions, on a program neither frame draws — so what is observed is the counter and not the scene |
| `a_released_program_re_encodes_and_the_refusal_stands` | after releasing the program the encode names, the frame is `Err(UnknownFunction)` on every attempt and the handle holds nothing | principle 6: the replay must not mask the refusal |
| `a_re_uploaded_program_is_a_new_id_and_draws_the_same_page` | the same instructions re-uploaded get a **new** id, the old handle is still refused, a scene naming the new id draws the same page — and pays for the shader again | the release reached the pipeline store (`forget_program`); a leaked pipeline would report zero compiles here |

### 2.1 `resource_generation` does move, and that was measured

The task asked for it to be checked rather than assumed. `ResourceStore::release` bumps the
counter in a single tail shared by all five id spaces, so a function release moves it by
construction — but "by construction" is what the atlas generation was believed to be too
(ADR 0050). So it was broken and re-run: with the bump suppressed for
`ResourceId::Function` only, **`an_unrelated_upload_replays_and_an_unrelated_release_re_encodes`
and `a_released_program_re_encodes_and_the_refusal_stands` both fail**, and the other four
tests pass. The counter moves, and two tests now say so.

Note the second-line defence the experiment also exposed, because it is worth a reader's
attention: even a stale replay does not *draw* a released program, because
`Executor::function_pipelines` resolves the analysis at draw time and refuses by name there.
So under the break the released-program test still saw its `Err(UnknownFunction)` — and
failed on the *next* assertion, `!retained.holds_encode()`: a replay had put the stale encode
back, and the handle was reporting that it still held one. That is why the test asserts the
handle's state and not only the error, and it is a small demonstration that "the frame was
refused" is a weaker claim than "nothing stale survived the refusal".

---

## 3. `gt`, `ge`, `lt`, `le` on a boolean: pinned, not endorsed

`doc/notes-function-wiring.md` §2.3 records that PLRM3 makes these a `typecheck` on a boolean
operand and that we compare numerically instead, and records why that is deliberately left
alone: it is the same shape as every other guarded error in `function_ops.wgsl`, and ADR 0053
§3.2 has the guard value open as a contract question with the caller. **That behaviour is
unchanged.**

What was missing is that nothing asserted it. The conformance corpus states these cases as
`Expectation::Error`, which means it asserts no value for them, so today's answers were
reachable by no test at all and could have moved silently.
`tests/function_order_typecheck.rs` pins them, through the compute harness at full `f32`
precision, using the corpus's own `{1}{0} ifelse` idiom (a comparison leaves a boolean, and
§7.10.5.3 forbids a boolean output):

| program | today | ISO 32000-2 / PLRM3 |
|---|---:|---|
| `true false gt` | 1 | `typecheck` |
| `true false lt` | 0 | `typecheck` |
| `true false ge` | 1 | `typecheck` |
| `true false le` | 0 | `typecheck` |
| `false true gt` | 0 | `typecheck` |
| `true true ge` | 1 | `typecheck` |
| `true true gt` | 0 | `typecheck` |
| `true 1 ge` | 1 | `typecheck` |
| `true 1 le` | 1 | `typecheck` |
| `true 1 eq` | 0 | **derived**: PLRM3's `eq` — "Simple objects are equal if their types and values are the same" |
| `true 1 ne` | 1 | **derived**, the negation of the above |

The last two rows are in the file on purpose: they are the ones a clause decides, and putting
them beside the nine that nothing decides is what shows the cost of the hold — today a
program can conclude that `true` is both at least and at most `1` while being unequal to it.
The file's header says in its first paragraph that nothing in it is derived from the
specification and none of it may be cited as though it were.

---

## 4. What holds on which adapter

Everything below ran on **both** adapters of this machine, by name —
`AMD Radeon 890M Graphics (RADV STRIX1)` and `llvmpipe (LLVM 22.1.8, 256 bits)` — selected
with `QUORRA_ADAPTER`, and every message in the three files names the adapter it ran on
because ADR 0053 promises no cross-adapter identity for this paint.

- **Full suite, default adapter: 396 passed, 0 failed** (the floor was 385; eleven tests are
  new). `clippy --all-targets --all-features -D warnings` clean, `cargo fmt --check` clean.
- **Full suite, `QUORRA_ADAPTER=RADV`: 0 failed.**
- The knockout measurement is **identical on the two adapters**: worst knockout deviation
  0.70 of 255, worst ordinary-group deviation 167.00, on both. That is an observation about
  three small fixtures, not a promise — ADR 0053's consequence stands.
- The retained tests are adapter-independent by construction: `EncodeSource` is decided by
  `EncodeKey` before any adapter is asked anything.

## 5. What is still not tested here

- **A knockout group over a function paint under a soft mask**, where §11.6.4.3's opacity and
  §11.6.4.2's shape differ for a second reason. `fs_shape` weights by `base_weight`, which
  includes the mask, and whether that is the right reading of §11.6.4.2 is a question ADR
  0025 answered for the other lanes and nothing re-asks here.
- **§11.4.6's two stages by name** (`Compose::DestOut` / `Compose::Plus`, ADR 0025) over a
  function paint. `Style::of` maps them, and the builder refuses a staged mark *inside* a
  knockout group, so the combination this file draws and the one that ADR names are disjoint;
  a staged function fill outside a knockout group has no test.
- **A retained frame whose function paint is inside a group or under a clip residue.** The
  page here is flat; `retained_frame.rs`'s artwork page carries the layer plan and the
  composite for the other lanes, and no page carries both.
