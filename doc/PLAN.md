# Plan

The brief is `RENDER_LIBRARY.md`; this file is the design in its current state of
belief, the order of work, and the state of both. Bare section numbers (§) are the
brief's; "clause" numbers are ISO 32000-2's.

The file has two parts. **Part 1 says how the library will work** — the architecture as
currently believed, with each piece naming the measurement that could overturn it,
because §11 is explicit that the design should turn on measurements rather than on
anyone's taste, and most of those measurements have not been taken yet. **Part 2 says
the steps that get there** — nine milestones, each with its work, its exit gates, and
the question it settles. When a Part 1 hypothesis is confirmed or overturned, the
decision becomes an ADR and this file is corrected in the same commit; a plan that
disagrees with the tree is worse than no plan.

## Where we are

**Coverage can come from the GPU, and the lane is proven but not yet reachable**
(2026-08-05, ADR 0016): Evan Wallace's method — one triangle per outline segment
fanned from an anchor, accumulated with additive blending so that what lands at a
sample *is* §8.5.3.3's winding number, plus a Loop-Blinn control triangle per curve
whose orientation alone decides whether the bulge is added or bitten out. Cubics
become quadratics **once, at upload**, so no step in the lane knows the device scale:
that is the answer to the thing the cull uncovered, where a zoom gesture makes every
cached tile cold on every frame. Signed accumulation rather than his parity trick,
because parity is even-odd only and §8.5.3.3.2's non-zero is PDF's default; samples in
an `rgba16float` texel's channels rather than packed into a byte's bits, so sample
count costs time and not memory; and the sample grid stated in our own code rather
than taken from the driver, so ADR 0006's cross-adapter identity survives.

**What is landed is the lane and its proof** — geometry (`outline.rs`), shaders
(`shaders/winding.wgsl`), pipelines, and the frame pass (`winding.rs`), exercised end
to end on a real device: an aligned square solid inside and empty outside, a
half-covered column reading exactly 128 as the 4×4 grid derives, and nested same-wound
squares filling under one rule and hollowing under the other. **The encoder does not
route anything to it yet**, and the three things wiring it needs are named in ADR 0016:
a scratch reservation without bytes, the frame budget pricing the winding texture
(`Sheet::device_bytes` is written and uncalled), and residue clips, which still
multiply into a coverage mask on the CPU. Until then the `allow(dead_code)` markers
name the commit that deletes them. The number that decides *when* a caller should
choose this lane — the magnification at which it overtakes the CPU one — cannot be
measured until the encoder can reach it, and no one should guess it in the meantime.

**A frame costs what it shows, not what the page holds** (2026-08-04): the caller
zoomed, and found that a frame got *more* expensive the further in a person went —
the encoder flattened all 5 933 commands of a page for a window displaying 24 of
them. ADR 0012's recorded lever is now taken (ADR 0015): a command whose device
bounds, inflated by two pixels for the glyph lane's quantised phase and the coverage
lanes' `floor`/`ceil`, miss `clip ∩ target` is rejected before its geometry is built,
and `Counters::commands_culled` reports how many. A **zoom gesture** — 1× to 20×
over 24 frames, no cached tile helping any of them — went from a worst frame of
**156 ms of encode to 9.3 ms**; a page with nothing off the target pays about 6–10%
more encode for the test, which is written down rather than hidden. Two things it
deliberately does not do: a group is not culled as a unit, and damage still is not a
cull. **What the cull uncovered is the next question**: at 20× the residual 6.8 ms is
30 glyph tiles of ~290 px rasterised again on every frame, because past
`MAX_GLYPH_DIM` a glyph never enters the atlas — a probe raising that constant takes
the same frame to 0.25 ms. That is an atlas *policy* question (what a large tile may
cost against the budget, whether a gesture's key churn can be kept from thrashing
it), not the GPU-coverage question of ADR 0008, and it wants its own ADR and its own
measurement. `examples/zoom.rs` is the harness; `tests/cull.rs` and a
deterministic count gate in `tests/perf_gate.rs` hold the behaviour.

