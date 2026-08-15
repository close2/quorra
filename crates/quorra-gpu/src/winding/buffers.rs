//! Everything the two passes read, built once per frame: the vertex buffer, the tile
//! instances, and one uniform per sample per pane.
//!
//! The two passes take these as arguments and never make one, which is what keeps a
//! pass a pass — a `queue.write_buffer` between `begin_render_pass` and its end is not
//! a thing that can happen if the buffers are already built. The ordering rule that
//! makes a pane's resolve a single draw lives here too: tiles go up in the plan's
//! order, and the vertices stay where the encoder put them.

use crate::pane::{Pane, Plan};
use crate::pipeline::PipelineStore;

use super::{SAMPLES_PER_PASS, Sheet, sample_offsets};

/// One group of four samples, as the offsets they place their geometry by.
///
/// The bind groups are built per pane rather than held here: the uniform carries the
/// pane, so one per sample would be one per pane per sample, and building them where
/// they are used keeps the pane from having to be threaded into a field.
pub(super) struct Group {
    offsets: Vec<[f32; 2]>,
}

impl Group {
    /// This group's uniforms for one pane: one per sample for the winding pass, and one
    /// for the resolve, which reads the sheet size and the pane and nothing else.
    pub(super) fn for_pane(
        &self,
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &PipelineStore,
        sheet: &Sheet,
        pane: &Pane,
    ) -> PaneGlobals {
        let layout = pipelines.winding_layout();
        let samples = self
            .offsets
            .iter()
            .enumerate()
            .map(|(channel, offset)| {
                globals_bind_group(gpu, queue, &layout, sheet, *offset, channel, pane)
            })
            .collect();
        let resolve = globals_bind_group(gpu, queue, &layout, sheet, [0.0, 0.0], 0, pane);
        PaneGlobals { samples, resolve }
    }
}

/// One group's uniforms, bound to one pane.
pub(super) struct PaneGlobals {
    pub(super) samples: Vec<wgpu::BindGroup>,
    pub(super) resolve: wgpu::BindGroup,
}

/// Everything the two passes read, built once per frame.
pub(super) struct Buffers {
    pub(super) vertices: wgpu::Buffer,
    pub(super) tiles: wgpu::Buffer,
    pub(super) groups: Vec<Group>,
}

impl Buffers {
    #[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
    #[allow(clippy::arithmetic_side_effects)] // a Vec length times its element count
    pub(super) fn new(
        gpu: &wgpu::Device,
        queue: &wgpu::Queue,
        sheet: &Sheet,
        plan: &Plan,
        samples: u32,
    ) -> Self {
        let vertices = create_buffer(
            gpu,
            queue,
            "quorra winding vertices",
            &to_bytes(&sheet.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        // Tiles go up in the plan's order, so a pane's instances are one contiguous
        // range and its resolve is one draw. The vertices stay where the encoder put
        // them — permuting the largest buffer in the frame would cost more than the
        // per-tile draw ranges the plan carries instead.
        let mut tile_data: Vec<f32> = Vec::with_capacity(sheet.tiles.len() * 6);
        for index in &plan.order {
            let tile = &sheet.tiles[*index as usize];
            tile_data.extend_from_slice(&tile.rect);
            tile_data.push(if tile.even_odd { 1.0 } else { 0.0 });
            tile_data.push(samples as f32);
        }
        let tiles = create_buffer(
            gpu,
            queue,
            "quorra winding tiles",
            &to_bytes(&tile_data),
            wgpu::BufferUsages::VERTEX,
        );

        let groups = sample_offsets(samples)
            .chunks(SAMPLES_PER_PASS as usize)
            .map(|chunk| Group {
                offsets: chunk.to_vec(),
            })
            .collect();

        Self {
            vertices,
            tiles,
            groups,
        }
    }
}

/// The 48-byte uniform one winding draw reads.
fn globals_bind_group(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sheet: &Sheet,
    offset: [f32; 2],
    channel: usize,
    pane: &Pane,
) -> wgpu::BindGroup {
    let buffer = create_buffer(
        gpu,
        queue,
        "quorra winding globals",
        &to_bytes(&globals_lanes(sheet, offset, channel, pane)),
        wgpu::BufferUsages::UNIFORM,
    );
    gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra winding globals"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// The twelve `f32` lanes of `winding.wgsl`'s `Globals`, in its order.
///
/// Written as one flat array rather than field by field, which works only because every
/// field of that struct happens to start where the previous one ends — the `vec4f`
/// channel mask lands on 16 of its own accord. Move it and the array would need padding
/// the shader's own alignment would insert; `tests` below is what says it still does not.
#[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
fn globals_lanes(sheet: &Sheet, offset: [f32; 2], channel: usize, pane: &Pane) -> [f32; 12] {
    let mut mask = [0.0_f32; 4];
    mask[channel.min(3)] = 1.0;
    [
        sheet.width.max(1) as f32,
        sheet.height.max(1) as f32,
        offset[0],
        offset[1],
        mask[0],
        mask[1],
        mask[2],
        mask[3],
        pane.origin[0] as f32,
        pane.origin[1] as f32,
        pane.size[0].max(1) as f32,
        pane.size[1].max(1) as f32,
    ]
}

/// A buffer with `data` in it. Zero-length data still makes a one-element buffer:
/// wgpu refuses a zero-sized buffer, and a frame with no triangles is a legitimate
/// frame that must still reach the passes and draw nothing.
fn create_buffer(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    data: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    let size = (data.len() as u64).max(4);
    let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !data.is_empty() {
        queue.write_buffer(&buffer, 0, data);
    }
    buffer
}

/// Floats as the little-endian bytes a wgpu buffer takes.
///
/// A copy rather than a cast: `#![forbid(unsafe_code)]` rules out reinterpreting the
/// slice, and the crate takes no `bytemuck` (`deny.toml`'s posture is that a
/// dependency has to earn its place). The copy is one `memcpy` per frame over data
/// the encoder has just built anyway, which no measurement has ever noticed.
fn to_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{Pane, Sheet, globals_lanes, to_bytes};
    use crate::shaders;
    use crate::shaders::layout::{Lane, check};

    /// The one uniform in the crate written as a flat float array rather than at named
    /// offsets, checked against the struct whose alignment it is relying on.
    ///
    /// `pane_origin` is the number ADR 0027 shipped without in one of its three places
    /// and drew nothing at all for every band after the first; a *fourth* place that
    /// could put it somewhere else is the array above, and this is what watches it.
    #[test]
    fn the_winding_globals_are_the_sheets_globals() {
        let sheet = Sheet {
            width: 11,
            height: 12,
            ..Sheet::default()
        };
        let pane = Pane {
            origin: [21, 22],
            size: [23, 24],
            first_tile: 0,
            tile_count: 0,
            vertex_runs: Vec::new(),
        };
        // Channel 2 of the four an ordered-grid pass accumulates into (ADR 0016).
        let bytes = to_bytes(&globals_lanes(&sheet, [0.25, -0.25], 2, &pane));
        check(
            shaders::WINDING,
            "Globals",
            &bytes,
            &[
                ("sheet_size", Lane::Vec2([11.0, 12.0])),
                ("sample_offset", Lane::Vec2([0.25, -0.25])),
                ("channel", Lane::Vec4([0.0, 0.0, 1.0, 0.0])),
                ("pane_origin", Lane::Vec2([21.0, 22.0])),
                ("pane_size", Lane::Vec2([23.0, 24.0])),
            ],
        );
    }
}
