# Every issue the caller's hayro reading list names, and what gates it here

`/home/cl/projects/pdf-viewer/doc/HAYRO_ISSUES_FOR_QUORRA.md` (written 2026-08-16) selects, from
167 issues on `LaurenzV/hayro`, the subset that touches what quorra owns. It is explicitly **not**
a defect list against us — "Nothing here is a claim that quorra has any of these problems" — and
this file is not a defect list either. It is the **coverage map**: one row per issue, what the
question is *for us*, and the gate that answers it.

**The standing instruction that produced it** (project owner, 2026-08-17): *make a unit test for
every possible issue mentioned in the file, even where we already do it right, where that is
easily possible.* A written argument that we are right decays; a gate does not. So a row whose
answer is "we already do this" is still owed a test, and a row whose answer is "that is settled
upstream" is owed a test **of our boundary** — because the thing that will break it is a future
widening on our side, and the assumption is ours to hold whether or not the behaviour is.

Rows marked **not ours** carry the reason. Rows marked **not testable** are method lessons rather
than behaviours, and are recorded so nobody re-derives the judgement.

## §1 — the rasteriser panics (`vello_cpu`'s rather than hayro's)

The general question, quoted, because it is principle 6 stated from outside this tree:

> If quorra has an equivalent ceiling anywhere in strip generation, the thing to check is not
> whether it can be raised but whether crossing it returns rather than aborts.

| their issue | the question for us | gate |
|---|---|---|
| #717 — 197 M flattened segments trips a hard assertion | does every ceiling a document can reach return an `Err` naming the limit? | `doc/notes-ceilings-audit.md` Q1 |
| #373 — SIMD flatten reads past its scratch tail | do we round any count up to a lane/block width, and is the tail padded? Invisible to every test whose length is a multiple of the width | Q3 |
| #646 — `attempt to add with overflow`; release **wraps** | every `#[allow(clippy::arithmetic_side_effects)]`: is the no-overflow argument written down and right, and does our release profile wrap or panic? | Q2 |
| #351/#352/#357 — a `/MediaBox` rounding to zero pixels in an axis | is a zero-sized viewport, target, layer, mask, tile or atlas caught at the top or indexed at the bottom? | Q4 |
| #40/#8/#63 — a panic at scale 2.0 and not at 1.0 | *"a defect that only appears above 1× is one that a test suite rendering everything at 1× cannot see"* — what scales does our own suite render at? | Q5 |

## §2 — conflation and thin marks

| their issue | the question for us | gate |
|---|---|---|
| #104 — thin strokes heavier than mupdf's, closed as "just conflation artifacts" | §10.7.4 decides whether a device pixel is painted. Does a mark **thinner than a device pixel** get coverage proportional to its width on both lanes, or does antialiasing decide? Their standing ask is `doc/QUORRA_HAIRLINE_MARKS.md` | **open** |
| #1023 — stroke weight regression bisected to one commit | is a stroke of a stated device width exactly that wide, at several widths and scales? | **open** |

The caller's position is the load-bearing part: they do **not** treat conflation as a fact of life,
and `pdf-render`'s `sub_pixel.rs` substitutes a one-pixel band at proportional coverage for a mark
thinner than a pixel. If our two lanes ever disagree with each other or with theirs on a hairline,
this is the vocabulary.

## §3 — shadings

| their issue | the question for us | gate |
|---|---|---|
| #3 — Coons/tensor patches tessellated at a **fixed** grid, triangles seaming | integration note 5: a mesh arrives **pre-rasterised** and we never re-triangulate. Testable as exactly that | **open** |
| #551 — a shading baked at a fixed low resolution regardless of output size | ADR 0053 evaluates a §7.10.5 program **on the device**, so a zoom re-bakes nothing. Testable as resolution independence | **open** |
| #41 — a gradient with a `stop-opacity` ramp fails to process | a shading carrying a transparency component | **open** |
| #102 — a gradient as the fill of **text** | a shading paint on a glyph-lane mark | **open** |
| #968 — a gradient **clipped incorrectly on a stroke** | §8.7.4.3 puts a shading's coordinates in the space of the page at the time its parent content stream began, *not* the space in force when the paint is used. "Easy to get wrong by one matrix" — and the two spaces compose in a defined order | **open, highest value in §3** |
| #394 — one line, a test-case name, label `rendering-quality` | nothing concrete to test | not testable |

