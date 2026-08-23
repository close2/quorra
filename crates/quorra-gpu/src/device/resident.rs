//! A resource uploaded once and resident until released — in both the forms it has:
//! the validated copy `resources.rs` holds, and the texture a frame draws from.
//!
//! They are one module because they are two halves of one lifetime. An upload is where
//! §4.7's refusals and the resource budget are answered, and it hands back an
//! identifier the caller references many times (§2.2 of the brief: the caller keys
//! these by `Arc::as_ptr` identity, so a zoom re-uploads nothing). The *texture* an
//! image, ramp or mesh becomes is made on the first frame that draws it and not at
//! upload, because startup and a page of text must not pay for a picture nothing
//! placed (§7). And [`Device::release`] drops both, so the budget's word stays true on
//! the GPU side as well as on ours.
//!
//! Nothing here decides what a frame *needs*: `crate::encode` names the ids and this
//! module realises them, still refusing an id it cannot find by name rather than
//! trusting that the encode validated it.

use quorra_scene::{
    FnOp, FunctionId, ImageId, ImageSpec, MeshId, MeshSpec, OutlineId, RampId, ResourceId, Segment,
    Stop,
};

use super::Device;
use super::ramp::{RAMP_RESOLUTION, sample_ramp};
use crate::encode::Encoded;
use crate::error::{DeviceError, RenderError};

