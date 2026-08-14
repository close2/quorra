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
#[allow(clippy::cast_precision_loss)] // sheet extents are far below f32's exact range
fn globals_bind_group(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sheet: &Sheet,
    offset: [f32; 2],
    channel: usize,
    pane: &Pane,
) -> wgpu::BindGroup {
    let mut mask = [0.0_f32; 4];
    mask[channel.min(3)] = 1.0;
    let data = [
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
    ];
    let buffer = create_buffer(
        gpu,
        queue,
        "quorra winding globals",
        &to_bytes(&data),
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
