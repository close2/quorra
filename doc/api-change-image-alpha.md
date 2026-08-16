# API change — an image's alpha is refused under its own name

For `/home/cl/projects/pdf-viewer`, which consumes this library as a git dependency. One
addition to `SceneError`; **nothing was removed or renamed**, and a caller that does
nothing keeps compiling and keeps drawing the same pixels.

## The change

```rust
SceneError::InvalidImageAlpha { alpha: f32 }
```

`SceneBuilder::image` refused an alpha outside `0..=1` with
`SceneError::InvalidGroupAlpha` — a variant it shared with `SceneBuilder::group`. The
value it named was right and the noun was wrong: a scene may carry an image and no group
at all, and the refusal then tells its reader to go and look at something that is not
there. `SceneBuilder::group` is unchanged and still refuses with `InvalidGroupAlpha`.

The two alphas are the *same parameter* — that is why one predicate still checks both.
ISO 32000-2 §11.6.4.4 ("Constant shape and opacity") defines one nonstroking alpha
constant and says where it applies:

> The nonstroking alpha constant shall also be applied when painting a transparency
> group's results onto its backdrop.

and §11.3.7.2 ("Source shape and opacity") is where the range comes from:

> All of the shape and opacity inputs shall have values in the range 0.0 to 1.0
> (inclusive), with a default value of 1.0.

So the range is the clause's, not a bound of ours. What is ours is the refusal of NaN and
the infinities on top of it: a PDF number is neither, so an alpha that is one means
something upstream went wrong, and §4.7 of the brief says such a value is refused rather
than clamped.

## What the caller must do

**Nothing to compile.** `render-quorra` converts a `SceneError` with `#[from]` into
`QuorraRasterError::Scene` (`crates/render-quorra/src/lib.rs:60`) and matches no variant
of it anywhere — the only mention in `crates/render-quorra/src/scene.rs` is a comment
about `NonIsolatedGroupUnsupported`. Adding a variant is therefore source-compatible for
this caller.

**What changes is a message.** An image drawn with an out-of-range alpha now reports

```
image alpha non-finite or outside 0..=1: 1.5
```

where it used to report `group alpha …`. If any test, log assertion or triage note of
theirs matches on that text, that is the one place to update.

**One thing that is not being decided here.** `SceneError` is not `#[non_exhaustive]`, so
this variant would break a downstream `match` without a `_` arm — theirs has none, but the
next added variant is the same question again. Whether to mark the enum `#[non_exhaustive]`
(which costs them a wildcard arm once, and costs us nothing thereafter) is a decision
neither side can take alone; it is put to them with this bump rather than taken here.

## Where to look

- `crates/quorra-scene/src/error.rs` — both variants, each carrying the offending value,
  with the clauses above quoted in their doc comments.
- `crates/quorra-scene/src/scene/validate.rs` — `constant_alpha_is_valid` (one predicate),
  `check_group_alpha` and `check_image_alpha` (two refusals), and the unit test
  `a_constant_alpha_is_refused_under_the_name_of_what_carried_it`, which drives NaN, both
  infinities and both sides of the interval through both builder calls and requires each
  to come back under its own name. Verified able to fail: with `check_image_alpha`
  returning `InvalidGroupAlpha` — the defect this change removes — the test fails on the
  first hostile value.
