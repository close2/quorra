//! Resources: uploaded once, referenced many times, and counted against a budget.
//!
//! §2.2 of the brief is the reason this registry exists: the 107 distinct outlines of
//! one dense page are uploaded once and referenced 5 933 times, and a zoom re-uploads
//! none of them. The caller keys its own pointer-identity map by the ids this module
//! hands out (`Arc::as_ptr` → [`OutlineId`]).
//!
//! Every upload is validated (§4.7: a 60 000×60 000 image and a 1e30 coordinate both
//! arrive from real files by way of a correct interpreter) and priced against a stated
//! budget before it is stored — a resource sized from document-derived arithmetic is a
//! decompression bomb with a different name (CLAUDE.md principle 3).
//!
//! The stored form is the validated CPU copy. The device-resident representation is
//! lane-specific — the atlas's R8 tiles (M4), the path lane's chosen geometry (M5),
//! the image lane's textures (M7) — and each lane builds its form from this copy when
//! it first needs it. That staging is deliberate and stated here rather than implied:
//! M2 owns identity, validation and budget; the lanes own bytes on the GPU.
//!
//! # The one thing this file is, and why it is not two
//!
//! It is **one admission, written five times**: a resource is checked against the clause
//! that governs it, converted into the form a lane will want, priced, given an identifier
//! and charged — *in that order* — or it is refused and none of those things happened.
//! CLAUDE.md's file-scale rule asks a file this long to say what its one thing is, and
//! that sentence is it. The obvious division — the five `upload_*` in one module, the
//! registry in another — is refused for a reason that is checkable rather than
//! aesthetic:
//!
//! - **[`ResourceStore::allocate_id`] has no caller anywhere but those five methods.** A
//!   seam there would separate a private helper from every call site it has, which is a
//!   cut through the middle of one operation rather than along a join between two.
//! - **The order is the subject.** "Nothing is stored and nothing is charged" is promised
//!   by each of the five and kept by `allocate_id` and [`budget::ResourceBudget::charge`]
//!   together, and the only thing that checks the five promises against the two
//!   implementations is a reader with both in front of them. `allocate_id` is called
//!   *before* the charge on purpose, and the bound it states — an identifier is never
//!   reused — is what [`ResourceStore::generation`]'s soundness rests on, three screens
//!   away.
//! - The five uploads are not five subjects but one shape repeated, and reviewing the
//!   fifth *is* comparing it with the first four.
//!
//! The counter itself **did** leave, in ADR 0075, and for the reason this comment used to
//! give against it: `charge` stopped having only these five callers the moment an
//! outline's quadratic form began converting on the frame that reads it, because that
//! conversion charges the same ceiling through a shared reference and refuses in a
//! frame's vocabulary. `budget.rs` is that number and its two refusals.
//!
//! What the length still buys is written down rather than left to be discovered: of the
//! lines here, fewer than two in three carry code and a third of those are the tests; the
//! longest single item is `upload_outline` at forty. The rest is clause citation and
//! stated invariant, which is the part a reader of a security boundary is here for — and
//! it is why the count that matters for this file is the code line rather than the line.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use quorra_scene::{
    FnOp, FunctionId, ImageId, ImageSpec, MAX_COORDINATE, MeshId, MeshSpec, OutlineId, RampId,
    Rect, ResourceId, Segment, Stop, axis_aligned_rect,
};

use crate::error::{DeviceError, RenderError, ResourceProblem};
use crate::outline::QuadOutline;

mod budget;

pub(crate) use budget::ResourceBudget;

