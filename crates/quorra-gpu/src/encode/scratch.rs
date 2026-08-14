//! The frame's scratch coverage sheet: every uncached coverage tile, shelf-packed
//! into one growing R8 image that is uploaded once.
//!
//! One door onto the sheet, [`ScratchPacker::reserve`], and two producers behind it —
//! the CPU lane copies its bytes in, the GPU lane hands the position to a pass that
//! draws there — which is what lets one sheet hold both kinds of tile without either
//! knowing the other exists (ADR 0016).
//!
//! The sheet is sized by what was placed on it rather than by a constant, in two
//! steps that pull the same way. While packing, every shelf is kept near one target
//! width, because a sheet is a rectangle and pays its widest shelf for every row
//! (ADR 0034). At [`ScratchPacker::finish`], the packed rows are restrided down to the
//! width the shelves actually reached (ADR 0021) — and that restride is where this
//! module's one defect lived: a row that moves left leaves the wide layout's bytes
//! behind it, and growing the buffer to the sheet's extent without cutting that tail
//! first drew 136 410 texels of another shape's coverage across a page (the caller's
//! `QUORRA_FEEDBACK.md` §20.4.1).

use crate::raster;

/// The frame's scratch coverage image: every uncached mask, shelf-packed into one
/// R8 upload.
#[derive(Debug)]
pub(crate) struct Scratch {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// The frame's scratch shelf packer: coverage tiles packed into one growing R8
/// image, uploaded once.
pub(super) struct ScratchPacker {
    width: u32,
    pub(super) max_height: u32,
    shelves: Vec<(u32, u32, u32)>, // (y, height, cursor)
    next_y: u32,
    data: Vec<u8>,
    /// Tiles placed on the sheet, for `Counters::tiles` — the count both lanes feed,
    /// since `reserve` is the one door onto the sheet.
    pub(super) placed: u32,
    /// Tile texels placed so far, and the widest tile seen — the two inputs to
    /// [`ScratchPacker::shelf_target`].
    tile_area: u64,
    widest: u32,
}

impl ScratchPacker {
    pub(super) fn new(width: u32, max_height: u32) -> Self {
        Self {
            width,
            max_height,
            shelves: Vec::new(),
            next_y: 0,
            data: Vec::new(),
            placed: 0,
            tile_area: 0,
            widest: 0,
        }
    }