## §4 — images

| their issue | the question for us | gate |
|---|---|---|
| #1315 — a stencil mask at a **different resolution** from its image, 5× slower | **§8.9.6.3**, not §8.9.6.4 — see the corrections below; it allows it and scanners produce it constantly, and the caller says *"the second is the one that is quorra's"* | `tests/mask_grid.rs` |
| #2 — downsampling a mask larger than its image | same mismatched grid, quality side | `tests/mask_grid.rs` |
| #1319 — per-sample `BitReader` + f32 interpolate to expand a 1-bit mask | decoding is the caller's; pixels reach us decoded | not ours |
| #1310 — overriding `/Interpolate` globally | integration note 1: the **resolved** filter decision arrives on the image *command*. So the gate is that we honour it and never substitute our own | **open** |
| #494 — `/Width`,`/Height` disagreeing with the JPEG codestream | §7.4.8 vs §7.4.9; resolved upstream, we receive pixels | not ours |

## §5 — glyphs

| their issue | the question for us | gate |
|---|---|---|
| #296 — every glyph outline began with a spurious `MoveTo((0,0))` | do we deposit a dot? **No, and unconditionally** — see the corrections below | `tests/degenerate_subpaths.rs` |
| #23 — bitmap glyph strikes resampled | font loading is §9's non-goal; outlines reach us already | not ours |
| #6 — font fallback | not our layer | not ours |

## §6 — colour on a device

| their issue | the question for us | gate |
|---|---|---|
| #4 — `/All` and `/None` colourants | `/None` "must composite as though it were never issued". Colour is not ours (integration note 6), so the question that **is** ours: does a mark contributing nothing leave the target byte-identical, including inside a knockout group where §11.4.6 *replaces*? | `tests/no_ink.rs` |
| #630 — `mul_add` falling into libc software emulation without FMA | do we call `f32::mul_add` on a hot CPU path? | `tests/mul_add_hazard.rs` |
| #205/#235/#355/#390 — the ICC engine panicking on a document's profile | an `ICCBased` profile is attacker-supplied data evaluated on the rendering path. **We reach no CMS at all** — `deny.toml` forbids a colour-management crate by design. Testable as the dependency assertion | **open** |
| #60 — `u16`/`f16`/`f32` channels and dithering | ADR 0010 settled rgba8 layers. Their question underneath is banding: *"what an 8-bit raster does to a mark whose ink is under one of its levels"* | **open** |

## §7 — the renderer/host boundary

| their issue | the question for us | gate |
|---|---|---|
| #1316 — expose the thread count through settings | `Options::encode_threads` exists (ADR 0054), and its determinism across thread counts is gated. Two independent embedders asking for this is a signal the knob belongs where we put it | gated by ADR 0054's fixture; **check the nested case** |
| #1343 — concurrent object resolution silently yielding nulls | not our layer, but `Scene: Send + Sync` and cheap to clone **is** ours, and is statically asserted (M2) | asserted; **confirm the assertion still exists** |
| #821 — compositing PDF content into a host's scene as drawing commands | this is what the presenter does (ADR 0056); the maintainer's hesitation — "vello doesn't support everything that is needed for correct rendering (for example masks)" — is the trade we make in the opposite direction | design, gated by `examples/present_thread/` |
| #1052 — cooperative cancellation | we have none. The caller solved the *decode* leaf structurally (a confined process); cancelling the **rasteriser** is the open frame-level question | design, not testable today |
| #1345 — reading only relevant parts of the file | not our layer | not ours |

## §8 — not defects, worth reading

| their entry | what it is | |
|---|---|---|
| #1195 — a profile taken at the wrong scale pointed at `memset`/`memcpy`; corrected, 65 % was interpretation | the same finding CLAUDE.md states as a standing rule, reached independently. **"A benchmark run at the wrong scale points at whatever scales with area"** — that is a trap, not a test | not testable |
| #1188 — a whole-repository LLM review posted as an issue | one real finding in a long list, and it took reading the code to know which. The reason every finding in our own rounds must name the document-derived input that reaches it | not testable |

## Two corrections for the owner to carry back to their document

Both checked against the sponsored EC3 text in this tree before being written here, and both
leave the caller's substantive point standing — each is a citation, not a reading.