/// A validated, resident outline.
#[derive(Debug)]
pub(crate) struct StoredOutline {
    /// The segments, as uploaded. Read every frame by the encoder — for device bounds,
    /// for flattening into the coverage lane, and for the segment counter — so this is
    /// the form the lanes work from, not an archive of the upload.
    pub segments: Box<[Segment]>,
    /// The same outline as closed contours of quadratic segments, for the GPU lane —
    /// converted by the **first frame that reads it**, and never on a host that never
    /// takes that lane (ADR 0075).
    ///
    /// Converted once either way, which is the whole reason the GPU lane's cost does not
    /// grow with magnification: the conversion depends on the outline and on nothing
    /// else, so a frame at 100× re-uses what the frame that first needed it built
    /// (ADR 0016).
    quads: OnceLock<QuadOutline>,
    /// The axis-aligned rectangle the outline traces, when it traces exactly one —
    /// recognised once, at upload, because §6.4 turns on it: a rectangular clip is
    /// four floats, never a mask, and the encoder asks this on every frame.
    pub rect_hint: Option<Rect>,
    /// What the segments cost, charged at upload.
    bytes: u64,
    /// What [`Self::quads`] cost, charged when it was converted and zero until then.
    ///
    /// Separate from `bytes` because the two are charged at different moments and only
    /// [`ResourceStore::release`] adds them up — and it does so through
    /// [`AtomicU64::into_inner`], which it may because a release owns the record.
    quad_bytes: AtomicU64,
}

impl StoredOutline {
    /// The GPU coverage lane's quadratics, converting and charging on the first ask.
    ///
    /// **This is the ask, and it is the only one** (ADR 0075). The caller's
    /// `QUORRA_FEEDBACK.md` §33 measured 156 ms of a 187.6 ms scene phase inside
    /// `upload_outline` on a 3 011 919-segment drawing, all of it converting a
    /// representation that frame never read: their default is
    /// [`Coverage::Cpu`](crate::startup::Coverage::Cpu) and
    /// [`Encoder::gpu_lane_admissible`](crate::encode::Encoder) answers `false` on sight
    /// under it. So the conversion moved here, behind the lane's own cheap tests.
    ///
    /// The budget is charged **when the bytes become resident**, not when the outline
    /// was uploaded, because a ceiling that priced a form it had not built would be
    /// pricing an estimate — and the estimate cannot be an upper bound worth having: one
    /// cubic converts to between one and 2⁸ quadratics
    /// ([`MAX_SPLIT_DEPTH`](crate::outline)), so a bound that could not lie would
    /// over-charge a page of straight edges by two orders of magnitude. The cost of that
    /// honesty is written down in ADR 0075: a device filled close to its ceiling can now
    /// refuse the *frame* that first crosses the coverage threshold, where before it
    /// refused the upload.
    pub(crate) fn quads(&self, budget: &ResourceBudget) -> Result<&QuadOutline, RenderError> {
        if let Some(built) = self.quads.get() {
            return Ok(built);
        }
        let built = QuadOutline::from_segments(&self.segments);
        let bytes = built.stored_bytes();
        budget
            .charge(bytes)
            .map_err(|over| RenderError::OutlineConversionBudgetExceeded {
                needed: over.needed,
                budget: over.limit,
            })?;
        match self.quads.set(built) {
            Ok(()) => self.quad_bytes.store(bytes, Ordering::Relaxed),
            // Another thread converted the same outline first and its charge stands, so
            // ours is returned rather than left on the budget. The two are the same
            // number: the conversion is a pure function of the segments.
            Err(_) => budget.refund(bytes),
        }
        // Set on either arm just above — by us, or by whoever we lost the race to — and
        // `OnceLock` has no infallible getter afterwards.
        #[allow(clippy::expect_used)]
        Ok(self.quads.get().expect("set on either arm above"))
    }

    /// Whether the converted form is resident, for the tests that state *when* it is.
    #[cfg(test)]
    pub(crate) fn quads_converted(&self) -> bool {
        self.quads.get().is_some()
    }
}

/// A validated, resident image.
#[derive(Debug)]
pub(crate) struct StoredImage {
    pub spec: ImageSpec,
    bytes: u64,
}

/// A validated, resident colour ramp.
#[derive(Debug)]
pub(crate) struct StoredRamp {
    pub stops: Box<[Stop]>,
    bytes: u64,
}

/// A validated, resident pre-rasterised mesh.
#[derive(Debug)]
pub(crate) struct StoredMesh {
    pub spec: MeshSpec,
    bytes: u64,
}

/// An admitted, resident §7.10.5 program (ADR 0053).
///
/// What is stored is the **analysis**, not the instruction list: every question a frame
/// asks — how many slots, which steps, which hash, whether the empty-stack decision is
/// relied on — was answered once at upload, and keeping the list beside it would be a
/// second copy of the same program that a later reader could believe was consulted.
#[derive(Debug)]
pub(crate) struct StoredFunction {
    pub analysis: crate::function::Analysis,
    bytes: u64,
}

