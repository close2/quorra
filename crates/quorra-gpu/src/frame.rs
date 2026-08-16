//! The frame, and the measurements that make it accountable.
//!
//! The caller gates performance in CI and attributes regressions by measurement. §8 of
//! the brief is blunt about what that requires, and §6.1 exists *because* the current
//! backend could not answer it: the readback could not be separated from the execution,
//! so a bytes-per-second estimate had to stand in for what a timestamp query would have
//! told them exactly.
//!
//! # The rule this module exists to enforce
//!
//! **A failed frame must not be reported as a drawn one.** A [`Frame`] is constructed
//! only after every fallible step of its render has succeeded; every earlier exit is a
//! [`RenderError`]. And a [`Timings`] whose `execute` is a wall clock rather than a
//! timestamp query says which it is ([`TimingProvenance`]), because a number whose
//! provenance is ambiguous cannot gate anything — wall clocks lie under load: the mean
//! of ten frames put one of §6.1's figures at 15 ms where the fastest of ten put it
//! at 12.
//!
//! [`RenderError`]: crate::error::RenderError

use std::time::Duration;

use crate::error::RenderError;
use crate::report::Report;

/// Straight-alpha RGBA8 pixels, row-major, top row first, no padding — the caller's
/// `Raster` shape and what PNG expects (§3 of the brief).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Raster {
    /// A raster from its parts. Internal: rasters are produced by rendering, and the
    /// length invariant (`width × height × 4`) is established by the readback path.
    pub(crate) fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        debug_assert_eq!(
            pixels.len() as u64,
            u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(4)
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel bytes: straight-alpha RGBA8, row-major, top row first, no padding.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Consume the raster and take its bytes.
    #[must_use]
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

/// Where a duration came from, because the two sources do not lie in the same ways.
///
/// Wall clocks lie under load; timestamp queries measure device time and do not. A
/// gate may trust `TimestampQueries`; a `WallClock` number is context, not evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingProvenance {
    /// Measured on the device with timestamp queries.
    TimestampQueries,
    /// The adapter offers no timestamp queries; the number is a host-side wall clock
    /// over the same span, and says so rather than pretending otherwise.
    WallClock,
}