    /// How wide the packer lets a shelf grow *this* tile, which is not the same as the
    /// width it may commit (ADR 0034).
    ///
    /// The sheet is a rectangle, so it costs the widest shelf's cursor times the sum of
    /// every shelf's height — and a run that fills one shelf to 6 026 while three others
    /// hold a single 2 100-wide tile pays 3 900 empty columns three times over. That is
    /// half of `issue16287.pdf`'s sheet at 4×, and the page refuses 4 % over the frame
    /// budget.
    ///
    /// Keeping every shelf near one target equalises them, and the target that minimises
    /// `w × h` for a shelf packing of area `A` is about `√(2A)` — the square-ish sheet.
    /// Online, `A` is what has been placed so far, so the target grows with the frame and
    /// a shelf packed under an earlier, smaller one is never invalidated: a cursor that
    /// has stopped growing only leaves room a later tile may still take.
    fn shelf_target(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation)] // isqrt of an area bounded by the
        // device dimension squared, which is inside u32 after the root
        let square = self.tile_area.saturating_mul(2).isqrt() as u32;
        square.max(self.widest).clamp(1, self.width)
    }

    /// Reserve a tile's place on the sheet, without writing anything into it.
    ///
    /// The shelf arithmetic, alone. Both producers go through it — the CPU lane then
    /// copies its bytes in, the GPU lane hands the position to a pass that draws there
    /// — which is what lets one sheet hold both kinds of tile without either knowing
    /// the other exists (ADR 0016).
    #[allow(clippy::arithmetic_side_effects)] // bounded by width/max_height checks
    pub(super) fn reserve(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width == 0 || height == 0 || width > self.width {
            return None;
        }
        self.widest = self.widest.max(width);
        let target = self.shelf_target();
        self.placed = self.placed.saturating_add(1);
        self.tile_area = self
            .tile_area
            .saturating_add(u64::from(width).saturating_mul(u64::from(height)));
        self.shelves
            .iter_mut()
            .find(|(_, shelf_height, cursor)| {
                *shelf_height >= height
                    && *shelf_height <= height.saturating_mul(2)
                    && cursor + width <= target
            })
            .map(|(y, _, cursor)| {
                let position = (*cursor, *y);
                *cursor += width;
                position
            })
            .or_else(|| {
                if self.next_y + height <= self.max_height {
                    let y = self.next_y;
                    self.next_y += height;
                    self.shelves.push((y, height, width));
                    Some((0, y))
                } else {
                    None
                }
            })
    }

    /// Pack a mask's bytes; `None` when the frame's scratch would outgrow the
    /// texture dimension limit (the byte budget was already charged by the caller).
    #[allow(clippy::arithmetic_side_effects)] // bounded by width/max_height checks
    pub(super) fn pack(&mut self, mask: &raster::CoverageMask) -> Option<(u32, u32)> {
        let position = self.reserve(mask.width, mask.height)?;
        let row_bytes = self.width as usize;
        let needed_rows = (position.1 + mask.height) as usize;
        if self.data.len() < needed_rows * row_bytes {
            self.data.resize(needed_rows * row_bytes, 0);
        }
        for row in 0..mask.height as usize {
            let src = &mask.coverage[row * mask.width as usize..(row + 1) * mask.width as usize];
            let dst_start = (position.1 as usize + row) * row_bytes + position.0 as usize;
            self.data[dst_start..dst_start + mask.width as usize].copy_from_slice(src);
        }
        Some(position)
    }

    /// The packed sheet, or `None` when nothing was placed on it.
    ///
    /// **Narrowed to the width the shelves actually reached** (ADR 0021). The packing
    /// width is the device's maximum dimension because a narrow one refuses real pages
    /// (the caller's feedback §3), but every tile sits left of the widest shelf cursor,
    /// so everything to the right of it is a texture nobody wrote and a texture nobody
    /// reads — 16 384 texels of it on this machine. Narrowing moves no tile: the
    /// coordinates the lanes recorded are all inside the kept region.
    ///
    /// `data` is padded to the whole sheet when the CPU lane wrote any of it, and left
    /// **empty** when every tile on the sheet is the GPU lane's — the device then
    /// clears rather than uploads, and the distinction is the one bit it needs.
    pub(super) fn finish(mut self) -> Option<Scratch> {
        if self.next_y == 0 {
            return None;
        }
        let used = self
            .shelves
            .iter()
            .map(|(_, _, cursor)| *cursor)
            .max()
            .unwrap_or(self.width)
            .clamp(1, self.width);
        if !self.data.is_empty() {
            // Rows the CPU lane actually wrote, counted in the *packing* stride before
            // compaction restrides them.
            let written = self
                .data
                .len()
                .checked_div(self.width as usize)
                .unwrap_or(0);
            self.compact_rows(used);
            // **Cut the tail before growing the sheet.** Compaction moves each row left
            // and leaves the old wide layout's bytes behind it, so the buffer is still
            // as long as it was and everything past `written × used` is stale coverage
            // rather than blank sheet. Resizing straight to the sheet's extent keeps
            // whatever of that tail happens to fall inside it — which is nothing at all
            // while every shelf holds CPU tiles (they write their own rows), and is a
            // page of somebody else's marks the moment a shelf below them belongs to a
            // lane that writes its rows on the device instead. The caller's
            // `transparency_group.pdf` drew 136 410 texels of another shape's coverage
            // that way, in horizontal streaks across the rows under its last CPU tile.
            self.data.truncate(written.saturating_mul(used as usize));
            // Both extents are bounded by the device dimension, so the product is far
            // inside a `usize`; saturating says so rather than relying on it.
            self.data
                .resize((used as usize).saturating_mul(self.next_y as usize), 0);
        }
        Some(Scratch {
            width: used,
            height: self.next_y,
            data: self.data,
        })
    }

    /// Restride the written rows from the packing width down to `used`, in place.
    ///
    /// Rows move left and never right, and row `r`'s destination start is always below
    /// its source start, so a forward copy cannot overwrite bytes it has yet to read.
    fn compact_rows(&mut self, used: u32) {
        if used >= self.width {
            return;
        }
        let (from, to) = (self.width as usize, used as usize);
        // `from` is the packing width, which `new` takes from the device dimension and
        // `reserve` refuses to place anything into when it is zero.
        let rows = self.data.len().checked_div(from).unwrap_or(0);
        for row in 1..rows {
            let source = row.saturating_mul(from);
            let destination = row.saturating_mul(to);
            self.data
                .copy_within(source..source.saturating_add(to), destination);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScratchPacker;

    /// **A shelf no CPU tile wrote is blank sheet, not the tail of the wide layout.**
    ///
    /// The packer lays rows out at the *packing* width and `finish` restrides them down
    /// to the width the shelves reached (ADR 0021). Compaction moves each row left and
    /// leaves the old bytes behind it, so growing the buffer to the sheet's extent
    /// straight afterwards keeps whatever of that tail falls inside — stale coverage,
    /// not blank sheet.
    ///
    /// Invisible while every shelf holds CPU tiles, because each writes its own rows.
    /// The GPU lane reserves rows it fills on the device, so its shelf is exactly the
    /// region that reads back whatever was left there: the caller's
    /// `transparency_group.pdf` drew 136 410 texels of another shape's coverage in
    /// horizontal streaks under its last CPU tile (`QUORRA_FEEDBACK.md` §20.4.1).
    #[test]
    fn a_shelf_the_cpu_lane_did_not_write_is_blank() {
        let mut packer = ScratchPacker::new(64, 64);
        let mask = |width: u32, height: u32| crate::raster::CoverageMask {
            left: 0,
            top: 0,
            width,
            height,
            coverage: vec![255; (width * height) as usize],
        };
        // Two tiles on one shelf: 16 of the 64 packing columns are used, so `finish`
        // restrides — which is the precondition for the tail to exist at all.
        assert!(packer.pack(&mask(8, 20)).is_some());
        assert!(packer.pack(&mask(8, 20)).is_some());
        // And a shelf below them that reserves rows without writing bytes, which is
        // what every GPU-lane tile does. Short enough that the shelf rule opens a new
        // one for it rather than seating it beside the two above.
        assert_eq!(packer.reserve(8, 4), Some((0, 20)));

        let scratch = packer.finish().expect("the sheet holds three tiles");
        assert_eq!((scratch.width, scratch.height), (16, 24));
        let stray = scratch.data[(16 * 20) as usize..]
            .iter()
            .filter(|byte| **byte != 0)
            .count();
        assert_eq!(stray, 0, "{stray} texels of somebody else's coverage");
    }

    /// **Shelves stay near one width, so the sheet stays near square** (ADR 0034).
    ///
    /// Six tiles of roughly 2 000 × 500 — `issue16287.pdf` at 4×, which is where this was
    /// found. Filling one shelf to its full width and leaving the rest holding a single
    /// tile each costs the sheet three times over, because a sheet is a rectangle and
    /// pays its widest shelf for every row. The property is stated as occupancy rather
    /// than as a layout, so a better packer passes it too — and as *no overlap*, because
    /// a packer that reached 95 % by putting two tiles in one place would satisfy the
    /// first half alone.
    #[test]
    fn the_sheet_stays_near_square_and_its_tiles_never_overlap() {
        let sizes = [
            (2042_u32, 541_u32),
            (2038, 490),
            (2115, 566),
            (2172, 624),
            (2224, 675),
            (1946, 397),
        ];
        let mut packer = ScratchPacker::new(16384, 16384);
        let mut placed = Vec::new();
        let mut area = 0_u64;
        for (width, height) in sizes {
            let at = packer.reserve(width, height).expect("the sheet has room");
            placed.push((at.0, at.1, width, height));
            area += u64::from(width) * u64::from(height);
        }
        for (i, a) in placed.iter().enumerate() {
            for b in placed.iter().skip(i + 1) {
                let apart =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(apart, "tiles {a:?} and {b:?} overlap");
            }
        }
        let scratch = packer.finish().expect("six tiles");
        let sheet = u64::from(scratch.width) * u64::from(scratch.height);
        let occupancy = 100 * area / sheet;
        assert!(
            occupancy >= 90,
            "the sheet is {}x{} for {area} texels of tiles — {occupancy}% used, and the \
             shelf-per-tile layout this replaced was 48%",
            scratch.width,
            scratch.height
        );
    }
}
