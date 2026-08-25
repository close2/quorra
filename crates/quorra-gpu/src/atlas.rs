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
//! holding all of them would cost, `atlas_overflow_tiles` for the marks that wanted an
//! entry and were drawn uncached instead, and `atlas_repacked` for the event that moves
//! them. The first two answer *how large is this page*; the third answers *what did this
//! frame pay*, and it took ADR 0063's corpus measurement to notice that no counter did
//! (`Limits::atlas_bytes` is the fourth, and says what the budget actually bought).
//!
//! # What the corpus says this holds and this loses (ADR 0063)
//!
//! Page one of 974 documents at 4×, one device throughout, `Coverage::Cpu`: **no page's
//! working set exceeds 4.10 MiB** against the 8 MiB default, and 74 820 marks on 19 of
//! 948 pages were still drawn uncached — every one of them because the sheet was full of
//! **earlier pages'** tiles, and none because a page was too large for it. `admits`
//! refused 40 marks in the whole corpus. So the cache's cost here is *accumulation*, the
//! repack after the frame is what clears it, and the budget is not what ran out.

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
        let (mut ix, fx) = (to_device.e.floor(), to_device.e - to_device.e.floor());
        let (mut iy, fy) = (to_device.f.floor(), to_device.f - to_device.f.floor());
        let (phase, px, py) = match quantum {
            Some(q) => {
                let fq = f32::from(q);
                // **`round` reaches `q` itself, and that is a carry rather than a wrap.**
                // A fraction within half a quantum of the next pixel — `fx ≥ 1 − 1/2q`,
                // which is 3.1 % of phases at the default quantum of 16 — rounds to the
                // *next pixel's* phase zero. Taking `% q` and leaving the integer part
                // alone drew such a mark a whole device pixel low: the tile was
                // rasterised at phase 0 and seated at `floor(e)`, where the placement
                // asked for `floor(e) + 1`. It is the one input the quantum is not a
                // bound for, so it must be added to the origin instead of discarded
                // (ADR 0073; found by `examples/lane_placement.rs`).
                let mut nx = (fx * fq).round() as u16;
                let mut ny = (fy * fq).round() as u16;
                if nx == q {
                    nx = 0;
                    ix += 1.0;
                }
                if ny == q {
                    ny = 0;
                    iy += 1.0;
                }
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
    /// page shape tried. The ADR records the numbers.
    ///
    /// ADR 0029's reason for that — "the census keeps single-use tiles out of the atlas,
    /// so a full atlas is one holding tiles that are being reused" — **holds on
    /// [`Coverage::Gpu`](crate::startup::Coverage::Gpu) and nowhere else**, because
    /// [`worth_caching`](CacheProspect::worth_caching) is read by `take_gpu_lane` alone
    /// and the census is not even taken under `Coverage::Cpu`. On the caller's default
    /// lane a full atlas is one holding **earlier pages'** tiles, 98.6 % of whose keys
    /// were placed exactly once (ADR 0063, ADR 0065). The measured conclusion is
    /// unchanged — a room test still buys nothing — but it is now the same conclusion for
    /// a different reason on each lane, and ADR 0065 is why filtering the other one is
    /// refused rather than merely unimplemented.
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
    ///
    /// **The question is about *this frame*, and that only answers the lane choice where
    /// a faster single-use lane exists.** Under `Coverage::Gpu` a `false` sends the tile
    /// to the device, which ADR 0029 measured at two to three times the scratch path for
    /// one use. Under `Coverage::Cpu` there is no such lane — the alternative is the same
    /// rasteriser writing to the sheet instead of the atlas, no faster now and with no
    /// entry on the next frame — so a `false` there would convert a one-off cost into a
    /// per-frame one. That asymmetry, not the census's 25 µs, is why this is consulted on
    /// one lane; ADR 0065 measured both sides of it.
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

/// A contiguous range of atlas rows the texture has not seen yet: `start..end`,
/// full-width, exclusive at the end.
///
/// Rows rather than rectangles, because the flush reads the [`AtlasStore`]'s own sheet
/// at the sheet's stride — a full-width span uploads as one borrowed slice, where a
/// tighter rectangle would need its rows repacked into a buffer of their own. The
/// texels a span carries beyond its tiles are bytes the sheet already holds (zero, or
/// earlier tiles being restated), so the width costs bandwidth only, and bandwidth is
/// not what the per-call price was made of (ADR 0078).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirtyRows {
    pub start: u32,
    pub end: u32,
}

/// The CPU-side state of the atlas: the packer, the key table, the sheet — a CPU
/// mirror of the texture's texels — and the rows the next flush owes the texture. The
/// `wgpu` texture itself lives on the device, which creates it lazily on the first
/// frame that needs a glyph (startup rule, §7).
///
/// **The sheet is the price of a batched flush, and it is one atlas of bytes.** Every
/// tile ever inserted is written into it, so any row range of it is uploadable at any
/// time — which is what lets a frame that packed fifty-eight thousand tiles hand the
/// texture one `write_texture` instead of fifty-eight thousand (ADR 0078). Allocated
/// on the first insert, never on the startup path, and bounded by the same
/// `2048 × max_dimension` cap as the texture it mirrors.
#[derive(Debug)]
pub(crate) struct AtlasStore {
    width: u32,
    height: u32,
    shelves: Vec<Shelf>,
    next_shelf_y: u32,
    entries: FastMap<GlyphKey, AtlasEntry>,
    /// Row-major R8 texels, `width × height`, allocated on first insert.
    sheet: Vec<u8>,
    /// Disjoint, sorted, coalesced-when-touching. A cold frame appends shelves
    /// contiguously, so its spans merge to one; a frame reusing scattered shelves
    /// keeps one span per cluster rather than one per tile.
    dirty: Vec<DirtyRows>,
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
    ///
    /// **The budget is a request and this is where it stops being one.** Two caps sit
    /// between them and neither is a function of the number the caller passed: the width
    /// never exceeds 2048, and neither side exceeds `max_dimension`, so no atlas can be
    /// larger than `2048 × max_dimension` however large the budget — 32 MiB on an adapter
    /// allowing 16 384 texels a side. A caller asking for more gets part of it and gets no
    /// error, which is legitimate (an atlas is a cache; a smaller one draws the same
    /// pixels) but was invisible until ADR 0063: [`byte_size`](AtlasStore::byte_size) is
    /// reported as `Limits::atlas_bytes` so the caller can compare
    /// `Counters::atlas_working_set_bytes` against what exists rather than against what
    /// was asked for.
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
            sheet: Vec::new(),
            dirty: Vec::new(),
            generation: 0,
        }
    }

    pub(crate) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The texture's area in bytes — an R8 texel is one — which is what the budget bought
    /// rather than what it asked for ([`AtlasStore::new`]).
    pub(crate) fn byte_size(&self) -> u64 {
        u64::from(self.width).saturating_mul(u64::from(self.height))
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

    /// Take the row spans owed to the texture (the device flushes them each frame).
    pub(crate) fn take_dirty(&mut self) -> Vec<DirtyRows> {
        std::mem::take(&mut self.dirty)
    }

    /// The sheet's texels for one span of rows, at the sheet's own stride — exactly
    /// the slice a full-width `write_texture` takes, borrowed rather than built.
    ///
    /// # Panics
    ///
    /// On a span outside the sheet, which no caller can construct: spans come from
    /// [`AtlasStore::take_dirty`] and were clamped by the insert that made them.
    #[allow(clippy::arithmetic_side_effects)] // `span` came from `mark_dirty`, whose
    // rows were clamped by the allocation, so `end × width` is at most the sheet's own
    // length — and a `u32 as usize` product cannot overflow a 64-bit usize at all
    pub(crate) fn rows(&self, span: DirtyRows) -> &[u8] {
        let start = span.start as usize * self.width as usize;
        let end = span.end as usize * self.width as usize;
        &self.sheet[start..end]
    }

    /// Record `start..end` as rows the texture has not seen, coalescing with any span
    /// it touches. Insertion order is packer order, so in practice a new span extends
    /// the last one; the general merge is kept because a frame that fills scattered
    /// shelves genuinely produces scattered spans.
    fn mark_dirty(&mut self, start: u32, end: u32) {
        let mut merged = DirtyRows { start, end };
        // Keep every span that does not touch the new one; fold the rest in.
        self.dirty.retain(|span| {
            if span.end < merged.start || span.start > merged.end {
                true
            } else {
                merged.start = merged.start.min(span.start);
                merged.end = merged.end.max(span.end);
                false
            }
        });
        let at = self
            .dirty
            .partition_point(|span| span.start < merged.start);
        self.dirty.insert(at, merged);
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
    #[allow(clippy::arithmetic_side_effects)] // row arithmetic is bounded by the
    // allocation: `allocate` returned a position, so `x + width ≤ self.width` and
    // `y + height ≤ self.height`, and the sheet is `width × height` bytes
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
        // First insert: the mirror the flush reads from. Not in `new`, so the startup
        // path never pays for it (§7); zeroed, which is what the texture's texels are
        // before anything names them.
        if self.sheet.is_empty() {
            self.sheet = vec![0; self.width as usize * self.height as usize];
        }
        let stride = self.width as usize;
        for row in 0..mask.height as usize {
            let to = (y as usize + row) * stride + x as usize;
            let from = row * mask.width as usize;
            self.sheet[to..to + mask.width as usize]
                .copy_from_slice(&mask.coverage[from..from + mask.width as usize]);
        }
        self.mark_dirty(y, y + mask.height);
        Some(entry)
    }

    /// Drop every entry and start packing afresh. Dirty spans die with the layout
    /// they were packed for, and the sheet is zeroed rather than freed — the repack
    /// that called this is about to refill it.
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
        self.sheet.fill(0);
        self.dirty.clear();
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
    use crate::raster::{CoverageMask, DeviceTransform, Rule};

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

    /// A placement of outline 1 at a device translation, through the default quantum.
    fn placed(e: f32, f: f32, quantum: Option<u16>) -> GlyphPlacement {
        GlyphPlacement::of(
            quorra_scene::OutlineId(1),
            &DeviceTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e,
                f,
            },
            Rule::NonZero,
            quantum,
        )
    }

    /// Where the placement says the mark's own origin lands, which is the sum the tile is
    /// seated at: the integer origin plus the phase it was rasterised for.
    fn lands_at(placement: &GlyphPlacement) -> (f32, f32) {
        (
            placement.origin[0] + placement.phase[0],
            placement.origin[1] + placement.phase[1],
        )
    }

    /// **The quantum is a bound on where a mark lands, and this is that bound.**
    ///
    /// `Options::glyph_quantum` rounds a placement's fractional device offset to `1/q` of
    /// a pixel so that repeats of one outline share a rasterisation (ADR 0009, §4.5's
    /// fifth decision). Rounding to the nearest of `q` buckets moves a mark by at most
    /// half a bucket, so the whole of what the setting costs in *position* is
    /// `1/2q` — 1/32 of a device pixel at the default 16, in each axis independently.
    #[test]
    fn a_quantised_phase_moves_a_mark_by_at_most_half_a_quantum() {
        for step in 0_u16..=512 {
            let offset = f32::from(step) / 512.0;
            let (x, y) = lands_at(&placed(20.0 + offset, 40.0 + offset, Some(16)));
            for (got, want) in [(x, 20.0 + offset), (y, 40.0 + offset)] {
                assert!(
                    (got - want).abs() <= 1.0 / 32.0,
                    "a placement at {want} landed at {got}, past half of a 1/16 quantum"
                );
            }
        }
    }

    /// **A fraction within half a quantum of the next pixel carries, and does not wrap.**
    ///
    /// The regression this file had no test for until 2026-08-22: `(fx * q).round()`
    /// reaches `q` for any `fx ≥ 1 − 1/2q`, and taking `% q` of it mapped such a placement
    /// to phase zero of the *same* pixel instead of phase zero of the next one — a whole
    /// device pixel, on 3.1 % of phases per axis, on the lane that draws text. Stated as
    /// the two halves it is made of, so that a future `%` cannot pass one of them.
    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "every value compared here is exact by construction: a phase of zero, an \
                  integer origin, and 0.875 = 14/16, none of which is the result of an \
                  approximation"
    )]
    fn a_phase_that_rounds_to_the_next_pixel_carries_into_the_origin() {
        let placement = placed(20.99, 40.99, Some(16));
        assert_eq!(
            placement.phase,
            [0.0, 0.0],
            "it is the next pixel's phase 0"
        );
        assert_eq!(
            placement.origin,
            [21.0, 41.0],
            "and the next pixel's origin"
        );
        assert_eq!(placement.key.phase, PhaseKey::Quantised(0, 0));
        // The bucket below it does not carry: 0.9 × 16 = 14.4, which rounds to 14.
        let below = placed(20.9, 40.9, Some(16));
        assert_eq!(below.origin, [20.0, 40.0]);
        assert_eq!(below.phase, [0.875, 0.875]);
    }

    /// An exact-phase placement is exact: no quantum, no carry, no bound to state.
    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "an unquantised placement is exact: that is the whole claim, so an \
                  approximate comparison would test something weaker than it states"
    )]
    fn an_unquantised_placement_lands_where_it_was_asked_to() {
        let placement = placed(20.99, 40.99, None);
        assert_eq!(lands_at(&placement), (20.99, 40.99));
        assert!(matches!(placement.key.phase, PhaseKey::Exact(_, _)));
    }

    /// Insert, hit, and the dirty rows carry the packed position's span, with the
    /// tile's bytes readable from the sheet at the sheet's stride.
    #[test]
    fn insert_then_hit() {
        let mut atlas = AtlasStore::new(64 * 64, 4096);
        let entry = atlas.insert(key(1, (0, 0)), &tile(10, 12)).expect("fits");
        assert_eq!(
            atlas.get(&key(1, (0, 0))).map(|e| (e.x, e.y)),
            Some((entry.x, entry.y))
        );
        assert!(atlas.get(&key(2, (0, 0))).is_none());
        let spans = atlas.take_dirty();
        assert_eq!(spans, vec![super::DirtyRows { start: 0, end: 12 }]);
        let rows = atlas.rows(spans[0]);
        let (width, _) = atlas.dimensions();
        assert_eq!(rows.len(), width as usize * 12);
        assert_eq!(rows[0], 255, "the tile's first texel is in the sheet");
        assert_eq!(rows[10], 0, "and the texel beside it is the sheet's zero");
    }

    /// **Adjacent spans coalesce and a taken span is owed once** (ADR 0078): a cold
    /// frame's appended shelves flush as one `write_texture`, and a later insert dirties
    /// only its own rows — while the sheet still holds every earlier tile, which is what
    /// makes restating a shared row correct.
    #[test]
    fn dirty_spans_coalesce_and_die_when_taken() {
        let mut atlas = AtlasStore::new(64 * 64, 4096);
        atlas.insert(key(1, (0, 0)), &tile(10, 12)).expect("fits");
        atlas.insert(key(2, (0, 0)), &tile(10, 12)).expect("fits");
        atlas.insert(key(3, (0, 0)), &tile(64, 20)).expect("fits");
        assert_eq!(
            atlas.take_dirty(),
            vec![super::DirtyRows { start: 0, end: 32 }],
            "one shelf of 12 and one of 20, packed contiguously, owed as one span"
        );
        assert!(atlas.take_dirty().is_empty(), "taken is taken");
        atlas.insert(key(4, (0, 0)), &tile(10, 12)).expect("fits");
        let spans = atlas.take_dirty();
        assert_eq!(
            spans,
            vec![super::DirtyRows { start: 0, end: 12 }],
            "a tile seated in the first shelf owes that shelf's rows again"
        );
        // The restated rows carry the first tile too: tile 1 sits at x 0, tile 2 at
        // x 10, and both are 255 wherever they are — the sheet is a mirror, not a
        // frame's transient.
        assert_eq!(atlas.rows(spans[0])[15], 255);
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
            atlas.take_dirty().is_empty(),
            "nor any rows owed — a zero-extent write is a wgpu validation error"
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
