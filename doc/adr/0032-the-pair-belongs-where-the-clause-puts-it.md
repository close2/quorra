# ADR 0032 — The staged pair belongs where the clause puts it

Status: accepted, 2026-08-12. Corrects ADR 0025's second refusal, which made the operators
it added unusable for the case they were added for. Answers the first half of the caller's
`QUORRA_FEEDBACK.md` §14.2.

## Context

ADR 0025 added `Compose::DestOut` and `Compose::Plus` so a caller can write §11.4.6's
second stage — `P' = (1 − f) × P + S` — as one mark each, for elements whose shape is not
their coverage. It then refused them in two positions on the reading that both "already
*are* that stage by another route, so a staged mark inside them would apply it twice". One
of those positions is a knockout group.

The caller wrote the translation and found what that means: **the only position their
interpreter emits the pair from is the one we refuse.** `pdf_render::Command::Shaped`
carries its own guarantee —

> This command appears only as a direct element of a [`Self::Group`] whose `knockout` is
> set. Outside one the shape is unused

— and that is not an accident of their tree. §11.4.6 is the only clause that uses shape
and opacity apart, which is why ADR 0025 exists at all. An operator that may be used
nowhere it is needed is not a vocabulary; it is a refusal with extra steps.

## What the clause says

ISO 32000-2 §11.4.6, the second step of the per-element computation:

> 𝛼gi = (1 − 𝑓si) × 𝛼gi−1 + 𝑓si × 𝛼t

with the same weighting applied to colour, and the clause's own description of it:
"compute a weighted average of this result with the object's immediate backdrop, **using
the source shape as the weighting factor**".

So a knockout group's per-element rule *is* the staged pair, weighted by that element's
**own** source shape `𝑓si`. An element that states the pair states its own `𝑓si` instead
of having one read off the alpha it happens to be drawn with. It **replaces** the group's
erase for that element; it does not add a second one. ADR 0025's "twice" was a property of
neither the clause nor this tree's encoder, which has always chosen the element's operator
over the enclosing group's style.

The clause also says why such elements exist:

> The separate shape value shall be computed in any group that is subsequently used as an
> element of a knockout group.

## Decision

**`Compose::DestOut` and `Compose::Plus` are accepted inside a knockout group.**
`StagedComposeReason::InsideKnockoutGroup` is deleted rather than left unreachable — a
refusal that cannot happen is a lie in the API — which makes this a breaking change for
anything matching on it. The caller has a test that fails the day we lift it, written for
that purpose.

**The blend-mode refusal stands.** §11.3.5 puts a mark carrying a non-Normal blend into an
implicit one-element group, so the operator would compose that group rather than the
element, which is not where the clause puts it. Nothing in §14.2 asks for it.

## What it buys, measured

`tests/staged_compose.rs`'s new fixture is a knockout group whose first element covers the
target opaquely, followed by one element written both ways. The element is a wedge under
an alpha soft mask, because that is the discriminating case: §11.6.4.3's mask is *opacity*
and §11.6.4.2's shape is geometry, so the element's shape is the wedge and its alpha is
half of it. Worst premultiplied deviation from the clause's line, over every pixel:

| | deviation |
|---|---|
| the staged pair | **0.77 of 255** (unorm rounding) |
| the same element as one soft-masked mark | **108.29 of 255** |

A fixture built from an ordinary solid fill would have shown 0.77 for both, because there
the coverage *is* the shape and the group stages it correctly on its own. That is worth
stating: this change is invisible except where §11.4.6's separate shape value is.

## What it does not fix

**Three of the caller's four refused pages are still refused, and the second half of §14.2
is why.** `SceneBuilder::group`, `stroke` and `image` carry no compositing operator, and
those pages state a `Shaped` whose two halves are *groups* — §11.6.4.2 makes a nested
group's shape the union of its elements', so the shape half cannot be written as a fill.
Only `knockout_smask.pdf` becomes expressible here.

**And no page moves until their side writes the translation.** Their adapter refuses
`Command::Shaped` before it reaches our builder, naming both obstacles; one of the two is
now gone, and the pages follow when they take it.

## Revisit when

The second ask is taken — a `compose` on `GroupSpec`, or the per-element shape channel
that would answer both at once and remove `Plus`'s saturation obligation from the caller.
ADR 0025 chose the operator pair over the shape channel because it was the smaller change
*and* because the caller had no preference; the population of real artwork §14.2 describes
— knockout groups whose elements are groups — is the argument for revisiting that, and it
is a decision about the scene vocabulary rather than about this refusal.
