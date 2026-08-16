# 0056 — The surface leaves the device, and a presenter holds it

Date: 2026-08-16. Status: **accepted, and built**.

Asked for in `pdf-viewer/doc/QUORRA_NONBLOCKING_RENDER.md`. The reply to that document —
including the soundness answer their §7(b) cannot settle from outside — is
`doc/answer-nonblocking-render.md`. The code is `src/present.rs` with `src/present/`,
`src/device/present.rs` and `src/shaders/present.wgsl`; the proof is
`crates/quorra-gpu/examples/present_thread/`, which CI runs under `Xvfb`.

## Context

Their number decides the shape, so it goes first. On the owner's machine, over 24 frames
of a 58 009-command page: the frame is **4 454.9 ms** and `execute` — our own timestamps,
the device's clock — is **6.7 ms of it, 0.15 %**. The graphics device is idle for 99.85 %
of such a frame. What makes a frame long is a processor running on the calling thread, and
the calling thread is the one that owns the event loop.

So the presenter they built — a clock at the surface's refresh rate, reprojecting the last
finished rendering under a composed affine — never gets to run: `Device::render` holds the
only `&mut Device`, and the reprojection that exists precisely for those milliseconds
needs the same one. Their measured result is a median interval between presents of
**167.4 ms** against a refresh of 8.333, with **1 present in 23 landing on the next
refresh**.

Their §3 also records what they asked for ten days earlier and why it would not have
helped: `Device::present_texture(&Texture, Affine, into: Target)` takes `&mut self` like
everything else on `Device`, so it is unreachable for exactly the interval in which it is
needed. **The problem is ownership, not an operation.**

## Decision

**The surface leaves the device.** `Device::detach_presenter() -> Option<Presenter>` hands
out the surface, its swapchain and the one pipeline that puts a texture on it;
`Device::attach_presenter(Presenter) -> Result<(), ForeignPresenter>` takes it back. While
it is out, that device refuses `Target::Surface` and `invalidate_surface` with a new
`RenderError::PresenterDetached` — **by name**, because a device that drew nowhere and
said nothing is principle 6's third state, and because the fix differs from
`NoSurface`'s.

`Presenter::present(&[Layer])` clears the window and draws each layer over it, in order,
under its own placement and filter. Nothing else about the device changes: `Readback`,
`Texture`, uploads and the atlas are untouched, and rendering into a host-owned texture
that the presenter then puts on the window is the whole arrangement.

### The three states, in one field

`Device::surface` is a `SurfaceSlot` — `Headless`, `Held(SurfaceState)`, `Detached` —
rather than an `Option` beside a flag. Two fields that must agree are a defect waiting for
a fourth combination; one field with three arms is a `match` that the compiler completes.

### The layer type: a named struct, not their three-tuple

Their sketch is `&[(&wgpu::Texture, Affine, ImageFilter)]`. We ship

```rust
pub struct Layer<'a> {
    pub texture: &'a wgpu::Texture,
    pub placement: Affine,
    pub filter: ImageFilter,
}
```

for one reason with a measurement of its own behind it: **`placement` is the field a
reader has to get right, and a tuple does not say which direction it goes.** The whole
point of the ask is a reprojection affine, and `.1` is not a name anybody can check a
composition against. `Affine` and `ImageFilter` are `quorra-scene`'s own, exactly as they
asked — no new vocabulary — and the slice stays a slice for their reason: a window is a
page *and* chrome, and the slice keeps the chrome exactly as stale as the page and no
worse. `PLAN.md`'s integration notes 7 and 8 are the precedent for deviating from an
illustrative signature and saying so.

`placement` maps the **layer's texel space to the surface's pixels**. The inverse is what
the shader uses; the forward direction is what a host composes, which is why it is the one
in the API.

### `attach_presenter` returns a `Result`, and the refusal carries the presenter back

Their sketch returns nothing. It has to be able to refuse: a surface's format was
negotiated against the adapter *its* device chose, so attaching it to another device
would put a swapchain nobody negotiated under a window. The check is the `Device::id` that
already exists to stop a retained encode being replayed through another device (ADR 0048),
and for the same reason — two live devices never share a number.

**The error carries the presenter** (`ForeignPresenter::into_presenter`). Consuming it on
the failing path would destroy the window's surface over a caller's mix-up, and no error
path in this crate should cost a host its window. The presenter is boxed inside the error
so that every `Ok` is not as wide as a presenter.

### `PresentCost` reports counts, and says "wall" in the name of every duration

```rust
pub struct PresentCost {
    pub layers: usize,
    pub reconfigured: bool,
    pub compiled: Option<Duration>,
    pub acquire_wall: Duration,
    pub record_wall: Duration,
    pub present_wall: Duration,
}
```

Two exact facts and three wall clocks that say so in their own names rather than in a doc
comment nobody adds up — §8 of the brief, ADR 0031, ADR 0052, and the rule that a claim
about "how many" is a count and exact while a claim about "how fast" on this machine is
not. **There is deliberately no timestamp-query number here**: a query must be resolved
and mapped to be read, which is a stall on the very thread whose freedom from stalls is
the point of the split.

