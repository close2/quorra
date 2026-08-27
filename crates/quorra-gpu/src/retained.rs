//! A scene the caller keeps, and the encode of its last frame (ADR 0048).
//!
//! # The measurement this module exists for
//!
//! A steady dense-text encode is **78 % recording** by instruction count, and over 40 %
//! of recording is a pure function of `(scene, viewport)` — the outline bounds, the
//! sub-pixel phase, the atlas keys, the lane choice, the instance bytes
//! (`doc/PLAN.md`, 2026-08-14). A person reading a page pays all of it again on every
//! frame for an answer that cannot differ. ADR 0045 priced the alternative on a
//! throwaway prototype at **0.154 ms against 1.538**; this module is that prototype
//! built, and `examples/retained.rs` is the same measurement through the real API.
//!
//! # Why the caller holds it rather than the device
//!
//! The device could cache the last frame's encode against the scene it was made from,
//! and the caller this library is written for would never hit it: their frame's scene
//! is rebuilt with fresh `Arc`s every frame (`pdf-viewer/doc/todo/44` §3), so a key on
//! scene identity misses every time. The unit that survives their frame loop is a
//! *handle they hold*, and making the handle own the scene has a second property worth
//! more than the first:
//!
//! **A [`RetainedScene`] cannot draw a scene other than the one it holds.** The
//! encode is replayed only for the scene it was made from — that is structural, not a
//! promise either side keeps — so this API adds no way to produce a wrong page that
//! passing a stale [`Scene`] to [`Device::render`] did not already have. Principle 6's
//! hazard here is a *stale* encode, and every input an encode reads is either the
//! scene the handle owns or is in the encode key beside it, which is compared by bits.
//!
//! [`Device::render`]: crate::device::Device::render
//!
//! # What it costs, because a cost in a doc comment is a cost nobody adds up
//!
//! A retained encode is not small: for the dense-text archetype it is about a third of
//! a megabyte, and three pages of the caller's corpus place 194 to 253 MB of coverage
//! tiles at 4× magnification. So the bytes are [`RetainedScene::retained_bytes`], a
//! number a caller can read and budget against, and the decision to hold one — for how
//! many pages, and for how long — is the caller's, where the knowledge of how many
//! pages are resident already lives (§11.5 of the brief puts that posture on `Scene`,
//! and this is the same posture one layer down).

use quorra_scene::Scene;

use crate::encode::Encoded;
use crate::frame::EncodeSource;
use crate::startup::Coverage;
use crate::viewport::Viewport;

/// Everything an encode depends on that is **not** the scene, as bits that compare.
///
/// The list is enumerated rather than discovered, because a retained encode is a
/// machine for producing a plausible-looking wrong page and the enumeration is the only
/// defence (ADR 0045's invalidation list, ADR 0048's mechanism). Each field is an input
/// `encode` reads:
///
/// - **the device** — an encode names atlas positions, resource ids and a coverage lane
///   that belong to one [`Device`](crate::device::Device); a handle carried to another
///   one re-encodes rather than replaying into a texture atlas it never saw;
/// - **the viewport's size and affine** — every device bound, every clip rectangle and
///   every atlas key is a position in *this* target's pixels (ADR 0045's survival
///   table: a fractional scroll or a zoom step reuses nothing per command);
/// - **the coverage lane** ([`Device::set_coverage`](crate::device::Device::set_coverage))
///   — it chooses which lane makes coverage bytes, per frame, by design;
/// - **the atlas generation** — a repack moves every tile, and the retained quad
///   instances carry absolute texel origins into the sheet. Insertion never moves an
///   existing entry, so only [`AtlasStore::reset`](crate::atlas) bumps this, and ADR
///   0050 is what stops a page too large for the atlas from resetting it every frame;
/// - **the resource generation** — a released id must never be drawn from a stale
///   instance stream. An upload cannot invalidate anything *while ids remain*: they are
///   minted by increment from a counter that has never wrapped, so a retained encode's
///   ids cannot come to name other bytes. `ResourceStore::allocate_id` states the bound
///   that claim rests on, which is the one this key does not cover.
///
/// **The damage list is deliberately absent**, and so is the target. `encode` never
/// reads `Viewport::damage` — damage is planned target-side (ADR 0012) — and phase 1
/// runs before any allocation and knows no target, so a damaged frame and a frame into
/// a different texture both replay a full frame's encode. The frame budget, the
/// adapter's maximum dimension, the glyph quantum and the encode instrument are fixed
/// for a device's life and are covered by the device field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EncodeKey {
    device: u64,
    width: u32,
    height: u32,
    /// The six affine coefficients by their `f32` bit patterns. Bits rather than
    /// equality: `NaN` is refused before this is built, and `-0.0` compares unequal to
    /// `0.0` here, which costs a re-encode that draws the same page and can never be
    /// the other error.
    transform: [u32; 6],
    coverage: Coverage,
    atlas_generation: u64,
    resource_generation: u64,
}

