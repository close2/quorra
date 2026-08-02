# A document renderer: the library this viewer wants

> **Provenance.** This is a verbatim copy of `doc/RENDER_LIBRARY.md` from the consuming
> project at `/home/cl/projects/pdf-viewer`, taken 2026-08-02, and it is the requirements
> document for this repository. It is kept as a copy rather than a symlink so that this
> tree builds and reasons on its own, and so that a change on their side is visible as a
> diff rather than as a surprise. Everything below is written from the caller's point of
> view — "we" and "this viewer" are them, "you" is us. Do not edit it here: an
> observation of ours about a requirement belongs in `doc/PLAN.md` or an ADR, and a
> requirement that needs to change is a conversation with the caller.

Written 2026-08-02. A specification of the *perfect* 2D rendering library for this PDF viewer,
for a team building one. It is a wishlist, not a contract — but every requirement in it is here
because something in this repository measured it, and the measurements are quoted so that you can
argue with them.

**The one-sentence brief.** Vello is a general 2D vector renderer that happens to be pointed at
documents; what this viewer wants is a **document** renderer — one whose fast paths assume that
most of a page is the same few glyph outlines repeated at many sub-pixel phases plus axis-aligned
rectangles, and which treats general curve filling as the rare case rather than the uniform one —
and which expresses ISO 32000-2 clause 11's transparency model *natively*, because that is the
part an SVG-shaped model cannot be patched into.

---

## 0. How to read this

**What we are.** A PDF viewer in Rust aiming at Acrobat-class fidelity and at being the fastest
one available. `CLAUDE.md` at the root states the five principles this project is held to; the two
that reach you are *the specification is the only source of truth* (we never tune output to match
another renderer) and *unsupported input must stay loud* (a backend refuses; it never draws
something plausible instead).

**What already exists here**, and what you are being asked to replace or sit beside:

| | |
|---|---|
| `pdf-render` | The neutral display list and the `Rasterizer` trait. **This is the contract**, and it does not change for you. |
| `render-cpu` | `tiny-skia`. Our **correctness oracle** and our **startup path**. Not going away. |
| `render-gpu` | Vello on wgpu. What a new library would replace. |

**Every number below was measured in this tree**, on an AMD Radeon 890M (RADV, Vulkan) with a
24-core CPU, and says which page it came from. Where something is not measured, it says so. The
ADR numbers point at `doc/adr/`, where each measurement's method is written down.

**The three numbers to read first**, all in §6.1, because they surprised us and should shape your
design more than anything else here: scene encoding is **1.1–1.6 ms and flat in resolution**;
between **55% and 92%** of an offscreen frame is paid before any of the page is drawn; and a page
of **5 933 glyph fills costs about what one rectangle costs** at the same target size.

---

## 1. What we hand you

The input is a **display list**: a flat, immutable sequence of drawing commands, built once by
interpreting a page's content streams and then shared behind an `Arc`. Building it is the
expensive part of showing a page and it does **not** depend on resolution, so one list is
rendered at many scales — that is the zoom and scroll case, and it is why the target is a separate
argument everywhere.

### 1.1 The shape of it

```rust
enum Command {
    Fill   { path: Arc<Path>, transform, fill_rule, paint, clip: Option<ClipId>,
             mask: Option<SoftMaskId>, blend: BlendMode },
    Stroke { path: Arc<Path>, transform, stroke: Stroke, paint, clip, mask, blend },
    Image  { image: Image, transform, alpha: f32, clip, mask, blend },
    Group  { commands: Vec<Command>, alpha: f32, clip, mask, blend, knockout: bool },
}
```

Four properties we rely on, and which you may rely on too:

1. **Every command carries its own absolute transform and clip.** Nothing is inherited from a
   position in the list. This is what lets a backend reorder or parallelise them, and our CPU
   backend now does exactly that (ADR 0139).
2. **The list is flat except for `Group`**, which is the one nested command, bounded at 16 deep.
3. **Clips and soft masks are referenced by identifier**, not carried, because one clip commonly
   applies to thousands of commands. A clip is a path plus a parent, so a chain is an
   intersection. **`DisplayList::add_clip` already deduplicates identical regions**: page 6 of ISO
   32000-2 states one clipping rectangle 303 times and gets one identifier (ADR 0132).
4. **Paths are shared.** Every occurrence of a letter on a page is the same `Arc<Path>`. Page 6 is
   **5 933 fills of 107 distinct outlines**.

