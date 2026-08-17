//! The glyph atlas: cached coverage tiles, keyed by what actually repeats.
//!
//! # The number that decides the design
//!
//! One dense page of the caller's corpus is **5 933 fills of 107 distinct outlines**
//! (§6.3 of the brief). A glyph's sub-pixel phase is an arbitrary float, so an
//! exactly-correct cache never hits; quantised to 1/16 of a pixel it hit 5.0× on that
//! page and left the caller's oracle unmoved, where 1/8 contradicted pages (their
//! ADR 0131). The quantum is therefore §4.5's fifth decision — the one that is the
//! caller's to make and ours to expose: [`crate::startup::Options::glyph_quantum`],
//! default 1/16, settable, and `None` switches quantisation off (exact-phase keying,
//! which still caches exact repeats).
//!
//! # Keying
//!
//! `(outline, linear part bit-exact, quantised phase)`. The linear part is keyed by
//! its f32 bit patterns rather than a bucket: every occurrence of a glyph at one font
//! size carries the *identical* matrix, so exact bits hit exactly where reuse exists,
//! and an arbitrary zoom animation simply misses (rasterising is the correct cost of
//! new geometry). Recorded with the design in ADR 0009.
//!
//! # Storage, eviction, and honesty
//!
//! One R8 texture sized from [`crate::startup::Options::atlas_budget`], shelf-packed.
//! A tile that does not fit falls through to the scratch path — drawn uncached, and
//! correctly — rather than failing the frame. Between frames the device may repack
//! (`Device::settle_atlas`), and only when repacking can change the outcome: an atlas
//! holding nothing but the tiles the frame itself put there re-packs to the layout it
//! already has, so taking the reset would buy nothing and would cost every retained
//! encode keyed on that layout (ADR 0024, ADR 0050).
//!
//! `Counters` reports `atlas_distinct_keys` — the count of distinct keys a frame asked
//! for, deliberately not a hit rate (§6.3's lesson: a hit rate describes the lookups you
//! made, never the ones you should have made) — `atlas_working_set_bytes` for what
//! holding all of them would cost, and `atlas_repacked` for the event that moves them.

use crate::keyhash::FastMap;
use crate::raster::{CoverageMask, DeviceTransform, Rule};
use quorra_scene::OutlineId;

/// The largest share of the atlas one tile may take, as a divisor (ADR 0024).
///
/// The bound this replaces was a *dimension* — 128 pixels a side — and what it
/// protected was a *budget*: "one zoomed-in letterform must not evict a page's worth of
/// text". Those are not the same rule, and the mismatch was a cliff. Past 128 a glyph
/// never entered the atlas, so a frame magnified past about 10× rasterised every visible
/// letterform again on every frame: 12.4 ms to draw thirty glyphs against 1.2 ms to draw
/// 5 933.
///
/// An eighth of an 8 MiB atlas is a 1 MiB tile — 1024×1024, or a letterform at roughly
/// 70× a reading size — so eight such tiles fill the atlas and a ninth evicts. That is
/// the protection stated against the quantity it was always about.
const MAX_TILE_SHARE: u64 = 8;

/// The sub-pixel phase part of a key: quantised to `1/q` when a quantum is set,
/// bit-exact otherwise. Two variants rather than a silently-exact quantum of 1, so
/// "off" is visible in the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PhaseKey {
    /// Numerator pair under the configured denominator.
    Quantised(u16, u16),
    /// Exact f32 bit patterns of the fractional translation.
    Exact(u32, u32),
}

/// What repeats, made hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    pub outline: u32,
    /// Bit patterns of the composed device linear part (a, b, c, d).
    pub linear: [u32; 4],
    pub phase: PhaseKey,
    /// §8.5.3.3's rule. Not decoration: an outline with a nested subpath wound the same
    /// way fills solid under non-zero and holes under even-odd, so a key without it
    /// hands the first picture to the second request. It was missing until ADR 0024,
    /// and invisible only because the dimension cap kept most such shapes out of the
    /// atlas entirely.
    pub rule: Rule,
}