/// What this frame cost, phase by phase (§8 of the brief).
///
/// # Which clock each number is on, because they are not all the same one
///
/// [`encode`](Timings::encode), [`upload`](Timings::upload) and
/// [`readback`](Timings::readback) are **host** spans: they are `Instant` measurements
/// around code running on the calling thread, and they are commensurable with a wall
/// clock a caller wraps around [`Device::render`]. [`Timings::host_total`] sums exactly
/// those three.
///
/// [`execute`](Timings::execute) is usually **not**: where the adapter offers timestamp
/// queries it is the *device's* clock over the drawing passes
/// ([`execute_provenance`](Timings::execute_provenance) says which). Subtracting it from
/// a host measurement mixes whatever the two clocks disagree by into the remainder —
/// the caller's feedback §13 found exactly that and downgraded its own "elsewhere" row
/// to a bound. So: **subtract `host_total`, not the sum of all four**, and read what is
/// left against the `"target acquire"` and `"present"` entries of
/// [`phases`](Timings::phases), which name the two host-side steps that are not in the
/// three.
///
/// [`Device::render`]: crate::device::Device::render
#[derive(Debug, Clone)]
pub struct Timings {
    /// CPU: turning a [`Scene`](quorra_scene::Scene) into device commands.
    pub encode: Duration,
    /// CPU: preparing and scheduling CPU→GPU transfers for this frame.
    pub upload: Duration,
    /// Device time for the drawing passes, from timestamp queries where available —
    /// see [`execute_provenance`](Timings::execute_provenance).
    pub execute: Duration,
    /// GPU→CPU: copy-out, map, and the premultiplied→straight conversion, as a wall
    /// clock (the span is CPU-bound and includes a wait, which is exactly what the
    /// caller pays). Zero for `Surface` and `Texture` targets.
    pub readback: Duration,
    /// Whether [`execute`](Timings::execute) came from timestamp queries or had to be
    /// a wall clock.
    pub execute_provenance: TimingProvenance,
    /// Per-pass device durations, present when timestamp queries are. M1 has one pass;
    /// entries accumulate as milestones add passes. May also carry one-off costs a
    /// frame absorbed, such as a first-use pipeline compilation.
    pub phases: Vec<(&'static str, Duration)>,
}

impl Timings {
    /// The three phases measured on **your** clock: encode, upload and readback.
    ///
    /// This is the number to subtract from a host-side wall clock around
    /// [`Device::render`](crate::device::Device::render); `execute` is deliberately
    /// excluded because it is usually the adapter's clock, and mixing the two makes the
    /// remainder a quantity with no meaning (the caller's feedback §13). What is left
    /// after subtracting it is the device wait plus the two steps `phases` names,
    /// `"target acquire"` and `"present"`.
    #[must_use]
    pub fn host_total(&self) -> Duration {
        self.encode
            .saturating_add(self.upload)
            .saturating_add(self.readback)
    }
}

/// What a frame's coverage sheet holds — the scratch R8 image every rasterised mark's
/// tile is packed into (ADR 0021, ADR 0034).
///
/// **The same four numbers whether the frame was drawn or refused**:
/// [`Counters::coverage`] on a drawn one, and inside
/// [`RenderError::ScratchExhausted`] on a frame the sheet's own ceiling refused. That
/// pairing is the point of the type. Until
/// ADR 0057 a refused frame carried no accounting at all, so "this page asks for more
/// coverage than any adapter would hold" and "this adapter's texture limit is small"
/// were the same message; they are now the same *comparison*, made on the same fields,
/// in both states.
///
/// Counts and not ratios, for CLAUDE.md's reason: an occupancy figure is a statement
/// about the sheet you packed, never about the coverage you should not have asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoverageSheet {
    /// Coverage tiles seated on the sheet.
    ///
    /// On a drawn frame this is [`Counters::tiles`], reached from the sheet's side; it
    /// is repeated here so that a refusal — which has no `Counters` — can report it too.
    pub tiles: u32,
    /// Texels those tiles hold: **what the frame rasterised, uploaded and sampled**.
    ///
    /// The number a clip that bounds its mark's tile moves, and the one no counter had
    /// before ADR 0057 — [`tiles`](CoverageSheet::tiles) is a count of tiles and says
    /// nothing about how large one was, so a page whose coverage fell by 402× reported
    /// exactly the same figures as before.
    pub texels: u64,
    /// The sheet's width, once every tile has a shelf.
    pub width: u32,
    /// The sheet's height, which is the sum of the shelf heights it opened.
    ///
    /// `width × height` is what was *allocated*; the difference from
    /// [`texels`](CoverageSheet::texels) is what shelf packing left empty. This is also
    /// the extent
    /// [`RenderError::ScratchExhausted`]
    /// bounds: an adapter's per-side texture limit is reached on this axis first,
    /// because a shelf fills in width before the sheet grows tall.
    pub height: u32,
}

impl std::fmt::Display for CoverageSheet {
    /// As it reads inside a refusal: extent, then what is on it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}x{} holding {} tiles and {} texels",
            self.width, self.height, self.tiles, self.texels
        )
    }
}

