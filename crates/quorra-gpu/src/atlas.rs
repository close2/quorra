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
//! When a frame's tiles no longer fit, the atlas resets and repacks what *this* frame
//! needs; tiles that still do not fit fall through to the scratch path (drawn
//! uncached, correctly) rather than failing the frame. `Counters` reports
//! `atlas_distinct_keys` — the count of distinct keys a frame asked for, deliberately
//! not a hit rate (§6.3's lesson: a hit rate describes the lookups you made, never
//! the ones you should have made).

use crate::keyhash::FastMap;
use crate::raster::{CoverageMask, Rule};

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
    /// Bumped on every reset; the device recreates its bind group when it changes.
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

    /// Insert a rasterised tile under its key. `None` when the tile cannot fit even
    /// in an empty atlas — the caller then draws it through the scratch path instead.
    ///
    /// On a full atlas this does **not** evict piecemeal: it fails, and the encoder
    /// resets and repacks the frame's working set (`reset`), which keeps the packing
    /// deterministic — the same scene always produces the same atlas layout (§4.6).
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
    use super::{AtlasStore, GlyphKey, PhaseKey};
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
}
