//! Tier 1's price: copy out, map, and convert to straight alpha, exactly once.
//!
//! §6.1 of the brief measured this path as the item that dominates an offscreen frame
//! — about 1.2 GB/s of copy, map and demultiply at 4× scale — which is why it lives
//! behind [`Target::Readback`](crate::target::Target::Readback) alone and why
//! `Timings::readback` prices it separately: §11.1 asks how much of the old backend's
//! fixed cost this was, and the split is the answer.
//!
//! The premultiplied→straight conversion here is the boundary conversion of §3, done
//! once; its rounding rule is quorra's own and is recorded in `doc/adr/0005`.

use crate::error::RenderError;
use crate::frame::Raster;

/// Copy the finished target out, map it, and convert premultiplied to straight alpha.
pub(crate) fn read_back(
    gpu: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    max_target_size: u32,
) -> Result<Raster, RenderError> {
    let bytes_per_row = width
        .checked_mul(4)
        .and_then(|b| b.checked_next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
        .ok_or(RenderError::TargetTooLarge {
            width,
            height,
            limit: max_target_size,
        })?;
    let size = u64::from(bytes_per_row)
        .checked_mul(u64::from(height))
        .ok_or(RenderError::TargetTooLarge {
            width,
            height,
            limit: max_target_size,
        })?;
    let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quorra readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("quorra readback copy"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    // Converted straight out of the mapped range: `read_buffer`'s `to_vec` would be a
    // second full-target copy — 8 MB at page size — of bytes this reads once and
    // discards (ADR 0022).
    let pixels = map_and_convert(gpu, &buffer, width, height, bytes_per_row)?;
    Ok(Raster::new(width, height, pixels))
}

/// Map the copy-out buffer and demultiply straight from it, without a staging `Vec`.
fn map_and_convert(
    gpu: &wgpu::Device,
    buffer: &wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_row: u32,
) -> Result<Vec<u8>, RenderError> {
    await_map(gpu, buffer)?;
    let pixels = {
        let view = buffer
            .get_mapped_range(..)
            .map_err(|e| RenderError::ReadbackFailed {
                detail: e.to_string(),
            })?;
        demultiply(&view, width, height, bytes_per_row)
    };
    buffer.unmap();
    Ok(pixels)
}

/// Map a `MAP_READ` buffer and copy its bytes out.
pub(crate) fn read_buffer(
    gpu: &wgpu::Device,
    buffer: &wgpu::Buffer,
) -> Result<Vec<u8>, RenderError> {
    await_map(gpu, buffer)?;
    let bytes = {
        let view = buffer
            .get_mapped_range(..)
            .map_err(|e| RenderError::ReadbackFailed {
                detail: e.to_string(),
            })?;
        view.to_vec()
    };
    buffer.unmap();
    Ok(bytes)
}

/// Ask for the map and block until the device has done it.
fn await_map(gpu: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), RenderError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        // The poll below drives this callback; a send failure would mean the
        // receiver was dropped, which only happens after this function returned.
        let _ = sender.send(result);
    });
    gpu.poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| RenderError::DeviceLost {
            detail: e.to_string(),
        })?;
    match receiver.recv() {
        Ok(Ok(())) => {}
        Ok(Err(source)) => {
            return Err(RenderError::ReadbackFailed {
                detail: source.to_string(),
            });
        }
        Err(_) => {
            return Err(RenderError::DeviceLost {
                detail: "the map callback was dropped without running".into(),
            });
        }
    }
    Ok(())
}

/// The rounding rule of `doc/adr/0005`, for every (alpha, channel) pair there is:
/// `straight = round(255·c / a)`, computed in integers as `(c·255 + a/2) / a` and
/// clamped to 255 — the clamp covers the ≤1-ulp cases where unorm blending leaves a
/// channel a hair above its alpha.
///
/// 64 KiB, built at compile time, indexed `[alpha << 8 | channel]`. It exists because
/// the loop below ran three integer divisions per pixel — six million on a page — and a
/// division is the most expensive integer operation a CPU has. The table is not an
/// approximation of the rule: it *is* the rule, evaluated ahead of time, which
/// `demultiply_matches_the_documented_division` checks over all 65 536 pairs
/// (ADR 0022).
static STRAIGHT: [u8; 65_536] = build_straight_table();

