//! How the GPU coverage lane's sheet is cut into what the winding target holds at once.
//!
//! The winding target is scratch: triangles accumulate into it, `fs_resolve` turns it
//! into the R8 sheet, and it is dead. Nothing needs it to hold the whole sheet, so its
//! extent is a *choice*, and this module is that choice — the arithmetic alone, with no
//! device in it, which is why it can be tested by reading numbers rather than pixels.
//!
//! ADR 0027 cut the sheet into horizontal bands. That bounded the target's height and
//! left its width the sheet's, which both lanes share: thirty shapes of 1 200 pixels
//! pack onto shelves of a sheet 15 600 texels wide, and a band of one shelf still asked
//! for 194 MB. ADR 0028 bounds both axes — a [`Pane`] is a rectangle over the lane's own
//! tiles — and a band is what a pane becomes when the sheet is narrower than the budget.
//!
//! # What is bounded, and what is not
//!
//! The target is one texture for the whole frame, so what it costs is the *largest* pane
//! in each axis. Choosing the panes first and measuring them afterwards makes that
//! product an outcome nobody bounded: a tall narrow pane and a short wide one cost their
//! two maxima multiplied, which can exceed either. So [`Plan`] goes the other way round
//! — it fixes the target's extent first, from [`PANE_BYTES`] and the tiles' own sizes,
//! and then cuts panes that fit inside it. The one thing that can push the target past
//! the budget is a single tile larger than it, because a tile is never split.

use crate::outline::WindingVertex;

/// Bytes one resolve instance occupies: the tile rectangle and its fill rule.
pub(crate) const TILE_STRIDE: u64 = 24;

/// Bytes of winding target a frame may hold at once.
///
/// Sixteen mebibytes is two thousand rows of a page-wide sheet, or a single tile of
/// 1 400 × 1 500. It is a target rather than a bound in one direction only: a tile
/// larger than this is a pane of its own, because the accumulate pass cannot draw half
/// a tile's triangles into one target and half into another.
pub(crate) const PANE_BYTES: u64 = 16 * 1024 * 1024;

/// Bytes one texel of the winding target costs: `rgba16float`, four winding numbers.
const TEXEL_BYTES: u64 = 8;

/// [`PANE_BYTES`] in texels, which is the unit the target's extent is chosen in.
const PANE_TEXELS: u64 = PANE_BYTES / TEXEL_BYTES;

/// One tile of coverage the resolve pass turns into bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tile {
    /// The tile in scratch pixels: min x, min y, max x, max y.
    pub rect: [f32; 4],
    /// ISO 32000-2 §8.5.3.3's rule: `false` non-zero, `true` even-odd.
    pub even_odd: bool,
    /// This tile's triangles, as a half-open range of vertices in `Sheet::vertices`.
    ///
    /// The lane appends a tile's vertices in one run, so a pane draws its own tiles'
    /// triangles and no others — which is what makes cutting the sheet finely cost
    /// nothing in vertex work (ADR 0028; under ADR 0027 every band drew every vertex).
    pub first_vertex: u32,
    pub vertex_count: u32,
}

impl Tile {
    /// The tile's bounds as whole texels: min x, min y, max x, max y.
    ///
    /// Sheet coordinates are integers stored as floats — the packer places tiles on
    /// whole texels — so this is exact for every value that reaches it, and rounds
    /// outwards for any that does not.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped below
    fn texels(&self) -> [u32; 4] {
        let low = |value: f32| value.max(0.0).min(f32::from(u16::MAX)) as u32;
        let high = |value: f32| value.ceil().max(0.0).min(f32::from(u16::MAX)) as u32;
        [
            low(self.rect[0]),
            low(self.rect[1]),
            high(self.rect[2]),
            high(self.rect[3]),
        ]
    }
}

/// One rectangle of the sheet the winding target holds at a time, and the run of tiles
/// whose coverage it resolves (ADR 0028).
///
/// Both shader stages learn the origin, and that is the delicate part of the design,
/// because the same agreement in a different form is the caller's `QUORRA_FEEDBACK.md`
/// §11: `vs_winding` subtracts it before mapping to clip space, `fs_winding` adds it
/// back to test a fragment against its tile, and `fs_resolve` subtracts it again when it
/// reads the target. Any one of the three alone draws the right shape in the wrong
/// place, or discards it entirely.
#[derive(Debug, Clone)]
pub(crate) struct Pane {
    /// The pane's top-left corner in sheet texels.
    pub origin: [u32; 2],
    /// Its extent in texels, which is the accumulate pass's viewport. Never larger than
    /// [`Plan::target`], and usually smaller — the last pane of a shelf is as wide as
    /// the tiles it caught.
    pub size: [u32; 2],
    /// The pane's tiles, as a range into [`Plan::order`] — which is the order the
    /// instance buffer takes, so a pane's resolve is one draw of a contiguous range.
    pub first_tile: u32,
    pub tile_count: u32,
    /// The vertices this pane's tiles own, coalesced into as few runs as they fall into.
    ///
    /// One run in the common case: tiles are drawn in the order the encoder met them
    /// and cut into panes in sheet order, and for a page of shapes met left-to-right,
    /// top-to-bottom those two orders agree.
    pub vertex_runs: Vec<core::ops::Range<u32>>,
}