**Feedback §8 is answered — bring-up is measured per step, and its largest step can
start before there is a window** (2026-08-04): the caller's owner decided that page
one goes to the graphics device, which put our bring-up on their time-to-first-page
— 45.1 ms of a 144.6 ms launch, 31% of it — and their §8 asked for the two things
that makes necessary. Both landed (ADR 0014). **`StartupTimings` is five numbers
instead of three**: `instance_creation`, `surface_creation`, `adapter_selection`,
`device_creation`, `pipeline_compilation`, plus `blocking_total()` over the four the
constructor actually waits for. The field it replaces, `adapter_enumeration`, named
one step and measured three, so nothing that moved inside it could be attributed —
which is now the tree's own worked example of the instrumentation rule about counts
versus rates. **`startup::create_instance()` plus `Device::headless_with_instance`
and `Device::for_surface_with_instance`** let a host build the instance on a thread
at `main`'s first line, in parallel with reading its document: measured headless on
RADV here, instance creation is ~80% of what bring-up blocks for (22.9–29.8 ms of
29.4–36.1 ms), and hoisting it leaves **5.1–9.2 ms** to pay after the window exists.
`instance_creation` is then `None` rather than zero — the step happened, on someone
else's clock, and a struct for attribution may not claim otherwise. The backend knob
§8.3 talks a host out of is **not** added, and ADR 0014 records the caller's
measurement that says why. `examples/startup.rs` is the measurement, one
configuration per process by design.

**Feedback §7 is answered — a refusal costs the surface nothing** (2026-08-04): the
viewer reproduced a permanent wedge (every acquire a 1-second `Timeout`, only a
resize recovering) whose cause was a budget refusal *after* the swapchain acquire —
the dropped, never-presented texture leaves an acquire semaphore no submission waits
on, and enough of those exhaust the swapchain. Three changes, one hardening: the
compositor's internal textures are now priced straight after encode, before the
target is bound, so a refused frame acquires nothing (`Options::max_frame_bytes`'s
"before anything is allocated" is now true of the acquire too, and
`tests/m1.rs::frame_budget_refusal_precedes_target_binding` pins the ordering
through the headless `NoSurface`-vs-`FrameBudgetExceeded` distinction); `Timeout`
now sets `needs_reconfigure` exactly as `Outdated` does, so the wedge is at worst
one bad frame; `Device::invalidate_surface()` is the host's explicit lever
(`NoSurface` on a headless device — a caller bug refused by name); and a frame that
fails *after* its texture was acquired (`run_frame`, the one remaining post-acquire
early return) now invalidates the surface on its way out, bounding that path at one
lost frame too. `FrameBudgetExceeded`'s message no longer claims "instance data"
when what overflowed was internal textures — it names scene-derived bytes, which is
what the shared budget prices. Awaiting the viewer's re-run of their one-drag
reproduction on a real surface; every headless-provable piece is gated.

**The corpus feedback is answered** (2026-08-03): the viewer measured the swapped
backend against its 974-document corpus and wrote up what came back
(`pdf-viewer/doc/QUORRA_FEEDBACK.md`); everything actionable landed the same day.
On this side: the frame's scratch sheet now spans the full device dimension —
capacity, not commitment, since bytes stay budget-charged per tile — and its
exhaustion is its own `RenderError::ScratchExhausted` naming the real limit,
replacing a refusal whose arithmetic contradicted itself (six real pages refused
under a 2048-wide sheet now draw; the corpus's one pathological page still
refuses, truthfully). On the adapter's side: §10.7.4's degenerate fills draw
through the viewer's shared split; the `Arc`-pinned caches gained LRU eviction to
half the resource budget (533 refusals at 4× scale became zero);
anisotropically-transformed strokes outline in path space instead of taking one
scalar width (three corpus pages moved from "differs in shape" to agreement); and
an empty mesh raster draws nothing, as both sibling backends and pdf.js's own fix
for the defective document do. Corpus after: **910 of 957 agree, 46 differ (29 at
the antialiasing floor), 1 refused** — from 900/50/7.

