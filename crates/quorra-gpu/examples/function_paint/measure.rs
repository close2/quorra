//! The three measurements, and the discipline they are taken under.
//!
//! `doc/HANDOVER.md`: "Wall clocks lie under load, and this machine is somebody's
//! desktop." So every variant is run round-robin, so drift falls on all of them
//! equally, and what is quoted is the **minimum** of the rounds rather than a mean.
//! The device column is a timestamp query, which load cannot touch at all.

use std::time::{Duration, Instant};

use crate::eval;
use crate::harness::{Canvas, Gpu, Paint, Timed, draw};
use crate::program::Op;

/// One thing to measure, named for the table.
pub(crate) struct Variant<'a> {
    /// Row label.
    pub(crate) label: String,
    /// The pipeline.
    pub(crate) paint: &'a Paint,
    /// Its bindings.
    pub(crate) bind: &'a wgpu::BindGroup,
}

/// The fastest each variant managed, and how many rounds it took.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Best {
    /// Fastest pass by the adapter's own clock.
    pub(crate) device: Option<Duration>,
    /// Fastest submit-to-idle on the host.
    pub(crate) wall: Duration,
}

/// Run every variant up to `rounds` times, interleaved, and keep each one's minimum.
///
/// The round count shrinks to fit `budget`, measured from one untimed warm-up round:
/// a software rasteriser running an interpreted 482-instruction program takes seconds
/// per frame, and twelve rounds of that answers nothing twelve times.
pub(crate) fn round_robin(
    gpu: &Gpu,
    canvas: &Canvas,
    variants: &[Variant<'_>],
    rounds: usize,
    budget: Duration,
) -> Vec<Best> {
    let mut best: Vec<Option<Best>> = vec![None; variants.len()];
    // One untimed round first: a pipeline's first use pays for whatever the driver
    // defers, and that cost belongs to the compile column, not to this one.
    let warm = Instant::now();
    for variant in variants {
        let _: Timed = draw(gpu, canvas, variant.paint, variant.bind);
    }
    let one_round = warm.elapsed().max(Duration::from_micros(1));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "both durations are positive and the ratio is clamped below"
    )]
    let affordable = (budget.as_secs_f64() / one_round.as_secs_f64()) as usize;
    let rounds = rounds.min(affordable).max(1);
    for _ in 0..rounds {
        for (index, variant) in variants.iter().enumerate() {
            let timed = draw(gpu, canvas, variant.paint, variant.bind);
            let entry = best[index].get_or_insert(Best {
                device: timed.device,
                wall: timed.wall,
            });
            entry.wall = entry.wall.min(timed.wall);
            entry.device = match (entry.device, timed.device) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
    }
    best.into_iter()
        .map(|entry| {
            entry.unwrap_or(Best {
                device: None,
                wall: Duration::ZERO,
            })
        })
        .collect()
}

/// Evaluate the program once per device pixel on the processor, as the caller does.
///
/// Single-threaded and allocation-free, which makes it the *floor* of what their
/// side can reach by removing allocations and before `rayon`: a fair anchor rather
/// than a straw man.
pub(crate) fn cpu_grid(ops: &[Op], width: u32, height: u32) -> (Duration, Vec<u8>) {
    let mut pixels = Vec::with_capacity((width as usize).saturating_mul(height as usize) * 4);
    let mut stack = Vec::with_capacity(64);
    let started = Instant::now();
    for row in 0..height {
        #[allow(
            clippy::cast_precision_loss,
            reason = "a device coordinate below 2^24 converts exactly"
        )]
        let y = (row as f32 + 0.5) / height as f32;
        for column in 0..width {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a device coordinate below 2^24 converts exactly"
            )]
            let x = (column as f32 + 0.5) / width as f32;
            let colour = eval::evaluate(ops, x, y, &mut stack);
            for component in colour {
                pixels.push(quantise(component));
            }
            pixels.push(255);
        }
    }
    (started.elapsed(), pixels)
}

/// Straight to 8 bits, the way an `Rgba8Unorm` attachment does it.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the clamp bounds the value to 0..=255 before the cast"
)]
fn quantise(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// How two rasters of the same program differ.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Agreement {
    /// Pixels whose three components all match exactly.
    pub(crate) exact: usize,
    /// Pixels differing by at most one 8-bit step — the rounding of the last bit.
    pub(crate) off_by_one: usize,
    /// Pixels differing by more than that: a branch that went the other way.
    pub(crate) differing: usize,
    /// The worst single-component difference seen.
    pub(crate) worst: u8,
}

/// Compare two RGBA8 rasters of equal size.
pub(crate) fn agreement(left: &[u8], right: &[u8]) -> Agreement {
    let mut result = Agreement::default();
    for (a, b) in left.chunks_exact(4).zip(right.chunks_exact(4)) {
        let worst = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
        result.worst = result.worst.max(worst);
        match worst {
            0 => result.exact = result.exact.saturating_add(1),
            1 => result.off_by_one = result.off_by_one.saturating_add(1),
            _ => result.differing = result.differing.saturating_add(1),
        }
    }
    result
}