/// The device's resource registry: five id spaces, one budget.
#[derive(Debug)]
pub(crate) struct ResourceStore {
    outlines: HashMap<u32, StoredOutline>,
    images: HashMap<u32, StoredImage>,
    ramps: HashMap<u32, StoredRamp>,
    meshes: HashMap<u32, StoredMesh>,
    functions: HashMap<u32, StoredFunction>,
    next_id: u32,
    budget: ResourceBudget,
    /// Bumped by every [`ResourceStore::release`], so that anything holding an encode
    /// that names resource ids can tell whether those ids still mean what they meant
    /// (`retained.rs`, ADR 0048).
    ///
    /// **Release only**, and that is sound because an upload mints an id by increment
    /// from a counter that is never allowed to wrap ([`ResourceStore::allocate_id`]) and
    /// never revives a released one: a stored encode's ids cannot come to name different
    /// bytes, so a release is the one operation that can take a referenced resource
    /// away. The counter *did* wrap until ADR 0050 audited this claim, which is why the
    /// bound is stated on `allocate_id` rather than assumed here.
    generation: u64,
}

impl ResourceStore {
    pub(crate) fn new(budget_bytes: u64) -> Self {
        Self {
            outlines: HashMap::new(),
            images: HashMap::new(),
            ramps: HashMap::new(),
            meshes: HashMap::new(),
            functions: HashMap::new(),
            next_id: 0,
            budget: ResourceBudget::new(budget_bytes),
            generation: 0,
        }
    }

    /// How many times a resource has been released from this store.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Bytes currently resident, for `Limits` and diagnostics.
    pub(crate) fn in_use_bytes(&self) -> u64 {
        self.budget.in_use()
    }

    /// The ceiling every resident byte is admitted against, for the one charge a *frame*
    /// takes: an outline's conversion into the GPU coverage lane's quadratics
    /// ([`StoredOutline::quads`], ADR 0075).
    pub(crate) fn budget(&self) -> &ResourceBudget {
        &self.budget
    }

    /// The resident outline behind an id, for the encoder.
    pub(crate) fn outline(&self, id: OutlineId) -> Option<&StoredOutline> {
        self.outlines.get(&id.0)
    }

    /// The resident image behind an id, for the image lane.
    pub(crate) fn image(&self, id: ImageId) -> Option<&StoredImage> {
        self.images.get(&id.0)
    }

    /// The resident ramp behind an id, for the shading lane.
    pub(crate) fn ramp(&self, id: RampId) -> Option<&StoredRamp> {
        self.ramps.get(&id.0)
    }

    /// The resident mesh behind an id, for the shading lane.
    pub(crate) fn mesh(&self, id: MeshId) -> Option<&StoredMesh> {
        self.meshes.get(&id.0)
    }

    /// The admitted program behind an id, for the function lane.
    pub(crate) fn function(&self, id: FunctionId) -> Option<&StoredFunction> {
        self.functions.get(&id.0)
    }

    /// Whether any resident program would generate the shader `hash` names.
    ///
    /// Two uploads of the same instructions are two identifiers and one content hash, so
    /// releasing one of them must not drop a pipeline the other still reaches. Asked at
    /// release only, over a map with one entry per uploaded program.
    pub(crate) fn holds_program(&self, hash: crate::function::ProgramHash) -> bool {
        self.functions
            .values()
            .any(|stored| stored.analysis.program_hash() == hash)
    }