**M9 is done — the swap happened** (2026-08-03): `render-quorra` in the caller's
tree implements their `Rasterizer` over this library and passes their cross-backend
and real-page suites at the Vello backend's own thresholds; the viewer's window now
presents through quorra's surface tier (no readback, −205 lines of host machinery),
verified under Xvfb with real key presses on ISO 32000-2 itself. The full record —
the integration refinements it forced here, the two adapter defects the caller's
instruments caught, and the one owner-level follow-up (the corpus sweep) — is in
the M9 section. The library's home is https://github.com/close2/quorra, and the
viewer consumes it from there.

**M8 is done** (2026-08-02): the rest of the performance contract, decided by
measurement. **Damage is honoured exactly** (ADR 0012): a valid `Viewport::damage`
against a retained `Texture` target renders the frame internally with every pass
scissored to the damage bounding box — sound because every pass is pixel-local —
and patches exactly the listed rectangles onto the target with REPLACE blits over
`LoadOp::Load`, so nothing outside the list is touched and nothing can
double-composite. `Surface`/`Readback` targets redraw fully and say so in a
`Report` naming the kind; malformed rects refuse by index; a list that clamps to
nothing touches no pixel. Measured (dense page, 1191×1684, one 12×18 caret rect):
RADV execute **0.136 → 0.047 ms**, llvmpipe **4.2 → 1.6 ms/frame**; encode still
walks the whole scene (~0.1 ms) — command culling and a bbox-sized root texture are
ADR 0012's recorded levers. **The pipeline-cache question closed against the
`unsafe` exception** (ADR 0013): construction is 19.9 ms adapter + 11.9 ms device,
neither cacheable; the warm set compiles in 9.4 ms on a thread `Device::headless`
never waits for — no user-visible number to win, so principle 3's bar is not met
and `#![forbid(unsafe_code)]` stands. **No texture pool** (and so no shrink
policy): internal-texture creation sits inside the measured 0.37 ms patched frame;
pooling waits for a measurement that says otherwise. **§11.5's verdict: hold the
scenes.** A dense-page `Scene` retains **570 KB** (the figure-laden page 571 KB),
so the dozen-resident-pages target costs ≈ 6.8 MB — noise next to a single
1191×1684 target's 8 MB, and `Scene::cost().retained_bytes` keeps it checkable.
Gates in `tests/m8.rs`.

**M7 is done** (2026-08-02): the rare-case lanes (ADR 0011). An image (§8.9.5), a
ramp shading (§8.7.4.5.2/.3) or a pre-rasterised mesh draws as **one uniform-driven
quad** inside the ordinary passes — no third instance stream for primitives the
brief's §0 calls rare. Both shaders map device pixels back through the inverse
transform: an axis-preserving image gets the rectangle lane's analytic edge coverage,
an oblique one paints centres-inside-the-unit-square (hard edges, stated); nearest
filtering is `textureLoad` and adapter-invariant, linear is the hardware sampler with
its variance stated and shape-gated. Ramps pre-sample on the CPU to 256 RGBA8 texels
indexed at `round(t·255)`, so the sweep arithmetic is ours — the axial projection and
the radial quadratic run in shading space and survive shears. Unextended sweep
regions paint *nothing* (§8.7.4.5.2), and therefore knock nothing out; the shading
question deferred from M2 closed on geometry-on-the-paint (integration note 9). GPU
textures realise lazily on first draw and die with `release`. Every lane rides
clip/mask/blend/knockout machinery unchanged. Measured (release, texture target,
1191×1684): the dense 5 933-rect page carrying 8 images, 6 shadings and a mesh runs
**0.63 ms/frame on RADV** (0.31 ms without the figures) and 4.2 ms on llvmpipe —
both far inside the 5.9 ms CPU baseline. Gates in `tests/m7.rs`: exact nearest
blocks, §8.9.5 orientation, clause-derived axial/radial bytes, extend-off
transparency, mesh anchoring, coverage agreement with the solid lane, unknown-id
refusals by name, and the cross-adapter ±2 bound on the deterministic paths. With
M7, **the refusal list is empty**: every scene command draws.