// The 64 KiB array is the point of the function, and it is a `static`: the local is
// the initialiser, evaluated at compile time and never on a stack.
#[allow(clippy::cast_possible_truncation)] // every value is clamped to 255 first
#[allow(clippy::large_stack_arrays)]
// Both loop counters stop at 256 and `channel * 255 + alpha / 2` is at most 65 152, so
// nothing here can overflow; a const fn that panicked would fail the build rather than
// the frame, which is the strongest form this bound could be checked in.
#[allow(clippy::arithmetic_side_effects)]
const fn build_straight_table() -> [u8; 65_536] {
    let mut table = [0_u8; 65_536];
    let mut alpha = 1_usize; // alpha 0 keeps its row of zeros: a transparent pixel has
    while alpha < 256 {
        // no straight colour, and the loop below never reads that row.
        let mut channel = 0_usize;
        while channel < 256 {
            let straight = (channel * 255 + alpha / 2) / alpha;
            table[alpha << 8 | channel] = if straight > 255 { 255 } else { straight as u8 };
            channel += 1;
        }
        alpha += 1;
    }
    table
}

/// Premultiplied RGBA8 rows (with copy padding) to straight-alpha RGBA8, tightly
/// packed. The conversion happens exactly once, at this boundary (§3 of the brief).
///
/// Three shapes of pixel, in the order a real page has them: fully transparent (most
/// of a page of text), fully opaque (a filled background, and every pixel of a photo),
/// and partial. The first two are byte copies; only the third reaches the table.
// Bounds make the arithmetic infallible: `alpha << 8 | channel` is below 65 536 by
// construction, and row indexing is bounded by the buffer layout the copy just wrote.
// Stated here once rather than checked per pixel in a hot loop.
#[allow(clippy::arithmetic_side_effects)]
fn demultiply(padded: &[u8], width: u32, height: u32, bytes_per_row: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let bytes_per_row = bytes_per_row as usize;
    let row_bytes = width * 4;
    // Written through a slice rather than `push`ed: eight million bounds-checked pushes
    // is the shape of the loop, not the work in it.
    let mut pixels = vec![0_u8; row_bytes * height];
    for row in 0..height {
        let start = row * bytes_per_row;
        let source = &padded[start..start + row_bytes];
        let destination = &mut pixels[row * row_bytes..(row + 1) * row_bytes];
        for (pixel, out) in source.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
            let alpha = pixel[3];
            if alpha == 0 {
                continue; // the destination is already zero
            }
            if alpha == 255 {
                out.copy_from_slice(pixel);
                continue;
            }
            let row = usize::from(alpha) << 8;
            out[0] = STRAIGHT[row | usize::from(pixel[0])];
            out[1] = STRAIGHT[row | usize::from(pixel[1])];
            out[2] = STRAIGHT[row | usize::from(pixel[2])];
            out[3] = alpha;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::demultiply;

    /// The documented rounding rule, checked at its edges: zero alpha zeroes the
    /// pixel, full alpha is the identity, and a half-covered premultiplied value
    /// rounds to nearest.
    #[test]
    fn demultiply_rounds_as_documented() {
        // One row, no padding: alpha 0, alpha 255, alpha 128.
        let padded = [
            9, 9, 9, 0, // fully transparent: everything zeroes
            10, 128, 255, 255, // opaque: unchanged
            64, 1, 128, 128, // half-covered: see the expected values below
        ];
        let out = demultiply(&padded, 3, 1, 12);
        assert_eq!(&out[0..4], &[0, 0, 0, 0]);
        assert_eq!(&out[4..8], &[10, 128, 255, 255]);
        // (64·255 + 64)/128 = 128; (1·255 + 64)/128 = 2; (128·255 + 64)/128 clamps to 255.
        assert_eq!(&out[8..12], &[128, 2, 255, 128]);
    }

    /// The table *is* the documented division, for every pair there is — the claim
    /// that makes replacing six million divisions a shortcut rather than a change.
    #[test]
    #[allow(clippy::cast_possible_truncation)] // both counters are bounded by 255
    fn demultiply_matches_the_documented_division() {
        for alpha in 1_u32..=255 {
            for channel in 0_u32..=255 {
                let divided = ((channel * 255 + alpha / 2) / alpha).min(255) as u8;
                let padded = [channel as u8, 0, 0, alpha as u8];
                let out = demultiply(&padded, 1, 1, 4);
                assert_eq!(
                    out[0], divided,
                    "alpha {alpha}, channel {channel}: table {} against division {divided}",
                    out[0]
                );
            }
        }
    }

    /// Copy padding between rows is dropped, not read into the raster.
    #[test]
    fn demultiply_skips_row_padding() {
        let mut padded = vec![0_u8; 512];
        // Two rows of one opaque white pixel each, 256-byte padded rows.
        padded[0..4].copy_from_slice(&[255, 255, 255, 255]);
        padded[256..260].copy_from_slice(&[255, 255, 255, 255]);
        let out = demultiply(&padded, 1, 2, 256);
        assert_eq!(out, vec![255, 255, 255, 255, 255, 255, 255, 255]);
    }
}