`Presenter::last()` returns `Option<PresentCost>` rather than a zeroed one. A cost of
nothing and a present that never happened are different things, and a type that cannot
tell them apart says something untrue about itself. A refused present leaves it alone.

### `Presenter::resize` is not a frame

It records a size and configures nothing; the swapchain follows the window at the next
present, which is also the call that can refuse. A presenter that has never been told a
size and whose device never presented a frame refuses with `PresenterUnsized` instead of
inventing one — a size guessed here configures a swapchain for a window nobody described.
A zero-size window (a minimised one) is a state rather than an error, and the present that
meets it is refused with the existing `ZeroSizeTarget`.

### A new pipeline and a new shader, because the blit is not one

`compose/blit.rs` and `blit.wgsl` are an unfiltered `textureLoad` with an origin and an
extent and **no linear part**; sampling under an arbitrary affine with a filter is a
different pass, so `Kind::Present` and `present.wgsl` are new rather than widened.
`Device::linear_sampler` — the filtering sampler that already existed and that the blit
does not use — is shared with the presenter rather than made again.

The sampling convention is **the target pixel's centre mapped back through the inverse**,
which is `function_lane.wgsl`'s reading of ISO 32000-2 §10.7.4 and is cited as such in the
shader. Outside the layer's rectangle the layer contributes transparency rather than a
clamped edge texel, which is `blit.wgsl`'s convention for a root smaller than its target
(ADR 0039). The pass blends premultiplied `OVER` — ADR 0010's factors — because a slice's
second layer must land *over* the first; **no clause is cited for that blend and the
shader says why**: every layer is a raster some earlier frame already finished, so nothing
here is a statement about a document's transparency model.

### The presenting pass joins the warm set