impl Device {
    /// Upload an outline: validated, priced against the resource budget, resident
    /// until [`Device::release`]. The id is what a scene's `fill`/`stroke`/`clip`
    /// reference — uploaded once, referenced many times (§2.2 of the brief: the
    /// caller keys these by `Arc::as_ptr` identity, so a zoom re-uploads nothing).
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidResource`] naming what §4.7 refused, or
    /// [`DeviceError::ResourceBudgetExceeded`] naming all three numbers. A device that
    /// has issued all `u32::MAX` identifiers refuses with
    /// [`DeviceError::ResourceIdsExhausted`] — ids are never reused, because a reissued
    /// one would make a retained encode draw a resource it did not name (ADR 0050).
    pub fn upload_outline(&mut self, path: &[Segment]) -> Result<OutlineId, DeviceError> {
        self.resources.upload_outline(path)
    }

    /// Upload a decoded image (straight-alpha RGBA8; the filtering decision arrives
    /// per placement on the command, M7 — integration note 1 in `doc/PLAN.md`).
    ///
    /// # Errors
    ///
    /// As [`Device::upload_outline`].
    pub fn upload_image(&mut self, image: &ImageSpec) -> Result<ImageId, DeviceError> {
        self.resources.upload_image(image)
    }

    /// Upload a colour ramp for the shadings of ISO 32000-2 §8.7.4.5 (drawn from M7).
    ///
    /// # Errors
    ///
    /// As [`Device::upload_outline`].
    pub fn upload_ramp(&mut self, stops: &[Stop]) -> Result<RampId, DeviceError> {
        self.resources.upload_ramp(stops)
    }

    /// Upload a pre-rasterised mesh (the caller's `MeshRaster`; integration note 5 —
    /// device-resolution by its design, so a zoom re-uploads meshes).
    ///
    /// # Errors
    ///
    /// As [`Device::upload_outline`].
    pub fn upload_mesh(&mut self, mesh: &MeshSpec) -> Result<MeshId, DeviceError> {
        self.resources.upload_mesh(mesh)
    }

    /// Upload a §7.10.5 type 4 function for [`Paint::Function`] to name (ADR 0053).
    ///
    /// **This is where a program is refused, and that placement is the whole point.** A
    /// program that cannot be lowered to a shader — a backward jump, a `roll` whose count
    /// came off the stack, a transcendental whose value reaches a comparison — is refused
    /// *here*, before the caller has built a scene, so that its fallback costs a branch
    /// rather than a page. `Device::render` never refuses a paint for anything about the
    /// program itself.
    ///
    /// One upload serves any number of shadings: the domain, matrix, range and background
    /// live on the paint, so two placements of one program share this identifier and the
    /// one generated shader it is cached by. What a page pays per *distinct* program is
    /// one shader compile, which is what `Scene::cost`'s `function_programs` counts.
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidFunction`] naming which of the upload's three questions was
    /// answered no — the structure, the analysing walk, or ADR 0053 §3's agreement
    /// classification — and by what. Also [`DeviceError::ResourceBudgetExceeded`] and
    /// [`DeviceError::ResourceIdsExhausted`], as [`Device::upload_outline`].
    ///
    /// [`Paint::Function`]: quorra_scene::Paint::Function
    pub fn upload_function(&mut self, program: &[FnOp]) -> Result<FunctionId, DeviceError> {
        self.resources.upload_function(program)
    }

    /// Release a resource and return its bytes to the budget.
    ///
    /// # Errors
    ///
    /// [`DeviceError::UnknownResource`] for an id this device never issued or already
    /// released — an error rather than a no-op, because a double release is a caller
    /// bug and hiding it would hide the defect (integration note 7 in `doc/PLAN.md`).
    pub fn release(&mut self, id: impl Into<ResourceId>) -> Result<(), DeviceError> {
        let id = id.into();
        // Read before the release, because the analysis that carries the hash is exactly
        // what the release removes.
        let released_program = match id {
            ResourceId::Function(program) => self
                .resources
                .function(program)
                .map(|stored| stored.analysis.program_hash()),
            _ => None,
        };
        self.resources.release(id)?;
        // The device-resident form goes with the CPU copy, so the budget's word
        // stays true on the GPU side too.
        match id {
            ResourceId::Image(ImageId(raw)) => {
                self.image_textures.remove(&raw);
            }
            ResourceId::Ramp(RampId(raw)) => {
                self.ramp_textures.remove(&raw);
            }
            ResourceId::Mesh(MeshId(raw)) => {
                self.mesh_textures.remove(&raw);
            }
            // A released program takes its compiled pipelines with it, unless another
            // resident program has the same instructions and would name the same key:
            // they are keyed by content, so keeping them would hold GPU memory for
            // something nothing can ask for again, and dropping one that is still
            // reachable would cost a recompile rather than a wrong picture.
            ResourceId::Function(_) => {
                if let Some(hash) = released_program
                    && !self.resources.holds_program(hash)
                {
                    self.pipelines.forget_program(hash);
                }
            }
            // An outline has no device-resident twin.
            ResourceId::Outline(_) => {}
        }
        Ok(())
    }

    /// Bytes currently resident across all uploaded resources, against
    /// [`Limits::max_resource_bytes`](crate::device::Limits::max_resource_bytes).
    ///
    /// **It can grow without an upload** (ADR 0075). An outline is converted into the
    /// GPU coverage lane's quadratics by the first frame that reads them, not by
    /// [`Device::upload_outline`], and those bytes are charged when they become
    /// resident. So a host on `Coverage::Cpu` sees only what it uploaded, and one that
    /// crosses into `Coverage::Gpu` sees this rise once per outline the page draws
    /// through that lane. Both readings are true: this counts what is resident, and
    /// what is resident is what the ceiling bounds.
    #[must_use]
    pub fn resource_bytes_in_use(&self) -> u64 {
        self.resources.in_use_bytes()
    }

    /// The admitted analysis of a resident §7.10.5 program, for the lane that draws it.
    ///
    /// The frame reads it rather than re-deriving it: everything static about a program
    /// was decided once at [`Device::upload_function`], and a second answer computed
    /// per frame would be a second answer.
    pub(crate) fn function_analysis(
        &self,
        program: FunctionId,
    ) -> Option<&crate::function::Analysis> {
        self.resources
            .function(program)
            .map(|stored| &stored.analysis)
    }

    /// Realise the frame's referenced images, ramps and meshes as textures, once
    /// per resident resource — created here rather than at upload so startup and
    /// pages without them never pay (§7). Returns the bytes written.
    ///
    /// The ids were validated during encode; a miss here still refuses by name
    /// rather than trusting that invariant silently.
    pub(super) fn ensure_paint_textures(&mut self, encoded: &Encoded) -> Result<u64, RenderError> {
        let mut bytes = 0_u64;
        for &id in &encoded.used_images {
            if self.image_textures.contains_key(&id) {
                continue;
            }
            let Some(stored) = self.resources.image(ImageId(id)) else {
                return Err(RenderError::UnknownImage { image: ImageId(id) });
            };
            let spec = stored.spec.clone();
            let pair = self.rgba_texture("quorra image", spec.width, spec.height, &spec.data);
            bytes = bytes.saturating_add(spec.data.len() as u64);
            self.image_textures.insert(id, pair);
        }
        for &id in &encoded.used_ramps {
            if self.ramp_textures.contains_key(&id) {
                continue;
            }
            let Some(stored) = self.resources.ramp(RampId(id)) else {
                return Err(RenderError::UnknownRamp { ramp: RampId(id) });
            };
            let samples = sample_ramp(&stored.stops);
            let pair = self.rgba_texture("quorra ramp", RAMP_RESOLUTION, 1, &samples);
            bytes = bytes.saturating_add(samples.len() as u64);
            self.ramp_textures.insert(id, pair);
        }
        for &id in &encoded.used_meshes {
            if self.mesh_textures.contains_key(&id) {
                continue;
            }
            let Some(stored) = self.resources.mesh(MeshId(id)) else {
                return Err(RenderError::UnknownMesh { mesh: MeshId(id) });
            };
            let spec = stored.spec.image.clone();
            let pair = self.rgba_texture("quorra mesh", spec.width, spec.height, &spec.data);
            bytes = bytes.saturating_add(spec.data.len() as u64);
            self.mesh_textures.insert(id, pair);
        }
        Ok(bytes)
    }
}