1. **Their §4 cites §8.9.6.4 for a mask on a grid of its own.** §8.9.6.4 is *Colour key masking*.
   The sentence they want is **§8.9.6.3** *Explicit masking*: "The base image and the image mask
   need not have the same resolution ( Width and Height values), but since all images shall be
   defined on the unit square in user space, their boundaries on the page will coincide; that is,
   they will overlay each other."

2. **Their §5 reasons from §8.4.3.3 that a leading degenerate `MoveTo` "can deposit a dot at the
   origin" under round or square caps.** §8.5.3.2 overrides that, and its last sentence carries no
   cap condition where the two above it do: "A single-point open subpath (specified by a trailing
   m operator) shall produce no output." The cap-dependent rule — round caps produce a filled
   circle, butt and projecting square produce nothing "because the orientation of the caps would
   be indeterminate" — is for a *closed* single-point path or two or more coincident points, not
   for a bare `m`. So the answer to their question is no dot, under every cap style.

3. **Their §3 cites §8.7.4.3 for a shading's coordinate space.** §8.7.4.3 is *Shading
   dictionaries*, and its NOTE 2 only names the target space. The rule is **§8.7.2**: "Changes
   to the page's transformation matrix that occur within the page's content stream, such as
   rotation and scaling, have no effect on the pattern; it maintains its original relationship
   to the page no matter where on the page it is used." And **§8.7.4.1** states it for the very
   operators their #968 and #102 are about: "…painting operators such as f (fill), S (stroke),
   Tj (show text) … When a shading is used in this way, the geometry of the gradient fill is
   independent of that of the object being painted." Their substantive point stands unchanged.

## What is open, in the order it is worth doing

1. **The two coverage lanes disagree about whether a thin mark is *there*** (#104) — not a
   citation and not a gap in a test, but a scan-conversion decision with a cost either way.
   See below; it wants an ADR and the caller's view.
2. **`/Interpolate` honoured, never overridden** (#1310) — integration note 1's whole point.
3. **No CMS is reachable** (#205 family) — a dependency assertion, cheap.
4. **Banding under one 8-bit level** (#60).
5. **`encode_threads` nested, and `Scene: Send + Sync` still asserted** (#1316, #1343).

**Closed 2026-08-17**: §8.7.2's coordinate space on a fill, a stroke and a glyph-sized outline
(`tests/shading_space.rs`, 7 tests, three drawings of the same device pixels required identical
to the byte); a mesh reproduced texel-for-texel and unstretched at three viewport scales
(`tests/mesh_raster.rs`, 7); a function paint evaluated at every device pixel's own centre with
its distinct-byte count going 32 → 64 → 128 as it magnifies, and a discontinuity landing at
column 16/32/**65** where a pre-zoom bake says 64 (`tests/function_resolution.rs`, 4); and thin
marks characterised on both lanes (`tests/thin_marks.rs`, 7).

## The one finding that is a decision rather than a gate

**A sampled coverage rule and an area coverage rule disagree about what is *there*, not only
about how much.** `Coverage::Gpu` samples a 4 × 4 ordered grid, so its columns sit a quarter of
a pixel apart. A 0.1-device-pixel bar, 768 tall, swept across ten sub-pixel positions:

| left edge | `Coverage::Cpu` | `Coverage::Gpu` |
|---|---|---|
| 20.0 / 20.2 / 20.4 / 20.5 / 20.7 / 20.9 | 0.10196 | **0** |
| 20.1 / 20.3 / 20.6 / 20.8 | 0.10196 | 0.25098 |

**Six of ten vanish; the other four draw 2.5× the ink.** Byte-identical on llvmpipe and RADV, so
it is the design and not an adapter. It is reachable rather than contrived: a long thin rule is
exactly the shape `take_gpu_lane` prefers. No sample count removes it — only an area rule does.
Gated as a characterisation that fails if the gap closes silently, and *not* fixed here, because
the fix is a scan-conversion decision with a cost either way.

**What §10.7.4 actually decides**, since the question presumes more than it says: its rule is
binary — paint any pixel the shape intersects — and §10.7.1's NOTE says the algorithm is
undefined by PDF. §11.3.7.2's NOTE 1 is where antialiased fractional coverage gets a meaning, as
*shape*. So proportionality is ADR 0005's choice and is asserted as ours; only "no disappearance"
and "ink ≥ the shape's area" are asserted as the clause's.
