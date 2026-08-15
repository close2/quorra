# Plan

The brief is `RENDER_LIBRARY.md`; this file is the design in its current state of
belief, the order of work, and the state of both. Bare section numbers (§) are the
brief's; "clause" numbers are ISO 32000-2's. **Picking the work up cold: `HANDOVER.md`
has what to do next and the traps that cost a round each.**

The file has two parts. **Part 1 says how the library will work** — the architecture as
believed, with each piece naming the measurement that could overturn it, because §11 is
explicit that the design should turn on measurements rather than on anyone's taste. Most
of those measurements have since been taken, and each piece carries what came back.
**Part 2 says the steps that got there** — nine milestones, each with its work, its exit
gates, and the question it settles; all nine are done. When a Part 1 hypothesis is
confirmed or overturned, the decision becomes an ADR and this file is corrected in the
same commit; a plan that disagrees with the tree is worse than no plan.

## Where we are

Nine milestones are done, the swap landed on 2026-08-03, and the caller consumes this
library from https://github.com/close2/quorra, pinned by their `Cargo.lock`.

**This section says what is true today and is rewritten freely. What happened is
`doc/history/` — one file per round, newest last, never edited after the fact — and why
each decision was taken is `doc/adr/`. What to do next is `HANDOVER.md`.**

### The numbers that stand

