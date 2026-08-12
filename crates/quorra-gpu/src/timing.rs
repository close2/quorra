//! Timestamp queries: the instrumentation that lets a frame prove what it cost.
//!
//! §8 of the brief exists because the old backend could not separate execution from
//! readback — a bytes-per-second estimate had to stand in for what a timestamp query
//! would have said exactly, and §6.1 spends two paragraphs on the resulting
//! uncertainty. This module is the query plumbing; the honesty rule it serves lives
//! in [`TimingProvenance`](crate::frame::TimingProvenance): a number that had to be a
//! wall clock says so.

use std::time::Duration;

use crate::error::RenderError;
use crate::frame::TimingProvenance;
use crate::readback;

/// Timestamp-query support, captured at device construction.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimestampSupport {
    /// Nanoseconds per timestamp tick, from the queue.
    pub period: f32,
}

/// The query set and buffers that carry one pass's two timestamps home.
#[derive(Debug)]
pub(crate) struct PassQuery {
    pub set: wgpu::QuerySet,
    pub resolve: wgpu::Buffer,
    pub map: wgpu::Buffer,
}

impl PassQuery {
    /// A two-timestamp query (pass begin, pass end) with its resolve and map buffers.
    pub(crate) fn new(gpu: &wgpu::Device) -> Self {
        Self {
            set: gpu.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("quorra pass timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: gpu.create_buffer(&wgpu::BufferDescriptor {
                label: Some("quorra timestamp resolve"),
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            map: gpu.create_buffer(&wgpu::BufferDescriptor {
                label: Some("quorra timestamp map"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }
}

/// Read a pass's two timestamps and convert ticks to time; fall back to the given
/// wall clock — and say so — when the adapter has no queries.
///
/// On the query path, the pass duration is also appended to `phases` under
/// `phase_name`, so per-pass attribution accumulates as milestones add passes.
pub(crate) fn read_pass(
    gpu: &wgpu::Device,
    support: Option<TimestampSupport>,
    query: Option<&PassQuery>,
    execute_wall: Duration,
    phase_name: &'static str,
    phases: &mut Vec<(&'static str, Duration)>,
) -> Result<(Duration, TimingProvenance), RenderError> {
    let (Some(support), Some(query)) = (support, query) else {
        return Ok((execute_wall, TimingProvenance::WallClock));
    };
    let bytes = readback::read_buffer(gpu, &query.map)?;
    let tick = |range: std::ops::Range<usize>| {
        let eight: [u8; 8] = bytes[range].try_into().unwrap_or([0; 8]);
        u64::from_le_bytes(eight)
    };
    let delta_ticks = tick(8..16).saturating_sub(tick(0..8));
    // Ticks-to-nanoseconds goes through f64: the tick count of any real pass is far
    // below 2^53, so the conversion is exact; the final truncation is of fractional
    // nanoseconds.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let nanos = (delta_ticks as f64 * f64::from(support.period)) as u64;
    let execute = Duration::from_nanos(nanos);
    phases.push((phase_name, execute));
    Ok((execute, TimingProvenance::TimestampQueries))
}