    /// Validate an outline, price its segments, and store them.
    ///
    /// The path must be non-empty, must start with a `MoveTo` — nothing else has a
    /// current point to draw from (ISO 32000-2 §8.5.2) — and every coordinate must be
    /// finite and within `MAX_COORDINATE`. Each failure is a
    /// [`DeviceError::InvalidResource`] naming its own [`ResourceProblem`]; a path over
    /// the store's budget is [`DeviceError::ResourceBudgetExceeded`]. Either way
    /// nothing is stored and nothing is charged.
    ///
    /// **What it no longer does is convert** (ADR 0075). The GPU coverage lane's
    /// quadratic form is built by the first frame that reads it and charged then;
    /// [`StoredOutline::quads`] is that ask and states why the budget follows the bytes
    /// rather than anticipating them.
    pub(crate) fn upload_outline(&mut self, path: &[Segment]) -> Result<OutlineId, DeviceError> {
        if path.is_empty() {
            return Err(DeviceError::InvalidResource {
                reason: ResourceProblem::OutlineEmpty,
            });
        }
        if !matches!(path[0], Segment::MoveTo(_)) {
            return Err(DeviceError::InvalidResource {
                reason: ResourceProblem::OutlineMissingMoveTo,
            });
        }
        for segment in path {
            let points: &[quorra_scene::Point] = match segment {
                Segment::MoveTo(p) | Segment::LineTo(p) => std::slice::from_ref(p),
                Segment::CubicTo { c1, c2, to } => &[*c1, *c2, *to],
                Segment::Close => &[],
            };
            for point in points {
                if !point.is_finite() {
                    return Err(DeviceError::InvalidResource {
                        reason: ResourceProblem::OutlineNonFinite,
                    });
                }
                if point.x.abs() > MAX_COORDINATE || point.y.abs() > MAX_COORDINATE {
                    return Err(DeviceError::InvalidResource {
                        reason: ResourceProblem::OutlineCoordinateTooLarge {
                            limit: MAX_COORDINATE,
                        },
                    });
                }
            }
        }
        let rect_hint = axis_aligned_rect(path);
        // The segments, and only the segments: those are what this call makes resident.
        // The converted form is charged by the frame that converts it, so that the
        // ceiling counts bytes that exist rather than bytes that might (ADR 0075).
        let bytes = (path.len() as u64).saturating_mul(size_of::<Segment>() as u64);
        let id = self.allocate_id()?;
        self.budget.charge(bytes)?;
        self.outlines.insert(
            id,
            StoredOutline {
                rect_hint,
                quads: OnceLock::new(),
                segments: path.into(),
                bytes,
                quad_bytes: AtomicU64::new(0),
            },
        );
        Ok(OutlineId(id))
    }

    /// Validate an image against its own dimensions, price it, and store it.
    ///
    /// The one check is [`ImageSpec::is_consistent`]: no zero side, and a byte length
    /// that is exactly `width × height × 4`. A disagreement is
    /// [`ResourceProblem::ImageInconsistent`] carrying all three numbers, because a
    /// 60 000×60 000 claim over four bytes and four bytes claimed over a real buffer
    /// are different defects upstream. Bytes over the budget are
    /// [`DeviceError::ResourceBudgetExceeded`]; neither path stores or charges.
    pub(crate) fn upload_image(&mut self, image: &ImageSpec) -> Result<ImageId, DeviceError> {
        if !image.is_consistent() {
            return Err(DeviceError::InvalidResource {
                reason: ResourceProblem::ImageInconsistent {
                    width: image.width,
                    height: image.height,
                    bytes: image.data.len(),
                },
            });
        }
        let bytes = image.byte_size();
        let id = self.allocate_id()?;
        self.budget.charge(bytes)?;
        self.images.insert(
            id,
            StoredImage {
                spec: image.clone(),
                bytes,
            },
        );
        Ok(ImageId(id))
    }