Paths are move/line/**cubic**/close — no quadratics, because PDF has no quadratic operator and
TrueType outlines are elevated during glyph loading, so the whole pipeline handles one curve type.

Paints are `Solid(Color)` or `Shading(Arc<Shading>)`, where a shading is axial, radial,
function-based, or a **`MeshRaster`** — a pre-rasterised triangle mesh that both our backends
share because neither rasteriser has the primitive and a second copy would drift (ADR 0051).

Images are decoded RGBA8, row-major, no padding, plus two flags that are **decisions we have
already made and you must inherit rather than re-take** — see §4.5.

### 1.2 What we would like your scene model to be

**A superset of the above, mapping one-to-one.** Our encoder should be mechanical: one of your
scene items per one of our commands, with no case where we have to choose between two of your
primitives or emulate one of ours out of several. Every place a translation has to *decide*
something is a place where our two backends can silently diverge, and this project has paid for
that four times (trap 2 in `doc/HANDOVER.md`).

Concretely, the four things a general vector API usually lacks and we need:

- a fill whose compositing is **Porter-Duff Source modulated by coverage** (§4.1);
- a soft mask that is a *rendered group* reduced by a stated rule, not an alpha texture (§4.2);
- all sixteen of ISO 32000-2 §11.3.5's blend modes, the four non-separable ones included (§4.3);
- a group that composites onto **transparency** and is then painted once (§4.4).

---

## 2. The API we want

Signatures below are illustrative; the properties they carry are the requirement.

### 2.1 Device

```rust
pub struct Device { /* … */ }

impl Device {
    /// Headless. No window, no surface. This is the form our test suite and our
    /// oracle use, and it must be a first-class citizen rather than an afterthought.
    pub fn headless(options: &Options) -> Result<Self, DeviceError>;

    /// Attached to a surface. `raw-window-handle` and nothing more specific.
    pub fn for_surface(handle: impl HasWindowHandle, options: &Options)
        -> Result<Self, DeviceError>;

    pub fn description(&self) -> &str;      // adapter name, for reports and goldens
    pub fn limits(&self) -> Limits;         // what this adapter can actually do
}
```

**`Device` must be constructible on a background thread and must not need one.** Page one of a
document renders on the CPU backend while the GPU initialises, which is `CLAUDE.md`'s startup
rule; a `Device::headless` that blocks a main thread for 200 ms is a design we cannot use even if
every frame afterwards is free. See §7.

### 2.2 Resources: uploaded once, referenced many times

```rust
pub struct OutlineId(u32);
pub struct ImageId(u32);
pub struct RampId(u32);
pub struct MeshId(u32);

impl Device {
    pub fn upload_outline(&mut self, path: &[Segment]) -> Result<OutlineId, DeviceError>;
    pub fn upload_image(&mut self, image: &ImageSpec) -> Result<ImageId, DeviceError>;
    pub fn upload_ramp(&mut self, stops: &[Stop]) -> Result<RampId, DeviceError>;
    pub fn upload_mesh(&mut self, mesh: &MeshSpec) -> Result<MeshId, DeviceError>;
    pub fn release(&mut self, id: impl Into<ResourceId>);
}
```

The point of separating upload from scene building is that **the 107 outlines of page 6 are
uploaded once and referenced 5 933 times**, and that a zoom does not re-upload them. Our display
list already gives us identity for free: `Arc::as_ptr` is the key.

### 2.3 Scene: retained, and independent of the viewport

```rust
pub struct SceneBuilder { /* … */ }
pub struct Scene { /* … */ }   // Send + Sync, cheap to clone (Arc inside)

impl SceneBuilder {
    pub fn fill(&mut self, outline: OutlineId, transform: Affine, rule: FillRule,
                paint: Paint, clip: Option<ClipId>, mask: Option<MaskId>,
                blend: BlendMode, compose: Compose);
    pub fn stroke(&mut self, outline: OutlineId, transform: Affine, stroke: &Stroke, …);
    pub fn rect(&mut self, rect: Rect, transform: Affine, paint: Paint, …);
    pub fn image(&mut self, image: ImageId, transform: Affine, alpha: f32, …);
    pub fn group(&mut self, group: GroupSpec, body: impl FnOnce(&mut SceneBuilder));

    pub fn clip(&mut self, outline: OutlineId, transform: Affine, rule: FillRule,
                parent: Option<ClipId>) -> ClipId;
    pub fn mask(&mut self, kind: MaskKind, body: impl FnOnce(&mut SceneBuilder)) -> MaskId;

    pub fn finish(self) -> Scene;
}
```