Everything below is a *minimum*, because this machine is somebody's desktop and its load
average is not a constant (see `HANDOVER.md`'s traps). The instrument is named beside each
row, since a number without one is not evidence.

| | | measured on |
|---|---:|---|
| dense text, presenting, steady — §6.2's bar is 2.0 ms | **1.816 ms** `wall − acquire` | `examples/surface_measure.rs`, RADV at the real display, 2026-08-14 |
| — encode | 1.126 ms | same |
| — recording, which is most of that encode | 1.130 ms of a 1.348 ms *instrumented* encode | same; the instrument costs a clock read per seam (ADR 0023) |
| — geometry | 0.161 ms | same; ADR 0044's chord floor costs nothing visible here |
| — execute: the GPU is about 4 % of the frame | 0.071 ms | same |
| the same frame, unchanged, replayed rather than encoded | **0.174 ms** against 1.107 | `examples/retained.rs`, headless RADV, ADR 0048 |
| — and now also when the page's glyph tiles overflow the atlas | 1 encode per page, not 1 per frame | `examples/retained.rs`'s overflow section, ADR 0050 |
| artwork — the corpus's p99 clip shape — steady | 43.3 ms, geometry 35.4 of it | `surface_measure`, RADV at the real display, 2026-08-14 — **before ADR 0049**, and not re-run on the display since |
| — the same page's encode, before → after ADR 0049 | geometry **37.8 → 28.9 ms**, encode 46.3 → 37.2 | `examples/residue_clip.rs`, headless RADV into a texture, three alternating rounds, minima, load 3.8–4.8, 2026-08-15 |
| first frames, presenting | pipeline compiles: **none**, eight of eight | same; ADR 0043 |
| the caller's corpus at scale 1 | **931** agree / 23 differ / 2 refused / 18 not comparable | their tree, one copy, 2026-08-15 |
| the caller's corpus at scale 4 | **936** / 10 / 5 / 23 | same copy, same hour |

**The whole matrix, base against the merged round, in one copy of their tree within one
hour** — which is the only form `HANDOVER.md` accepts. Base is `6ed67f0`, the commit before
this round; the corpus cannot be run against the `87898c6` their lock pins, because their
working tree already uses `RetainedScene` and does not compile against it.

| lane, scale | base `6ed67f0` | merged round |
|---|---|---|
| CPU coverage, scale 1 | 930 / 24 / 2 / 18 | **931 / 23** / 2 / 18 |
| GPU coverage, scale 1 | 928 / 26 / 2 / 18 | **929 / 25** / 2 / 18 |
| CPU coverage, scale 4 | 936 / 10 / 5 / 23 | 936 / 10 / 5 / 23 |

**Two page lines move, out of 956, and every other line is identical to the character.**
`issue2177.pdf` joins the oracle on both lanes; `issue11473.pdf` moves by 0.0001 of a mean
and 0.03 of a worst tile. Nothing moved away from the oracle at either scale on either lane,
and no refusal moved. The GPU-lane row is the one that had to be run rather than reasoned
about: ADR 0050 changes atlas residency between frames, and under `Coverage::Gpu` residency
can change which lane a tile takes.

An older row here read `934 / 20` at scale 1 on 2026-08-14. **It is not a baseline**: the
same quorra commit re-run a day later reads 930 / 24, because their tree moved under us.
930 → 931 is what this round is worth.

**§6.2's success bar is met and its clear-win figure is not.** A third of the CPU
backend's 5.9 ms is 2.0 and we are at 1.816; a tenth is 0.6, and the thing that points at
it is the retained encode above — which needs the caller to stop rebuilding a frame's
scene when nothing changed (`pdf-viewer/doc/QUORRA_RETAINED_FRAME.md`). A minimum on a
desktop is not a margin, and it is not the same claim as 2.0 ms inside the caller's own
frame loop on their machine.

**What the frame is made of is the durable finding**, and it survived every round that
moved the total: the device is ~4 % of a presenting dense-text frame, so nothing done to
a shader can matter to it, and the work is CPU recording — which is why the last two
rounds were a bound memo, three probe removals and a retained encode rather than anything
on the GPU.

### What is still open

- **The residue-clip seam, half taken.** The residue itself is now rasterised once per
  chain rather than once per clipped command (ADR 0049): artwork's encode geometry is
  37.8 → 28.9 ms and its 600 residue rasterisations are 185. What is *not* taken is the
  reason two pages at 4× refuse with `ScratchExhausted` — that is the coverage **sheet**,
  one tile per clipped command, and ADR 0049 leaves `Counters::tiles` unchanged on every
  archetype on purpose. `HANDOVER.md` item 2 holds what is left, and it is tiling work.
  (The refusal count in this bullet used to read "three pages at 4× and one at scale 1",
  and "the only reason any corpus frame is refused". Both were too strong: today it is
  **two** at 4× and none at scale 1, and the corpus's other three refusals are a different
  budget or a correct clause refusal each.)
- **The caller's adoption round** — they pin twenty commits back, three sections of their
  `QUORRA_FEEDBACK.md` have drafted answers waiting, and `RetainedScene` is an API they
  must take up rather than merely receive. `HANDOVER.md` item 1. Two `Counters` fields
  land with ADR 0050 — `atlas_working_set_bytes` and `atlas_repacked` — and one
  `DeviceError` variant, `ResourceIdsExhausted`; all three are additive, and
  `doc/api-change-retained-atlas.md` is what the bump owes them.
- **A paint the device evaluates** — the caller's `QUORRA_FUNCTION_PAINT.md` asks for a
  §7.10.5 function evaluated per fragment. ADR 0053 answers **yes, type 4 only, generated
  shader only**, and nothing is built: a full page goes 4 988 ms on their processor to
  **0.060 ms** on RADV, and the agreement question resolves to a static classification of
  the program rather than to any of the three tolerances they offered. What is open is
  their answer on two contract questions, a conformance test per operator, and the paint
  measured inside the compositor rather than as a bare pass.
- **§11.2's census** still has not run against a real corpus in the form M5 asked for; the
  path lane's design stands on the shapes `doc/corpus-profile.md` measured instead, and
  ADR 0008 names the lever if the census ever overturns it.

---

# Part 1 — How the library will work

## 1.1 The shape of it: a sorter, five lanes, one compositor

The one-sentence brief calls for a renderer whose fast paths assume what a document
actually contains. The architecture that follows from it is a **sorting renderer**: at
frame time, every command in the scene is classified into one of five lanes by what it
*is*, each lane maps to the cheapest device primitive that draws it exactly, and all
five lanes draw into a compositor that implements clause 11 natively. Vello's design
premise — every fill is a general curve fill, handled by one uniform tile-binned
pipeline — is exactly the premise §6.1 measured and found backwards for this workload;
ours is the opposite premise, held to the same standard of measurement.

| lane | what lands in it | device primitive |
|---|---|---|
| **glyph** | a fill of an uploaded outline whose device-space size fits the atlas — §1.1's dominant case, 5 933 of one dense page's commands over 107 distinct outlines | one instanced quad sampling the R8 coverage atlas (§6.3) |
| **rectangle** | axis-aligned rectangles under axis-preserving transforms: rules, backgrounds, underlines, table cells — and most clips | exact analytic coverage in the fragment shader; no tiling, no binning, no edge list (§6.4) |
| **path** | everything else: large fills, arbitrary transforms, strokes — the rare case, by assumption until §11.2's census makes it a number | the general coverage path, whose design M5 chooses *after* the census (§1.6 below) |
| **image** | decoded RGBA8 with the filter decision already resolved upstream (§4.5, integration note 1) | a textured quad |
| **mesh** | the caller's pre-rasterised mesh, shared between its backends on purpose (integration note 5) | drawn as the raster it already is; never re-triangulated |

Two properties of the sorter matter more than the lanes themselves:

- **Classification happens at encode time, per frame — never at scene-build time.**
  Which lane a command takes is a device-space question: the same glyph outline is a
  quad at 100% zoom and a general path at 6400%, when its device size outgrows what an
  atlas entry can hold. Putting the sorter in `render` is what keeps the `Scene`
  viewport-free (§2.3), which the brief calls the most important property in the
  document. The budget for the whole encode is the number the current backend already
  achieves: **1.1–1.6 ms, flat in resolution** (§6.1). Ours may not regress it, because
  it is a function of the command list and not of the pixels, and that flatness is
  structural, not accidental.
- **The sort is a pure function of the command list and the viewport.** Same scene,
  same viewport → same lanes, same batches, same draw order. Determinism (§4.6) is
  designed in here, not tested in later.

**Overturned by:** §11.2. If the corpus census shows the path lane is not rare — that a
substantial share of real commands miss the glyph and rectangle lanes — then the path
lane's design gets the engineering attention this table currently gives the atlas, and
this section is rewritten with the number in it.

## 1.2 A frame, from call to pixels

`Device::render(scene, viewport, target)` runs five phases, each bracketed by
timestamp queries where the adapter offers them, so that `Timings` reports what §8
requires and §6.1 could not get: the split between encode, upload, execute and
readback, measured rather than inferred.

1. **Classify and count.** One CPU walk over the commands: sort into lanes, resolve
   each clip chain to its rectangle-and-residue form (§1.4), discover the group and
   mask jobs and their dependency order, and **count everything** — instances per
   lane, layer targets, mask targets, bytes to upload. Nothing has been allocated yet.
2. **Allocate and upload.** Every buffer is sized from phase 1's counts and checked
   against the stated budget before creation. This is §5's first preference — count,
   then allocate — and it is why the failure mode this library exists to eliminate
   cannot occur: there is no fixed-size table for a scene to overflow on the device,
   so a page is drawn or the *allocation* fails with an `Err` naming the limit. A count
   of zero is legitimate everywhere (a blank scene is a legitimate scene, §5) and never
   becomes a zero-length buffer handed to `wgpu`.
3. **Execute.** Passes in dependency order: atlas fills for glyphs not yet resident;
   mask groups rendered and reduced (§1.3); then each layer bottom-up, its commands
   drawn as batched instanced draws in scene order. A batch is a maximal run of
   commands in the same lane with `BlendMode::Normal` and the same clip state; a
   non-Normal blend, a group boundary or a clip change cuts it. Batch cuts are a pure
   function of the list, and `Counters` reports how many there were, because a page
   that cuts often is a page this design serves badly and we want to learn that from a
   counter rather than from a regression.
4. **Resolve.** `Surface` and `Texture` targets composite the finished page and are
   done — no readback, which per §6.1 deletes the largest single cost in the current
   backend's frame. `Readback` copies out, maps, and converts premultiplied to
   straight alpha once at the boundary (§3).
5. **Account.** `Timings` from the query results (and saying so when a wall clock had
   to stand in — a number whose provenance is ambiguous cannot gate anything),
   `Counters`, `Report`s, and a `Frame` whose every claim about itself is true.

**Overturned by:** §11.1, answered in M1. If the readback is nearly all of the fixed
cost, tiers 2 and 3 are the whole performance story, and the effort this plan spends on
per-pixel work in phases 3–4 is re-ranked accordingly.

## 1.3 Clause 11 is the compositor, not an effect

This is the part an SVG-shaped model cannot be patched into, so it is the part designed
first and compromised never.

**Groups are layers, painted once.** A `Group` becomes an offscreen premultiplied
target; its children draw into it; the finished layer is composited onto its parent
exactly once, under the group's constant alpha and blend mode (§4.4; clause 11.4.1,
11.4.5). What the layer is *initialised to* is `GroupSpec::isolated`: transparent for
clause 11.4.5's isolated group, which is the default and what the brief's §4.4 promised
would be the only case, or a copy of the backdrop for clause 11.4.4's non-isolated one,
whose composite is then an interpolation rather than §11.3.6 (ADR 0019, from the
caller's feedback §16). Nesting is bounded at 16 (§1.1), so the layer stack is
countable in phase 1 like everything else. The page itself renders onto transparency, always, because clause
11.4.7 makes the page group isolated and compositing onto the medium is the caller's
job (§3).

**Sixteen blend modes, ours.** `Normal` is hardware fixed-function blending and is the
fast path that keeps batches long. The other fifteen — the twelve separable and the
four non-separable, written from clause 11.3.5's `Lum`, `ClipColor`, `SetLum` and
`SetSat` — need the backdrop as an input, which `wgpu` has no framebuffer-fetch for, so
a non-Normal draw costs a batch cut and a backdrop read (a copy of the affected bounds
into a sampled texture, or a ping-pong of the layer — M6 measures which, on scenes
where it matters). Each WGSL blend function carries its clause number in a comment, and
the implementation is deliberately not shared with the caller's CPU backend: a shared
one would make the cross-backend comparison compare an implementation with itself
(§4.3).

**Shape is its own channel.** The knockout rule (§4.1; clause 11.4.6) replaces *a
coverage-fraction* of the accumulated group with the element composited against the
group's initial backdrop — `lerp(accumulated, element, coverage)`, per pixel. The
design consequence is a rule that binds every lane: **coverage is a first-class value
in the fragment shader at the moment of composite** — computed analytically in the
rectangle lane, sampled from the atlas in the glyph lane, produced by the coverage
machinery in the path lane — and is never irreversibly folded into premultiplied alpha
before the compose decision is applied. Two candidate mechanisms, decided at M6 with
the diagonal-edge fixture as judge: dual-source blending where the adapter offers it,
and a per-element compose pass where it does not. Knockout groups are rare; per-element
passes inside one are affordable if that is what correctness costs.

**Soft masks are rendered groups, reduced on the device.** A mask group is built
through the same `SceneBuilder` as everything else and rendered through the same layer
machinery at device resolution; a single reduction pass then produces the R8 mask —
`Alpha` (clause 11.5.2) takes the group's alpha, `Luminosity` (clause 11.5.3)
composites the group onto a fully opaque backdrop of the mask's colour *first* and
takes the luminosity of the *result* (the order is the clause's, and getting it
backwards produces a plausible picture), then the optional 256-entry `/TR` table is
applied by lookup. No readback, no round trip — the current backend's per-mask-per-frame
CPU round trip is the thing §4.2 exists to delete. The reduction arithmetic mirrors the
caller's `SoftMask::value` in the same 8-bit integer domain, because that function is
the shared definition of what the pixels mean and our shader is a second implementation
that must agree with it **to the byte** — the conformance test over all 256 mask values
ships with the shader (M6), not after it.

**Open, and flagged now:** the layer targets' precision. Premultiplied internally is
settled (§3); whether a layer is 8-bit or wider is not, and it is entangled with
byte-agreement obligations that live in 8-bit space. M6 decides it with the blend and
mask conformance tests in hand, and writes the ADR.

## 1.4 Clips are mostly rectangles, and the design says so

A clip chain is an intersection (§4.7). Phase 1 resolves each chain once into two
parts: the intersection of its axis-aligned rectangular links — which is itself a
single rectangle, four floats and a comparison in the fragment shader, or a scissor
when it bounds the whole batch — and the non-rectangular residue, which becomes an R8
clip mask through the path lane's coverage machinery.

The caller's numbers say the residue is the exception: its page 6 states one clipping
rectangle 303 times and its display list already collapses them to one identifier
(§1.1), and §6.4 is blunt that a rectangular clip must never become a mask texture.
Where a residue mask is built, the chain is rasterised **once over the region it
occupies** and every mark takes a window on it (ADR 0049). The key is the chain's deepest
non-rectangular link, which under one viewport *determines* the region — the transform is
fixed for the frame — so the failure the identifier rule warns about cannot happen here:
two commands under one chain cannot get two different masks. (That rule is the caller's
clip-mask cache, which once answered all 303 lookups a page made and built 303 identical
page-wide masks because its key was a name — their ADR 0132, restated in CLAUDE.md.) What
a chain-identity key cannot do is *unify* two chains that happen to resolve to the same
region, and `Counters::clip_residue_regions` would show that honestly, as two regions.
`Counters` reports the count of distinct regions and the count of chains that paid per tile
instead — keys, never a hit rate. An empty clip admits nothing, which is a different thing
from an absent clip, and both have tests.

## 1.5 Memory that grows

The rule is principle 6's, the mechanism is §1.2's phase discipline, and the posture is
worth stating as design rather than leaving implicit in the phases:

- **Per frame:** count, then allocate. No working buffer is sized by a constant.
- **Across frames:** pools persist on the device and grow geometrically when a frame's
  counts exceed them; they never shrink mid-frame, and shrinking at all is a deliberate
  policy decision for M8, not an emergent behaviour.
- **Every allocation derived from scene content** — instance buffers, layer targets,
  mask targets, the atlas — is checked against a stated budget before creation, and
  exceeding it is a typed `Err` naming the limit and the number that hit it, so the
  caller can fall back to its CPU backend, which its window already knows how to do
  (§5).

  A **cache** is the one place where the answer is *decline* rather than `Err`: the
  residue regions of ADR 0049 are checked against a quarter of `max_frame_bytes` before
  allocation, and a region that does not fit is not built — the frame draws it per tile
  instead, which is what every frame did before. Refusing a drawable frame because a cache
  filled up would be principle 6 pointed the wrong way, and the atlas has said so since
  ADR 0029.
- **Before the frame:** `Scene::cost()` against `Device::limits` gives the caller the
  same arithmetic we will do, so a refusal can happen before a frame is attempted at
  all — §5's second preference, satisfied in addition to the first, not instead of it.

## 1.6 The path lane: designed after the census, and here is the shortlist

The one lane whose design is deliberately not chosen yet, because §11.2 asks the
question and the honest answer is a measurement we do not have: **how many of a real
corpus's commands miss the glyph and rectangle lanes?** The candidates, so that M5
chooses among named options rather than improvising:

1. **Tile-binned compute, à la Vello but with counted allocation.** Known to scale to
   the hard case; brings the machinery §6.1 measured as overkill for the common case —
   defensible only if the census says the hard case is common enough.
2. **CPU flattening at encode into device-space geometry, GPU coverage accumulation.**
   Flattening tolerance is a device-space question and phase 1 is device-space, so this
   fits the frame anatomy; costs CPU time that scales with the lane's population, which
   is exactly why the census comes first.
3. **Stencil-then-cover with multisample.** The classical fallback; its coverage
   quantisation (an MSAA sample count's worth of levels) risks the oracle's bound where
   analytic coverage would not, so it must clear the oracle before it clears anything
   else.

Strokes take the path lane after expansion to fill outlines at encode time — the caller
has already resolved device widths, dashing and degenerate subpaths (§4.5), so
expansion is caps, joins and miters and nothing else. Whether hairline strokes deserve
their own primitive is a question the census's stroke population answers, not one this
plan decides.

## 1.7 Determinism, stated as a design posture

§4.6 requires byte equality for same scene, same viewport, same adapter — and the
caller's CI currently *relies* on RADV and lavapipe agreeing byte-for-byte across
adapters, which is §11's question 4 and is not assumed here.

Within one adapter, determinism is arranged rather than hoped for: the sort, the
batches and the draw order are pure functions of the list (§1.1); blending happens in
draw order, which the GPU guarantees per draw call sequence; no accumulation whose
result depends on scheduling order (atomics races, workgroup timing) is permitted in
any pass that touches pixels; nothing reads a clock or a random source.

Across adapters, the hypothesis was that simple fragment arithmetic would preserve
the RADV/lavapipe identity the current backend enjoys. **M1 measured it and the
hypothesis failed** — not in the shader arithmetic, which is deterministic, but in the
fixed-function float→unorm8 store conversion, whose rounding is the driver's
(ADR 0006). Same-adapter identity holds and is gated exactly; cross-adapter output is
gated to a stated bound; and the decision about restoring identity by owning the final
quantisation in shader code belongs to M6's compositor ADR, where its cost can be
measured against the caller's CI reliance on it.

## 1.8 Startup: the device returns before it is warm

§7's sequence, as this library will implement it:

1. `Device::headless` / `for_surface` enumerates the adapter and creates the device —
   `pollster` over wgpu's two awaits, on whatever thread called it, which may be a
   background thread and need not be (§2.1).
2. It returns as soon as the device exists. **No pipeline compilation is on the
   critical path of construction.** The warm set — glyph quads, rectangle fills, the
   composite — compiles immediately but asynchronously; a render that arrives before a
   pipeline it needs waits for exactly that pipeline, and `is_warm` answers whoever
   wants to hand over from the CPU backend only when frames will be full-speed.
3. Everything else — shadings, meshes, the fifteen non-Normal blends, mask reduction —
   compiles on first use, so a page of plain text never pays for machinery it does not
   touch.
4. `Options::pipeline_cache` takes a blob from a previous launch; a rejected blob is a
   `Report`, never a silent recompile, because a silently recompiled cache is a startup
   regression nobody can attribute (§7). Whether the backend supports the cache at all
   is the driver's answer through wgpu's door (ADR 0002), and we report which answer we
   got.
5. Startup cost is reported split three ways — adapter enumeration, device creation,
   pipeline compilation — and CI gates the numbers from the first commit that produces
   them.

## 1.9 The scene, and the resources it references

`quorra-scene` is pure data and cannot reach a device by construction (ADR 0001).
A `Scene` is the flat command list plus its side tables — the clip chains, the group
tree, the mask definitions — behind one `Arc`: `Send + Sync` by a compile-time
assertion, cheap to clone, buildable on the caller's interpreter thread while the GPU
is still initialising. Outlines, images, ramps and meshes live on the device, uploaded
once and referenced by `u32` handles (§2.2) — the caller keys uploads by `Arc::as_ptr`
identity, so one dense page's 107 outlines are uploaded once and referenced 5 933
times, and a zoom re-uploads nothing. **Those handles are never reused**, and the space is
a `u32`: the upload that would exhaust it is refused with `DeviceError::ResourceIdsExhausted`
rather than wrapping. The counter wrapped until ADR 0050 audited ADR 0048's key — a
reissued id would have made a retained encode draw a resource it never named, with every
generation counter agreeing that nothing had moved. `Scene::cost()` is computed at `finish`
time, so asking it costs nothing per frame. §11's question 5 — what a scene costs to hold,
against a target of a dozen resident pages — gets its number in M2 and its verdict in
M8.

## 1.10 The five questions, where each is answered, and what turns on each

| §11 question | answered | what turns on the answer |
|---|---|---|
| 1. How much of the fixed cost is the readback? | **Answered at M1**: ~90% of an offscreen dense-page frame on RADV (4.1 ms of 4.6; execute is 48 µs). The M1 record is in `doc/history/`. | tiers 2–3 are confirmed as the headline; per-pixel work is second-order for tier 2/3 hosts |
| 2. Does the glyph path want tiles at all? | M5, corpus census before design | which of §1.6's three candidates the path lane becomes — and whether §1.1's premise survives |
| 3. What does the atlas cost on a page it cannot help? | M4 | whether the atlas is unconditional, adaptive, or off by default on low-reuse pages |
| 4. Is byte-identical output across adapters achievable? | **Answered at M1**: no, for the fixed-function path (ADR 0006); same-adapter identity holds, cross-adapter is bounded and gated | M6's compositor ADR decides whether shader-owned quantisation buys identity back; the caller's CI model needs the conversation now |
| 5. What does a `Scene` cost to hold? | M2 number, M8 verdict | whether the dozen-resident-pages roadmap item needs a compact encoding or gets it for free |

---

# Part 2 — The steps

## How the order was chosen

Not by intuition, and not from the top of the brief. §6.1 measured the current
Vello-based backend and the result reversed the obvious plan: **between 55% and 92% of
an offscreen frame is paid before any of the page is drawn**, scene encoding is flat in
resolution at 1.1–1.6 ms, and 5 933 glyph fills cost about what one rectangle costs.
The brief's own ranking follows from those numbers, and this plan follows the ranking:

> the surface and texture target paths first, because they delete the largest single
> item; then whatever makes the per-pixel floor cheaper for a target that is mostly
> untouched; then the atlas and the rectangle path; then the retained scene; then
> damage.

Two consequences worth stating, because they are easy to get backwards:

- **The first milestone is a measurement, not a feature.** §11's first question cannot
  be answered with a wall clock, and the answer decides whether the atlas is a headline
  or a second-order effect. It needs timestamp queries, which means it needs a device,
  which is why M1 is a device and a rectangle and nothing else.
- **Correctness work is not deferred to the end.** §4 is the reason this library exists
  at all; what is deferred is *breadth*. The knockout group's diagonal edge, the
  sixteen blend modes and a full page at a real window size are the three scenes that
  will find bugs on day one (§10), so each lands with the milestone that first makes it
  possible rather than in a conformance push afterwards.

## M1 — A device, a rectangle, and the measurement that settles §11.1

**Deliverable:** `Device::headless`, `Device::for_surface`, all three targets of §2.4,
one analytically-covered axis-aligned rectangle, `Timings` with real timestamp queries,
`Counters`, `Report`, and a `Frame` that tells the truth. Nothing else — no path, no
glyph, no group. A rectangle is the primitive that needs no tiling, no binning and no
edge list, so it isolates the per-pixel floor from everything else.

**The work, in order:**

1. `Options`, `DeviceError`, `RenderError` — the error variants name what failed, from
   the first commit.
2. Adapter enumeration and device creation, timed separately; `description`, `limits`.
3. The lazy-pipeline scaffolding of §1.8 — built now while there are two pipelines, not
   retrofitted at M6 when there are ten — and `is_warm`.
4. The rectangle fill pipeline: exact analytic coverage in the fragment shader.
5. The three targets, including the readback path with its one straight-alpha
   conversion at the boundary, `#[must_use]` and named for what it costs (§8).
6. Timestamp queries around every phase of §1.2, with the wall-clock fallback that
   *says it is one*; `Timings`, `Counters`, `Frame::reports`.
7. The measurement: execute versus readback, per target kind, at 1×, 2× and 4× of a
   window-scale target — §6.1's table, re-taken with the instrument it lacked.
8. The harness: headless golden renders to PNG, the byte-equality gate (same scene,
   same viewport, same adapter, repeated renders), the RADV-versus-lavapipe
   cross-adapter gate, and CI perf gates for the startup split and the frame numbers.

**Done when:** §11.1 has a number per target kind and resolution; startup has its
three-way split gated in CI; a blank scene renders to `Ok` on all three targets; the
cross-adapter gate has run on both adapters and its verdict — either way — is written
down; and a failed frame cannot report itself drawn (tested, not asserted).

**Done** (2026-08-02). The record and the two answered questions are in "Where we
are"; the verdicts live in ADRs 0005 and 0006; the gates live in
`crates/quorra-gpu/tests/{m1,perf_gate}.rs`. The surface path is proven end to end:
`examples/window_smoke.rs` presents real frames to a real window under `Xvfb`
(lavapipe, the caller's own CI arrangement), verified by reading the window's pixels
back with `xwd` — the centre and field pixels match the scene within ADR 0006's
bound. CI runs the smoke on every push; presenting on RADV to the user's live display
still awaits the user, since Xvfb has no DRI3 for it.

## M2 — The scene, retained and viewport-free

**Deliverable:** `quorra-scene`'s real types — `geom`, `paint` (solid half), `scene` —
plus `SceneBuilder`, `Scene: Send + Sync`, `Scene::cost()`, and the upload/release path
of §2.2 on the device side. The API integration notes below are settled with the caller
before this milestone freezes signatures.

**The work, in order:**

1. `geom`: `f32` throughout matching the caller; move/line/cubic/close and no
   quadratics; `Affine` with `preserves_axes` and `max_stretch`, because §1.1's sorter
   and §6.3's scale bucket ask for exactly those.
2. Input validation at the boundary (§4.7): coordinates and transforms outside stated
   limits are refused loudly with typed errors; no NaN survives into geometry; no
   allocation is sized from an unchecked number.
3. `SceneBuilder` and `Scene`: the flat command list, clip chains as data, group
   nesting checked against the bound of 16, `finish` computing `cost`.
4. `Device::upload_outline` / `upload_image` / `upload_ramp` / `upload_mesh` /
   `release`, each upload checked against the resource budget.
5. The fuzz target on the builder and the encoder — structured scene input, run from
   this milestone onwards, every crasher a permanent regression test (principle 3).
6. The M2 tests the scene skeleton already names: no viewport anywhere (a scene renders
   byte-identically wherever it was built); `Send + Sync` statically asserted; a blank
   scene is legitimate; encode-order independence.

**Done when:** the caller-shaped round trip works — 107 outlines uploaded once,
referenced thousands of times, a zoom re-uploading nothing; `Scene::cost()` returns the
number §11.5 asks about, recorded for M8; byte equality across repeated encodes of one
scene is gated.

**Done** (2026-08-02), with one honest boundary: the upload/identity/budget half of
the round trip is proven (`tests/m2.rs`); the *drawable* half — the outlines actually
painting — is M4/M5's proof by definition. §11.5's first number: a 5 933-fill scene
costs ~380 KB retained (64 bytes per command), so a dozen resident dense pages are
~5 MB of commands — comfortably inside the brief's 70 MB affordability line, before
any compaction. Deviations and refinements are integration notes 1, 5 and 7.

## M3 — Rectangles and rectangular clips, analytically

**Deliverable:** `SceneBuilder::rect`, clip chains resolved and honoured, the
rectangle-and-residue split of §1.4 (with the residue refused loudly for now — the
path lane that draws it is M5, and a refusal that names the reason beats a silent
approximation, §5).

**The work, in order:**

1. Phase-1 clip resolution: chains to a single intersected rectangle plus residue;
   scissor where the rectangle bounds the batch; four floats and a comparison
   otherwise. Never an R8 mask for a rectangle (§6.4).
2. Empty-clip and absent-clip semantics, tested as distinct.
3. The distinct-region counter (§1.4) — the count of resolved regions, not a hit rate.
4. The scene suite grows its full-page fixture: **one full page at a real window
   size**, because a suite of small scenes tests small scenes, and the first real page
   at a real size came back blank in the caller's tree with nothing able to see it
   (trap 12b).

**Done when:** a page-6-shaped scene — thousands of rect fills, 303 identical clip
states collapsing to one region — renders correctly and the counter proves the
collapse; the caller's worst case of 3 608 chains is synthesised and stays within
budget; perf gates cover the rectangle path.

**Done** (2026-08-02). The fixtures live in `crates/quorra-gpu/tests/m3.rs`; the
resolution design is ADR 0007. One deliberate narrowing to note: the recogniser
accepts line-edged rectangles only — a rectangle drawn as collinear cubics takes the
M5 residue path, and loosening that is a measurement-backed decision for M5 if real
corpora need it.

## M4 — The glyph atlas

**Deliverable:** the R8 coverage atlas with eviction and a caller-set budget, keyed on
`(outline, scale bucket, sub-pixel phase)` with the **quantum settable, documented, and
switchable off** — §4.5's fifth decision, the one that is ours to expose. Default 1/16
of a pixel: 1/16 reused 5.0× on a dense page and left the oracle's verdicts unmoved;
1/8 contradicted pages.

**The work, in order:**

1. The key and the quantum plumbing through `Options` — a silent quantum would move the
   text and nobody could attribute it.
2. The packer, eviction, and the budget check; `atlas_entries` and
   `atlas_distinct_keys` in `Counters` — the distinct-key count, not the hit rate.
   Two more joined them at ADR 0050. `atlas_working_set_bytes` is what holding **all**
   of a frame's distinct glyph keys would cost, which is the number `Options::atlas_budget`
   is compared against and the only one that tells "the atlas is too small for this page"
   apart from "the atlas is holding another page". `atlas_repacked` is whether the atlas
   was thrown away and re-packed after this frame — the one event that makes a retained
   encode stale. A page that settles reports it true on at most one frame; true on frame
   after frame is thrash, and the counter exists so that state has a name.
3. The rasteriser decision, as an ADR with a measurement: coverage rasterised on the
   GPU, or on the CPU by `tiny-skia` — the latter makes the glyphs come from the same
   code that is the caller's correctness oracle, a correctness argument no other
   arrangement gets for free, against a dependency and a per-glyph transfer this
   library would otherwise not have (§6.3 offers it as persuasive, not prescriptive).
4. The glyph lane in the sorter: fills whose device size fits an entry become quads;
   the atlas-miss path (too large, budget exhausted) falls through to the path lane —
   or, until M5 exists, to a loud refusal.
5. The measurement for §11.3: the atlas's cost on `tracemonkey.pdf`-shaped reuse
   (1.3×) as well as its win on dense-text reuse (5.0×), because a cache that is 5× on
   one page and a net loss on another is a decision, not a feature — and the decision
   needs both numbers.

**Done when:** a dense-text scene at window scale beats its M3-era self by a measured
margin; the low-reuse cost is known and the decision it forces is written down; the
quantum is proven settable and off-able by test.

**Done** (2026-08-02): ADRs 0008/0009, the record in `doc/history/`, gates in
`tests/{m2,m45}.rs`. The tiny-skia question resolved as "neither": our own
rasteriser, for determinism and the dependency posture — the oracle argument waits
for M9 where the oracle actually is.

## M5 — General path coverage, designed after the census

**Deliverable:** fills and strokes of arbitrary cubic outlines, both fill rules, nested
subpaths wound the same way, non-rectangular clip residues — the path lane, built as
whichever of §1.6's candidates the census justifies.

**The work, in order:**

1. **The census first (§11.2):** over the caller's corpus display lists, count what
   share of commands miss the glyph and rectangle lanes, and what they are. The corpus
   lives in the caller's tree and the two trees stay independent until M9, so the
   fixture is *data* — display lists serialised to a neutral form and carried over —
   never a dependency edge.
2. The design ADR: which candidate, chosen by the census plus the oracle constraint
   (coverage quantisation must not move oracle verdicts).
3. Fills: both rules, the caller's deep-nesting cases, clip residues through the same
   machinery, the mask targets it produces feeding §1.4's region-keyed cache.
4. Strokes: expansion to fill outlines at encode — caps, joins, miter limits only,
   because widths, dashing and degenerate subpaths arrive resolved (§4.5) and our job
   is not to undo any of it.
5. The atlas-miss fall-through from M4 becomes real: a glyph at extreme zoom takes this
   lane and the seam is invisible (tested at the boundary scale).

**Done when:** the census number is in the ADR; the three day-one scenes that involve
paths pass; every command a real corpus page contains renders through some lane with no
refusals left except genuine budget refusals.

**Done in implementation, open in measurement** (2026-08-02): fills (both rules,
nested same-winding pinned end to end), strokes (caps/joins/miter, cross-checked
against the rectangle they degenerate to), residue clips (the triangle-clip fixture
masks correctly), oblique rectangles — all drawable; the remaining M6-refusals are
groups, non-Normal blends and `Compose::Src`, each named. The census (§11.2) has not
run — it needs corpus fixtures from the caller's tree — and stays the recorded
condition for revisiting the lane design (ADR 0008).

## M6 — Clause 11, natively

**Deliverable:** the four things a general vector API lacks, and the reason this
library exists (§1.3's design, made real).

1. **Coverage-modulated Porter-Duff Source** (§4.1) — the compose mechanism decided
   between dual-source blending and a per-element compose pass, with the diagonal-edge
   knockout fixture as judge, because axis-aligned rectangles would agree while being
   wrong.
2. **Soft masks reduced on the device** (§4.2) — `Alpha`, `Luminosity { backdrop }`,
   the optional 256-entry `/TR` table, the mask group built through the same
   `SceneBuilder`. No readback, no round trip. The conformance test is part of the
   definition of done: all 256 mask bytes through both rules against the caller's
   `SoftMask::value`, byte-equal; a luminosity mask with a non-black backdrop; a
   non-identity transfer sampled at both endpoints.
3. **Sixteen blend modes** (§4.3), each WGSL function carrying its clause number, the
   four non-separable ones written from clause 11.3.5's `Lum`, `ClipColor`, `SetLum`
   and `SetSat` — our own implementation, deliberately unshared, tested against the
   caller's sixteen-mode fixture that once found three of `tiny-skia`'s wrong by up to
   113 of 255.
4. **Groups compositing onto transparency** (§4.4), isolated, painted once under the
   group's own alpha and blend mode, depth bounded at 16 — plus the backdrop-read
   mechanism for non-Normal blends, measured on scenes where the batch cuts bite.

The layer-precision ADR (§1.3's open question) is written here, with the conformance
tests in hand.

**Done when:** the caller's three day-one scenes pass — the knockout diagonal, the
sixteen modes, the full page at window scale — and the soft-mask byte-agreement test is
green on both adapters.

**Done** (2026-08-02): ADR 0010, gates in `tests/m6.rs` — the 256-byte agreement is
exact, the sixteen modes match a clause-transcribed reference, the knockout diagonal
matches §11.4.6's formula, isolation and group alpha hold, and the compositor is
byte-deterministic per adapter. The layer-precision question closed on the side of
rgba8 layers (quantisation between commands is the CPU reference's model and the
mask byte-agreement's precondition); full-target layer textures are the recorded
optimisation candidate for a bbox-bounded version, with M8's measurements. Item 4's
"onto transparency" is the isolated group only, which is what §4.4 promised at the
time; clause 11.4.4's other initial backdrop arrived later, in ADR 0019.

## M7 — Shadings, meshes, images

**Deliverable:** axial, radial and function-based shadings from a `RampId` (clause
8.7.4.5); the caller's pre-rasterised mesh consumed as-is (integration note 5); decoded
RGBA8 images with the filter decision arriving resolved (integration note 1). Straight
alpha at the boundary, premultiplied internally, rendered onto transparency, always.
All of these pipelines compile on first use (§1.8) — a page of plain text never pays
for them. The open `ShadingKind` question — geometry on the paint versus resolved at
upload — is decided here with a measurement and an ADR.

**Done** (2026-08-02): ADR 0011, the record in `doc/history/`, gates in
`tests/m7.rs`. The `ShadingKind` decision landed on **geometry on the paint** (six
floats per placement; the uploaded ramp serves every placement, which is §2.2's
economy — integration note 9). The caller's *sampled* function shadings map to
images on this side, and meshes arrive pre-rasterised at device resolution
(integration note 5) and sample at absolute device pixels. Image, shading and mesh
pipelines compile on first use; the warm set is unchanged, so startup did not move.

## M8 — Damage, and the rest of the performance contract

**Deliverable:** per-tile dirty tracking against a retained scene and a cheap viewport,
so a caret blink redraws a few tiles rather than a page (§6.5) — `Viewport::damage`
honoured exactly, and a damage list we cannot honour meaning a full redraw *and a
`Report`*, never a stale region. The persisted pipeline cache round-trips through a
real second launch — **which requires an ADR first**: wgpu 30's
`create_pipeline_cache` is an `unsafe fn` (the blob is trusted input), and this tree
is `#![forbid(unsafe_code)]`; the ADR weighs a scoped exception (principle 3's
benchmark-plus-invariant route) against not offering the cache, with the startup
measurement in hand. Pool-shrink policy decided. §11.5's verdict: `Scene` memory
against the dozen-resident-pages target, with M2's number as the input.

**Done** (2026-08-02): ADRs 0012 (damage as scissored rendering plus rectangle
patching — honoured exactly on `Texture` targets, reported on the others, refused
when malformed) and 0013 (no pipeline-cache blob: the benchmark showed nothing
user-visible to win, so the `unsafe` exception was declined and
`#![forbid(unsafe_code)]` stands). The record and numbers are in `doc/history/`;
gates in `tests/m8.rs`. The plan's own phrasing was ahead of the measurement in one
place: there is no texture pool to give a shrink policy to — per-frame creation is
inside the measured budget, and the pool waits for a number that demands it.

## M9 — The swap

**Deliverable:** the caller's `render-gpu` replaced — an adapter in *their* tree
implementing their `Rasterizer` over quorra, which is where the bridging of integration
note 2 lives. Held to §10 in full: the cross-backend scene suite; the 1 794-page
oracle, which has refused two of the caller's own optimisations after they passed every
unit test and will not be kind; byte equality where we claim it; and a window driven by
real key presses under `Xvfb`. Not before: this is the last milestone, and the two
trees stay independent until it.

**Progress** (2026-08-02): the adapter exists and passes the caller's bar.
`crates/render-quorra` in the caller's tree implements their `Rasterizer` over this
library: the display list maps command-for-command (the two vocabularies were shaped
by one contract); dashes cut through the same `kurbo::dash` their Vello backend uses
and §8.5.3.2's zero-length rules through their shared helpers; sampled shadings —
which their GPU backend *refuses* — draw as domain images clipped to the fill;
meshes go through their shared `MeshRaster`; the medium is imposed by their own
function. Two integration findings became fixes on this side: `Paint::Shading` grew
its own transform (a shading anchors to the page, §8.7.4.3 — the command-space
geometry M7 chose could not express the caller's model), and `Stroke::width`'s doc
now tells the truth (device pixels). Two adapter defects were found by the caller's
own instruments and fixed: an ABA bug in the `Arc`-identity caches (pointer keys
without pinning served stale outlines by allocator mood — the caches now hold the
`Arc`), and stroke widths mistaking their `device_width()` for device units (it
answers in path units; ×`max_stretch` carries it over, and page 6's rules stopped
being 2× thin at window scale). **Gates, all green on RADV:** the eleven
cross-backend scenes at the Vello suite's own thresholds (all sixteen blend modes,
knockout, soft masks, an interpreted PDF); the real-page suite — with exact phases
quorra sits at the Vello backend's distance from the CPU oracle on every case
(worst tile ≤ 3.0 vs their 3.2; one case *better*: 2.6 vs 4.2), and the 1/16
quantum's cost is pinned as its own envelope. **Perf at the trait boundary** (page
6, fastest of 12): 4.4 ms vs CPU 4.9 / Vello 6.9 at scale 1.0; 10.4 ms vs CPU 6.1 /
Vello 11.6 at window scale — both GPU backends readback-bound exactly as §11.1
measured, which is why the remaining work is the part that escapes the trait.
**Done** (2026-08-03): the swap itself landed. `render-quorra` grew the tier-2
`QuorraPresenter` — quorra's device owns the window's surface, and one call draws
the page, the selection, the sidebar and the modal card (all display lists through
the same translation the headless gates exercise) and presents, with no readback
anywhere. To make one scene carry several lists at their own placements, the
adapter bakes each list's target transform into its commands and leaves the
viewport at identity. `viewer-ui`'s render path moved onto the presenter
(−205 lines net: the RenderContext, intermediate texture, blitter and banding
machinery all left with the backend that needed them); the CPU backend keeps its
oracle and fallback roles, the fallback now presenting through the same quorra
surface as an image. **Verified under Xvfb with real key presses**: the ISO
32000-2 cover (images, chrome) and Page-Down navigation to the dense table of
contents, zero fallback notes. One follow-up remains with the owner: a
corpus-scale quorra-vs-oracle sweep (the 1 794-page oracle itself judges the CPU
backend by design, so this is an additional gate, not a missing one). The
CI-reachability question closed on 2026-08-03: this library lives at
https://github.com/close2/quorra, and the viewer consumes it as a git dependency —
revision pinned by its Cargo.lock, source named in its deny.toml, with a
documented `[patch]` route for developing against a local checkout.

## What we must build ourselves, and when

§10 tells us what we will be judged by, which means the harness is ours to write rather
than to wait for:

| | lands with |
|---|---|
| Headless golden renders, PNG artefacts, byte-equality gate | M1 |
| Adapter-to-adapter byte equality, RADV against lavapipe (§11.4) | M1, re-checked every milestone |
| Perf gates: startup split, encode, execute, readback, one real page at a window size | M1 |
| A scene suite that starts small and includes **one full page at a real window size** | M3 |
| Fuzzing the scene boundary (principle 3) | M2 |
| The corpus fixture: the caller's display lists as neutral data, for the census and beyond | M5 |
| The knockout group with its diagonal edge; the sixteen-mode scene; the soft-mask agreement test | M6 |

## Which milestone fills which module

The skeleton's modules each state their contract (ADR 0003); this is the map of who
deletes which `text` block.

| module | filled by |
|---|---|
| `quorra-gpu`: `device`, `pipeline`, `target`, `frame`, `viewport`, `report`, `error`, `surface`, `readback`, `timing` | M1 ✓ (and every milestone after adds to `pipeline`) |
| `quorra-gpu`: `resources` | M2 ✓ |
| `quorra-scene`: `geom`, `scene`, `paint` (solid half), `image`/`mesh` (upload specs) | M2 ✓ (geom landed with M1) |
| `quorra-gpu`: `atlas` | M4 ✓ |
| `quorra-scene`: `mask`; `quorra-gpu`: `mask` | M6 ✓ |
| `quorra-scene`: `paint` (shading half), `image`, `mesh` | M7 ✓ |

## Integration notes against the caller — the recorded divergences from the brief

These are the places where the brief and `pdf-render` did not line up exactly. None was a
problem; each was a question whose answer belongs in the API rather than in a translation
layer's judgement (§4.5: a decision either side can make alone is a decision neither side
has made). They were raised before M2 froze the signatures, and each note carries where it
was settled and what it settled as. **The numbering is load-bearing** — module docs in the
tree cite these notes by number, so a note is corrected in place and never renumbered.

1. **The image filter flags are not flags.** §4.5 names `Image::is_smoothed` and
   `Image::area_averaged` as flags to honour. In the tree they are *methods taking the
   placement transform* — `is_smoothed(placement)`, `area_averaged(placement) ->
   Option<Image>`. So what must reach us is the **resolved** decision for the placement
   the command carries, not the flag it was derived from. Refined at M2, from the
   caller's types: since the decision is per *placement* and an upload is per *image*,
   the resolved filter belongs on the image **command** (M7), and `ImageSpec` carries
   pixels only — which is also what lets one upload serve every zoom level.
2. **`Viewport` versus `TargetSpec`.** §2.4's viewport carries a full affine — scale,
   y-flip and tile offset. The trait in the tree carries a scale and a pixel budget
   (`TargetSpec::for_page(list, scale, max_pixels)`). The affine is the more general
   of the two and is what tiled output and damage need; the bridging belongs in the
   caller's adapter, and the rounding rule for a fractional page stays theirs.
3. **The soft-mask transfer function.** The brief says a 256-entry lookup table because
   a mask value is one byte. The tree has a `Transfer` type; we take `[u8; 256]` and
   the caller samples. Confirm the sampling convention — inclusive of both endpoints —
   in the conformance test, not in prose.
4. **Which `raw-window-handle`.** §2.1 asks for `raw-window-handle` and nothing more
   specific, but the version has to be the one `wgpu` was built against or a surface
   cannot be created. Reach it through a `wgpu` re-export if one exists in 30.0.0;
   otherwise pin the same version here and say so.
5. **`MeshRaster` is shared on purpose.** Both of the caller's backends share the
   pre-rasterised mesh because neither rasteriser has the primitive and a second copy
   would drift. We inherit that: we consume the mesh, we do not re-triangulate it.
   Confirmed at M2 from the caller's type: a `MeshRaster` is a **device-resolution**
   positioned raster, so a mesh upload is viewport-dependent and a zoom re-uploads its
   meshes — the cost of the shared-rasteriser correctness argument, taken upstream and
   inherited knowingly.
6. **Colour is not ours.** Device RGB arrives; `ColourSpace::to_rgb` upstream is the
   only place a colour becomes RGB, and adding a second one is forbidden. If we offer
   colour management the caller cannot use us.
7. **`release` returns a `Result`, deviating from §2.2's `()` signature.** A release
   of an id the device never issued — or issued and already released — is
   `DeviceError::UnknownResource`, because a double release is a caller bug and a
   no-op would hide it. The brief calls its signatures illustrative; the property
   (no silent error swallowing) outranks the shape, and this is flagged for the API
   conversation before M9.
8. **The `mask` parameter comes last**, in every builder method, rather than beside
   `clip` as the brief's illustrative signatures place it: growing the vocabulary
   milestone by milestone was then a mechanical widening at every call site. Flagged
   with note 7 for the API conversation before M9.
9. **Shading geometry travels on the paint; sampled shadings become images.**
   Decided at M7 (ADR 0011), refined at M9: `Paint::Shading { ramp, kind,
   transform }` carries the axial or radial geometry (six floats) in the
   **shading's own space**, with the paint's transform mapping it into the scene —
   §8.7.4.3's shading matrix, anchoring a shading to the page rather than to the
   path it fills, exactly as the caller's `Shading { kind, transform }` states it.
   One uploaded ramp serves every placement and zoom — §2.2's economy, and the
   scene stays viewport-free (§2.3). The caller's `ShadingKind::Sampled` — its
   display list holds a *grid*, not a function — maps to an image upload plus an
   image command in the M9 adapter; `ShadingKind::Mesh` maps to `upload_mesh` +
   `Paint::Mesh` (note 5's device-resolution caveat applies).

## Not planned, ever

§9's non-goals: colour management, font loading, shaping, hinting, text layout, bidi,
filters and effects beyond §4, any document format, a scene graph or animation, hit
testing. `deny.toml` enforces as much of the list as a dependency policy can express.
