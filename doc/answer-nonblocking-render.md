# Draft answer for the caller's `QUORRA_NONBLOCKING_RENDER.md`

**This is a draft for the owner, not a document either project publishes.** It is written
so that it can be carried across to
`/home/cl/projects/pdf-viewer/doc/QUORRA_NONBLOCKING_RENDER.md` — a tree this side never
edits.

It answers §1 to §8. **It deliberately does not answer §9** (`recording`, and what a frame
is made of at 58 009 commands): that is a measurement being taken separately, and the two
halves are assembled by the owner.

The decision and its costs are `doc/adr/0056`. Everything below is built, gated, and ran
before this sentence was written.

---

## The answer is yes, and it is the split you asked for

`Device::detach_presenter` / `Device::attach_presenter`, a `Send` `Presenter` holding the
surface, its swapchain and one pipeline, and `Presenter::present(&[Layer])` that puts
finished rasters on the window under their own affines. Your §4's shape survives almost
unchanged; §5's four reasons are all accepted, including the one that decided the most —
the pipeline belongs in our warm set, so a host that detaches pays no compile and puts no
shader on its own launch path.

The one number that shaped every choice below is yours: **`execute` is 6.7 ms of a
4 454.9 ms run.** A design that made presenting cheaper would have been answering the
wrong question; what the split does is make presenting *reachable* while the processor is
busy, and nothing in the present path does a readback, an upload, or an encode.

## The API as built, and the four places it differs from your sketch

```rust
impl Device {
    #[must_use]
    pub fn detach_presenter(&mut self) -> Option<Presenter>;
    pub fn attach_presenter(&mut self, presenter: Presenter) -> Result<(), ForeignPresenter>;
}

pub struct Presenter { /* Send */ }

impl Presenter {
    pub fn present(&mut self, layers: &[Layer<'_>]) -> Result<(), RenderError>;
    pub fn resize(&mut self, width: u32, height: u32);
    pub fn size(&self) -> Option<(u32, u32)>;
    pub fn last(&self) -> Option<PresentCost>;
}

pub struct Layer<'a> {
    pub texture: &'a wgpu::Texture,
    /// The layer's texel space → the surface's pixels.
    pub placement: Affine,
    pub filter: ImageFilter,
}
```

**1. `Layer` is a named struct where you wrote `(&Texture, Affine, ImageFilter)`.** The
slice stays a slice, for your reason, unchanged. What a tuple costs is the field that
matters most: `placement` has a direction, the whole ask is about composing one, and `.1`
is not a name anyone can check a composition against. `Affine` and `ImageFilter` are
`quorra-scene`'s own — no new vocabulary, as your §5.4 asks.

The direction, stated once: **`placement` maps the layer's own texel space to the
surface's pixels.** Identity puts texel (0, 0) at the window's corner, one texel per pixel.
Your `settled.transform⁻¹ ∘ asked.transform` composes into exactly this, and when a render
lands it becomes the identity again.

**2. `attach_presenter` returns a `Result`, and the refusal hands the presenter back.** It
has to be able to refuse — a surface's format was negotiated against the adapter *its*
device chose — and it is checked against the same device number that already stops a
retained encode replaying through another device. The presenter comes back inside
`ForeignPresenter::into_presenter()`, because losing a window's surface to a caller's
mix-up is not a cost any error path here should impose.

**3. `last()` returns `Option<PresentCost>`.** Before the first present there is no cost to
report, and a zeroed struct would say "a present that cost nothing" — which is the shape of
untruth `Frame` is not allowed either. A *refused* present leaves `last()` alone.

**4. `PresentCost` is two counts and three wall clocks, and the durations say so in their
names:**

```rust
pub struct PresentCost {
    pub layers: usize,               // exact
    pub reconfigured: bool,          // exact: the swapchain was replaced this present
    pub compiled: Option<Duration>,  // Some only if this present compiled the pass
    pub acquire_wall: Duration,
    pub record_wall: Duration,
    pub present_wall: Duration,
}
```

No timestamp-query number, on purpose: a query must be resolved and mapped to be read,
which is a stall on the very thread whose freedom from stalls is the point. `reconfigured`
is the field to watch when one present is unlike its neighbours — a resize, a suboptimal
or outdated surface, or a recovery from a timeout all land there, and it is exact where a
duration on anybody's machine is not.