**The single most important property in this document: a `Scene` must contain no reference to a
viewport, a resolution, a device transform, or a target size.** Zoom, scroll, window resize and
tiled output are all *the same scene at a different viewport*. If building a scene is a function
of the target, then every zoom step re-does it, and we measured what that costs in §6.

A corollary we would like stated in your documentation, because it is what makes the design
worth having: **`Scene: Send + Sync`, and building one requires no device**. Our interpreter runs
on a worker thread and would build scenes there.

Note `rect` beside `fill`. A rectangle is not a special case of a path for a document renderer —
see §6.4.

### 2.4 Frames

```rust
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    /// Maps the scene's coordinate space to target pixels. Carries the scale, the
    /// y-flip and any tile offset.
    pub transform: Affine,
    /// Rows/regions known to have changed. Empty means "all of it".
    pub damage: &[Rect],
}

impl Device {
    pub fn render(&mut self, scene: &Scene, viewport: &Viewport, into: Target)
        -> Result<Frame, RenderError>;
}

pub enum Target<'a> {
    /// Tier 1: we want the pixels. Straight-alpha RGBA8, our `Raster`.
    Readback,
    /// Tier 2: draw into the surface you were constructed with.
    Surface,
    /// Tier 3: a texture we own, for a host that composites it itself.
    Texture(&'a wgpu::Texture),
}

pub struct Frame { /* … */ }
impl Frame {
    pub fn reports(&self) -> &[Report];   // §5
    pub fn timings(&self) -> Timings;     // §8
    pub fn into_raster(self) -> Result<Raster, RenderError>;  // Readback only
}
```

Three target kinds because we have three kinds of host and the difference is measurable: §6.1
shows the readback dominating an offscreen frame. A library that offers only readback is a
library whose window path we cannot make fast.

### 2.5 Reports

```rust
pub struct Report {
    pub kind: ReportKind,      // enumerated, not a string
    pub detail: String,        // for a person
}
```

Emitted for anything the device could not draw *as asked*. See §5 — this is not a logging
convenience, it is the difference between a viewer that tells a person what is missing and one
that lies to them.

---

## 3. Coordinate and colour conventions

- **Colours reaching you are already device RGB**, premultiplied nowhere until you composite.
  Colour management happens upstream: ICC profiles, `DeviceCMYK`, `CalRGB`, `Separation`,
  rendering intents, black point compensation. `ColourSpace::to_rgb` is the *only* place in this
  tree a colour becomes RGB and adding a second one is forbidden (trap 6). **Do not colour-manage
  anything.** If you offer it, we will not use it, and if it is on by default we cannot use the
  library at all.
- **The page's own space is y-up**; the y flip is in the viewport transform, not in the scene.
- **A finished page is composited onto the medium by us**, after you return it, because
  §11.4.7 makes the page group *isolated* and painting the medium first is a different picture.
  So: **render onto transparency**, always, and hand back straight-alpha RGBA8 for `Readback`.
- **Straight alpha at the boundary, premultiplied internally.** Converting once at your boundary
  is cheaper than converting per comparison, and it is what PNG and our harness expect.

---

## 4. The correctness contract

This section is why a general library does not fit. Each item names the clause and, where we have
one, the failure it caused here.

### 4.1 Shape is not opacity — knockout groups (§11.4.6)

> In a knockout group, each individual element shall be composited with the group's initial
> backdrop rather than with the stack of preceding elements in the group.

The backdrop is transparent, so compositing an element with it yields the element; the group's
accumulated result is then replaced by *a fraction* of that, and the fraction is the element's
**shape**. For a rasteriser, shape is the coverage the element was drawn with.

**This is where Vello could not be patched.** Its layers composite over the layer's whole
*bounding box*, so `Compose::Copy` erased a row of pixels outside the shape entirely. A raster of
premultiplied samples carries opacity and not shape, and no arrangement of an SVG-shaped API
recovers the difference.

**What we need**: a compose mode that is Porter-Duff Source **modulated by coverage**, applied
per element, so that an element with 40% coverage replaces 40% of what was there and leaves the
rest. If your model carries shape as its own channel, this is free and several other things below
become free with it.

The scene we test it with has a **diagonal edge** on purpose: the two backends reach the clause
through different arithmetic and they are not the same arithmetic at fractional coverage, so a
scene of axis-aligned rectangles would agree while being wrong.

### 4.2 Soft masks, evaluated on the device (§11.5, §11.6.5.1)

