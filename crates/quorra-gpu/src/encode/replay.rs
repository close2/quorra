//! Record replay: the walk, paid for once and replayed per viewport (ADR 0087).
//!
//! ADR 0084 stage A. A steady zoom re-walks a retained scene every frame — command
//! dispatch, one hash probe per fill, clip resolution, lane choice — for answers that
//! are a pure function of `(scene, viewport)`, and on the caller's worst page that
//! walk is 16–26 ms of every step. Under `Coverage::Compute` almost every command's
//! encode is three cheap steps (compose, cull, seat) once the per-scene answers are
//! in hand, so the first encode writes those answers down as **records**, and a later
//! frame at a *different viewport* replays the records instead of the scene.
//!
//! # What a record is, and what it is not
//!
//! A record is everything about one command that does **not** depend on the viewport,
//! denormalised so the replay probes nothing: the outline id beside its control box
//! and its §10.7.4 collapse table, the command transform, paint, rule, style and clip
//! id. What a record is *not* is pixels or placements — every device-space answer
//! (the composed transform, the cull, the tile seat, the instance bytes) is computed
//! fresh per viewport, by the same arithmetic the walk uses, so a replayed frame's
//! bytes are the bytes the walk would have produced.
//!
//! Three kinds, and the third is the honesty valve:
//!
//! - [`ReplayRecord::Rect`] — a `Command::Rect`, replayed through `encode_rect`
//!   itself (the arm is already probe-free).
//! - [`ReplayRecord::SolidFill`] — the compute lane's fill: marks, the analytic
//!   rectangle route, or a compute tile, decided per viewport exactly as the walk
//!   decides them ([`Encoder::replay_solid_fill`]).
//! - [`ReplayRecord::Slow`] — everything else, replayed by re-dispatching the scene
//!   command at that index through the ordinary walk: a stroke re-expands, a rare
//!   paint re-resolves, an image re-places. The few pay full price; the many pay
//!   three steps.
//!
//! # When the list exists, and when it dies
//!
//! Recording is attempted only under [`Coverage::Compute`] — the lane the caller
//! moves views on — and is abandoned (`None`, permanently, for that encode) the
//! moment the walk meets something whose *frame-wide* state a per-record replay
//! cannot rebuild: a child layer, a soft mask, a residue clip, an atlas or winding
//! tile. Those are per-frame structures with cross-command state; a scene that uses
//! them re-walks, byte for byte as today. The admission is structural, not a promise:
//! the sites that build those structures are the sites that abandon the list.
//!
//! Replay is keyed like the retained encode itself minus the viewport
//! ([`EncodeKey::replay_compatible`](crate::retained::EncodeKey)): same device, same
//! lane, same resource generation — the records carry denormalised control boxes and
//! collapse tables, which resource ids' never-rebound guarantee keeps true. The atlas
//! generation is deliberately not compared: a replayable encode named no atlas tile.

use quorra_scene::{Affine, ClipId, Color, Rect};

use super::device_space::{compose, transform_preserves_axes};
use super::fill::MarkInk;
use super::{DrawStyle, Encoder};
use crate::error::RenderError;
use crate::resources::CollapsedMark;

/// One command's viewport-free half, in encounter order.
#[derive(Debug, Clone)]
pub(crate) enum ReplayRecord {
    /// A `Command::Rect`: replayed through `encode_rect`, which probes nothing.
    Rect {
        rect: Rect,
        transform: Affine,
        color: Color,
        clip: Option<ClipId>,
    },
    /// A solid fill under the compute lane, with the outline's per-scene answers
    /// denormalised so the replay probes nothing.
    SolidFill {
        outline: u32,
        control_box: [f32; 4],
        /// The outline's axis-aligned-rectangle hint, for the analytic route.
        rect_hint: Option<Rect>,
        /// The outline's §10.7.4 collapse table, copied — almost always empty, and an
        /// empty boxed slice allocates nothing.
        marks: Box<[CollapsedMark]>,
        transform: Affine,
        even_odd: bool,
        color: Color,
        clip: Option<ClipId>,
        style: DrawStyle,
    },
    /// Everything else: the scene command at this index, re-dispatched through the
    /// ordinary walk.
    Slow { index: u32 },
}