    /// Validate a colour ramp's stops, price it, and store it.
    ///
    /// A ramp must have at least one stop, offsets that are finite, within `0..=1` and
    /// ascending, and colours that are valid. Ascending order is a requirement rather
    /// than something to sort into place: sampling walks the stops in the order given
    /// and interpolates between neighbours, so reordering them here would silently
    /// redraw a document's gradient rather than refuse it (§4.7). Each failure
    /// names itself — [`ResourceProblem::RampEmpty`],
    /// [`ResourceProblem::RampOffsetOutOfRange`], [`ResourceProblem::RampUnordered`],
    /// [`ResourceProblem::RampColorInvalid`] — and a ramp over the budget is
    /// [`DeviceError::ResourceBudgetExceeded`].
    pub(crate) fn upload_ramp(&mut self, stops: &[Stop]) -> Result<RampId, DeviceError> {
        if stops.is_empty() {
            return Err(DeviceError::InvalidResource {
                reason: ResourceProblem::RampEmpty,
            });
        }
        let mut previous = 0.0_f32;
        for stop in stops {
            if !stop.offset.is_finite() || !(0.0..=1.0).contains(&stop.offset) {
                return Err(DeviceError::InvalidResource {
                    reason: ResourceProblem::RampOffsetOutOfRange {
                        offset: stop.offset,
                    },
                });
            }
            if stop.offset < previous {
                return Err(DeviceError::InvalidResource {
                    reason: ResourceProblem::RampUnordered,
                });
            }
            previous = stop.offset;
            if !stop.color.is_valid() {
                return Err(DeviceError::InvalidResource {
                    reason: ResourceProblem::RampColorInvalid,
                });
            }
        }
        let bytes = (stops.len() as u64).saturating_mul(size_of::<Stop>() as u64);
        let id = self.allocate_id()?;
        self.budget.charge(bytes)?;
        self.ramps.insert(
            id,
            StoredRamp {
                stops: stops.into(),
                bytes,
            },
        );
        Ok(RampId(id))
    }

    /// Validate a pre-rasterised mesh, price it, and store it.
    ///
    /// A mesh is an image plus a device-space anchor, so the check is the image's:
    /// [`ResourceProblem::ImageInconsistent`] on a zero side or a byte length that
    /// disagrees with the dimensions, [`DeviceError::ResourceBudgetExceeded`] on the
    /// budget. The anchor is validated by nothing here because there is nothing to
    /// validate — `left` and `top` are integers, so no value of them is non-finite, and
    /// one that puts the raster off the target is culled at encode like any other mark.
    pub(crate) fn upload_mesh(&mut self, mesh: &MeshSpec) -> Result<MeshId, DeviceError> {
        if !mesh.image.is_consistent() {
            return Err(DeviceError::InvalidResource {
                reason: ResourceProblem::ImageInconsistent {
                    width: mesh.image.width,
                    height: mesh.image.height,
                    bytes: mesh.image.data.len(),
                },
            });
        }
        let bytes = mesh.byte_size();
        let id = self.allocate_id()?;
        self.budget.charge(bytes)?;
        self.meshes.insert(
            id,
            StoredMesh {
                spec: mesh.clone(),
                bytes,
            },
        );
        Ok(MeshId(id))
    }

    /// Admit a §7.10.5 program, price it, and store what a frame will ask of it.
    ///
    /// The admission is [`crate::function::admit`]: the structural check, the analysing
    /// walk, and ADR 0053 §3's agreement classification, in that order. It runs **here**
    /// rather than at the scene boundary because a program is a resource — so a caller
    /// learns its program is unsupported before it has built a scene at all, which is §5
    /// of the brief's "discoverable before the frame" satisfied properly rather than by
    /// accident.
    ///
    /// The content hash is computed by the walk, once, and kept with the analysis: it is
    /// what the pipeline cache keys on, and computing it per frame per command would make
    /// the cache's lookup cost a function of the page rather than of the program.
    pub(crate) fn upload_function(&mut self, program: &[FnOp]) -> Result<FunctionId, DeviceError> {
        let analysis = crate::function::admit(program)
            .map_err(|reason| DeviceError::InvalidFunction { reason })?;
        let bytes = analysis.stored_bytes();
        let id = self.allocate_id()?;
        self.budget.charge(bytes)?;
        self.functions
            .insert(id, StoredFunction { analysis, bytes });
        Ok(FunctionId(id))
    }