/// What this frame did, in counts (§8 of the brief).
///
/// The atlas and tiling counters exist from M1 with honest zeros: the fields are the
/// contract, and the milestones that build those subsystems start filling them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Scene commands encoded into this frame.
    pub commands: u32,
    /// Distinct outlines referenced (M2 onwards; 0 until then).
    pub distinct_outlines: u32,
    /// Entries resident in the glyph atlas after this frame (M4; 0 until then).
    pub atlas_entries: u32,
    /// §6.3 of the brief: the count of **distinct keys** this frame asked the atlas
    /// for — deliberately not a hit rate. A hit rate is a statement about the lookups
    /// you made, never about the ones you should have made: a clip-mask cache once
    /// answered all 303 lookups a page made and built 303 identical page-wide masks,
    /// because the key was a name rather than the region.
    pub atlas_distinct_keys: u32,
    /// Atlas bytes this frame's distinct glyph keys asked for, hits included — the
    /// **working set**, which is the number [`Options::atlas_budget`] has to be compared
    /// against (ADR 0050).
    ///
    /// [`atlas_distinct_keys`](Counters::atlas_distinct_keys) says how many things a
    /// page repeats; this says what holding all of them would cost. A page whose working
    /// set exceeds the budget cannot keep its glyphs cached however the packer behaves,
    /// and will rasterise the remainder into the scratch sheet on every frame that
    /// encodes — so this is the number a host raises the budget from, and the only one
    /// that distinguishes "the atlas is too small for this page" from "the atlas is
    /// holding another page's tiles".
    ///
    /// [`Options::atlas_budget`]: crate::startup::Options::atlas_budget
    pub atlas_working_set_bytes: u64,
    /// Whether the atlas was repacked after this frame — the one event that moves every
    /// tile, and so the one that makes a [`RetainedScene`] encode stale (ADR 0050).
    ///
    /// **The instrument for a thrashing atlas.** A repack happens when the atlas is
    /// holding tiles this frame did not use and this frame's own working set would fit
    /// without them; a page that changes settles after at most one. What this counter
    /// exists to make visible is the pathology: if it is true on frame after frame, the
    /// atlas is being repacked for a working set it cannot hold, every retained encode
    /// dies with the layout it named, and every frame pays a full encode. That state was
    /// reachable before ADR 0050 and is not now, and a counter that says so is worth
    /// more than an argument that it cannot happen.
    ///
    /// Always `false` on a replayed frame, which repacks nothing because it inserted
    /// nothing.
    ///
    /// [`RetainedScene`]: crate::retained::RetainedScene
    pub atlas_repacked: bool,
    /// §6.4's instrument: how many **distinct clip regions** this frame resolved,
    /// after chains collapsed to device-space rectangles. Deliberately not a hit
    /// rate, and keyed by the resolved region rather than by identifier — the
    /// caller's clip-mask cache once answered all 303 lookups a page made and built
    /// 303 identical page-wide masks, because its key was a name (its ADR 0132; the
    /// same page collapses to 1 here).
    pub clip_distinct_regions: u32,
    /// Distinct **residue clip regions** this frame rasterised: a chain whose links are
    /// not all rectangles becomes one coverage region, cut into a window for every mark
    /// drawn under it (ADR 0049).
    ///
    /// The other half of §6.4's instrument, and keys again rather than lookups: a page
    /// that states one curved clip and draws six hundred marks under it reports **1**
    /// here and 600 in [`tiles`](Counters::tiles).
    pub clip_residue_regions: u32,
    /// Residue rasterisations this frame charged to a single command's tile — the work
    /// [`clip_residue_regions`](Counters::clip_residue_regions) did **not** remove.
    ///
    /// A chain pays per tile when its region would cost more than the tiles it would
    /// replace (a page-sized clip over a paragraph of small marks) or when the frame's
    /// regions have reached their share of `Options::max_frame_bytes`. Neither is an
    /// error and neither changes a pixel; this is the counter that says how often it
    /// happened, because a decline nobody counts is a cost nobody adds up.
    pub clip_residue_tiles: u32,
    /// Coverage tiles this frame placed on the scratch sheet — the general path lane's
    /// output, and the GPU lane's, since both go through the one packer.
    ///
    /// Reported from M5's landing and honest since ADR 0024; it read zero for three
    /// milestones after the lane that fills it shipped, which is the shape of counter a
    /// `Frame` must not have.
    pub tiles: u32,
    /// What this frame's coverage cost: the sheet's extent and the texels its tiles hold
    /// (ADR 0057).
    ///
    /// [`tiles`](Counters::tiles) says how many marks reached the sheet and never how
    /// large one was, which is why the two rounds before ADR 0057 could measure a
    /// four-hundred-fold change in a page's coverage and move no counter at all. This is
    /// that number.
    pub coverage: CoverageSheet,
    /// Path segments processed (M5; 0 until then).
    pub segments: u32,
    /// Commands rejected for reaching no pixel of the target — outside it, or clipped
    /// to nothing — before any geometry was built for them (ADR 0015).
    ///
    /// The instrument that says how much of a frame the page did not need: at 20×
    /// magnification a viewer hands over a whole page for a window showing a
    /// fortieth of it, and this counts what that costs nothing. A frame drawing
    /// everything it was given reports zero; it is not an error either way.
    pub commands_culled: u32,
    /// Child layers this frame built and then did not composite, because the clip the
    /// composite would have applied left them no pixel to contribute (ADR 0041).
    ///
    /// A group, and every element with a non-Normal blend mode, is a child layer. This
    /// counts the ones whose whole rendering — a texture, a pass for each plan beneath
    /// them, and the composite itself — was skipped because the result was identical to
    /// the backdrop it would have been written over.
    ///
    /// **Not [`commands_culled`](Counters::commands_culled)'s kind of saving**, which is
    /// why it is a second counter rather than more of the first: a culled command never
    /// had its geometry built, while a culled layer was encoded in full and only its
    /// rendering was saved. Multiplying this by what a group costs to encode would
    /// overstate it.
    pub layers_culled: u32,
    /// Bytes scheduled for CPU→GPU transfer this frame.
    pub bytes_uploaded: u64,
    /// Internal layer textures this frame allocated — the compositor's peak, not its
    /// total (ADR 0020).
    ///
    /// A group and every element with a non-Normal blend mode is a plan, and each plan
    /// accumulates in **one** texture the size of its own bounds (ADR 0036, ADR 0038);
    /// textures are reused between siblings, so this follows the **depth** of the plan
    /// tree rather than its size. One of them is not a plan's: while a child is being
    /// composited, a copy of the pixels it covers is alive beside it, because a pass
    /// cannot read the attachment it writes.
    ///
    /// **The unit is textures alive at one instant, and it is not bytes.** It was never
    /// bytes, but until ADR 0036 every layer texture was the size of the target, so
    /// multiplying this by the target's size answered the budget question by accident.
    /// It no longer does — the textures differ in size — and the number
    /// `Limits::max_frame_bytes` is spent on is the budget refusal's own arithmetic,
    /// not anything derivable from here. What this is good for is unchanged: a frame
    /// that reports 4 allocated 4, whatever its group count says.
    ///
    /// A layer culled by ADR 0041 allocates nothing and is counted by
    /// [`layers_culled`](Counters::layers_culled) instead, so the two move in opposite
    /// directions on the same page.
    pub layer_textures: u32,
}