A soft mask is not an alpha texture. It is a **transparency group, rendered at device resolution**,
reduced to mask values by one of two rules:

- **Alpha** (§11.5.2): the group's alpha, colours ignored.
- **Luminosity** (§11.5.3): the group composited onto *a fully opaque backdrop of a specified
  colour*, then the luminosity of the result. The backdrop colour is the mask's, defaulting to
  black — which is what makes the area outside a mask group's marks mask everything away.

Then optionally §11.6.5.1's `/TR`, a transfer function we hand you as a **256-entry lookup table**
(we sample it exactly; a mask value is one byte, so the table holds every value the function can
be asked about).

**What today's backend does and what we want instead.** `render-gpu` renders each mask group to
its own texture, **reads it back to the CPU**, converts it with `pdf_render::SoftMask::value`, and
uploads it again as an alpha layer. That round trip is per mask per frame. We want the reduction
to happen on the device: `MaskKind::Alpha`, `MaskKind::Luminosity { backdrop: Color }`, plus an
optional `transfer: [u8; 256]`, with the group built through the same `SceneBuilder`.

The catch that makes this a specification item rather than an optimisation: `SoftMask::value` is
shared by both our backends *on purpose*, so that what the pixels mean is decided once. If the
reduction moves onto the device, your shader becomes a second implementation of that function and
must agree with ours to the byte. We would want a conformance test for exactly that (§10).

### 4.3 Sixteen blend modes (§11.3.5)

All sixteen, including the four **non-separable** ones — Hue, Saturation, Color, Luminosity —
which are defined by the clause's `Lum`, `ClipColor`, `SetLum` and `SetSat` functions over all
three components at once. No per-component formula produces them, and a backend that gets one
subtly wrong still produces a plausible picture.

We do not take these from a library any more. `render-cpu` implements them itself rather than
using `tiny-skia`'s, because three of `tiny-skia`'s were wrong (ADR 0047) and because sharing an
implementation between our two backends would make the cross-backend comparison compare one
implementation with itself. **Yours must be your own, and we will test it against ours.**

The scene that found this: fourteen cross-backend fixtures existed and every command in all of
them carried `BlendMode::Normal`, so sixteen blend functions had never been compared at all. Three
of them disagreed by 113 of 255 (ADR 0046).

### 4.4 Groups composite onto transparency (§11.4.1, §11.4.5, §11.6.6)

A group's elements draw onto a fully transparent backdrop; the result is then painted once, under
the group's own constant alpha and blend mode. Compositing the elements onto the page one at a
time instead is visibly different wherever two of them overlap, and it is what §11.6.6's
initialisation of the alpha constants exists to prevent.

We decide isolation upstream and only emit a group where the computation is provably the isolated
one, so **you can assume every group is isolated** — but please say so in your documentation
rather than leaving it implicit.

### 4.5 Four decisions that are ours, not yours

These are settled in `pdf-render` precisely so that two backends cannot answer them differently
(trap 2: *a decision either backend can make alone is a decision neither has made*).

| decision | where | what you do |
|---|---|---|
| `Image::is_smoothed` | §8.9.5.3's `/Interpolate` | honour the flag; do not choose a filter yourself |
| `Image::area_averaged` | a documented departure from §10.7.4 | honour it |
| `Stroke::device_width` | §8.4.3.2 with §10.7.5 — a `0 w` line is **one device pixel** | take the width we give you |
| degenerate subpaths | §8.5.3.2 — a zero-length subpath is a dot under round caps and *nothing* under butt or square | we pre-split them; draw what you are given |

The last two are worth an extra sentence because they cost us. `tiny-skia` draws a zero-width
stroke as one device pixel, which happens to be exactly what §8.4.3.2 says — so the rule was never
written down and **every `0 w` line was invisible on the GPU for fifteen sessions**. And on
§8.5.3.2's zero-length stroke, three libraries gave three answers and none was the standard's.

**A fifth decision we want to *make* and need you to expose**: the sub-pixel quantum of any glyph
cache (§6.3). It must be a stated, settable parameter. If you quantise glyph positions silently,
you have changed where the text sits and our oracle will contradict pages without anyone knowing
why — we measured that it contradicts at 1/8 of a pixel and is clean at 1/16 (ADR 0131).

### 4.6 Determinism

- **Same scene, same viewport, same adapter → the same bytes.** Not "visually identical". Our
  gates compare rasters, and the CPU backend's own parallel path is held to byte equality at every
  strip count for this reason (ADR 0139).
