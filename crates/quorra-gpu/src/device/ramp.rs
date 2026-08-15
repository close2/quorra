//! A colour ramp, sampled: the sweep between two stops, evaluated once on the CPU.
//!
//! ISO 32000-2 §8.7.4.5's axial and radial shadings are a colour function over a
//! parameter, and this device draws them by reading a table rather than by evaluating
//! one per pixel. The table's arithmetic is deliberately ours rather than the driver's
//! (ADR 0011): the shader reads it with `textureLoad` at a rounded index, so every
//! adapter gets the same texel, where filtering between two texels would not promise
//! that.
//!
//! Nothing here touches the GPU, which is why it is not in the file that owns one. The
//! texture these bytes become is [`super::textures`]'s, and which ramps a frame needs
//! at all is [`super::resident`]'s.

use quorra_scene::{Color, Stop};

/// Texels per sampled ramp. 4096 rather than the 256 first chosen: a ramp with
/// *hard* stop boundaries (a banded shading) has its boundaries snapped to this
/// grid, and a page-spanning axis divided by 510 was a visible ~3.5 px band
/// displacement on a real page (the corpus's `issue10572.pdf`). Divided by 8190
/// it is under an eighth of a pixel on the same page, for 16 KiB per resident
/// ramp — priced against the resource budget like everything else.
pub(super) const RAMP_RESOLUTION: u32 = 4096;

/// Sample a validated ramp to [`RAMP_RESOLUTION`] straight-RGBA8 texels, on the
/// CPU (ADR 0011).
///
/// The shader indexes the result with `textureLoad` at `round(t·(N−1))`, reading
/// N from the texture itself, so the sweep's colour arithmetic is this
/// function's — deterministic across adapters — rather than the driver's
/// texture filtering.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // round of 0..=255
#[allow(clippy::cast_precision_loss)] // i < RAMP_RESOLUTION, far below 2^24
pub(super) fn sample_ramp(stops: &[Stop]) -> Vec<u8> {
    let entries = RAMP_RESOLUTION as usize;
    let mut out = Vec::with_capacity(entries.saturating_mul(4));
    let last = (RAMP_RESOLUTION.saturating_sub(1)) as f32;
    for i in 0..RAMP_RESOLUTION {
        let color = ramp_color_at(stops, i as f32 / last);
        for component in [color.r, color.g, color.b, color.a] {
            // Components were validated into 0..=1 at upload.
            out.push((component * 255.0).round() as u8);
        }
    }
    out
}

/// The ramp's colour at `t`: constant before the first and after the last stop,
/// linearly interpolated between neighbours. At coincident offsets the later stop
/// wins — a PDF type 2/3 stitching boundary is half-open, the next function owning
/// its start (ISO 32000-2 §7.10.4).
fn ramp_color_at(stops: &[Stop], t: f32) -> Color {
    // Upload refused empty ramps; transparent black would still be an honest
    // answer for one, not an approximation of anything.
    let Some(first) = stops.first() else {
        return Color::new(0.0, 0.0, 0.0, 0.0);
    };
    if t <= first.offset {
        return first.color;
    }
    let mut previous = *first;
    for stop in stops.iter().skip(1) {
        if t <= stop.offset {
            let span = stop.offset - previous.offset;
            if span <= 0.0 {
                return stop.color;
            }
            let u = (t - previous.offset) / span;
            let mix = |a: f32, b: f32| a + (b - a) * u;
            return Color::new(
                mix(previous.color.r, stop.color.r),
                mix(previous.color.g, stop.color.g),
                mix(previous.color.b, stop.color.b),
                mix(previous.color.a, stop.color.a),
            );
        }
        previous = *stop;
    }
    previous.color
}
