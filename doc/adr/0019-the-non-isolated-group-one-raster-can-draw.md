# ADR 0019 — The non-isolated group a one-accumulator raster can draw

Status: accepted, 2026-08-11. Decided from the caller's feedback §16, which is a
request rather than a defect report. Widens the brief's §4.4, which said we could
assume every group is isolated.

## Context

`GroupSpec` opened a layer on transparency, drew into it, and composited the result
once. That is ISO 32000-2 §11.4.5's **isolated** group exactly, and it is what the
brief promised would be the only case:

> We decide isolation upstream and only emit a group where the computation is provably
> the isolated one, so **you can assume every group is isolated**.

§11.4.4 defines the other initial backdrop. The difference is visible only where an
element *blends* — with every element painting Normal the backdrop is composited in and
removed again exactly, which is why the promise held for as long as it did. It stopped
holding on four documents of the caller's corpus (`bug1755507.pdf`,
`issue12798_page1_reduced.pdf`, `issue13520.pdf`, `issue18032.pdf`), every one of them
Illustrator or InDesign artwork: nested groups under `/Luminosity` soft masks with
`Screen` and `Multiply` elements. Those four had been counted as *agreeing* between
their two backends, because both substituted the same wrong initial backdrop; when
their CPU backend learned §11.4.4 they became refusals, and the page fell back to the
processor with a note naming us.

**What made this look impossible is NOTE 4.** §11.4.4's Result step removes the
backdrop's contribution by dividing by Table 140's *group alpha* — "the accumulated
source alphas of group elements E1 to Ei, **excluding the initial backdrop**" — which
is not the alpha a premultiplied raster holds, and the clause's own advice is to keep a
second set of accumulators for it:

> For shape and alpha, backdrop removal can be accomplished by maintaining two sets of
> variables to hold the accumulated values.

## The claim this rests on

**The quantity the removal divides out is multiplied straight back in.** The group's
computed alpha *is* Table 140's group alpha, so when §11.3.6 composites the group's
result with the same backdrop under the **Normal** blend function, the two cancel. With
`B` the backdrop, `E(B)` the elements composited onto it — both premultiplied — and `w`
the group's constant alpha times its soft mask, clip coverage and clip residue at the
pixel:

```text
result = (1 − w) × B + w × E(B)
```

`tests/non_isolated_groups.rs` is a transcription of §11.4.4's initialisation,
recurrence and Result step, of §11.4.5's `a0 = 0.0` alteration and of §11.3.6's
composite, written from the standard and independent of `composite.wgsl`. Over 200 000
fixed pseudo-random configurations — backdrop colour and alpha, group alpha, one to
four elements with random alphas and blend modes:

| configuration | worst deviation from the clause |
|---|---|
| non-isolated, group blend **Normal** | **5.6 × 10⁻¹⁶** — exact in double precision |
| non-isolated, group blend Multiply / Screen / Difference | 0.77 / 0.81 / 0.91 of full scale |
| the same construction applied to an **isolated** group | 0.76 of full scale |

The first row is the decision. The second and third are why the conditions below are
refusals rather than documentation, and why the flag is not decoration: applied to the
wrong kind of group the construction is a visibly different picture.

The caller derived the same identity independently (their ADR 0237) and measured the
same 5.6 × 10⁻¹⁶. That agreement is evidence about our reading of the clause, not its
source: the transcription in our test is ours, and it is what the test asserts against.

## Decision

### 1. One flag on `GroupSpec`, defaulting to what exists

`GroupSpec::isolated: bool` — Table 145's `/I`, the one entry the vocabulary was
missing. `true` is §11.4.5 and is every group a caller emits today.

### 2. The layer is seeded, and the composite is an interpolation

A non-isolated child's first texture is filled with the parent's accumulated content
before the child renders (`Executor::seed_layer`), and the child's passes load rather
than clear. `composite.wgsl` then branches on a flag in its existing parameter padding:
`mix(b, s, w)` instead of §11.3.6. Two halves of one change — the caller asked whether
we could do the buffer half, and the answer is that neither half means anything alone.

**A blit rather than `copy_texture_to_texture`**, for two reasons: it needs no
`COPY_SRC`/`COPY_DST` usage on every internal texture in every frame, and it is
scissored by the same rule as every other pass, so a damage-patched frame (ADR 0012)
seeds only the pixels it is allowed to touch. `blit.wgsl` is a `textureLoad` and a
store with no blending, so between two `Rgba8Unorm` textures it is exact.