- **Across adapters**, we already depend on more than that: RADV and lavapipe produce
  byte-identical Vello output, which is why our goldens need not be per-adapter and why CI can use
  a software rasteriser. We would like you to state whether you offer this, and to have a test
  that pins it — not because we assume it, but because if it fails we need to learn it from your
  suite rather than from a golden.
- **No dependence on the order in which independent commands are executed.** You may reorder and
  parallelise; the result may not change when you do.

### 4.7 Geometry that a page really contains

Not exotic, but each one has broken something here:

- **Both fill rules**, non-zero and even-odd, including a nested subpath wound the same way.
- **An empty clip admits nothing** — which is different from an absent clip.
- **A clip chain is an intersection**, and chains are deep: our worst page holds 3 608 of them.
- **Dash patterns including zero-length dashes**, whose caps face along the path. Skia's dasher
  loses the direction and paints them upright; the clause says the direction is the path's, and on
  a diagonal dotted line the two answers cover different pixels. We do our own dashing for that
  reason and hand you the result, so what you need is not to undo it.
- **Very large coordinates and degenerate transforms** arrive from real files. Refuse them
  loudly; do not produce NaN geometry.

---

## 5. The failure contract

**This is the requirement we care about most after §4.1, and it is the one Vello fails.**

> A backend may refuse a scene. It may not silently draw nothing.

Vello sizes its GPU working buffers from a table of constants whose own comment says they were
"hand picked to accommodate the vello test scenes". A scene needing more overflows them *on the
device*, which sets a flag, stops filling, and returns `Ok(())` over a blank target. Page 6 of ISO
32000-2 at 1132×1600 — an A4 page fitted to a laptop window — is such a scene: it needed 4% more
tile records than the buffer held. **A person reported a black page; nothing in our test suite
could see it** (ADR 0127). We now band the target and halve it until it fits, which is a rescue
and should not have to exist.

What we want instead, in order of preference:

1. **Memory that grows.** Two passes — count tiles and segments, then allocate exactly — or a
   growable arena with a fence that reruns on overflow. Then a page is drawn or the *allocation*
   fails, and there is no third state.
2. **If a limit must exist, it is discoverable before the frame**, through `Device::limits` and a
   `Scene::cost()` a caller can compare against it.
3. **A failure is an `Err` that names what overflowed**, so a caller can act on it. Ours would
   fall back to the CPU backend, which is what `viewer-ui` already does.

And the corresponding rule for content: **anything you cannot draw as asked is a `Report`, not a
silent approximation.** A gradient drawn opaque, a blend mode substituted, a mask ignored — each
of those is a plausible-looking wrong page, which is the worst outcome this project has a name
for. We would rather have a hole and a sentence.

A related failure we hit through a dependency and would like designed out: turning on a feature
flag for one effect brought another — Vello's `debug_layers` also makes it hand wgpu a zero-length
buffer slice whenever a scene produces no lines, and wgpu panics on it, which under
`panic = "abort"` kills the viewer. **A blank scene is a legitimate scene.**

---

## 6. The performance contract

### 6.1 The measurement nobody had taken, and it reverses the obvious plan

`doc/gpu.txt` closed by asking how much of a frame is CPU scene encoding versus device execution,
and noted that a retained scene was "probably the largest interactive win". We measured it —
`crates/render-gpu/examples/frame_split.rs`, fastest of ten frames after a warm-up, AMD 890M under
RADV, through the *offscreen* path that reads the pixels back:

| page | target | scene encoding | whole frame | the same target, one rectangle |
|---|---|---|---|---|
| ISO 32000-2 p. 6 | 596×842 | 1.42 ms | 6.34 ms | **3.48 ms** |
| ISO 32000-2 p. 6 | 1191×1684 | 1.11 ms | 12.07 ms | **8.77 ms** |
| ISO 32000-2 p. 6 | 2382×3368 | 1.11 ms | 29.13 ms | **26.73 ms** |
| ISO 32000-2 p. 101 | 1191×1684 | 1.61 ms | 13.86 ms | **7.58 ms** |
| `tracemonkey.pdf` | 1224×1584 | 1.17 ms | 13.78 ms | **8.51 ms** |

The last column is the same viewport rendered from a display list of **one small rectangle**:
texture allocation, the pipeline over the whole target, one submit, the readback and the
demultiply. Three things follow, and they are the design guidance in this document.