One addition you did not ask for and can ignore: `Presenter::size()`, so a host can read
back what it last said.

## §6, item by item — what it must not cost

**Determinism.** Nothing in the present path encodes a scene, allocates from scene-derived
arithmetic, or touches the atlas; a `Presenter` is reachable only from a device built with
a surface, and your corpus gate and oracle have none. `tests/presenter.rs` holds the
negative: a headless device has no presenter to detach, still refuses `Target::Surface` as
`NoSurface`, and renders a readback frame exactly as before. No golden in this tree can
reach a present.

**The launch path.** `detach_presenter` clones three `wgpu` handles and a reference count
and moves the surface state. It asks the pipeline store nothing, so it cannot compile, cannot wait for warmth and
cannot block. The pass it will use — `Kind::Present` — is in the warm set of every device
built for a surface (your §5.2's argument, accepted), compiled on the thread nobody waits
on. If you detach before that thread finishes, the **first present** compiles it inline and
tells you so in `PresentCost::compiled`, which is ADR 0043's rule unchanged. The Xvfb proof
asserts `compiled == None` on a device that was waited warm, which is the end-to-end half
of the claim.

**`Target::Surface` keeps working for every host that does not detach**, and a device whose
presenter is out refuses it **by name**: a new `RenderError::PresenterDetached`, distinct
from `NoSurface` because the two have different fixes. `Device::invalidate_surface` refuses
the same way. Attaching the presenter back makes both work again from the next frame — the
Xvfb proof does exactly that and then checks the window.

**Soundness.** Below, because it deserves its own section.

## §7(b) — the soundness answer, which is the part only we could give

**We accept your reading, and we checked it against `wgpu` 30's source rather than against
the sentence.**

- Every type a `Presenter` holds is asserted `Send + Sync` by `wgpu` itself, in compiled
  code rather than in prose: `static_assertions::assert_impl_all!(T: Send, Sync)` appears
  for `Surface<'_>`, `Device`, `Queue`, `Sampler`, `ShaderModule`, `RenderPipeline`,
  `BindGroup` and `Texture` in `wgpu-30.0.0/src/api/*.rs`, under a `send_sync` cfg that
  `build.rs` defines as `any(native, …)` — everything that is not wasm32.
- **`Sync` in safe Rust is the statement that concurrent `&self` use is sound.** There is
  nothing beyond that to establish about memory safety, and our position lets us say so
  without hedging: the crate is `#![forbid(unsafe_code)]`, so `Presenter: Send` is an
  auto-trait derivation and not an `unsafe impl` anybody could have got wrong.
  `tests/presenter.rs` asserts it at compile time, the way `RetainedScene`'s `Send` is
  asserted.
- The pipeline store the presenter shares was already `Send + Sync` — the warm-up thread
  had it — so the presenter adds no new sharing. Compilation happens under that store's one
  lock, so your event thread's first present and your render thread's first frame cannot
  compile the same pipeline twice; the second one to arrive blocks briefly and finds it
  done.

**Two caveats, both `wgpu`'s and neither of them ours to fix.** They are stated here rather
than left for you to find:

1. **Surface *creation* panics off the main thread on macOS/Metal** (`wgpu`'s own
   `SurfaceTarget` documentation). This is one more reason the API is detach-and-return
   rather than a constructor: the surface is created where your window lives, and only the
   *presenting* moves.
2. **Presenting from two threads at once is two swapchains' worth of questions asked of one
   swapchain.** `present` takes `&mut self` and there is exactly one presenter per device,
   so the type system already says this; it is worth knowing that the restriction is
   deliberate rather than incidental.

## The provenance check, and the one hole in it

Your §4 says every texture must come from the device the presenter was detached from. We
check it, and here is exactly how far the check reaches.

Everything a texture can be asked about itself is asked **before the swapchain is
acquired** — format, `TEXTURE_BINDING` usage, dimension, sample count, array layers, and
that the placement is finite and invertible — each refused as `RenderError::LayerRefused {
index, reason }` naming which layer and which clause. **Which device made it is not one of
the questions `wgpu` 30 answers**: `wgpu::Texture` exposes size, format, usage and shape and
no device. So the question is put to `wgpu` instead, by offering the texture to this
device's bind group inside a validation error scope, still before the acquire; what the
scope catches becomes `LayerProblem::Unbindable` carrying wgpu's own `DeviceMismatch` text.
Two devices on one instance, one texture, one named refusal — there is a unit test.

**The hole**: a texture from a device of a *different* `wgpu::Instance` is not a foreign
resource to wgpu-core but a **non-existent** one, because resource ids are per-instance, and
the lookup panics inside wgpu before any error scope is consulted. This is not new and it is
not the presenter's: `Target::Texture` has had the same hole since M1. The remedy is the one
`Device::headless_with_instance` and `Device::for_surface_with_instance` already exist for —
**one instance per process, every device built from it** — which is what your `startup.rs`
already does for its own reasons.

## What your side owes the arrangement

Small, and all of it is in the error messages if you get it wrong:

- **Both usages on the page textures.** `Target::Texture` needs `RENDER_ATTACHMENT` and
  sampling needs `TEXTURE_BINDING`; a texture that has been a render target every frame is
  still unpresentable without the second. This is the trap your document's §3 half-spotted,
  and it is `LayerProblem::NotSampleable` by name.
- **Tell the presenter the window's size** (`resize`) at least once before the first
  present, and on every resize. A resize configures nothing — the swapchain follows at the
  next present — so it is cheap to call whenever your window system speaks. A presenter
  that was never told refuses with `PresenterUnsized`; a zero-size window (minimised) is a
  state rather than an error, and the present that meets it is refused with
  `ZeroSizeTarget`.
- **Handle `SurfaceUnavailable` on the presenter as you handle it on the device.** It is
  the same `SurfaceProblem` set and the same "try again" — the presenter reconfigures
  itself on a timeout or an outdated surface, exactly as `Device::render` does.
- **Layers are premultiplied** — which is what a `Target::Texture` frame leaves behind;
  §3's straight-alpha conversion happens at readback and nowhere else. A linear filter
  therefore interpolates in the right space.
- **The window is cleared to transparency on every present**, then the layers land in
  order. An empty slice is a legitimate present: it clears the window. Your chrome layer at
  the identity over your page layer under the reprojection is exactly the intended shape.

## §8 — the fallback you offered, and whether we agree with your argument against it

**We agree, and we did not take it.** `Device::adapter() -> &wgpu::Adapter` would work and
it would be one line plus a field (we do not keep the adapter today), and your three
objections are the right ones: it moves a shader onto your launch path outside our warm
set, it costs us a host that exercises our swapchain code, and it leaves two answers in the
world to "what does this surface accept". The third is the one we would have written first.

Since the split is built, the fallback is moot — but for the record, if a tier-3 host ever
does need to configure its own surface, the smaller thing to expose is the **negotiated
format**, not the adapter. That keeps the negotiation ours and hands over only its result.
We have not built it, and nothing asks for it yet.

## What we could not prove, and who can

**Whether it holds the rate.** Your §7(c) said this first and it is still true from here:
`Xvfb` reports a refresh of 0.00, `--newmode` does not take, and 120 Hz cannot be observed
on this machine at all. What we can show is that the arrangement works and that the
presenting path contains no readback, no upload and no encode — one textured quad per
layer, on a pipeline that is already built. The arithmetic in your §7(a) is yours to close
on the display that states its own refresh, behind the trace lines your ADR 0383 already
prints.

**What the proof does show**, under `Xvfb` and lavapipe, in CI on every push
(`examples/present_thread/`): a page rendered on a second thread while the main thread
presents — **three to five presents got through a single render**, where today's
arrangement produces none — the finished page put on the window under
`scale(2) ∘ translate(64, 32)` with the chrome over it at the identity, and six points of
the window read back with `xwd` and checked against where those two affines say they
should be — then the same page again under `ImageFilter::Linear`, so that both filter
branches are run rather than only compiled. The gate was verified able to fail in three of
the ways it exists to catch: with the offset at zero the uncovered strip shows the page
instead of the clear, with the scale at 1 the mark's edge lands a pixel out, and with the
sampler's coordinate mis-normalised the linear present reads the wrong part of the page.
