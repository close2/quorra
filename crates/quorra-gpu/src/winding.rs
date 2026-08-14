//! The GPU coverage lane's frame work: two passes, once per group of four samples.
//!
//! The encoder hands over a [`Sheet`] — outline triangles in sheet space, and the
//! tiles they belong to. This module turns that into the same R8 scratch texture the
//! CPU lane uploads, so that everything downstream of coverage (the quad lanes, clips,
//! knockout, the compositor) cannot tell which lane produced it. That is the whole
//! integration: one texture, two producers, no second code path (ADR 0016).
//!
//! # Why a group of four
//!
//! An `rgba16float` texel holds four signed winding numbers exactly, so four sample
//! positions cost one texel and no packing. A frame that wants sixteen samples runs
//! this pair of passes four times, clearing the winding texture between rounds and
//! adding each round's quarter into the sheet — so **sample count costs time, never
//! memory**, which is the trade a document renderer wants: the GPU is idle here
//! (`execute` is tens of microseconds) and memory is what a zoomed page runs out of.
//!
//! # This file, and the three modules under it
//!
//! What is left here is the lane as the frame meets it: where the samples sit, whether
//! the sheet is this lane's to clear, and [`render_into`]'s loop over panes and rounds.
//! The rest is one module each — `sheet` for what the encoder built and what it costs,
//! `buffers` for what the passes read, `passes` for the winding target and the two
//! passes that write and read it.

use crate::error::RenderError;
use crate::pipeline::{Kind, PipelineStore};

mod buffers;
mod passes;
mod sheet;

use buffers::{Buffers, PaneGlobals};
pub(crate) use passes::WindingTexture;
pub(crate) use sheet::Sheet;

/// The winding texture's format. `f16` is exact on integers to 2048, which bounds the
/// winding number this lane can represent — four hundred times any winding a real page
/// produces, and stated here because the format is where the bound comes from.
const WINDING_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Samples per pass: one per channel of the winding texel.
pub(crate) const SAMPLES_PER_PASS: u32 = 4;

/// Who else has already written into the coverage sheet this lane draws onto
/// (ADR 0016).
///
/// One texture, two producers: the CPU lane uploads its rasterised tiles into the sheet
/// and this lane draws its own into the gaps the packer left between them. Which of the
/// two arrives first is the caller's fact, not this module's, and it decides exactly one
/// thing here — whether the first pass may clear the texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SheetUse {
    /// This lane owns the whole texture: nothing else put bytes in it, so the first
    /// pass clears it and the rest add to what it drew.
    LaneAlone,
    /// The CPU lane's coverage bytes are in the sheet already. Each tile's quad covers
    /// only its own rectangle, so this lane's tiles land beside those bytes without
    /// touching them — and a clear would take them out.
    BesideCpuBytes,
}

impl SheetUse {
    /// Whether the first pass of this frame may clear the sheet.
    fn clears_the_sheet(self) -> bool {
        matches!(self, Self::LaneAlone)
    }
}

/// The ordered sample grid, in pixels relative to the pixel's centre.
///
/// `count` samples on an `n × n` grid, `n = √count`, the k-th at
/// `((k mod n) + ½)/n − ½` across and `((k / n) + ½)/n − ½` down. Ours rather than the
/// driver's, so two adapters place them identically (ADR 0006's promise), and ordered
/// rather than jittered so that a frame is reproducible without carrying a seed.
#[allow(clippy::arithmetic_side_effects)] // `side` is at least 1 and at most 16
#[allow(clippy::cast_precision_loss)] // grid indices, far below f32's exact range
pub(crate) fn sample_offsets(count: u32) -> Vec<[f32; 2]> {
    let side = count.isqrt().max(1);
    #[allow(clippy::cast_precision_loss)] // side is at most 16 by Options' validation
    let step = 1.0 / side as f32;
    (0..count)
        .map(|index| {
            #[allow(clippy::cast_precision_loss)]
            let (x, y) = ((index % side) as f32, (index / side) as f32);
            [(x + 0.5).mul_add(step, -0.5), (y + 0.5).mul_add(step, -0.5)]
        })
        .collect()
}