**Scene encoding is 1.1 to 1.6 ms and does not grow with resolution**, because it is a function of
the command list and not of the pixels — flat across a sixteenfold range of them. So a retained
scene is worth 4% of a frame at 4×, 9–12% at a window's scale and 22% at a thumbnail's. That is
real and worth having, and it is **not** the headline that `gpu.txt` expected. It stays
requirement §2.3 for two other reasons: it is what makes smooth zoom possible at all, and 1.5 ms
of CPU per frame is 1.5 ms the interpreter is not getting.

**Between 55% and 92% of a frame is paid before any of this page is drawn** — 3.48 of 6.34 ms at
1×, 8.77 of 12.07 at 2×, 26.73 of 29.13 at 4×. That share is what a document renderer should be
attacking, and most of it scales with *bytes*: 26.7 ms for a 32 MB target is about 1.2 GB/s, which
is what a mapped readback and a demultiply cost. **A window presenting to a swapchain does not pay
the readback at all**, which is why §2.4 asks for three target kinds and not one.

**The page's own content costs 2.4 to 6.3 ms and barely grows with resolution** — 2.86, 3.31 and
2.40 ms for page 6 at 1×, 2× and 4×, which is flat within the noise of a subtraction between two
measured frames. Five thousand nine hundred and thirty-three glyph fills cost about what one
rectangle costs at the same size. Vello's per-pixel work is not what we are paying for.

**Two honest limits on this table.** It is a wall clock, and this project's own habit is that wall
clocks lie under load — the mean of ten frames put the 2× figure at 15 ms where the fastest of ten
puts it at 12. And it cannot separate the readback from the execution, which is §11's first
question and the reason §8 asks for timestamped phases: we had to infer from a bytes-per-second
estimate what a timestamp query would have told us exactly.

**So the ranking we would give you**, from these numbers rather than from intuition: the surface
and texture target paths (§2.4) first, because they delete the largest single item; then whatever
makes the per-pixel floor cheaper for a target that is mostly untouched; then the atlas and the
rectangle path (§6.3, §6.4); then the retained scene; then damage.

### 6.2 The baseline you have to beat, and it is not Vello

Our CPU backend draws a page on every core, byte-identically to the serial render, since ADR 0139.
Fastest of five, at a laptop window's scale:

| page, at 1191×1684 | CPU backend | GPU backend today, offscreen with readback |
|---|---|---|
| ISO 32000-2 p. 6 | **5.9 ms** | 12.1 ms |
| ISO 32000-2 p. 101 | **10.1 ms** | 13.9 ms |

The comparison is not quite fair — the CPU figure has no readback because the pixels are already
in main memory — and that is the point. **A GPU backend for this viewer has to beat a
multi-threaded `tiny-skia` including the cost of getting the pixels back to whoever wants them.**
For a tier-2 host presenting to a surface it does not pay that cost; for our oracle it does.

We would consider the library a success at **a third of the CPU backend's time on a dense text
page at window resolution, presenting to a surface**, and a clear win at a tenth.

### 6.3 The glyph atlas, with the number that decides its design

Page 6 of ISO 32000-2 is **5 933 fills of 107 distinct outlines**, and today every one of the
5 933 is flattened again on every frame. We priced a coverage cache for the *CPU* backend and
refused it, and the measurement transfers (ADR 0131):

| keying | reuse on p. 6 | reuse on `tracemonkey.pdf` | oracle |
|---|---|---|---|
| exact position | 116 hits of 5 933 | **not once** | clean |
| quantised to 1/8 pixel | — | — | **contradicts pages** |
| quantised to 1/16 pixel | **5.0×** | 1.3× | clean |

A glyph's sub-pixel phase is an arbitrary float, so an exactly-correct cache never hits. At 1/16
of a pixel it hits five times over on a dense page and our oracle's verdicts do not move. We
refused it on the CPU because `tiny-skia` provides no blitter for a cached coverage bitmap; **on a
GPU the blitter is a textured quad, which is the natural primitive**, so the same number becomes a
reason to build rather than a reason to stop.

Requirements that follow:

- Key on `(outline, scale bucket, sub-pixel phase)` with the **quantum settable and documented**
  (§4.5). Default it to 1/16 if you like, but let us set it and let us turn it off.
- An **R8 coverage atlas** with eviction, sized from a budget we set.
- **Tell us the hit rate.** A cache that reports a perfect hit rate can still be missing: our clip
  mask cache answered all 303 lookups page 6 made and built 303 identical page-wide masks, because
  the key was a *name* rather than the region (ADR 0132). **Instrument the count of distinct keys,
  not the hit rate** — a hit rate is a statement about the lookups you made, never about the ones
  you should have made.

