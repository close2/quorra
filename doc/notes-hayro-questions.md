# Four questions from the caller's hayro reading, answered here

The caller wrote `/home/cl/projects/pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` on
2026-08-16, after reading all 167 issues on `LaurenzV/hayro`. It says of itself that it is
a reading list and **not** a defect list — "Nothing here is a claim that quorra has any of
these problems" — and that the reusable part of any entry is *why* something is right.

This file is that half, for the four entries that are concretely checkable against this
tree. Each question ends in a gate, including the two whose answer is "settled upstream",
because a gate on an assumption we depend on is worth more than a gate on a behaviour we
control: it fails when the assumption stops being true, which is the only way either side
learns.

**No behaviour changed in this round.** Four test files are new and no source file is
edited. Two of the four answers name a divergence that is deliberate and inherited, and
both are now written down on this side of the boundary as well as on theirs.

| question | answer | gate |
|---|---|---|
| 1. a leading degenerate `MoveTo` — do we deposit a cap? | **No**, under any cap style, and §8.5.3.2 says that is right | `crates/quorra-gpu/tests/degenerate_subpaths.rs` |
| 2. is a "no ink" paint a no-op in the compositor? | **Yes where its shape is zero**, in all four group kinds, byte for byte — and correctly **not** where only its opacity is zero and it is inside a knockout group | `crates/quorra-gpu/tests/no_ink.rs` |
| 3. a stencil mask on a grid other than its image's | **Cannot reach us**; resolved in `pdf-model`. The mismatch we *do* have is a soft mask, and it is sampled on the device grid | `crates/quorra-gpu/tests/mask_grid.rs` |
| 4. `mul_add` on a hot CPU path | **Not on one.** Two `src/` sites, both once-a-frame; the target has no FMA | `crates/quorra-gpu/tests/mul_add_hazard.rs` |

**Both adapters.** The three gates that open a device were run on llvmpipe — the suite's
pinned default, and `tests/common/headless.rs` says why it is pinned — and on RADV, by
temporarily letting that fixture read `QUORRA_ADAPTER`. All fifteen pass on both, and the
override was reverted; the fourth file opens no device.

---

## 1. A leading degenerate `MoveTo` (their §5, hayro #296)

### The clause answers it in one sentence, and it is not the sentence the question expects

Their note reasons from §8.4.3.3 — caps are applied "at both ends of open subpaths", and a
subpath of one point is an open subpath — and concludes that a round or square cap "can
deposit a dot at the origin". §8.4.3.3 does say that, but §8.5.3.2's last paragraph is
more specific and it overrides:

> If a subpath is degenerate (consists of a single-point closed path or of two or more
> points at the same coordinates), the S operator shall paint it only if round line caps
> have been specified, producing a filled circle centred at the single point. If butt or
> projecting square line caps have been specified, S shall produce no output, because the
> orientation of the caps would be indeterminate. This rule shall apply only to
> zero-length subpaths of the path being stroked, and not to zero-length dashes in a dash
> pattern of a non-degenerate subpath. In the latter case, the line caps shall always be
> painted, since their orientation is determined by the direction of the underlying path
> except in the case of a degenerate subpath. **A single-point open subpath (specified by
> a trailing m operator) shall produce no output.**

Three distinct shapes, three answers:

| shape | §8.5.3.2 |
|---|---|
| a bare `m` — a single-point **open** subpath | no output, **under every cap** |
| a single-point **closed** path (`m h`) | a filled circle under round caps; nothing under butt or square |
| two or more points at the same coordinates | the same as the closed case |

hayro #296's spurious leading `MoveTo` is the first row. So the clause's answer to the
question is unconditional: **no dot, whatever the cap.** The parenthetical "specified by a
trailing m operator" describes how such a subpath usually arises; the normative subject is
the single-point open subpath, and a spurious leading one is that.

### What this tree does

Nothing, which is right. `raster::flatten` (`crates/quorra-gpu/src/raster.rs`) keeps a
subpath only when it has more than one point, so a lone `MoveTo` is dropped before caps are
considered; `raster::stroke_polylines` declines a polyline that dedupes to one point, which
catches the coincident-points shape. Both lanes share that one expansion — the inline path
and `encode/parallel.rs`'s threaded one — so there is one answer, not two.

### The part that is a divergence, and it is §4.5's

The second and third rows above are a **disc under round caps**, and this tree paints
nothing there either. That is not §8.5.3.2; it is `RENDER_LIBRARY.md` §4.5 in force:

> degenerate subpaths | §8.5.3.2 — a zero-length subpath is a dot under round caps and
> *nothing* under butt or square | we pre-split them; draw what you are given

and the caller's implementation of it is `pdf-render/src/degenerate.rs`, which is worth
reading because it settles the question with measurements rather than by preference. Its
`split_degenerate` separates a stroked path into `stroked` and `dots`, and `dots` is
documented as "the circles §8.5.3.2 asks for, to be **filled** with the stroking paint".
Its module comment carries the measurement that made it necessary — at width 10 on a
100-unit page, `m h` under round caps is 77.5 units of ink in `tiny-skia`, 0.0 in Vello and
78.5 in the clause; under square caps it is 100.0, 0.0 and *nothing*. So the disc reaches
us as a `Command::Fill` of its own geometry, and a `Command::Stroke` carrying a degenerate
subpath is a thing the contract says cannot arrive.

**Neither side may quietly change its mind about that**, which is why the gate states our
half rather than leaving it to be inferred: if the caller stopped splitting, or if this
side grew its own disc, a round-cap dot would be lost or doubled and no test either side
had would see it. Their mirror test is
`render-cpu/tests/degenerate_subpath.rs::a_single_point_closed_path_is_a_disc_under_round_caps`;
ours is
`degenerate_subpaths.rs::a_single_point_closed_path_draws_nothing_here_because_4_5_places_the_disc_upstream`,
and the two are consistent because they are about different sides of one split.

### The gate

`crates/quorra-gpu/tests/degenerate_subpaths.rs`, six tests, all through `SceneBuilder` with
the outline built by hand:

- a spurious `MoveTo` **before** the real subpath is byte-identical to the same stroke
  without it, under butt, round and square;
- the same for a spurious `MoveTo` **after** it;
- a path that is *only* a `MoveTo` draws nothing at all;
- `m h` draws nothing (the §4.5 divergence, stated as one);
- `m` + a `LineTo` back to the same point draws nothing (the same, by a second route: this
  one survives `flatten` and is stopped by `stroke_polylines`);
- and a control that the caps this file asserts are absent are ones the target can see —
  the two round caps of an ordinary line are a disc of 113 pixels and the two square caps
  are 144, both checked against §8.4.3.3's Table 53 arithmetic.

**Verified able to fail.** With `raster::flatten`'s one-point guard relaxed and
`stroke_polylines` made to deposit a pair of caps at a lone point — hayro #296's exact
hazard — five of the six fail and the control passes.