/// Where a frame's device commands came from: this call's encode, or one retained
/// from an earlier frame of the same scene at the same viewport (ADR 0048).
///
/// **An observable rather than an inference from [`Timings::encode`].** A replayed
/// frame's `encode` is a few microseconds of key comparison, and "small" is not a
/// state a test or a gate can assert on: the two variants of that state are named
/// here so that a caller — and this tree's own suite — can tell which happened
/// without believing a clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeSource {
    /// The scene was walked and encoded by this call, as every
    /// [`Device::render`](crate::device::Device::render) frame is.
    Encoded,
    /// An encode retained by a [`RetainedScene`](crate::retained::RetainedScene) was
    /// replayed: the walk, the coverage rasterisation and the instance layout of this
    /// frame were all paid for by an earlier one.
    Replayed,
}

/// A drawn frame: what it cost, what it counted, and what it could not do as asked.
///
/// Everything a `Frame` says about itself is true. It exists only if the render
/// succeeded; a refused frame is a [`RenderError`], never a `Frame` that looks drawn.
/// A frame over a blank scene is a legitimate frame: zero commands, no reports, `Ok`.
#[derive(Debug)]
pub struct Frame {
    pub(crate) timings: Timings,
    pub(crate) counters: Counters,
    pub(crate) reports: Vec<Report>,
    pub(crate) payload: Payload,
    pub(crate) encode_source: EncodeSource,
}

/// What the render call produced besides accounting. Private: which payload a frame
/// carries is fully determined by the `Target` the caller passed.
#[derive(Debug)]
pub(crate) enum Payload {
    /// `Surface` and `Texture` targets: the pixels are where the caller asked.
    None,
    /// `Readback`: the pixels, held until [`Frame::into_raster`].
    Raster(Raster),
}

impl Frame {
    /// Everything the device could not draw as asked. Empty on a fully-honoured frame.
    #[must_use]
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }

    /// What this frame cost, phase by phase.
    #[must_use]
    pub fn timings(&self) -> &Timings {
        &self.timings
    }

    /// What this frame did, in counts.
    #[must_use]
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// Whether this frame encoded its scene or replayed a retained encode (ADR 0048).
    ///
    /// Always [`EncodeSource::Encoded`] for a frame from
    /// [`Device::render`](crate::device::Device::render), which retains nothing.
    #[must_use]
    pub fn encode_source(&self) -> EncodeSource {
        self.encode_source
    }

    /// Take the pixels of a `Readback` frame.
    ///
    /// The readback itself — the expensive part — already happened inside `render` and
    /// was priced in [`Timings::readback`]; this is a move.
    ///
    /// # Errors
    ///
    /// [`RenderError::NotAReadbackFrame`] if the frame was rendered to a `Surface` or
    /// `Texture` target: those pixels are already where the caller asked, and
    /// pretending otherwise here would misreport what the frame did.
    pub fn into_raster(self) -> Result<Raster, RenderError> {
        match self.payload {
            Payload::Raster(raster) => Ok(raster),
            Payload::None => Err(RenderError::NotAReadbackFrame),
        }
    }
}