One design note we would find persuasive rather than prescriptive: if the atlas is rasterised on
the CPU with `tiny-skia`, the glyphs come from *the same code that is our correctness oracle*,
which is a correctness argument no other arrangement gets for free.

### 6.4 A rectangle is not a path

Rules, backgrounds, underlines, table cells, and — importantly — **most clips**. Exact analytic
coverage in a fragment shader, no tiling, no binning, no edge list.

The clip case is the one with a measurement behind it: `DisplayList::add_clip` collapses
identical regions, so page 6's 303 clip states become one, and that deduplication is what makes
caching clip masks viable at all. A clip that is an axis-aligned rectangle should not become an
R8 mask texture; it should become four floats and a comparison.

We also know from ADR 0139 that an axis-aligned edge is exactly reproducible under a horizontal
cut where an oblique edge and a curve are not — measured, in `render-cpu/tests/strip_cut_exactness.rs`
— which is a hint that the rectangle path can be made both faster *and* more exactly composable
than the general one.

### 6.5 Damage

Persistent geometry plus per-tile dirty tracking. A caret blink, a selection change or a hover
highlight should redraw a few tiles, not a page. Our interactive chrome — selection quads, resize
handles, a caret — deliberately crosses to the host as *geometry* rather than pixels so that a
native host can draw it in the platform's own accent colour, so what damage buys us is the page
underneath not being redrawn when only the chrome moved.

`Viewport::damage` in §2.4 is how we would express it. If a scene is retained and a viewport is
cheap, this is nearly free; if either is not, it is impossible.

---

## 7. Startup

`CLAUDE.md` makes launch latency a first-class requirement, and it is the reason this project has
two backends at all: **page one renders on the CPU while the GPU initialises on another thread**,
and the GPU takes over once ready.

What we ask of you:

- **Few pipelines, compiled lazily.** Vello compiles about twenty compute shaders up front. A
  hybrid design plausibly needs five or six — glyph quads, rectangle fills, general path coverage,
  composite, mask build, image blit — and only the first three are needed for a page of text.
  Shadings, meshes and the exotic blend modes can compile on first use.
- **`Device::headless` must be able to return before every pipeline exists**, with the rest
  compiled in the background and a way to ask whether the device is fully warm.
- **A pipeline cache we can persist.** If the driver will hand us a binary blob, let us save it
  and hand it back next launch, and tell us when it was rejected.
- **Tell us what startup cost.** `Timings` (§8) for device creation, split into adapter
  enumeration, device creation and pipeline compilation. We will put a number on it in CI and gate
  regressions, because that is what we do with the numbers we have.

---

## 8. Observability

We gate performance in CI and we attribute regressions by measurement, so a library that is a
black box costs us the ability to do our job. Concretely:

```rust
pub struct Timings {
    pub encode: Duration,        // CPU: turning a Scene into device commands
    pub upload: Duration,        // CPU→GPU transfers this frame
    pub execute: Duration,       // device time, from timestamp queries where available
    pub readback: Duration,      // GPU→CPU, zero for Surface and Texture targets
    pub phases: &[(&'static str, Duration)],   // per-pass, when timestamps are available
}

pub struct Counters {
    pub commands: u32,
    pub distinct_outlines: u32,
    pub atlas_entries: u32,
    pub atlas_distinct_keys: u32,   // §6.3: not the hit rate
    pub tiles: u32,
    pub segments: u32,
    pub bytes_uploaded: u64,
}
```

§6.1 exists because we could not separate the readback from the execution, and that is the shape
of the problem: we had to infer from a bytes-per-second estimate what a timestamp query would have
told us exactly.

Two more, both learned here:

- **A cost written down beside one call is not a cost anybody adds up.** If an operation is
  expensive, make it expensive *in the API* — a distinct type, a `must_use`, a name with `slow` in
  it — not in a doc comment.
- **A failed frame must not be reported as a drawn one.** Our window once answered "presented"
  when its GPU path had refused the page, so the core recorded the page as shown, never asked
  again, and kept the *previous* page under a title bar naming the new one (ADR 0125). Whatever
  your `Frame` says about itself must be true.

---

## 9. Non-goals

Things we will never ask for, so you can leave them out and be faster:

- **Colour management** (§3). We hand you device RGB.
- **Font loading, shaping, hinting or the Adobe Glyph List.** We hand you outlines. PDF content
  streams carry already-positioned glyphs; shaping them again would move them away from where the
  document specifies.
