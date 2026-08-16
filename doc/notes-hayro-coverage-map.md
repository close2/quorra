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
| #1315 — a stencil mask at a **different resolution** from its image, 5× slower | §8.9.6.4 allows it and scanners produce it constantly; the caller says *"the second is the one that is quorra's"* | `doc/notes-hayro-questions.md` Q3 |
| #2 — downsampling a mask larger than its image | same mismatched grid, quality side | Q3 |
| #1319 — per-sample `BitReader` + f32 interpolate to expand a 1-bit mask | decoding is the caller's; pixels reach us decoded | not ours |
| #1310 — overriding `/Interpolate` globally | integration note 1: the **resolved** filter decision arrives on the image *command*. So the gate is that we honour it and never substitute our own | **open** |
| #494 — `/Width`,`/Height` disagreeing with the JPEG codestream | §7.4.8 vs §7.4.9; resolved upstream, we receive pixels | not ours |

## §5 — glyphs

| their issue | the question for us | gate |
|---|---|---|
| #296 — every glyph outline began with a spurious `MoveTo((0,0))` | a degenerate leading `MoveTo` is invisible to a fill and **not** to a stroke: §8.4.3.3's caps apply to the ends of every open subpath, and a one-point subpath is open. Do we deposit a dot? | `doc/notes-hayro-questions.md` Q1 |
| #23 — bitmap glyph strikes resampled | font loading is §9's non-goal; outlines reach us already | not ours |
| #6 — font fallback | not our layer | not ours |

## §6 — colour on a device

| their issue | the question for us | gate |
|---|---|---|
| #4 — `/All` and `/None` colourants | `/None` "must composite as though it were never issued". Colour is not ours (integration note 6), so the question that **is** ours: does a mark contributing nothing leave the target byte-identical, including inside a knockout group where §11.4.6 *replaces*? | `doc/notes-hayro-questions.md` Q2 |
| #630 — `mul_add` falling into libc software emulation without FMA | do we call `f32::mul_add` on a hot CPU path? | Q4 |
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

## What is open, in the order it is worth doing

1. **§8.7.4.3's coordinate space on a stroke and on a glyph** (#968, #102) — a paint anchored to the
   page rather than to the mark, composed in a defined order, and wrong by one matrix if it is
   wrong at all.
2. **Thin marks** (#104, #1023) — the caller's standing ask, and the place our two lanes could
   disagree with each other.
3. **`/Interpolate` honoured, never overridden** (#1310) — integration note 1's whole point.
4. **A mesh is drawn as the raster it already is** (#3) — integration note 5.
5. **A function paint is evaluated, not baked** (#551) — ADR 0053's claim, stated as resolution
   independence.
6. **No CMS is reachable** (#205 family) — a dependency assertion, cheap.
7. **Banding under one 8-bit level** (#60).
8. **`encode_threads` nested, and `Scene: Send + Sync` still asserted** (#1316, #1343).