    /// Release a resource and return its bytes to the budget.
    ///
    /// An unknown or already-released id is an error, not a no-op: a double release
    /// is a caller bug, and hiding it would hide the defect (the departure from the
    /// brief's `()`-returning signature is recorded in `doc/PLAN.md`, integration
    /// note 7).
    pub(crate) fn release(&mut self, id: ResourceId) -> Result<(), DeviceError> {
        let freed = match id {
            // Both charges come back, and the second is zero unless a frame converted
            // this outline for the GPU coverage lane (ADR 0075). `into_inner` is sound
            // rather than merely convenient: a release owns the record it removed, so no
            // shared reference to the counter can exist.
            ResourceId::Outline(OutlineId(raw)) => self
                .outlines
                .remove(&raw)
                .map(|r| r.bytes.saturating_add(r.quad_bytes.into_inner())),
            ResourceId::Image(ImageId(raw)) => self.images.remove(&raw).map(|r| r.bytes),
            ResourceId::Ramp(RampId(raw)) => self.ramps.remove(&raw).map(|r| r.bytes),
            ResourceId::Mesh(MeshId(raw)) => self.meshes.remove(&raw).map(|r| r.bytes),
            ResourceId::Function(FunctionId(raw)) => self.functions.remove(&raw).map(|r| r.bytes),
        };
        match freed {
            Some(bytes) => {
                self.budget.refund(bytes);
                self.generation = self.generation.wrapping_add(1);
                Ok(())
            }
            None => Err(DeviceError::UnknownResource { id }),
        }
    }

    /// One id space across the five families, so a stale id of one kind can never
    /// alias a live resource of another — and an id is never reused, so a stale id of
    /// one *era* cannot alias a live resource of the next.
    ///
    /// **That second property is what `generation`'s claim rests on**, and until ADR
    /// 0050 audited it the counter wrapped: after `u32::MAX` uploads an id would have
    /// been reissued, `HashMap::insert` would have silently replaced whatever still held
    /// it, `generation` would not have moved — it counts releases — and a retained
    /// encode naming that id would have drawn the new resource through the old
    /// instances, with every check in `retained::EncodeKey` agreeing that nothing had
    /// changed. A `u32` is four billion uploads and no document reaches it; a wrong page
    /// that no counter can see is not a thing to leave behind a bound nobody stated.
    ///
    /// Called **before** the budget is charged, so that a refusal here charges nothing;
    /// the gap a later refusal leaves in the id space costs nothing, because ids are
    /// opaque and never enumerated.
    fn allocate_id(&mut self) -> Result<u32, DeviceError> {
        let next = self
            .next_id
            .checked_add(1)
            .ok_or(DeviceError::ResourceIdsExhausted { limit: u32::MAX })?;
        let id = self.next_id;
        self.next_id = next;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use quorra_scene::{ImageSpec, Point, ResourceId, Segment, Stop};

    use super::ResourceStore;
    use crate::error::{DeviceError, RenderError, ResourceProblem};

    fn square() -> Vec<Segment> {
        vec![
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(1.0, 0.0)),
            Segment::LineTo(Point::new(1.0, 1.0)),
            Segment::Close,
        ]
    }

    /// A curve, so that the conversion this file defers has real work to defer: a
    /// cubic subdivides until Loop and Blinn's bound holds, which is the 490
    /// instructions per segment ADR 0075 moved off the upload.
    fn curved() -> Vec<Segment> {
        vec![
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::CubicTo {
                c1: Point::new(0.0, 40.0),
                c2: Point::new(60.0, 40.0),
                to: Point::new(60.0, 0.0),
            },
            Segment::CubicTo {
                c1: Point::new(60.0, -40.0),
                c2: Point::new(0.0, -40.0),
                to: Point::new(0.0, 0.0),
            },
            Segment::Close,
        ]
    }

    /// What an upload charges is the segments and **nothing else** (ADR 0075): the
    /// converted form does not exist yet, and a budget counts what is resident.
    #[test]
    fn an_upload_charges_the_segments_and_converts_nothing() {
        let mut store = ResourceStore::new(u64::MAX);
        let id = store.upload_outline(&curved()).expect("valid outline");
        assert_eq!(
            store.in_use_bytes(),
            (curved().len() * size_of::<Segment>()) as u64,
            "the upload's charge is the segments it stored"
        );
        let stored = store.outline(id).expect("resident");
        assert!(
            !stored.quads_converted(),
            "no frame has asked for the GPU lane's geometry, so none was made"
        );
    }