**M6 is done** (2026-08-02): clause 11, natively (ADR 0010). Groups are layers
composited once through an in-shader implementation of §11.3.6 with all sixteen
§11.3.5 blend functions (REPLACE target state: the arithmetic is ours, not the blend
unit's — which is what ADR 0006 demanded); knockout and `Compose::Src` run as
erase/add pass pairs strictly per element, and the diagonal-edge fixture holds the
result to §11.4.6's own formula; soft masks render through the same machinery and
reduce on the device via a mirror of the caller's `SoftMask::value` — **all 256 bytes
of both rules agree exactly**, non-black backdrop and non-identity transfer included
(`tests/m6.rs`). An element with a non-Normal blend becomes an implicit one-element
group, so §11.3.5 has one implementation. Flat frames still draw straight into the
target — the M1 fast path is untouched — and layered frames price their internal
textures against the frame budget before creating any. The scene API grew `mask()`
and the mask parameter (integration note 8: mask comes last, a recorded divergence
from the brief's illustrative order). At M6's close only images remained refused;
M7 closed that too.

**M4 and M5 are done** (2026-08-02), on one shared foundation: a CPU coverage
rasteriser of our own (`raster.rs`, ADR 0008) — exact trapezoid accumulation, both
fill rules, cubic flattening at a stated tolerance, stroke expansion with §8.4.3's
caps, joins and miter limit — feeding two lanes. The **glyph lane** caches R8 tiles
in a persistent atlas keyed `(outline, linear part bit-exact, quantised phase)` with
the 1/16 quantum settable and off-able (`Options::glyph_quantum`, ADR 0009); the
**path lane** rasterises uncached coverage into a per-frame scratch image — large
fills, strokes, oblique rectangles, and the non-rectangular clip residues that M3
deferred, multiplied in per link. Both draw as instanced quads with `textureLoad`
(no sampler, no filtering) and the analytic clip rectangle in the shader. Scene order
is preserved across lanes by batch breaks, never reordering.

The §11.2 census **remains open** — the path-lane design was chosen as the smallest
correct one the census can overturn, and ADR 0008 names the compute-shader lever if
it does. What the M4/M5 record already shows (release, RADV, texture target,
1191×1684): a dense page of 5 933 *curved* glyph fills runs **1.0 ms/frame** steady
state — warm encode 0.73 ms (inside the caller's 1.1–1.6 ms budget), execute 58 µs —
with a 1.9 ms cold frame to rasterise its 107 tiles; the atlas-hostile page (§11.3:
fresh phases everywhere) pays ~7 ms on its cold frame and is indistinguishable warm,
so the atlas's failure mode is bounded by CPU rasterisation throughput and its win is
the caller's 5.0× reuse made real (5 933 fills → 107 keys → 107 entries, pinned in
`tests/m2.rs`). Cross-lane gates (`tests/m45.rs`): the analytic rectangle and the
rasterised rectangle agree within one premultiplied step; atlas-backed and
atlas-starved frames are byte-identical; the cross-adapter bound holds at ±2 for the
new lanes because the coverage bytes themselves are CPU-made and adapter-invariant.

**M3 is done** (2026-08-02): rectangular clips, analytically. Clip chains resolve at
encode time to one device-space rectangle each — memoised across shared prefixes, the
region (never the identifier) counted in `Counters::clip_distinct_regions` — and the
rectangle lane applies a clip by intersection on the CPU, so a rectangular clip costs
the device nothing at all (ADR 0007; the brief's shader-side comparison arrives with
the glyph lane, which cannot pre-intersect). The M3 fixtures pin the two numbers the
milestone exists for: 303 identical clip states collapse to **1** region on a full
page at 1191×1684, and the 3 608-chain worst page resolves within the ordinary
budgets, every distinct region counted. Empty-admits-nothing versus absent-clip is
tested as two different answers; a non-rectangular clip is refused by name until M5's
residue masks. `SceneBuilder::rect` gained its clip parameter; `axis_aligned_rect` in
`quorra-scene` recognises rectangle outlines once, at upload.

**M2 is done** (2026-08-02): the scene vocabulary of §2.3 minus what later milestones
own — `fill`, `stroke`, `rect`, `clip` chains and bounded `group`s, every input
validated loudly at the builder (§4.7) — plus the device's resource registry:
`upload_outline`/`upload_image`/`upload_ramp`/`upload_mesh`/`release`, each upload
validated and priced against a stated budget (`Options::max_resource_bytes`,
discoverable through `Device::limits`). `Scene::cost()` now reports commands, clips,
group depth and retained bytes, computed once at `finish`. A command whose lane does
not exist yet is refused by name (`RenderError::NotYetDrawable` says which command,
what kind, and which milestone delivers it) — drawn or refused, no third state. The
scene boundary is fuzzed from this milestone on: a deterministic structured fuzzer
(`tests/fuzz_scene.rs`) drives hostile builder/upload/render sequences on every push;
coverage-guided `cargo-fuzz` needs a nightly toolchain and stays outside the pinned
tree, a recorded choice, not an omission. The drawable half of §2.2's round trip —
107 outlines actually painting 5 933 fills — is M4/M5's proof, and its test lands
there. Also landed since the M1 record: the surface path is proven against a real
window (`examples/window_smoke.rs` under Xvfb, pixels verified via `xwd`; in CI on
every push).

**M1 is done** (2026-08-02): a device (headless and surface-attached), all three
targets of §2.4, the analytic rectangle lane, timestamped and truthful frames, the
startup split of §7 — plus the harness (goldens against a CPU reference, byte-equality
and bounded-difference gates, refusal tests, a perf gate with measured thresholds).
Two of §11's questions now have measured answers; the M1 record below has the numbers.
One deviation from the original M1 scope is recorded rather than silent: the pipeline
cache blob moved wholly to M8, because wgpu 30 exposes it only through an `unsafe`
constructor and this tree is `#![forbid(unsafe_code)]` — weighing that exception is
M8's ADR.

Every number quoted in Part 1 below was measured in the caller's tree against the
Vello-based backend this library replaces; the **M1 record** is the first set of
numbers measured in *this* tree.

### The M1 record (fastest of ten, release, this machine, 2026-08-02)

`examples/floor.rs`, 5 933 rectangles (a dense page's command count; rectangles stand
in for glyphs until M4) at 1191×1684, phases from timestamp queries:

| adapter | encode | execute | readback | whole frame, Readback | whole frame, Texture |
|---|---|---|---|---|---|
| RADV (890M) | 0.035 ms | **0.048 ms** | 4.10 ms | 4.58 ms | **0.22 ms** |
| llvmpipe | 0.035 ms | 2.59 ms | 4.26 ms | 8.07 ms | 2.98 ms |

Startup on RADV: adapter enumeration 22.8 ms, device creation 15.4 ms, warm pipeline
compilation 2.65 ms (off the critical path; `headless` returns before it). That first
figure is the pre-split one — instance creation plus adapter selection, and mostly the
driver loader; ADR 0014 re-attributes it and the "Where we are" entry above carries
the current numbers.

Three honest caveats: rectangles are not glyphs (no atlas, no clip states); the encode
translates our own scene rather than the caller's display list; and the caller's
5.9 ms/12.1 ms baselines were measured on their harness, not ours. The *structural*
findings survive the caveats, and they are the two answers below.

**§11.1 answered: the readback is essentially the whole fixed cost.** On the real GPU,
device execution for a dense page at window scale is 48 µs; the readback is 4.1 ms —
roughly 90% of the offscreen frame — and a texture-target frame costs 0.22 ms total.
The brief's ranking (surface and texture paths first) is confirmed, emphatically: tier
2/3 hosts skip what is by far the largest item. Also reproduced on our design: 5 933
fills execute in 48 µs against 7 µs for one rectangle — the per-command device cost is
noise compared to the per-byte costs.

**§11.4 answered: cross-adapter byte identity is not achievable through the
fixed-function raster path** (ADR 0006). The float→unorm8 store conversion rounds
differently on RADV and llvmpipe — measured on a single opaque rectangle, before any
blending. Same-adapter byte identity holds and is gated exactly; cross-adapter output
is gated to a stated bound (±1 unorm step per blend stage, ≤ ±2 after straight-alpha
conversion on the golden). The design lever: identity returns if the compositor owns
final quantisation in shader code, which M6 must weigh anyway for the fifteen
non-Normal blend modes — the caller's CI reliance on identity against the measured
price of shader-side quantisation.

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
target initialised to transparent; its children draw into it; the finished layer is
composited onto its parent exactly once, under the group's constant alpha and blend
mode (§4.4; clause 11.4.1, 11.4.5). Every group is isolated — the caller guarantees it
and asks us to document the assumption, which this sentence and the rustdoc both do.
Nesting is bounded at 16 (§1.1), so the layer stack is countable in phase 1 like
everything else. The page itself renders onto transparency, always, because clause
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
Where a residue mask is built, it is cached **keyed by the resolved region under the
current viewport, never by an identifier** — the caller's clip-mask cache once answered
all 303 lookups a page made and built 303 identical page-wide masks because the key was
a name (ADR 0132 lesson, restated in CLAUDE.md) — and `Counters` reports the count of
distinct regions, not a hit rate. An empty clip admits nothing, which is a different
thing from an absent clip, and both have tests.

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
times, and a zoom re-uploads nothing. `Scene::cost()` is computed at `finish` time, so
asking it costs nothing per frame. §11's question 5 — what a scene costs to hold,
against a target of a dozen resident pages — gets its number in M2 and its verdict in
M8.

## 1.10 The five questions, where each is answered, and what turns on each

| §11 question | answered | what turns on the answer |
|---|---|---|
| 1. How much of the fixed cost is the readback? | **Answered at M1**: ~90% of an offscreen dense-page frame on RADV (4.1 ms of 4.6; execute is 48 µs). See the M1 record. | tiers 2–3 are confirmed as the headline; per-pixel work is second-order for tier 2/3 hosts |
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

**Done** (2026-08-02): ADRs 0008/0009, the record in "Where we are", gates in
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
optimisation candidate for a bbox-bounded version, with M8's measurements.

## M7 — Shadings, meshes, images

**Deliverable:** axial, radial and function-based shadings from a `RampId` (clause
8.7.4.5); the caller's pre-rasterised mesh consumed as-is (integration note 5); decoded
RGBA8 images with the filter decision arriving resolved (integration note 1). Straight
alpha at the boundary, premultiplied internally, rendered onto transparency, always.
All of these pipelines compile on first use (§1.8) — a page of plain text never pays
for them. The open `ShadingKind` question — geometry on the paint versus resolved at
upload — is decided here with a measurement and an ADR.

**Done** (2026-08-02): ADR 0011, the record in "Where we are", gates in
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
`#![forbid(unsafe_code)]` stands). The record and numbers are in "Where we are";
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

## Integration notes against the caller — settle these before M2 freezes the API

These are places where the brief and `pdf-render` as it stands today do not line up
exactly. None is a problem; each is a question whose answer belongs in the API rather
than in a translation layer's judgement (§4.5: a decision either side can make alone is
a decision neither side has made). The numbering is load-bearing — module docs in the
tree cite these notes by number.

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