/// Draws `sheet`'s tiles into `coverage_view`, which is the frame's scratch sheet.
///
/// [`SheetUse`] says whether this lane has the sheet to itself, which is what decides
/// whether the first pass clears it.
///
/// # Errors
///
/// [`RenderError::TargetTooLarge`] when the packed sheet exceeds the adapter's texture
/// dimension — the same limit, named the same way, as any other target of ours.
#[allow(clippy::too_many_arguments)] // the pass's inputs, named once at the one call
pub(crate) fn render_into(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    pipelines: &PipelineStore,
    reuse: &mut WindingTexture,
    coverage_view: &wgpu::TextureView,
    sheet: &Sheet,
    samples: u32,
    sheet_use: SheetUse,
    max_dimension: u32,
) -> Result<(), RenderError> {
    if sheet.width > max_dimension || sheet.height > max_dimension {
        return Err(RenderError::TargetTooLarge {
            width: sheet.width,
            height: sheet.height,
            limit: max_dimension,
        });
    }
    // One pane at a time (ADR 0028): the target is scratch, so it is sized by what a
    // pass holds — a rectangle over this lane's own tiles — rather than by what the
    // page came to on a sheet both lanes share.
    let plan = sheet.plan();
    let extent = wgpu::Extent3d {
        width: plan.target[0].max(1),
        height: plan.target[1].max(1),
        depth_or_array_layers: 1,
    };
    // **Kept between frames**, which ADR 0012 declined to do for the compositor's
    // textures "until a measurement says otherwise". This is that measurement: at 20x
    // the sheet is 2.5 million texels, and allocating and zero-initialising eight
    // bytes of each, every frame, cost 10.7 ms of a 15 ms frame — more than the
    // rasterising the lane exists to avoid. One texture, grown when a frame needs a
    // larger one and never shrunk while the device lives.
    let winding_view = reuse.view_for(gpu, extent).clone();

    let buffers = Buffers::new(gpu, queue, sheet, &plan, samples);
    let (winding_pipeline, _) = pipelines.get(Kind::Winding, WINDING_FORMAT)?;
    let (resolve_pipeline, _) =
        pipelines.get(Kind::WindingResolve, wgpu::TextureFormat::R8Unorm)?;
    let texture_layout = pipelines.sampled_layout();
    let winding_source = gpu.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("quorra winding source"),
        layout: &texture_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&winding_view),
        }],
    });

    let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quorra coverage"),
    });
    for (index, pane) in plan.panes.iter().enumerate() {
        for (round, group) in buffers.groups.iter().enumerate() {
            let globals = group.for_pane(gpu, queue, pipelines, sheet, pane);
            passes::accumulate(
                &mut encoder,
                &winding_view,
                &winding_pipeline,
                &buffers,
                &globals,
                pane,
            );
            passes::resolve(
                &mut encoder,
                coverage_view,
                &resolve_pipeline,
                &buffers,
                &globals,
                pane,
                &winding_source,
                // The coverage sheet is cleared once, by the first pass that touches
                // it, and only when this lane owns it: every later pane and round adds
                // to what is there.
                sheet_use.clears_the_sheet() && index == 0 && round == 0,
            );
        }
    }
    queue.submit([encoder.finish()]);
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)] // test-file policy as in `raster.rs`: a fixture that cannot run must fail loudly
mod tests {
    use super::{Sheet, render_into, sample_offsets};
    use crate::device::Device;
    use crate::outline::QuadOutline;
    use crate::startup::Options;
    use quorra_scene::{Point, Segment};

    /// The software adapter, as everywhere in this crate's tests.
    fn device() -> Device {
        Device::headless(&Options {
            adapter: Some("llvmpipe".into()),
            ..Options::default()
        })
        .expect("llvmpipe is present wherever this suite runs")
    }

    const SIDE: u32 = 16;