/// One placement of an outline, resolved into the key it would be cached under and the
/// two parts its translation splits into.
///
/// The split is the whole of what the quantum does: a tile is rasterised at the
/// *quantised fractional* offset and then drawn at the *integer* one, so every placement
/// sharing a phase bucket shares one rasterisation and lands where its own translation
/// says. Computed once per solid fill, because both the lane choice and the atlas path
/// need the same answer and computing it twice is how two readings of one number begin.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphPlacement {
    /// What the tile is cached under.
    pub key: GlyphKey,
    /// The integer part of the device translation: where the tile's pixels go.
    pub origin: [f32; 2],
    /// The quantised fractional part: the offset the tile is rasterised at.
    pub phase: [f32; 2],
}

impl GlyphPlacement {
    /// Splits a placement's translation and builds its key.
    ///
    /// `quantum` is §4.5's fifth decision, the one that is ours to expose: `Some(q)`
    /// rounds the phase to `1/q` of a pixel so that repeats collide, `None` keys the
    /// exact bits so that only exact repeats do.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a fraction
    // times a `u16`, both bounded: `fx` is in `0..1` by construction below
    #[allow(clippy::arithmetic_side_effects)] // `q` is non-zero (`Options` validates it)
    pub(crate) fn of(
        outline: OutlineId,
        to_device: &DeviceTransform,
        rule: Rule,
        quantum: Option<u16>,
    ) -> Self {
        let (ix, fx) = (to_device.e.floor(), to_device.e - to_device.e.floor());
        let (iy, fy) = (to_device.f.floor(), to_device.f - to_device.f.floor());
        let (phase, px, py) = match quantum {
            Some(q) => {
                let fq = f32::from(q);
                let nx = (fx * fq).round() as u16 % q;
                let ny = (fy * fq).round() as u16 % q;
                (
                    PhaseKey::Quantised(nx, ny),
                    f32::from(nx) / fq,
                    f32::from(ny) / fq,
                )
            }
            None => (PhaseKey::Exact(fx.to_bits(), fy.to_bits()), fx, fy),
        };
        Self {
            key: GlyphKey {
                outline: outline.0,
                linear: [
                    to_device.a.to_bits(),
                    to_device.b.to_bits(),
                    to_device.c.to_bits(),
                    to_device.d.to_bits(),
                ],
                phase,
                rule,
            },
            origin: [ix, iy],
            phase: [px, py],
        }
    }
}

/// What the atlas would do for one placement — the question the coverage lane is chosen
/// by (ADR 0029).
#[derive(Debug, Clone, Copy)]
pub(crate) enum CacheProspect {
    /// A tile this size may not be cached at all: it does not fit the texture, or it
    /// would take more than [`MAX_TILE_SHARE`] of it (ADR 0024).
    ///
    /// A tile the atlas would take but has no *room* for is deliberately not a case
    /// here. Asking about room was implemented and measured, and it moved nothing on any
    /// page shape tried — the census keeps single-use tiles out of the atlas, so a full
    /// atlas is one holding tiles that are being reused, and those are worth their space.
    /// The ADR records the numbers.
    TooLarge,
    /// It may be cached, under this key.
    Admitted {
        placement: GlyphPlacement,
        /// The entry, when one exists already — so this placement costs a quad and
        /// nothing else.
        ///
        /// The *entry* rather than a `resident` flag, because the lane and the draw
        /// both need the answer and asking twice hashed the key twice for every glyph
        /// on the page: ADR 0024 recorded that shape as the reason `keyhash` exists,
        /// and it was still here. Carrying it grows a `Copy` enum that lives in one
        /// local by 28 bytes, and makes it the same read *by construction* — which is
        /// the property `prospect`'s comment already asks for and could previously
        /// only state.
        entry: Option<AtlasEntry>,
        /// The scene places this shape exactly once, so an entry made for it would be
        /// written and read one time each.
        once: bool,
    },
}

impl CacheProspect {
    /// Whether putting this placement through the atlas buys anything.
    ///
    /// Three ways to answer no, and each is a measurement rather than a taste
    /// (`tests/lane_crossover.rs`): a tile the atlas refuses is rasterised into the
    /// scratch sheet on **every** frame, where the device is two to three times faster;
    /// a tile placed once is rasterised, uploaded and read once, which is the atlas's
    /// whole cost and none of its benefit. Yes has one shape — an entry that is read
    /// more than it is written — and it is worth twenty to sixty times what either lane
    /// can do, which is why the test is asked in this direction.
    pub(crate) fn worth_caching(self) -> bool {
        match self {
            Self::TooLarge => false,
            Self::Admitted { entry, once, .. } => entry.is_some() || !once,
        }
    }