impl EncodeKey {
    /// Whether an encode held under `self` may be **record-replayed** for a frame
    /// keyed `asked` (`encode/replay.rs`, ADR 0087): same device, same lane, same
    /// resource generation — the records carry denormalised control boxes and collapse
    /// tables, which resource ids' never-rebound guarantee keeps true — at **any**
    /// viewport. The atlas generation is deliberately not compared: a replayable
    /// encode named no atlas tile, which is part of what admission means.
    pub(crate) fn replay_compatible(&self, asked: &EncodeKey) -> bool {
        self.device == asked.device
            && self.coverage == asked.coverage
            && self.resource_generation == asked.resource_generation
    }

    /// The key of the frame about to be encoded. Built by the device, which is where
    /// every one of these lives.
    pub(crate) fn new(
        device: u64,
        viewport: &Viewport<'_>,
        coverage: Coverage,
        atlas_generation: u64,
        resource_generation: u64,
    ) -> Self {
        let t = viewport.transform;
        Self {
            device,
            width: viewport.width,
            height: viewport.height,
            transform: [
                t.a.to_bits(),
                t.b.to_bits(),
                t.c.to_bits(),
                t.d.to_bits(),
                t.e.to_bits(),
                t.f.to_bits(),
            ],
            coverage,
            atlas_generation,
            resource_generation,
        }
    }
}

/// One retained encode and the key it is valid under.
#[derive(Debug)]
struct Held {
    key: EncodeKey,
    encoded: Encoded,
    /// Counted once, when the encode is stored, so that asking is free.
    bytes: u64,
}

/// A [`Scene`] the caller keeps across frames, and the encode of its last frame.
///
/// Render it with
/// [`Device::render_retained`](crate::device::Device::render_retained). A frame whose
/// scene, viewport and device state are unchanged replays the retained encode and skips
/// phase 1 entirely; any change at all re-encodes, and the frame says which happened
/// ([`Frame::encode_source`](crate::frame::Frame::encode_source)).
///
/// # The one obligation this puts on a caller
///
/// **Call [`set_scene`](RetainedScene::set_scene) when the content changes.** What is
/// drawn is the scene this handle holds, so a handle whose scene is stale draws a stale
/// page — exactly as passing that same stale [`Scene`] to
/// [`Device::render`](crate::device::Device::render) would. That is the whole of the
/// difference between this and immediate mode, and it is why the scene is *owned* here
/// rather than passed alongside: there is no second scene for the retained encode to
/// disagree with.
///
/// # What survives, and what cannot
///
/// | the next frame differs by | what happens |
/// |---|---|
/// | nothing | the encode is replayed |
/// | its damage list | replayed — `encode` never reads damage (ADR 0012) |
/// | the target it draws into | replayed — phase 1 knows no target |
/// | a scroll of a whole number of device pixels | **re-encoded**: the atlas tiles are still valid and are reused, but every device bound, cull and clip rectangle is an absolute position |
/// | a scroll of a fraction of a pixel, or a zoom | **re-encoded**, and nothing in the glyph lane survives it: the quantised phase and the linear part are inside every atlas key (ADR 0009) |
/// | the scene | **re-encoded** |
///
/// A zoom step is a different rasterisation of every glyph on the page, so no design
/// reuses one — ADR 0045 refuses that hope rather than leaving it to be discovered
/// after an API is built on it.
///
/// # Threads
///
/// `Send` and **not** `Sync`: a handle can be built on one thread and rendered from
/// another, and cannot be rendered from two at once — which is the same posture
/// [`Device`](crate::device::Device) has, since rendering takes `&mut` on both. The
/// [`Scene`] inside stays `Send + Sync` and can be cloned out of
/// [`scene`](RetainedScene::scene) and shared as freely as ever.
///
/// # Example
///
/// ```no_run
/// # use quorra_gpu::{Device, Options, RetainedScene, Target, Viewport};
/// # use quorra_scene::{Affine, SceneBuilder};
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut device = Device::headless(&Options::default())?;
/// let mut page = RetainedScene::new(SceneBuilder::new().finish());
/// let viewport = Viewport::full(1191, 1684, Affine::IDENTITY);
///
/// // Every frame of a page nobody is touching: the second and later ones replay.
/// for _ in 0..60 {
///     device.render_retained(&mut page, &viewport, Target::Readback)?;
/// }
///
/// // The document changed: hand over the new scene, and the next frame encodes it.
/// page.set_scene(SceneBuilder::new().finish());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct RetainedScene {
    scene: Scene,
    held: Option<Held>,
}

