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

use std::collections::HashMap;

use quorra_scene::{
    ImageId, ImageSpec, MAX_COORDINATE, MeshId, MeshSpec, OutlineId, RampId, Rect, ResourceId,
    Segment, Stop, axis_aligned_rect,
};

use crate::error::{DeviceError, ResourceProblem};

/// A validated, resident outline.
#[derive(Debug)]
pub(crate) struct StoredOutline {
    /// The segments, as uploaded. Read every frame by the encoder — for device bounds,
    /// for flattening into the coverage lane, and for the segment counter — so this is
    /// the form the lanes work from, not an archive of the upload.
    pub segments: Box<[Segment]>,
    /// The same outline as closed contours of quadratic segments, for the GPU lane.
    ///
    /// Built here, once, and that is the whole reason the GPU lane's cost does not
    /// grow with magnification: the conversion depends on the outline and on nothing
    /// else, so a frame at 100× re-uses what the upload built (ADR 0016).
    pub quads: crate::outline::QuadOutline,
    /// The axis-aligned rectangle the outline traces, when it traces exactly one —
    /// recognised once, at upload, because §6.4 turns on it: a rectangular clip is
    /// four floats, never a mask, and the encoder asks this on every frame.
    pub rect_hint: Option<Rect>,
    bytes: u64,
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

/// The device's resource registry: four id spaces, one budget.
#[derive(Debug)]
pub(crate) struct ResourceStore {
    outlines: HashMap<u32, StoredOutline>,
    images: HashMap<u32, StoredImage>,
    ramps: HashMap<u32, StoredRamp>,
    meshes: HashMap<u32, StoredMesh>,
    next_id: u32,
    in_use_bytes: u64,
    budget_bytes: u64,
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
            next_id: 0,
            in_use_bytes: 0,
            budget_bytes,
            generation: 0,
        }
    }

    /// How many times a resource has been released from this store.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Bytes currently resident, for `Limits` and diagnostics.
    pub(crate) fn in_use_bytes(&self) -> u64 {
        self.in_use_bytes
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

    /// Validate an outline, price it, and store both the segments and the quadratic
    /// form the GPU lane needs.
    ///
    /// The path must be non-empty, must start with a `MoveTo` — nothing else has a
    /// current point to draw from (ISO 32000-2 §8.5.2) — and every coordinate must be
    /// finite and within `MAX_COORDINATE`. Each failure is a
    /// [`DeviceError::InvalidResource`] naming its own [`ResourceProblem`]; a path over
    /// the store's budget is [`DeviceError::ResourceBudgetExceeded`]. Either way
    /// nothing is stored and nothing is charged.
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
        let quads = crate::outline::QuadOutline::from_segments(path);
        // Both forms are charged: the quadratics are as resident as the segments, and
        // a budget that priced only half of what it stores would be a budget in name.
        let bytes = (path.len() as u64)
            .saturating_mul(size_of::<Segment>() as u64)
            .saturating_add(quads.stored_bytes());
        let id = self.allocate_id()?;
        self.charge(bytes)?;
        self.outlines.insert(
            id,
            StoredOutline {
                rect_hint,
                quads,
                segments: path.into(),
                bytes,
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
        self.charge(bytes)?;
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
        self.charge(bytes)?;
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
        self.charge(bytes)?;
        self.meshes.insert(
            id,
            StoredMesh {
                spec: mesh.clone(),
                bytes,
            },
        );
        Ok(MeshId(id))
    }

    /// Release a resource and return its bytes to the budget.
    ///
    /// An unknown or already-released id is an error, not a no-op: a double release
    /// is a caller bug, and hiding it would hide the defect (the departure from the
    /// brief's `()`-returning signature is recorded in `doc/PLAN.md`, integration
    /// note 7).
    pub(crate) fn release(&mut self, id: ResourceId) -> Result<(), DeviceError> {
        let freed = match id {
            ResourceId::Outline(OutlineId(raw)) => self.outlines.remove(&raw).map(|r| r.bytes),
            ResourceId::Image(ImageId(raw)) => self.images.remove(&raw).map(|r| r.bytes),
            ResourceId::Ramp(RampId(raw)) => self.ramps.remove(&raw).map(|r| r.bytes),
            ResourceId::Mesh(MeshId(raw)) => self.meshes.remove(&raw).map(|r| r.bytes),
            // No `upload_function` on this device yet, so no identifier it issued can be
            // one — `None` here is the `UnknownResource` below, which is the truth.
            ResourceId::Function(_) => None,
        };
        match freed {
            Some(bytes) => {
                self.in_use_bytes = self.in_use_bytes.saturating_sub(bytes);
                self.generation = self.generation.wrapping_add(1);
                Ok(())
            }
            None => Err(DeviceError::UnknownResource { id }),
        }
    }

    /// Count, then admit: the budget is checked before anything is stored.
    fn charge(&mut self, bytes: u64) -> Result<(), DeviceError> {
        let needed = self.in_use_bytes.saturating_add(bytes);
        if needed > self.budget_bytes {
            return Err(DeviceError::ResourceBudgetExceeded {
                needed,
                in_use: self.in_use_bytes,
                budget: self.budget_bytes,
            });
        }
        self.in_use_bytes = needed;
        Ok(())
    }

    /// One id space across the four families, so a stale id of one kind can never
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
    use crate::error::{DeviceError, ResourceProblem};

    fn square() -> Vec<Segment> {
        vec![
            Segment::MoveTo(Point::new(0.0, 0.0)),
            Segment::LineTo(Point::new(1.0, 0.0)),
            Segment::LineTo(Point::new(1.0, 1.0)),
            Segment::Close,
        ]
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
