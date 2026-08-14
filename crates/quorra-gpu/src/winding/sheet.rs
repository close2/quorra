//! What the encoder hands the lane: the triangles, the tiles that own them, and what
//! the whole of it will cost on the device.
//!
//! No device and no pass — a [`Sheet`] is built while the encoder walks the scene, on
//! whatever thread the encode runs on, and is priced *before* anything is allocated
//! because a buffer sized from document-derived arithmetic is what principle 3 says to
//! check first. That pricing is the reason this is its own module: the condition under
//! which a sheet costs nothing has to be the same condition under which nothing is
//! allocated, and saying it once is what stops the pre-flight and the allocation from
//! disagreeing.

use crate::outline::WindingVertex;
use crate::pane::{Plan, TILE_STRIDE, Tile, vertex_floats};

/// What the encoder built for the GPU lane this frame.
#[derive(Debug, Default)]
pub(crate) struct Sheet {
    /// Every triangle of every tile, in sheet space, as the vertex buffer's floats.
    pub vertices: Vec<f32>,
    /// One entry per packed tile, each owning a run of `vertices`.
    pub tiles: Vec<Tile>,
    /// The scratch sheet's size in pixels.
    pub width: u32,
    pub height: u32,
}

impl Sheet {
    /// Adds a tile and the triangles that fill it.
    ///
    /// The tile records where its vertices went, which is what lets a pane draw its own
    /// tiles' triangles and nobody else's (`crate::pane`).
    pub(crate) fn push_tile(&mut self, rect: [f32; 4], even_odd: bool, vertices: &[WindingVertex]) {
        // A tile with no triangles is not a tile: it would resolve to transparent,
        // which the sheet already is there.
        if vertices.is_empty() {
            return;
        }
        // A frame with 2^32 vertices was refused by the byte budget long before it
        // reached this cast, and saturating keeps the ranges monotonic if one ever does.
        let first_vertex =
            u32::try_from(self.vertices.len() / WindingVertex::FLOATS).unwrap_or(u32::MAX);
        vertex_floats(vertices, &mut self.vertices);
        self.tiles.push(Tile {
            rect,
            even_odd,
            first_vertex,
            vertex_count: u32::try_from(vertices.len()).unwrap_or(u32::MAX),
        });
    }

    /// How this frame's tiles are cut into what the winding target holds at once.
    pub(crate) fn plan(&self) -> Plan {
        Plan::new(&self.tiles, [self.width, self.height])
    }

    /// Whether this frame drew anything through the GPU lane.
    pub(crate) fn is_empty(&self) -> bool {
        self.tiles.is_empty() || self.vertices.is_empty()
    }

    /// Bytes this sheet costs on the device, for the frame budget: the winding texture
    /// plus the vertex and instance buffers. Counted before anything is allocated,
    /// because a buffer sized from document-derived arithmetic is exactly what
    /// principle 3 says to check first.
    ///
    /// **A sheet with no tiles costs nothing, whatever extent it carries**, and that
    /// condition lives here rather than at either caller. The extent is the *scratch*
    /// sheet's, which both lanes share: `width` and `height` are filled in from it on
    /// every frame that packs a tile, including one the GPU lane never ran. Pricing
    /// the winding texture from that extent charges a CPU-lane frame for a texture
    /// [`render_into`](super::render_into) is never asked to make — the frame is refused
    /// for bytes nobody would have allocated, which is principle 6's failure with the
    /// sign flipped: a page that draws, refused. Five real corpus pages were, at up to
    /// 1.2 GB claimed against a 256 MiB budget for an empty sheet 16 384 texels wide.
    #[allow(clippy::cast_possible_truncation)] // lengths of Vecs this frame just built
    pub(crate) fn device_bytes(&self) -> u64 {
        // Not merely an optimisation of the arithmetic below: `is_empty` is exactly the
        // condition `Device::upload_scratch` allocates under, and saying it once is what
        // stops the pre-flight and the allocation from disagreeing again.
        if self.is_empty() {
            return 0;
        }
        // Saturating throughout: the number this returns is *checked against* a budget,
        // so a sheet too large to size must come back too large rather than wrap to
        // something affordable. That is principle 3's rule about allocations derived
        // from scene content, applied to the arithmetic that describes them.
        // The winding target holds one *pane*, not the sheet (ADR 0028).
        let winding = self.plan().target_bytes();
        let vertices = (self.vertices.len() as u64).saturating_mul(4);
        let tiles = (self.tiles.len() as u64).saturating_mul(TILE_STRIDE);
        winding.saturating_add(vertices).saturating_add(tiles)
    }
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
// test-file policy as in `raster.rs`: a fixture that cannot run must fail loudly
mod tests {
    use super::{Sheet, TILE_STRIDE};
    use crate::outline::QuadOutline;
    use quorra_scene::{Point, Segment};

    fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Segment> {
        vec![
            Segment::MoveTo(Point::new(x0, y0)),
            Segment::LineTo(Point::new(x1, y0)),
            Segment::LineTo(Point::new(x1, y1)),
            Segment::LineTo(Point::new(x0, y1)),
            Segment::Close,
        ]
    }

    /// A sheet with no tiles costs nothing, however large the extent it carries.
    ///
    /// The extent is the scratch sheet's and arrives on every frame that packs a tile,
    /// so this is the ordinary shape of a CPU-lane frame — not an edge case. Charging
    /// it `width × height × 8` for an `rgba16float` texture nothing asks for is what
    /// refused five real pages; `tests/coverage_lanes.rs` holds the same invariant end
    /// to end.
    #[test]
    fn a_sheet_with_no_tiles_costs_nothing_however_large_its_extent() {
        let empty = Sheet {
            width: 16384,
            height: 8760,
            ..Sheet::default()
        };
        assert!(empty.is_empty());
        assert_eq!(empty.device_bytes(), 0);

        // And the target is priced the moment a tile makes the texture real. This sheet
        // is 64 × 4, well inside the pane budget, so the target is the whole of it —
        // which is what makes this the same arithmetic it was before panes.
        let mut one_tile = Sheet {
            width: 64,
            height: 4,
            ..Sheet::default()
        };
        let mut vertices = Vec::new();
        QuadOutline::from_segments(&rect_path(0.0, 0.0, 4.0, 4.0)).append_triangles(
            |p| [p.x, p.y],
            [0.0, 0.0, 4.0, 4.0],
            &mut vertices,
        );
        one_tile.push_tile([0.0, 0.0, 4.0, 4.0], false, &vertices);
        let vertex_count = vertices.len() as u64;
        assert_eq!(
            one_tile.device_bytes(),
            64 * 4 * 8 + vertex_count * crate::outline::WindingVertex::STRIDE + TILE_STRIDE
        );
    }
}