    /// The key and offsets this placement would use, and the entry already holding it,
    /// when the atlas would take it.
    ///
    /// Both together and not two accessors: a lane chosen on one reading of the cache
    /// and drawn on another is how a tile ends up rasterised twice, which is the
    /// hazard [`AtlasStore::prospect`] is written against.
    pub(crate) fn admission(self) -> Option<(GlyphPlacement, Option<AtlasEntry>)> {
        match self {
            Self::TooLarge => None,
            Self::Admitted {
                placement, entry, ..
            } => Some((placement, entry)),
        }
    }
}

/// A resident tile: where it sits in the atlas, and how the tile's pixels relate to
/// the quantised origin it was rasterised against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AtlasEntry {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Tile origin relative to the (integer-translated) glyph origin.
    pub tile_left: i32,
    pub tile_top: i32,
}

/// One shelf of the packer: a horizontal strip of fixed height filling left to right.
#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    cursor: u32,
}

/// A pending pixel upload into the atlas texture, staged CPU-side until the device
/// flushes it before the frame's passes.
#[derive(Debug)]
pub(crate) struct AtlasUpload {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The CPU-side state of the atlas: the packer, the key table, and the uploads the
/// next flush owes the texture. The `wgpu` texture itself lives on the device, which
/// creates it lazily on the first frame that needs a glyph (startup rule, §7).
#[derive(Debug)]
pub(crate) struct AtlasStore {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    entries: FastMap<GlyphKey, AtlasEntry>,
    pending: Vec<AtlasUpload>,
    /// Bumped by [`AtlasStore::reset`] and by nothing else — insertion appends and
    /// never moves an entry, so this is exactly a count of the times every texel origin
    /// in the sheet stopped meaning what it meant.
    ///
    /// Read by one thing: the key a [`RetainedScene`](crate::retained::RetainedScene)
    /// encode is stored under (ADR 0048). The texture is not recreated on a reset and
    /// neither is any bind group — the stale pixels are simply never named again,
    /// because the entries that named them are gone.
    pub generation: u64,
}

impl AtlasStore {
    /// An atlas sized from the byte budget: near-square (an R8 texel is one byte),
    /// width capped at 2048 and both sides clamped to the device's texture limit.
    pub(crate) fn new(budget_bytes: u64, max_dimension: u32) -> Self {
        #[allow(clippy::cast_possible_truncation)] // isqrt of a u64 budget fits u32 here
        let side = (budget_bytes.isqrt().max(1) as u32)
            .min(2048)
            .min(max_dimension.max(1));
        let width = side;
        // Width is at least 1 by the max(1) above, so the division is total.
        #[allow(clippy::cast_possible_truncation, clippy::arithmetic_side_effects)]
        let height = ((budget_bytes / u64::from(width)).max(1) as u32).min(max_dimension.max(1));
        Self {
            width,
            height,
            shelves: Vec::new(),
            next_shelf_y: 0,
            entries: FastMap::default(),
            pending: Vec::new(),
            generation: 0,
        }
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// What this atlas would do for one placement of a tile this size (ADR 0029).
    ///
    /// The coverage lane turns on this rather than on the tile's area, so it is answered
    /// in one place and by the atlas itself: whether a tile that size may be cached at
    /// all, whether the entry exists already, and whether a scene that places it once
    /// has now done so twice running. What one *scene* does with the entry is the half
    /// the atlas cannot know, and the caller supplies it as `placed_once`.
    ///
    /// **The answer depends on nothing but this frame**, which is a property rather than
    /// an accident: a placement takes the same lane every time an unchanged scene is
    /// drawn, so a page redrawn on a scroll tick is the same pixels as the page that was
    /// there. A version of this that remembered which keys the *previous* frame declined
    /// to cache was measured and rejected — see the ADR; it made the third frame of a
    /// static page a different picture from the first.
    pub(crate) fn prospect(
        &self,
        placement: GlyphPlacement,
        width: u32,
        height: u32,
        placed_once: bool,
    ) -> CacheProspect {
        if !self.admits(width, height) {
            return CacheProspect::TooLarge;
        }
        let entry = self.get(&placement.key);
        CacheProspect::Admitted {
            once: placed_once && entry.is_none(),
            entry,
            placement,
        }
    }

    /// Whether a tile of this size may be cached at all (ADR 0024).
    ///
    /// Two conditions, both about this atlas rather than about a constant: the tile must
    /// fit the texture — a wider one can never be packed — and it must take no more than
    /// [`MAX_TILE_SHARE`] of it. A tile the rule refuses still draws, uncached, through
    /// the scratch path; the caller decides nothing by asking.
    pub(crate) fn admits(&self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 || width > self.width || height > self.height {
            return false;
        }
        let tile = u64::from(width).saturating_mul(u64::from(height));
        let whole = u64::from(self.width).saturating_mul(u64::from(self.height));
        tile.saturating_mul(MAX_TILE_SHARE) <= whole
    }

    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn get(&self, key: &GlyphKey) -> Option<AtlasEntry> {
        self.entries.get(key).copied()
    }

    /// Take the uploads owed to the texture (the device flushes them each frame).
    pub(crate) fn take_pending(&mut self) -> Vec<AtlasUpload> {
        std::mem::take(&mut self.pending)
    }

    /// Insert a rasterised tile under its key. `None` when there is no room for it —
    /// the caller then draws it through the scratch path instead, which is correct and
    /// uncached.
    ///
    /// On a full atlas this does **not** evict piecemeal: it fails, and the *device*
    /// may repack between frames (`reset`, `Device::settle_atlas`), which keeps the
    /// packing deterministic — the same scene always produces the same atlas layout
    /// (§4.6). Insertion itself never moves an entry that is already here, which is what
    /// lets a retained encode name absolute texel origins and still be replayed after a
    /// frame that inserted more tiles.
    pub(crate) fn insert(&mut self, key: GlyphKey, mask: &CoverageMask) -> Option<AtlasEntry> {
        let (x, y) = self.allocate(mask.width, mask.height)?;
        let entry = AtlasEntry {
            x,
            y,
            width: mask.width,
            height: mask.height,
            tile_left: mask.left,
            tile_top: mask.top,
        };
        self.entries.insert(key, entry);
        self.pending.push(AtlasUpload {
            x,
            y,
            width: mask.width,
            height: mask.height,
            pixels: mask.coverage.clone(),
        });
        Some(entry)
    }

    /// Drop every entry and start packing afresh. Pending uploads die with the
    /// layout they were packed for.
    ///
    /// **Called between frames and never inside an encode**, and that is load-bearing
    /// rather than incidental: a retained encode is keyed on the generation read
    /// *before* the walk (`retained::EncodeKey`), so a reset taken half way through one
    /// would leave the first half of the encode naming texels that had moved while the
    /// key still read valid — a plausible wrong page, which is principle 6's worst
    /// outcome. The one call site is `Device::settle_atlas`, after the frame is drawn.
    pub(crate) fn reset(&mut self) {
        self.shelves.clear();
        self.next_shelf_y = 0;
        self.entries.clear();
        self.pending.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    #[allow(clippy::arithmetic_side_effects)] // packer arithmetic is bounded by the texture dims
    fn allocate(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width == 0 || height == 0 || width > self.width {
            return None;
        }
        // First shelf tall enough with room; heights are capped at 2× the request so
        // a short tile cannot squat in a tall shelf forever.
        for shelf in &mut self.shelves {
            if shelf.height >= height
                && shelf.height <= height.saturating_mul(2)
                && shelf.cursor + width <= self.width
            {
                let position = (shelf.cursor, shelf.y);
                shelf.cursor += width;
                return Some(position);
            }
        }
        if self.next_shelf_y + height <= self.height {
            let opened = Shelf {
                y: self.next_shelf_y,
                height,
                cursor: width,
            };
            self.next_shelf_y += height;
            let placement = (0, opened.y);
            self.shelves.push(opened);
            return Some(placement);
        }
        None
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // test tile sizes are tiny and literal
mod tests {
    use super::{AtlasStore, GlyphKey, GlyphPlacement, PhaseKey};
    use crate::raster::{CoverageMask, Rule};

    fn key(outline: u32, phase: (u16, u16)) -> GlyphKey {
        GlyphKey {
            rule: Rule::NonZero,
            outline,
            linear: [0, 0, 0, 0],
            phase: PhaseKey::Quantised(phase.0, phase.1),
        }
    }

    fn tile(width: u32, height: u32) -> CoverageMask {
        CoverageMask {
            left: 0,
            top: 0,
            width,
            height,
            coverage: vec![255; (width * height) as usize],
        }
    }

    /// Insert, hit, and the pending upload carries the packed position.
    #[test]
    fn insert_then_hit() {
        let mut atlas = AtlasStore::new(64 * 64, 4096);
        let entry = atlas.insert(key(1, (0, 0)), &tile(10, 12)).expect("fits");
        assert_eq!(
            atlas.get(&key(1, (0, 0))).map(|e| (e.x, e.y)),
            Some((entry.x, entry.y))
        );
        assert!(atlas.get(&key(2, (0, 0))).is_none());
        let uploads = atlas.take_pending();
        assert_eq!(uploads.len(), 1);
        assert_eq!((uploads[0].width, uploads[0].height), (10, 12));
    }

    /// A tile wider than the atlas can never fit; a full atlas refuses without
    /// panicking; a reset clears both entries and layout.
    #[test]
    fn overflow_refuses_and_reset_clears() {
        let mut atlas = AtlasStore::new(16 * 16, 16);
        assert!(atlas.insert(key(1, (0, 0)), &tile(64, 4)).is_none());
        assert!(atlas.insert(key(2, (0, 0)), &tile(16, 16)).is_some());
        assert!(atlas.insert(key(3, (0, 0)), &tile(16, 16)).is_none());
        let generation = atlas.generation;
        atlas.reset();
        assert_eq!(atlas.entry_count(), 0);
        assert!(atlas.generation > generation);
        assert!(atlas.insert(key(3, (0, 0)), &tile(16, 16)).is_some());
    }

    /// **A tile with a zero side is refused rather than packed**, and refusing it is a
    /// `None` the coverage lane already handles by rasterising into the frame's sheet
    /// instead — not a frame that fails.
    ///
    /// A zero extent is what a mark that rounds to no pixel in one axis produces, and
    /// `doc/PLAN.md` states that a blank scene and the zero-length buffer slice that
    /// follows from one are both legitimate. What must not happen is the packing
    /// arithmetic running on it: a zero-width shelf entry would sit at the same cursor as
    /// its neighbour, and a zero-height one would open a shelf that every later tile is
    /// seated inside.
    #[test]
    fn a_tile_with_a_zero_side_is_never_admitted() {
        let mut atlas = AtlasStore::new(64 * 64, 4096);
        assert!(!atlas.admits(0, 12), "zero width is not admitted");
        assert!(!atlas.admits(10, 0), "nor zero height");
        assert!(atlas.insert(key(1, (0, 0)), &tile(0, 12)).is_none());
        assert!(atlas.insert(key(2, (0, 0)), &tile(10, 0)).is_none());
        assert_eq!(atlas.entry_count(), 0, "and neither is stored");
        assert!(
            atlas.take_pending().is_empty(),
            "nor queued for upload — a zero-extent write is a wgpu validation error"
        );
        // The atlas is untouched by the refusals: an ordinary tile still packs at the
        // origin, which it would not if a zero-height shelf had been opened above it.
        let entry = atlas.insert(key(3, (0, 0)), &tile(10, 12)).expect("fits");
        assert_eq!((entry.x, entry.y), (0, 0));
    }

    /// The census's half: a shape the scene places once is not worth an entry, and one
    /// it places again is — which is the whole of ADR 0029's criterion, on an atlas with
    /// room to spare so that nothing else can be the cause.
    #[test]
    fn a_single_placement_is_not_worth_an_entry() {
        let atlas = AtlasStore::new(64 * 1024, 256);
        let placement = GlyphPlacement {
            key: key(1, (0, 0)),
            origin: [0.0, 0.0],
            phase: [0.0, 0.0],
        };
        assert!(
            !atlas.prospect(placement, 16, 16, true).worth_caching(),
            "written once, read once: the cache's whole cost and none of its benefit"
        );
        assert!(
            atlas.prospect(placement, 16, 16, false).worth_caching(),
            "placed more than once, so the second placement reads what the first wrote"
        );
    }
}
