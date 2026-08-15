//! One pass's bindings: the uniform bytes a shader's `Params` mirrors, and the bind
//! group that carries them and the textures the pass reads.
//!
//! Each function here writes a fixed-layout byte array at literal offsets. **Those
//! offsets are a contract with a WGSL struct that no compiler checks** — `wgpu` refuses
//! a buffer of the wrong size, and nothing at all catches a field at the wrong offset,
//! which is why every one of them names the shader it mirrors and the clause its pass
//! implements. Composite is ISO 32000-2 §11.4.5's group composition, reduce is §11.5's
//! soft mask, blit is pixels moved and not changed (ADR 0038, ADR 0039), and the
//! globals every pass reads are the attachment's extent and the device corner its
//! texel (0, 0) is (ADR 0036).
//!
//! The image and shading quads want the same thing for the rare-case lanes and get it
//! from `super::rare`. What any of these bindings is *drawn with* is `crate::compose`'s
//! throughout: this module knows a pass's inputs and not its order.

use super::Device;
use crate::compose::Region;
use crate::encode::{ChildOp, MaskPlan};
use crate::mask::MaskPlacement;

impl Device {
    /// The globals a pass rendering into `region` reads: the attachment's size, and the
    /// device corner its texel (0, 0) is (ADR 0036).
    pub(crate) fn region_globals(&self, region: Region) -> wgpu::BindGroup {
        #[allow(clippy::cast_precision_loss)] // extents inside f32's exact integer range
        let values = [
            region.width as f32,
            region.height as f32,
            region.x as f32,
            region.y as f32,
        ];
        let mut bytes = [0_u8; 16];
        for (slot, value) in bytes.chunks_exact_mut(4).zip(values) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        let buffer = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, &bytes);
        self.bind_globals(&buffer)
    }

    fn bind_globals(&self, globals: &wgpu::Buffer) -> wgpu::BindGroup {
        let layout = self.pipelines.globals_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra globals"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        })
    }

    /// The lane bind group: atlas, scratch, soft mask and where that mask sits
    /// (dummies and [`MaskPlacement::ABSENT`] where there is none).
    pub(crate) fn lane_bind(
        &self,
        atlas: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        mask: &wgpu::TextureView,
        placement: MaskPlacement,
    ) -> wgpu::BindGroup {
        let uniform = self.quad_uniform("quorra mask placement", &placement.bytes());
        let layout = self.pipelines.textures_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra lane sources"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(mask),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// The composite pass's uniform + bind group for one `ChildOp` (§11.4.5).
    // Offsets below are literal layout positions inside fixed 64/288-byte arrays;
    // the index arithmetic cannot leave them.
    #[allow(clippy::arithmetic_side_effects)]
    #[allow(clippy::too_many_arguments)] // one pass's inputs, named once at its one call
    #[allow(clippy::cast_precision_loss)] // extents inside f32's exact integer range
    pub(crate) fn composite_bind(
        &self,
        op: &ChildOp,
        region: Region,
        backdrop: (&wgpu::TextureView, Region),
        child: (&wgpu::TextureView, Region),
        mask: (&wgpu::TextureView, MaskPlacement),
        scratch: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let mut bytes = [0_u8; 128];
        bytes[0..4].copy_from_slice(&op.mode.to_le_bytes());
        bytes[4..8].copy_from_slice(&op.alpha.to_le_bytes());
        let non_isolated = u32::from(!op.isolated);
        bytes[8..12].copy_from_slice(&non_isolated.to_le_bytes());
        // The word `_pad1` used to hold: §11.4.6's stage this group is, if it is one.
        bytes[12..16].copy_from_slice(&op.compose.to_le_bytes());
        for (i, v) in op.clip_rect.iter().enumerate() {
            let at = 16 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[40..44].copy_from_slice(&op.residue_origin[0].to_le_bytes());
        bytes[44..48].copy_from_slice(&op.residue_origin[1].to_le_bytes());
        for (i, v) in op.residue_rect.iter().enumerate() {
            let at = 48 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        // Where this pass writes, and where the two textures it reads live (ADR 0036,
        // ADR 0038).
        for (i, v) in [
            region.x as f32,
            region.y as f32,
            child.1.x as f32,
            child.1.y as f32,
            child.1.width as f32,
            child.1.height as f32,
            backdrop.1.x as f32,
            backdrop.1.y as f32,
        ]
        .into_iter()
        .enumerate()
        {
            let at = 64 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[96..128].copy_from_slice(&mask.1.bytes());
        let uniform = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra composite params"),
            size: 128,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, &bytes);
        let layout = self.pipelines.composite_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra composite"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(backdrop.0),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(child.0),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask.0),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(scratch),
                },
            ],
        })
    }

    /// The reduce pass's uniform + bind group for one mask (§11.5; byte-agreed).
    #[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 288-byte array
    pub(crate) fn reduce_bind(&self, plan: &MaskPlan, src: &wgpu::TextureView) -> wgpu::BindGroup {
        let mut bytes = [0_u8; 288];
        bytes[0..4].copy_from_slice(&plan.kind_word.to_le_bytes());
        for (i, v) in plan.backdrop.iter().enumerate() {
            let at = 16 + i * 4;
            bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[32..288].copy_from_slice(&plan.table);
        let uniform = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quorra reduce params"),
            size: 288,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, &bytes);
        let layout = self.pipelines.reduce_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra reduce"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
            ],
        })
    }

    /// The blit pass's bind group: read from `origin` in a source `size` texels across,
    /// and write transparency outside it (ADR 0038, ADR 0039).
    ///
    /// `[0.0, 0.0]` is a copy between two textures of one size, which is §11.4.4's seed;
    /// a positive origin is the composite's backdrop, a rectangle inside its parent; a
    /// negative one is the frame's hand-off, whose destination is the whole target while
    /// its source is only what the page marks.
    pub(crate) fn blit_bind(
        &self,
        src: &wgpu::TextureView,
        origin: [f32; 2],
        size: [f32; 2],
    ) -> wgpu::BindGroup {
        let mut bytes = [0_u8; 16];
        bytes[0..4].copy_from_slice(&origin[0].to_le_bytes());
        bytes[4..8].copy_from_slice(&origin[1].to_le_bytes());
        bytes[8..12].copy_from_slice(&size[0].to_le_bytes());
        bytes[12..16].copy_from_slice(&size[1].to_le_bytes());
        let uniform = self.quad_uniform("quorra blit placement", &bytes);
        let layout = self.pipelines.blit_layout();
        self.gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quorra blit"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }

    /// One single-quad uniform buffer, written whole.
    pub(super) fn quad_uniform(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        let uniform = self.gpu.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform, 0, bytes);
        uniform
    }
}