Their §5.2 argues it should, and it does (ADR 0043's second set, extended). `Kind::Present`
is compiled for the surface's negotiated format **whenever a device is built for a
surface**, including when that format is `WARM_FORMAT` — it is the one kind with no first
set to be in, because a presenter draws onto the surface and onto nothing else. A headless
device compiles none of it.

**`detach_presenter` therefore compiles nothing, waits for nothing and blocks on nothing**:
it clones three `wgpu` handles and a reference count, moves the surface state, and asks
the pipeline store nothing at all. If a host detaches before the warm-up finishes, the first present compiles
the pass inline and says so in `PresentCost::compiled` — ADR 0043's rule, unchanged.

### What is refused, and where the questions are asked

Every layer is checked **before the swapchain is acquired**, in `Device::render`'s own
refusal order and for its own reason: a texture acquired and then dropped unpresented
leaves an acquire semaphore no submission will ever wait on, and enough of those time out
every later acquire, permanently — the caller measured that. `LayerProblem` names five
things:

| variant | what it catches |
|---|---|
| `Format` | not `Rgba8Unorm`, and says what it is |
| `NotSampleable` | **the trap**: `Target::Texture` needs `RENDER_ATTACHMENT`, sampling needs `TEXTURE_BINDING`, and a texture that has been a render target every frame can still be unpresentable |
| `Shape` | multisampled, arrayed, or not 2D |
| `Placement` | a non-finite or degenerate affine — refused, never substituted by an identity (§4.7) |
| `Unbindable` | what `wgpu` said when the texture was offered to this device |

There is **no** variant for an empty texture: `wgpu` will not create one (WebGPU requires
every extent to be at least 1), and a refusal no input can reach makes "how often does this
happen?" a question without an answer.

## The provenance question, answered as far as it can be

Their constraint is that a layer must be proven to come from this device. **`wgpu` 30's
`Texture` does not say which device made it** — it exposes size, format, usage, shape, and
nothing else — so the question is put to `wgpu`, inside a validation error scope, when the
bind group is built (ADR 0042's mechanism, whose alternative is a panic on whatever thread
the host put the presenter on). A unit test builds two devices on one instance and shows
the refusal arriving as `LayerProblem::Unbindable` carrying wgpu's own `DeviceMismatch`
text, with an accepting control beside it.

**The hole, stated rather than left to be found**: a texture from a device of a *different*
`wgpu::Instance` is not a foreign resource to wgpu-core but a non-existent one — resource
ids are per-instance — and the lookup panics before any error scope is consulted. This is
`wgpu`'s behaviour and it is not new: `Target::Texture` has had exactly the same hole since
M1. The answer is the one the hoisting constructors already offer — a host with more than
one device builds them all from one instance — and it is in the reply to the caller.

## Soundness: `Presenter: Send`, and why we accept their reading

Their §6 offers the argument and says it is ours to accept or refuse. **We accept it, and
we checked it against `wgpu` 30's own source rather than against the sentence.**

- `wgpu` asserts `Send + Sync` for every type a presenter holds —
  `Surface<'_>`, `Device`, `Queue`, `Sampler`, `ShaderModule`, `RenderPipeline`,
  `BindGroup`, `Texture` — with `static_assertions::assert_impl_all!` under its own
  `send_sync` cfg, which its `build.rs` defines as `any(native, …)`, i.e. everything that
  is not wasm32. Those assertions are in `wgpu-30.0.0/src/api/*.rs` and they are compiled,
  not commented.
- `Sync` in safe Rust **is** the statement that concurrent `&self` use is sound. There is
  nothing further to prove about memory safety, and nothing here can weaken it: the crate
  is `#![forbid(unsafe_code)]`, so `Presenter: Send` holds by construction and by an
  auto-trait derivation, not by an `unsafe impl`. `tests/presenter.rs` asserts it the way
  `tests/retained_handle.rs` asserts `RetainedScene: Send`.
- The pipeline store behind the `Arc` was already `Send + Sync` — it is shared with the
  warm-up thread — so the presenter adds no new sharing there. Compilation happens under
  the store's one lock, so a first present on the presenter's thread and a first frame on
  the render thread cannot compile the same pipeline twice.

Two caveats that are `wgpu`'s and are stated rather than discovered: **surface creation**
panics off the main thread on macOS/Metal (`wgpu`'s own `SurfaceTarget` documentation),
which is one more reason the presenter is *detached from* a device built where the window
lives rather than constructed on the thread that will use it; and presenting from two
threads at once is two swapchains' worth of questions asked of one swapchain, which is why
`present` takes `&mut self` and there is exactly one presenter per device.

## Determinism: nothing here can move a pixel of a page

Nothing in `present.rs` encodes a scene, allocates from scene-derived arithmetic or
touches the atlas. The corpus gate and the oracle use `Target::Readback` and have no
surface; `Presenter` is reachable only from a device built with one, and
`tests/presenter.rs` holds the negative — a headless device has no presenter to detach,
still refuses `Target::Surface` as `NoSurface`, and renders a readback frame exactly as
before. No golden file in this tree can reach a present.

## The proof

`examples/present_thread/` under `Xvfb` and lavapipe, in CI beside `window_smoke`. It
opens a window, builds a device for it, **waits until warm** (which is what makes the
compile assertion mean something), renders the chrome into a host texture, detaches the
presenter, and then:

- renders the page into a second host texture **on another thread** while the main thread
  presents — on this machine, **4 presents got through one 30.6 ms render**, where today's
  arrangement produces none;
- has that thread assert `Target::Surface` is refused with `PresenterDetached` while it
  holds the device;
- presents the finished page under `scale(2) ∘ translate(64, 32)` with the chrome over it
  at the identity, and reads the window back with `xwd -name`, checking six points: the
  strip the page does not reach (black), the page's field, the page's mark, the two pixels
  either side of the mark's left edge, and the chrome's corner;
- presents the same page under `ImageFilter::Linear` — the sampler branch, whose
  normalised coordinates nothing else exercises — and checks it lands in the same place
  and alone, since a slice is what is on the window rather than what was;
- refuses a present before any size is known (`PresenterUnsized`) and at a window with no
  pixels (`ZeroSizeTarget`), which is a minimised window and a state rather than an error;
- offers the presenter to a device that did not hand it out, and recovers it from the
  refusal;
- attaches it back and draws through `Target::Surface` again, checking the window shows a
  picture the presenter could not have produced;
- asserts `PresentCost`: two layers, no reconfigure, and **no compile**.

**The gate was verified able to fail, in three of the ways it exists to catch**: with the
offset at zero the uncovered strip shows the page's field; with the scale at 1 the mark's
left edge is field where it should be mark; and with the sampler's normalised coordinate
divided by twice the extent, the linear present reads the field where the mark is.

Beside it, headless and adapter-independent: the uniform-layout gate for
`present.wgsl`'s `Params` (verified able to fail by exchanging two fields of one width),
the layer contract's five refusals with an accepting control, the two-device bind refusal,
and the warm-set assertions — `Kind::Present` present for the surface's format whatever it
is, absent for a headless device, and compiled inline with a duration by the first ask
when the warm set missed it.

## The costs, written down

- **A second surface API.** `Target::Surface` and `Presenter::present` are two ways to put
  pixels on one window, and a host can now be in a state where the obvious one refuses.
  That is the price of the split and it is bounded by the refusal being *by name*: the
  error says what to do about it.
- **A full-screen triangle per layer.** The present pass draws the whole window for each
  layer rather than the layer's own quad, and discards outside it in the fragment stage. A
  layer under an arbitrary affine has no axis-aligned quad to draw, and the clear
  construction wins here because nothing has measured the alternative — a transformed
  bounding quad is available later if a measurement ever asks for it.
- **One more pipeline in the warm set** for every device built with a surface, on the
  thread nobody blocks on, whether or not the host ever detaches. `is_warm` arrives
  correspondingly later, which ADR 0040 and ADR 0043 already accepted for the same reason.
- **A presenter can outlive its device**, because it holds its own clones. That is
  deliberate — dropping the device first is legitimate and costs only the ability to
  attach back — and it means a host that loses its device still owns its surface.
- **We cannot answer whether it holds the rate.** Their §7(c) says so first: `Xvfb` reports
  a refresh rate of 0.00 and 120 Hz cannot be observed on this machine at all. The
  arithmetic is theirs to close on the display that states its own refresh.