/// The frame's winding work, cut to fit one target.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Tile indices in instance-buffer order: by top row, then by column.
    pub order: Vec<u32>,
    /// The panes that order is cut into, in sheet order.
    pub panes: Vec<Pane>,
    /// The extent the winding target must hold: the pane budget spent on the shape the
    /// frame's tiles need, and at least the largest tile.
    pub target: [u32; 2],
}

impl Plan {
    /// Cuts `tiles` into panes that fit one target no larger than [`PANE_BYTES`] — with
    /// the single exception a tile larger than the budget forces.
    pub(crate) fn new(tiles: &[Tile], sheet: [u32; 2]) -> Self {
        if tiles.is_empty() {
            return Self::default();
        }
        let order = sheet_order(tiles);
        let target = target_extent(tiles, sheet);
        let panes = cut(tiles, &order, target);
        Self {
            order,
            panes,
            target,
        }
    }

    /// Bytes the winding target costs on the device, for the frame budget.
    pub(crate) fn target_bytes(&self) -> u64 {
        u64::from(self.target[0])
            .saturating_mul(u64::from(self.target[1]))
            .saturating_mul(TEXEL_BYTES)
    }
}

/// Tiles by top row, then by column.
///
/// This is the order the shelf packer laid them out in, near enough: shelf-mates share a
/// top row and sort left to right, so a pane's tiles are a contiguous run of it and the
/// panes sweep the sheet the way the tiles were placed.
fn sheet_order(tiles: &[Tile]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..tiles.len())
        // A frame with more tiles than a `u32` counts was refused by the byte budget
        // long before it packed them.
        .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
        .collect();
    order.sort_by(|a, b| {
        let (left, right) = (tiles[*a as usize].texels(), tiles[*b as usize].texels());
        left[1].cmp(&right[1]).then_with(|| left[0].cmp(&right[0]))
    });
    order
}

/// The extent the target takes: the largest tile, then the rest of the budget spent
/// widening it, then heightening it.
///
/// **Width first**, because the sheet is shelf-packed: consecutive tiles in sheet order
/// share a shelf and sit side by side, so width is what lets a pane hold more than one
/// of them. Height is worth spending on only once the panes are as wide as the sheet,
/// and then it recovers ADR 0027's bands exactly — a narrow sheet gets full-width panes
/// several shelves tall, which is what that ADR cut and what its measurements priced.
fn target_extent(tiles: &[Tile], sheet: [u32; 2]) -> [u32; 2] {
    let (mut widest, mut tallest) = (1_u32, 1_u32);
    for tile in tiles {
        let [x0, y0, x1, y1] = tile.texels();
        widest = widest.max(x1.saturating_sub(x0));
        tallest = tallest.max(y1.saturating_sub(y0));
    }
    // The sheet is a bound on both: no tile lies outside it, so no pane needs to.
    let (sheet_width, sheet_height) = (sheet[0].max(widest), sheet[1].max(tallest));
    // How far one axis may go with the other fixed. `budget_for` is the whole of the
    // arithmetic; both uses clamp to the axis's floor, which is the largest tile — so a
    // tile too large for the budget widens the target rather than being cut in half.
    let budget_for = |fixed: u32, floor: u32, limit: u32| -> u32 {
        u32::try_from(
            PANE_TEXELS
                .checked_div(u64::from(fixed.max(1)))
                .unwrap_or(u64::from(u32::MAX)),
        )
        .unwrap_or(u32::MAX)
        .clamp(floor, limit)
    };
    let width = budget_for(tallest, widest, sheet_width);
    let height = if width >= sheet_width {
        budget_for(width, tallest, sheet_height)
    } else {
        tallest
    };
    [width, height]
}

