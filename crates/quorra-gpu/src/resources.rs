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
    /// The segments, as uploaded.
    #[allow(dead_code)] // consumed by the lanes from M4/M5; identity and budget are M2's part
    pub segments: Box<[Segment]>,
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
        }
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
        let bytes = (path.len() as u64).saturating_mul(size_of::<Segment>() as u64);
        self.charge(bytes)?;
        let id = self.allocate_id();
        self.outlines.insert(
            id,
            StoredOutline {
                rect_hint: axis_aligned_rect(path),
                segments: path.into(),
                bytes,
            },
        );
        Ok(OutlineId(id))
    }

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
        self.charge(bytes)?;
        let id = self.allocate_id();
        self.images.insert(
            id,
            StoredImage {
                spec: image.clone(),
                bytes,
            },
        );
        Ok(ImageId(id))
    }

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
        self.charge(bytes)?;
        let id = self.allocate_id();
        self.ramps.insert(
            id,
            StoredRamp {
                stops: stops.into(),
                bytes,
            },
        );
        Ok(RampId(id))
    }

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
        self.charge(bytes)?;
        let id = self.allocate_id();
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
        };
        match freed {
            Some(bytes) => {
                self.in_use_bytes = self.in_use_bytes.saturating_sub(bytes);
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
    /// alias a live resource of another.
    fn allocate_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
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