    /// The first ask converts and charges; every ask after it is the same borrow and
    /// costs nothing. That is the whole of ADR 0075's contract at this level.
    #[test]
    fn the_first_ask_converts_and_charges_and_no_later_ask_does() {
        let mut store = ResourceStore::new(u64::MAX);
        let id = store.upload_outline(&curved()).expect("valid outline");
        let uploaded = store.in_use_bytes();

        let stored = store.outline(id).expect("resident");
        let first = stored
            .quads(store.budget())
            .expect("the budget is boundless");
        assert!(!first.is_empty(), "a closed cubic contour covers area");
        let triangles = first.triangle_count();
        let converted = store.in_use_bytes();
        assert!(
            converted > uploaded,
            "the conversion made bytes resident, so the ceiling counted them"
        );
        assert!(stored.quads_converted());

        let second = stored.quads(store.budget()).expect("already converted");
        assert_eq!(
            second.triangle_count(),
            triangles,
            "one conversion, re-read"
        );
        assert_eq!(
            store.in_use_bytes(),
            converted,
            "a second ask charges nothing, or a page of repeats would charge per mark"
        );
    }

    /// A release returns **both** charges, so a device that drew through the GPU lane
    /// and then released everything is a device with nothing resident.
    #[test]
    fn a_release_returns_the_conversions_charge_too() {
        let mut store = ResourceStore::new(u64::MAX);
        let id = store.upload_outline(&curved()).expect("valid outline");
        store
            .outline(id)
            .expect("resident")
            .quads(store.budget())
            .expect("the budget is boundless");
        assert!(store.in_use_bytes() > 0);
        store.release(ResourceId::Outline(id)).expect("release");
        assert_eq!(
            store.in_use_bytes(),
            0,
            "a conversion charged at frame time is refunded at release like any other byte"
        );
    }

    /// **The cost of charging honestly, stated as a test** (ADR 0075). A ceiling with
    /// exactly the segments' room admits the upload and refuses the conversion — by
    /// name, without charging, and without leaving a half-built form behind.
    #[test]
    fn a_conversion_over_the_ceiling_is_a_frames_refusal() {
        let segments_only = (curved().len() * size_of::<Segment>()) as u64;
        let mut store = ResourceStore::new(segments_only);
        let id = store.upload_outline(&curved()).expect("the segments fit");
        let stored = store.outline(id).expect("resident");
        match stored.quads(store.budget()) {
            Err(RenderError::OutlineConversionBudgetExceeded { needed, budget }) => {
                assert_eq!(budget, segments_only);
                assert!(
                    needed > budget,
                    "a refusal names what it would have come to"
                );
            }
            other => panic!("expected the conversion to be refused, got {other:?}"),
        }
        assert_eq!(
            store.in_use_bytes(),
            segments_only,
            "a refused conversion charges nothing"
        );
        assert!(
            !stored.quads_converted(),
            "and stores nothing, so a device given more room can ask again"
        );
    }

    /// Upload, reference, release: the bytes come back, and a second release of the
    /// same id is the loud error the module docs promise.
    #[test]
    fn release_returns_bytes_and_refuses_double_release() {
        let mut store = ResourceStore::new(1024);
        let id = store.upload_outline(&square()).expect("valid outline");
        assert!(store.in_use_bytes() > 0);
        store
            .release(ResourceId::Outline(id))
            .expect("first release");
        assert_eq!(store.in_use_bytes(), 0);
        assert!(matches!(
            store.release(ResourceId::Outline(id)),
            Err(DeviceError::UnknownResource { .. })
        ));
    }

    /// **An id is never reused, and the store says so rather than wrapping** (ADR 0050).
    ///
    /// The counter is wound to its last value rather than four billion uploads being
    /// performed, which is the only way to reach this at all — and is why it wrapped
    /// unnoticed until the retained encode's key was audited. A reissued id would have
    /// replaced a live resource inside `HashMap::insert`, moved no generation counter
    /// (that one counts releases) and left a retained encode drawing bytes it never
    /// named: a plausible wrong page, arrived at through arithmetic nobody had bounded.
    #[test]
    fn identifiers_run_out_loudly_rather_than_wrapping() {
        let mut store = ResourceStore::new(1 << 20);
        let first = store.upload_outline(&square()).expect("valid outline");
        store.next_id = u32::MAX;
        match store.upload_outline(&square()) {
            Err(DeviceError::ResourceIdsExhausted { limit }) => assert_eq!(limit, u32::MAX),
            other => panic!("an exhausted id space must refuse by name: {other:?}"),
        }
        assert_eq!(
            store.next_id,
            u32::MAX,
            "a refusal issues nothing and consumes nothing"
        );
        assert!(
            store.outline(first).is_some(),
            "and it leaves every resource that was already resident exactly where it was"
        );
    }