/// Greedy over tiles in sheet order: a pane grows until the next tile would take its
/// bounding box outside `target`, and never splits one.
fn cut(tiles: &[Tile], order: &[u32], target: [u32; 2]) -> Vec<Pane> {
    let mut panes: Vec<Pane> = Vec::new();
    for (position, index) in order.iter().enumerate() {
        let tile = &tiles[*index as usize];
        let [x0, y0, x1, y1] = tile.texels();
        // `position` indexes `order`, whose length is `tiles`', which a `u32` counts.
        let position = u32::try_from(position).unwrap_or(u32::MAX);
        let grown = panes
            .last()
            .and_then(|open| grow(open, [x0, y0, x1, y1], target));
        match (panes.last_mut(), grown) {
            (Some(open), Some((origin, size))) => {
                open.origin = origin;
                open.size = size;
                open.tile_count = open.tile_count.saturating_add(1);
                add_vertices(&mut open.vertex_runs, tile);
            }
            _ => panes.push(Pane {
                origin: [x0, y0],
                size: [x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1)],
                first_tile: position,
                tile_count: 1,
                vertex_runs: vec![vertex_run(tile)],
            }),
        }
    }
    panes
}

/// The pane `open` becomes when it takes one more tile, or `None` when that would not
/// fit the target.
fn grow(open: &Pane, tile: [u32; 4], target: [u32; 2]) -> Option<([u32; 2], [u32; 2])> {
    let [x0, y0, x1, y1] = tile;
    let origin = [open.origin[0].min(x0), open.origin[1].min(y0)];
    let far = [
        open.origin[0].saturating_add(open.size[0]).max(x1),
        open.origin[1].saturating_add(open.size[1]).max(y1),
    ];
    let size = [
        far[0].saturating_sub(origin[0]),
        far[1].saturating_sub(origin[1]),
    ];
    (size[0] <= target[0] && size[1] <= target[1]).then_some((origin, size))
}

/// This tile's vertices as a range of the frame's vertex buffer.
fn vertex_run(tile: &Tile) -> core::ops::Range<u32> {
    tile.first_vertex..tile.first_vertex.saturating_add(tile.vertex_count)
}

/// Adds a tile's vertices to a pane's runs, joining the run it continues.
///
/// Only the last run is examined, which is what makes this O(1) per tile: tiles reach a
/// pane in sheet order, so a run that is not the last is one the encoder emitted out of
/// sheet order, and a separate draw is the right answer for it.
fn add_vertices(runs: &mut Vec<core::ops::Range<u32>>, tile: &Tile) {
    let run = vertex_run(tile);
    match runs.last_mut() {
        Some(last) if last.end == run.start => last.end = run.end,
        _ => runs.push(run),
    }
}

/// The vertices `tile` contributes, as the floats a buffer takes.
///
/// Here rather than at the one caller because the stride is this module's business: a
/// [`Tile`]'s vertex range counts vertices, and only this conversion knows how many
/// floats one is.
pub(crate) fn vertex_floats(vertices: &[WindingVertex], out: &mut Vec<f32>) {
    for vertex in vertices {
        out.extend_from_slice(&vertex.floats());
    }
}

#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // test-file policy as in `raster.rs`: fixtures state whole numbers as the floats a
// sheet coordinate is, and read them back the same way
mod tests {
    use super::{PANE_BYTES, PANE_TEXELS, Plan, Tile};

    /// A tile at `(x, y)` of `(width, height)`, with a vertex run the caller states.
    fn tile(x: u32, y: u32, width: u32, height: u32, first_vertex: u32) -> Tile {
        Tile {
            rect: [x as f32, y as f32, (x + width) as f32, (y + height) as f32],
            even_odd: false,
            first_vertex,
            vertex_count: 3,
        }
    }

    /// Every tile is in exactly one pane, and inside the pane that holds it — the
    /// invariant the two shader stages rely on, checked over a plan rather than argued.
    fn covers_every_tile(plan: &Plan, tiles: &[Tile]) {
        let mut seen = vec![0_u32; tiles.len()];
        for pane in &plan.panes {
            for slot in pane.first_tile..pane.first_tile + pane.tile_count {
                let index = plan.order[slot as usize] as usize;
                seen[index] += 1;
                let [x0, y0, x1, y1] = tiles[index].rect.map(|v| v as u32);
                assert!(
                    x0 >= pane.origin[0]
                        && y0 >= pane.origin[1]
                        && x1 <= pane.origin[0] + pane.size[0]
                        && y1 <= pane.origin[1] + pane.size[1],
                    "tile {index} at {:?} is outside its pane {pane:?}",
                    tiles[index].rect
                );
            }
            assert!(
                pane.size[0] <= plan.target[0] && pane.size[1] <= plan.target[1],
                "pane {pane:?} does not fit the target {:?}",
                plan.target
            );
        }
        assert!(
            seen.iter().all(|count| *count == 1),
            "each tile belongs to exactly one pane: {seen:?}"
        );
    }