**A uniform flag rather than a second pipeline**, because §7 counts pipelines on the
critical path and this one would compile on first use of a rare feature. The branch is
uniform across the draw, so it costs no divergence.

### 3. The three conditions are refused at the builder

`SceneError::NonIsolatedGroupUnsupported` carries a `NonIsolatedReason`:
`GroupBlendNotNormal`, `KnockoutGroup`, `InsideKnockoutGroup`. Checked before the
group's body is built, so a refusal costs nothing that was constructed inside it, and
the builder stays usable afterwards.

The third needs the builder to know what a group is *nested in*, which it did not
track: `OpenFrame` gains `inside_knockout`, and a soft mask's body starts a fresh stack
because §11.6.5 renders the mask group on its own — a knockout group around the `mask()`
call is not above the mask's content.

## What it costs

Measured on this machine at 1191×1684, device time from timestamp queries, best of
nine, one page rectangle plus N groups each holding a Multiply element:

| | RADV, 1 group | RADV, 4 groups | llvmpipe, 1 group | llvmpipe, 4 groups |
|---|---|---|---|---|
| isolated, before this change | 0.384 ms | 1.102 ms | 10.66 ms | 37.4 ms |
| isolated, after | 0.386 ms | 1.132 ms | 10.33 ms | 37.9 ms |
| **non-isolated** | **0.496 ms** | **1.391 ms** | **11.79 ms** | **40.2 ms** |

So a seed is **~0.11 ms** at page size on RADV and ~1.4 ms on the software rasteriser,
and **the isolated path pays nothing measurable** for the branch — the before/after rows
differ by less than the run-to-run spread.

**No new allocation and no change to the frame budget.** The seed writes into the
layer pair the group already had, so `internal_texture_bytes` is untouched and a page
that refused before refuses identically. (A scene of eight such groups at page size
still exceeds the default 256 MiB budget — 273 MB of layer pairs — and is refused by
name, which is the pre-existing pricing working, not a new limit.)

**Cross-adapter identity is unchanged, which is to say still "no" (ADR 0006).** On one
scene through both lanes, RADV and llvmpipe differ by at most 1 unorm step — the same
worst case, from the same implementation-defined float→unorm store, that the isolated
lane shows on the same scene.

## Held against the caller's corpus

Their 956-page gate, run here on RADV from a copy of their tree with `[patch]` pointed
at this working tree, once against `HEAD` and once against this change — the only other
edit being the one line that passes the flag through instead of refusing:

| | agree | differ | refused | not comparable |
|---|---|---|---|---|
| before | 910 | 35 | 11 | 18 |
| after | **913** | 35 | **8** | 18 |

**Three documents moved from refused to agreeing and nothing else moved at all.**
`bug1755507.pdf`, `issue13520.pdf` and `issue18032.pdf` — Illustrator and InDesign
artwork — now agree with their CPU backend's independent implementation of §11.4.4,
within the gate's own tolerance, and the `differ` list is identical page for page. Two
transcriptions of one clause, in two languages, on real files.

The fourth document §16 named, `issue12798_page1_reduced.pdf`, is still refused — and
now says why it really is: "a page composited in a four-component blending colour
space (§11.4.7)", which is their feedback §17 and nothing to do with isolation. Its
§11.4.4 refusal was in front of that one.

## What is deliberately not done

- **The seed is the whole target, not the group's clip rectangle.** Outside the clip
  `w` is zero and the result is the backdrop regardless, so a clip-bounded seed would
  be correct and cheaper. It is an optimisation with a measurement attached to it, and
  the measurement above says the thing being optimised is 0.11 ms; it waits for a page
  that makes it matter.
- **No `Compose::Src` on groups.** The caller observed that the interpolation is what
  `Compose::Src` already describes, and offered that as an alternative route. A flag
  that names §11.4.4 says what it means at the call site; an operator that happens to
  compute the same thing would leave the seeding half unexplained.
- **Nothing about knockout is widened.** §11.4.6's group keeps exactly the model
  ADR 0010 gave it.

## Revisit when

A page arrives whose non-isolated group is also a knockout group, or sits inside one.
That is the case §11.4.4's two-accumulator advice actually exists for, and drawing it
would mean carrying the group alpha as a second channel — a real design change, and one
the refusal makes visible instead of hiding. The caller reports the groups it cannot
send us for the same reason.