- **Text layout, line breaking, bidi.**
- **Filters, effects, image adjustments** beyond what §4 names.
- **SVG, CSS, or any document format at all.** You will never see a PDF byte — which is also your
  trust boundary: everything you receive is data this tree produced from untrusted input, never
  the untrusted input itself. That is why our `#![forbid(unsafe_code)]` rule, which binds every
  crate that touches PDF bytes, does not bind you.
- **A scene graph, retained widget tree, animation or timeline.** Our `Scene` is built by an
  interpreter and thrown away when the page changes.
- **Hit testing.** We do it against our own display list, in page space, because selection cannot
  wait for a render round trip.

---

## 10. How we would judge it

Not as a promise, but so you know what it will be held to and can build the harness yourself:

1. **The cross-backend scene suite.** Our fixtures render through both backends and compare. They
   exist because of specific failures, and three of them will find your bugs on day one: the
   knockout group with its diagonal edge, the sixteen blend modes, and a full-page scene at a real
   window's resolution. That last one is there because **a suite of small scenes tests small
   scenes** — the first real page at a real size came back blank and nothing in the tree could see
   it (trap 12b).
2. **The oracle.** 1 794 pages of the pdf.js corpus rendered by us, poppler, mupdf and
   ghostscript, with a verdict per page from a bound derived from how far the references sit from
   *each other*. It is the instrument that has refused two of our own optimisations after they
   passed every unit test (ADRs 0131, 0138). It will not be kind.
3. **Byte equality where we claim it.** Same scene, same viewport, same adapter, any number of
   internal threads or reorderings.
4. **The window.** Key press to command to frame to window, on `Xvfb` with a software rasteriser,
   because no gate we have turns a page and every defect of three consecutive sessions lived
   there.

Our own numbers, today, for calibration: 980 tests; 74 of 974 corpus documents report something
they could not draw; 846 oracle pages agree and 72 are contradicted with every group argued in
`tests/oracle.rs`; 98.2% of `pdftotext`'s words read back.

---

## 11. Questions we would like you to settle by measurement

Not rhetorical — we do not know the answers, and the design should turn on them rather than on
anyone's taste.

1. **How much of §6.1's "fixed cost" is the readback?** We could not separate it. If it is nearly
   all of it, then tier-2 and tier-3 targets are the whole performance story and the atlas is a
   second-order effect. This is the first thing to measure and it needs a timestamp query, not a
   wall clock.
2. **Does a document renderer want tiles at all for the glyph path?** If glyphs are quads against
   an atlas and rectangles are analytic, the general tile-binned path may be reached by a small
   minority of commands. Measure that minority on our corpus before designing for it.
3. **What does the atlas cost on a page it cannot help?** `tracemonkey.pdf` reuses 1.3×. A cache
   that is 5× on one page and a net loss on another is a decision, not a feature.
4. **Is byte-identical output across adapters achievable for your design?** We rely on it today
   between RADV and lavapipe. If your answer is no, say so early — it changes how our CI works.
5. **What does a `Scene` cost to hold?** Our worst page is 3 608 clip chains and 7 050 commands,
   and a moving window of interpreted pages is on our roadmap. A whole 1 023-page document cannot
   be resident — 70 MB of draw records would be affordable and the 4.0 s to interpret them is not
   — but a dozen pages should be.

---

## Appendix: where each measurement lives

| claim | source |
|---|---|
| 5 933 fills, 107 outlines; 1/16-pixel reuse 5.0×; oracle contradicts at 1/8 | ADR 0131, `examples/glyph_reuse` |
| 303 clip identifiers for one region; 4.7× on a dense page | ADR 0132 |
| Vello's buffers overflow silently; page 6 at 1132×1600 | ADR 0127, `render-gpu/tests/real_pages.rs` |
| `Compose::Copy` erased a row outside the shape | ADR 0128, trap 2 |
| three of `tiny-skia`'s blend modes wrong; sixteen-mode scene | ADRs 0046, 0047 |
| a failed frame reported as a drawn one | ADR 0125 |
| CPU backend parallel, byte-identical; 5.9 / 10.1 ms | ADR 0139, `examples/strip_spans` |
| axis-aligned edges survive a cut, oblique ones and curves do not | ADR 0139, `render-cpu/tests/strip_cut_exactness.rs` |
| the frame split of §6.1 | `render-gpu/examples/frame_split.rs`, this document |

`doc/HANDOVER.md` is the state of play and carries the traps and habits these requirements came
from; `CLAUDE.md` is the project's principles. Both are worth an hour before you start.