    /// ADR 0028's case: thirty shapes of 1 200 device pixels, shelf-packed onto a sheet
    /// thirteen of them wide. Under ADR 0027 the target was a shelf of that sheet —
    /// 15 600 × 1 560 × 8, 194 MB — and the frame was refused against a 256 MiB budget.
    #[test]
    fn a_page_of_very_large_shapes_fits_the_budget() {
        let (width, height) = (1200_u32, 1560_u32);
        let mut tiles = Vec::new();
        for index in 0..30_u32 {
            let (column, row) = (index % 13, index / 13);
            tiles.push(tile(column * width, row * height, width, height, index * 3));
        }
        let sheet = [13 * width, 3 * height];
        let plan = Plan::new(&tiles, sheet);
        assert!(
            plan.target_bytes() <= PANE_BYTES,
            "the target is {} bytes",
            plan.target_bytes()
        );
        covers_every_tile(&plan, &tiles);
    }

    /// A tile larger than the whole budget is a pane of its own, and the target grows to
    /// hold it: a tile is never split, so this is the one way past [`PANE_BYTES`], and it
    /// is stated here rather than discovered on a page.
    #[test]
    fn a_tile_larger_than_the_budget_is_its_own_pane() {
        let tiles = [tile(0, 0, 4000, 4000, 0), tile(4000, 0, 8, 8, 3)];
        let plan = Plan::new(&tiles, [4008, 4000]);
        assert_eq!(
            plan.target,
            [4000, 4000],
            "the target holds the largest tile"
        );
        assert!(plan.target_bytes() > PANE_BYTES);
        assert_eq!(plan.panes.len(), 2, "the small tile cannot join it");
        covers_every_tile(&plan, &tiles);
    }

    /// A narrow sheet gets full-width panes several shelves tall — ADR 0027's band,
    /// which is what a pane becomes when width is not the scarce axis. This is the shape
    /// the lane's measured crossover was taken at, so it is the case that must not have
    /// changed.
    #[test]
    fn a_narrow_sheet_is_banded_as_it_was_before() {
        let tiles: Vec<Tile> = (0..8_u32)
            .map(|index| tile(0, index * 64, 700, 64, index * 3))
            .collect();
        let plan = Plan::new(&tiles, [700, 512]);
        assert_eq!(plan.target[0], 700, "as wide as the sheet");
        assert_eq!(plan.target[1], 512, "and then as tall as the sheet");
        assert_eq!(plan.panes.len(), 1, "one band holds all eight shelves");
        covers_every_tile(&plan, &tiles);
    }

    /// Tiles the encoder emitted in sheet order draw as one range: cutting the sheet
    /// finely must not cost a draw call per tile when the vertices are already in order.
    #[test]
    fn vertices_in_sheet_order_coalesce_into_one_run() {
        let tiles: Vec<Tile> = (0..4_u32)
            .map(|index| tile(index * 300, 0, 300, 300, index * 3))
            .collect();
        let plan = Plan::new(&tiles, [1200, 300]);
        assert_eq!(plan.panes.len(), 1);
        assert_eq!(plan.panes[0].vertex_runs, vec![0..12]);

        // And out-of-order emission costs one run each, rather than one wrong range.
        let reversed: Vec<Tile> = (0..4_u32)
            .map(|index| tile(index * 300, 0, 300, 300, (3 - index) * 3))
            .collect();
        let plan = Plan::new(&reversed, [1200, 300]);
        assert_eq!(plan.panes[0].vertex_runs, vec![9..12, 6..9, 3..6, 0..3]);
    }

    /// The empty frame: no tiles, no panes, no target, nothing charged.
    #[test]
    fn a_frame_with_no_tiles_plans_nothing() {
        let plan = Plan::new(&[], [16384, 8760]);
        assert!(plan.panes.is_empty());
        assert_eq!(plan.target_bytes(), 0);
    }

    /// The budget is spent on width before height, and both stay inside it.
    #[test]
    fn the_target_spends_its_budget_on_width_first() {
        let tiles: Vec<Tile> = (0..40_u32)
            .map(|index| tile(index * 700, 0, 700, 910, index * 3))
            .collect();
        let plan = Plan::new(&tiles, [40 * 700, 910]);
        assert_eq!(plan.target[1], 910, "height is the tallest tile, no more");
        assert_eq!(
            plan.target[0],
            (PANE_TEXELS / 910) as u32,
            "width takes the rest of the budget"
        );
        assert!(plan.target_bytes() <= PANE_BYTES);
        covers_every_tile(&plan, &tiles);
    }
}