    /// The budget is checked before storing, and the refusal names all three numbers.
    #[test]
    fn budget_is_a_stated_refusal() {
        let mut store = ResourceStore::new(16);
        match store.upload_outline(&square()) {
            Err(DeviceError::ResourceBudgetExceeded {
                needed,
                in_use,
                budget,
            }) => {
                assert_eq!(budget, 16);
                assert_eq!(in_use, 0);
                assert!(needed > budget);
            }
            other => panic!("expected ResourceBudgetExceeded, got {other:?}"),
        }
        assert_eq!(store.in_use_bytes(), 0, "a refused upload must not charge");
    }

    /// §4.7 at the upload boundary: empty, headless, non-finite and oversized
    /// outlines are each refused by name.
    #[test]
    fn outline_validation_names_each_refusal() {
        let mut store = ResourceStore::new(u64::MAX);
        assert!(matches!(
            store.upload_outline(&[]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::OutlineEmpty
            })
        ));
        assert!(matches!(
            store.upload_outline(&[Segment::LineTo(Point::new(1.0, 1.0))]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::OutlineMissingMoveTo
            })
        ));
        assert!(matches!(
            store.upload_outline(&[Segment::MoveTo(Point::new(f32::NAN, 0.0))]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::OutlineNonFinite
            })
        ));
        assert!(matches!(
            store.upload_outline(&[Segment::MoveTo(Point::new(2e9, 0.0))]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::OutlineCoordinateTooLarge { .. }
            })
        ));
    }

    /// An inconsistent image — the 60 000×60 000 case arrives from real files — is
    /// refused before any allocation, and a consistent one is priced by its bytes.
    #[test]
    fn image_validation_and_pricing() {
        let mut store = ResourceStore::new(u64::MAX);
        let inconsistent = ImageSpec {
            width: 4,
            height: 4,
            data: Arc::from(vec![0_u8; 4].as_slice()),
        };
        assert!(matches!(
            store.upload_image(&inconsistent),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::ImageInconsistent { .. }
            })
        ));
        let good = ImageSpec {
            width: 2,
            height: 2,
            data: Arc::from(vec![0_u8; 16].as_slice()),
        };
        store.upload_image(&good).expect("consistent image");
        assert_eq!(store.in_use_bytes(), 16);
    }

    /// Ramp stops must be ascending, in range, and validly coloured.
    #[test]
    fn ramp_validation_names_each_refusal() {
        let mut store = ResourceStore::new(u64::MAX);
        let color = quorra_scene::Color::new(0.0, 0.0, 0.0, 1.0);
        assert!(matches!(
            store.upload_ramp(&[]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::RampEmpty
            })
        ));
        assert!(matches!(
            store.upload_ramp(&[Stop { offset: 1.5, color }]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::RampOffsetOutOfRange { .. }
            })
        ));
        assert!(matches!(
            store.upload_ramp(&[Stop { offset: 0.8, color }, Stop { offset: 0.2, color },]),
            Err(DeviceError::InvalidResource {
                reason: ResourceProblem::RampUnordered
            })
        ));
        store
            .upload_ramp(&[Stop { offset: 0.0, color }, Stop { offset: 1.0, color }])
            .expect("a valid two-stop ramp");
    }

    /// Ids never alias across families: an outline id released as an image id is
    /// unknown, not somebody else's resource.
    #[test]
    fn one_id_space_prevents_cross_family_aliasing() {
        let mut store = ResourceStore::new(u64::MAX);
        let outline = store.upload_outline(&square()).expect("valid outline");
        assert!(matches!(
            store.release(ResourceId::Image(quorra_scene::ImageId(outline.0))),
            Err(DeviceError::UnknownResource { .. })
        ));
    }
}