impl RetainedScene {
    /// A handle over `scene`, holding no encode yet.
    ///
    /// Needs no device and no viewport: a host can build one on the thread that built
    /// the scene, before a [`Device`](crate::device::Device) exists.
    #[must_use]
    pub fn new(scene: Scene) -> Self {
        Self { scene, held: None }
    }

    /// The scene this handle draws.
    #[must_use]
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Draw a different scene from now on, dropping the retained encode.
    ///
    /// The call to make when the document, the page or anything the scene was built
    /// from has changed. Handing over an *identical* scene is not wrong — it costs one
    /// encode, which is what every frame costs today — but it buys nothing, so a host
    /// that can tell whether its content moved should ask before rebuilding.
    pub fn set_scene(&mut self, scene: Scene) {
        self.scene = scene;
        self.held = None;
    }

    /// Drop the retained encode, keeping the scene: the next frame encodes again.
    ///
    /// For a host under memory pressure — [`retained_bytes`](RetainedScene::retained_bytes)
    /// is what it gives back. Nothing else needs it: every way an encode can go stale
    /// is detected, and forgetting one is never required for correctness.
    pub fn forget(&mut self) {
        self.held = None;
    }

    /// Whether an encode is retained right now.
    #[must_use]
    pub fn holds_encode(&self) -> bool {
        self.held.is_some()
    }

    /// Heap bytes the retained encode holds, or zero when none is retained.
    ///
    /// Every buffer in it: the two instance streams, the coverage sheet, the GPU lane's
    /// vertices and tiles, the layer plans and their op lists, the mask plans and the
    /// resource id lists. Counted once when the encode is stored, so asking is free.
    ///
    /// **This is the number to budget against.** The dense-text archetype retains about
    /// a third of a megabyte; a page of the caller's corpus that places a quarter of a
    /// gigabyte of coverage tiles at 4× retains that. A handle per resident page is a
    /// decision with a size, and this is the size.
    #[must_use]
    pub fn retained_bytes(&self) -> u64 {
        self.held.as_ref().map_or(0, |held| held.bytes)
    }

    /// The encode to draw this frame: the retained one when `key` still holds, a fresh
    /// one from `encode` otherwise.
    ///
    /// A failed encode leaves the handle holding nothing, which is the honest state: the
    /// key that was there did not match, so what it held was stale, and a refusal is not
    /// a reason to keep a stale encode alive.
    pub(crate) fn prepare<E>(
        &mut self,
        key: EncodeKey,
        build: impl FnOnce(&Scene, Option<&crate::encode::ReplayList>) -> Result<Encoded, E>,
    ) -> Result<(EncodeSource, &Encoded), E> {
        // Taken and put back rather than matched in place: `Option::insert` returns the
        // stored value infallibly, so the encode this call draws is the encode this
        // call stored, with no unwrap and no unreachable arm between the two.
        let (source, held) = match self.held.take() {
            Some(held) if held.key == key => (EncodeSource::Replayed, held),
            // A different viewport over the same everything-else, and the held encode
            // wrote its records down: the walk is replayed from them instead of the
            // scene (ADR 0087). A failure falls through to nothing held, exactly as a
            // failed encode does — what was held was for another viewport anyway.
            Some(held) if held.key.replay_compatible(&key) && held.encoded.replay.is_some() => {
                // Admission was checked one line up; a race between the two reads is
                // not possible on `&mut self`.
                #[allow(clippy::expect_used)]
                let list = held
                    .encoded
                    .replay
                    .as_ref()
                    .expect("checked by the guard above");
                let encoded = build(&self.scene, Some(list))?;
                let bytes = encoded.retained_bytes();
                (
                    EncodeSource::RecordReplayed,
                    Held {
                        key,
                        encoded,
                        bytes,
                    },
                )
            }
            _ => {
                let encoded = build(&self.scene, None)?;
                let bytes = encoded.retained_bytes();
                let held = Held {
                    key,
                    encoded,
                    bytes,
                };
                (EncodeSource::Encoded, held)
            }
        };
        Ok((source, &self.held.insert(held).encoded))
    }
}
