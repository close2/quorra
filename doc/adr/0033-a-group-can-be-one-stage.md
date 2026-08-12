# ADR 0033 — A group can be one stage of §11.4.6

Status: accepted, 2026-08-12. The second half of the caller's `QUORRA_FEEDBACK.md` §14.2,
after ADR 0032 took the first.

## Context

ADR 0025 gave the caller §11.4.6's second stage as two operators on a *fill*, and ADR 0032
let them be used where the clause puts them. §14.2 says that is still not enough for three
of their four refused pages, and the reason is a sentence of the standard rather than a
gap in their translation. ISO 32000-2 §11.3.7.2:

> The shape of a group object shall be the union (as defined in 11.3.7.3, "Result shape
> and opacity") of the shapes of the objects it contains.

A knockout element that is itself a group therefore has a shape no fill can state: it is
the union of the shapes of everything the group marks. Their four pages state exactly
that — a group at alpha ½, a knockout group of one half-opaque fill, a group of two
`Multiply` fills — and `SceneBuilder::group` carried no compositing operator at all, so
neither half of the pair could be written with a group as its source.

## Decision

**`GroupSpec::compose`**, beside `blend`, where §14.2 said a compose would sit.

- `Compose::SrcOver` is the ordinary group and the default every existing scene means.
- `Compose::DestOut` composites the group as §11.4.6's erase: `P' = (1 − f) × P`, with
  `f` the group's own alpha. A caller writes the shape half as the same content drawn
  opaque, which is the only way a group's shape reaches a raster — Table 140's group alpha
  is not carried in a premultiplied texture, and computing it would be a second buffer per
  group for a quantity only this clause reads.
- `Compose::Plus` composites it as the deposit: `P' = P + S`.
- `Compose::Src` is **refused** on a group: an element that replaces the backdrop where it
  marks is what `GroupSpec::knockout` states, and a group asking for both would be asking
  the same question twice.

The composite shader gains the two branches and nothing else — it already computes
§11.3.6 itself rather than leaning on a blend state, so the stages are two early returns
before §11.4.4's interpolation and §11.3.6's formula, which they replace rather than join.
The group's constant alpha, soft mask and clip scale both, because those are the caller's
statements about *this half*.

**Two refusals, both §5's kind.** A staged group carrying a blend mode is refused: the
blend composites the group by §11.3.5, which is the step the pair replaces. A staged group
that is not isolated is refused: §11.4.4 seeds a non-isolated group's buffer with its own
backdrop, so the alpha the erase half reads as a shape would carry that backdrop's alpha
too — a plausible-looking wrong page rather than an error.

## What it buys, measured

`tests/staged_compose.rs`, two overlapping half-opaque wedges as the element — overlapping
because that is where a group's union-of-shapes and its alpha differ most. Worst
premultiplied deviation from `P' = (1 − f) × P + S` over every pixel:

| | deviation |
|---|---|
| the two groups, `DestOut` then `Plus` | **0.77 of 255** (unorm rounding) |
| the same group composited ordinarily | **114.95 of 255** |

## What it costs

**`GroupSpec` gains a field, which breaks every literal construction of it** — thirty-two
in this tree, and the caller's adapter besides. There is no way to add a compositing
operator to a struct a caller builds by hand without one, and `#[non_exhaustive]` would
trade this break for a permanent one at every construction site. The caller pins by
revision and takes releases when they choose.

**`Plus`'s saturation obligation now applies to a group too.** ADR 0025 recorded it as the
one item in the vocabulary whose correctness the builder cannot check: addition alone
drives a premultiplied channel past its alpha, and one mark — or now one group — cannot
tell a library whether the other is coming. Stated in `GroupSpec::compose`'s own
documentation, as it is in `Compose::Plus`'s.

**And no page of theirs moves until their side writes the translation.** Their adapter
refuses `Command::Shaped` before reaching our builder, naming two obstacles; both are now
gone, and the four pages follow when they take it.

## Revisit when

The per-element shape channel is reconsidered. It would answer both halves of §14.2 at
once and remove the saturation obligation, at the cost of a wider instance and a change to
every lane; ADR 0025 chose the operator pair because it was smaller and the caller had no
preference. Two ADRs later the pair has grown a group-level twin, which is the argument
for pricing the other design again rather than a reason to regret this one.