    /// The R8 coverage bytes of a sheet rendered from these segments.
    fn coverage(segments: &[Segment], even_odd: bool, samples: u32) -> Vec<u8> {
        let device = device();
        device.wait_until_warm();
        let (gpu, queue) = device.wgpu();
        let outline = QuadOutline::from_segments(segments);
        let mut vertices = Vec::new();
        outline.append_triangles(
            |p| [p.x, p.y],
            [0.0, 0.0, SIDE as f32, SIDE as f32],
            &mut vertices,
        );
        let mut sheet = Sheet {
            width: SIDE,
            height: SIDE,
            ..Sheet::default()
        };
        sheet.push_tile([0.0, 0.0, SIDE as f32, SIDE as f32], even_odd, &vertices);
        let texture = gpu.create_texture(&wgpu::TextureDescriptor {
            label: Some("winding test sheet"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut reuse = super::WindingTexture::default();
        render_into(
            gpu,
            queue,
            device.pipeline_store(),
            &mut reuse,
            &view,
            &sheet,
            samples,
            super::SheetUse::LaneAlone,
            device.limits().max_target_size,
        )
        .expect("the sheet is inside every limit");

        // Copy out, 256-aligned as wgpu requires, and drop the padding.
        let row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = SIDE.next_multiple_of(row);
        let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("winding test readback"),
            size: u64::from(padded) * u64::from(SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.poll(wgpu::PollType::wait_indefinitely())
            .expect("the copy completes");
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("the buffer is mapped");
        let mut pixels = Vec::with_capacity((SIDE * SIDE) as usize);
        for y in 0..SIDE as usize {
            let start = y * padded as usize;
            pixels.extend_from_slice(&mapped[start..start + SIDE as usize]);
        }
        drop(mapped);
        buffer.unmap();
        pixels
    }

    fn at(pixels: &[u8], x: u32, y: u32) -> u8 {
        pixels[(y * SIDE + x) as usize]
    }

    fn rect_path(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Segment> {
        vec![
            Segment::MoveTo(Point::new(x0, y0)),
            Segment::LineTo(Point::new(x1, y0)),
            Segment::LineTo(Point::new(x1, y1)),
            Segment::LineTo(Point::new(x0, y1)),
            Segment::Close,
        ]
    }

    /// A pixel-aligned square is solid inside and empty outside — the coarsest thing
    /// the lane must get right, and the one that catches a sign or winding error.
    #[test]
    fn an_aligned_square_is_solid_inside_and_empty_outside() {
        let pixels = coverage(&rect_path(4.0, 4.0, 12.0, 12.0), false, 16);
        assert_eq!(at(&pixels, 8, 8), 255, "the middle is covered");
        assert_eq!(at(&pixels, 4, 4), 255, "the first covered pixel");
        assert_eq!(at(&pixels, 11, 11), 255, "the last covered pixel");
        assert_eq!(at(&pixels, 3, 8), 0, "one pixel left of the square");
        assert_eq!(at(&pixels, 12, 8), 0, "one pixel right of the square");
        assert_eq!(at(&pixels, 0, 0), 0, "the corner nothing reaches");
    }

    /// An edge through the middle of a pixel column covers exactly half of it, and the
    /// sample grid says so exactly: with a 4×4 grid the columns sit at ±0.125 and
    /// ±0.375 from the centre, so two of four are inside — 8 of 16 samples, and
    /// `round(0.5 × 255)` is 128.
    #[test]
    fn a_half_covered_column_reads_one_hundred_and_twenty_eight() {
        let pixels = coverage(&rect_path(4.5, 4.0, 12.0, 12.0), false, 16);
        assert_eq!(at(&pixels, 4, 8), 128, "half of column 4 is inside");
        assert_eq!(at(&pixels, 5, 8), 255, "column 5 is wholly inside");
        assert_eq!(at(&pixels, 3, 8), 0, "column 3 is wholly outside");
    }

    /// Two nested squares wound the same way: non-zero fills the hole (winding 2),
    /// even-odd leaves it (winding 2 is even). The two rules differing on the *same*
    /// geometry is what proves the sign survived accumulation — a lane that only ever
    /// counted crossings could not tell these apart.
    #[test]
    fn the_two_fill_rules_differ_where_the_clause_says_they_do() {
        let mut nested = rect_path(2.0, 2.0, 14.0, 14.0);
        nested.extend(rect_path(6.0, 6.0, 10.0, 10.0));
        let non_zero = coverage(&nested, false, 16);
        let even_odd = coverage(&nested, true, 16);
        assert_eq!(
            at(&non_zero, 8, 8),
            255,
            "§8.5.3.3.2: winding two is not zero, so the inner square is filled"
        );
        assert_eq!(
            at(&even_odd, 8, 8),
            0,
            "§8.5.3.3.3: winding two is even, so the inner square is a hole"
        );
        assert_eq!(at(&non_zero, 3, 8), 255, "the outer ring fills either way");
        assert_eq!(at(&even_odd, 3, 8), 255);
    }

    /// The sample grid is an ordered grid, stated rather than the driver's: the k-th of
    /// sixteen sits at a quarter-pixel step, symmetric about the centre.
    #[test]
    fn the_sample_grid_is_ordered_and_centred() {
        let offsets = sample_offsets(16);
        assert_eq!(offsets.len(), 16);
        assert!((offsets[0][0] + 0.375).abs() < 1e-6, "{:?}", offsets[0]);
        assert!((offsets[15][1] - 0.375).abs() < 1e-6, "{:?}", offsets[15]);
        let sum: f32 = offsets.iter().map(|o| o[0] + o[1]).sum();
        assert!(
            sum.abs() < 1e-5,
            "the grid is balanced about the centre: {sum}"
        );
    }
}