impl ReplayRecord {
    /// Heap bytes this record holds beyond its own size, for the retained budget.
    pub(crate) fn heap_bytes(&self) -> u64 {
        match self {
            Self::SolidFill { marks, .. } => {
                (marks.len() as u64).saturating_mul(size_of::<CollapsedMark>() as u64)
            }
            Self::Rect { .. } | Self::Slow { .. } => 0,
        }
    }
}

/// The walk's viewport-free half, kept beside the retained encode.
#[derive(Debug, Clone, Default)]
pub(crate) struct ReplayList {
    pub(crate) records: Vec<ReplayRecord>,
    /// Counters the replay cannot recount because it skips the probes that count
    /// them: the distinct-outline set and the segment total are per-scene facts, so
    /// the encode that walked the scene wrote them down.
    pub(crate) distinct_outlines: u32,
    pub(crate) segments: u32,
}

impl ReplayList {
    pub(crate) fn retained_bytes(&self) -> u64 {
        let records = (self.records.len() as u64).saturating_mul(size_of::<ReplayRecord>() as u64);
        self.records
            .iter()
            .fold(records, |sum, r| sum.saturating_add(r.heap_bytes()))
    }
}

impl Encoder<'_> {
    /// Note the start of a command, so a downgrade can replace exactly its records.
    pub(super) fn begin_command_records(&mut self, index: usize) {
        self.current_command = index;
        if let Some(list) = self.replay.as_ref() {
            self.command_records_start = list.records.len();
        }
    }

    /// Append a record for the current command.
    pub(super) fn record(&mut self, record: ReplayRecord) {
        if let Some(list) = self.replay.as_mut() {
            list.records.push(record);
        }
    }

    /// The current command needs the full walk at every viewport: replace whatever it
    /// recorded so far with one [`ReplayRecord::Slow`], exactly once.
    pub(super) fn record_slow(&mut self) {
        let index = self.current_command;
        let start = self.command_records_start;
        if let Some(list) = self.replay.as_mut() {
            list.records.truncate(start);
            list.records.push(ReplayRecord::Slow {
                index: u32::try_from(index).unwrap_or(u32::MAX),
            });
            // One Slow per command: later downgrades of the same command find the
            // truncation point just below the Slow they already pushed.
            self.command_records_start = list.records.len().saturating_sub(1);
        }
    }

    /// This frame cannot be replayed at another viewport: a child layer, a soft mask,
    /// a residue clip, an atlas or winding tile — frame-wide state a per-record
    /// replay cannot rebuild. Permanent for this encode.
    pub(super) fn unreplayable(&mut self) {
        self.replay = None;
    }

    /// One [`ReplayRecord::SolidFill`], replayed: the same three answers the walk
    /// gives — §10.7.4 marks, the analytic rectangle, or a compute tile — decided
    /// fresh under this viewport, with every per-scene input read off the record.
    #[allow(clippy::too_many_arguments)] // one record's fields, destructured once
    pub(super) fn replay_solid_fill(
        &mut self,
        outline: u32,
        control_box: [f32; 4],
        rect_hint: Option<Rect>,
        marks: &[CollapsedMark],
        transform: Affine,
        even_odd: bool,
        color: Color,
        clip: Option<ClipId>,
        style: DrawStyle,
    ) -> Result<(), RenderError> {
        let to_device = compose(transform, self.viewport);
        let resolved = self.resolve_clip(clip)?;
        let bounds = super::fill::corner_bounds(control_box, &to_device);
        if self.culled(bounds, &resolved) {
            return Ok(());
        }
        self.encode_collapsed_marks(
            marks,
            &to_device,
            &resolved,
            MarkInk::Solid(color),
            style,
            None,
        )?;
        if resolved.residues.is_none()
            && transform_preserves_axes(&to_device)
            && let Some(hint) = rect_hint
        {
            match self.clipped_device_rect(hint, &to_device, &resolved) {
                Some(device_rect) => self.push_rect_instance(device_rect, color, style, None)?,
                None => self.note_culled(),
            }
            return Ok(());
        }
        self.fill_compute(
            quorra_scene::OutlineId(outline),
            &to_device,
            bounds,
            even_odd,
            color,
            style,
            None,
            &resolved,
        )
    }
}
