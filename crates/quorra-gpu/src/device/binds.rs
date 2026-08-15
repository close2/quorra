//! One pass's bindings: the uniform bytes a shader's `Params` mirrors, and the bind
//! group that carries them and the textures the pass reads.
//!
//! Each `*_bytes` function here writes a fixed-layout byte array at literal offsets.
//! **Those offsets are a contract with a WGSL struct that no compiler checks** — `wgpu`
//! refuses a buffer of the wrong size, and nothing in the toolchain catches a field at
//! the wrong offset, which is why every one of them names the shader it mirrors and the
//! clause its pass implements. Composite is ISO 32000-2 §11.4.5's group composition,
//! reduce is §11.5's soft mask, blit is pixels moved and not changed (ADR 0038,
//! ADR 0039), and the globals every pass reads are the attachment's extent and the
//! device corner its texel (0, 0) is (ADR 0036).
//!
//! Each is a function of its arguments and nothing else, apart from the bind group that
//! carries it, because that is what lets the `tests` module below hand one a distinct
//! value per field and find each one at the offset `crate::shaders::layout` derives from
//! the WGSL. A byte array built inside `create_bind_group`'s argument list would be a
//! contract only a rendered frame could check.
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
        let bytes = globals_bytes(region);
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
    #[allow(clippy::too_many_arguments)] // one pass's inputs, named once at its one call
    pub(crate) fn composite_bind(
        &self,
        op: &ChildOp,
        region: Region,
        backdrop: (&wgpu::TextureView, Region),
        child: (&wgpu::TextureView, Region),
        mask: (&wgpu::TextureView, MaskPlacement),
        scratch: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let bytes = composite_params_bytes(op, region, backdrop.1, child.1, mask.1);
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
    pub(crate) fn reduce_bind(&self, plan: &MaskPlan, src: &wgpu::TextureView) -> wgpu::BindGroup {
        let bytes = reduce_params_bytes(plan);
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
        let uniform =
            self.quad_uniform("quorra blit placement", &blit_placement_bytes(origin, size));
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

/// The 16 bytes `rect.wgsl`'s and `coverage.wgsl`'s `Globals` read: the attachment's
/// size, then the device corner its texel (0, 0) is (ADR 0036).
#[allow(clippy::cast_precision_loss)] // extents inside f32's exact integer range
fn globals_bytes(region: Region) -> [u8; 16] {
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
    bytes
}

/// The 128 bytes `composite.wgsl`'s `Params` reads, in its order (§11.4.5).
// The offsets are literal layout positions inside a fixed 128-byte array; the index
// arithmetic cannot leave it.
#[allow(clippy::arithmetic_side_effects)]
#[allow(clippy::cast_precision_loss)] // extents inside f32's exact integer range
fn composite_params_bytes(
    op: &ChildOp,
    region: Region,
    backdrop: Region,
    child: Region,
    mask: MaskPlacement,
) -> [u8; 128] {
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
    // `residue.xy` is left zero: the shader reads the residue region's corner from
    // `residue_rect`, and only `.zw` — the scratch texel origin — from this field.
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
        child.x as f32,
        child.y as f32,
        child.width as f32,
        child.height as f32,
        backdrop.x as f32,
        backdrop.y as f32,
    ]
    .into_iter()
    .enumerate()
    {
        let at = 64 + i * 4;
        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    bytes[96..128].copy_from_slice(&mask.bytes());
    bytes
}

/// The 288 bytes `reduce.wgsl`'s `Params` reads, in its order (§11.5).
#[allow(clippy::arithmetic_side_effects)] // fixed-layout offsets in a 288-byte array
fn reduce_params_bytes(plan: &MaskPlan) -> [u8; 288] {
    let mut bytes = [0_u8; 288];
    bytes[0..4].copy_from_slice(&plan.kind_word.to_le_bytes());
    for (i, v) in plan.backdrop.iter().enumerate() {
        let at = 16 + i * 4;
        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    bytes[32..288].copy_from_slice(&plan.table);
    bytes
}

/// The 16 bytes `blit.wgsl`'s `Placement` reads: the source texel this pass's texel
/// (0, 0) takes, and how far the source extends (ADR 0038, ADR 0039).
fn blit_placement_bytes(origin: [f32; 2], size: [f32; 2]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    bytes[0..4].copy_from_slice(&origin[0].to_le_bytes());
    bytes[4..8].copy_from_slice(&origin[1].to_le_bytes());
    bytes[8..12].copy_from_slice(&size[0].to_le_bytes());
    bytes[12..16].copy_from_slice(&size[1].to_le_bytes());
    bytes
}

/// Each writer above against the WGSL struct it mirrors, field by field.
///
/// Every field gets a value no other field has, so a pair exchanged inside the array
/// moves a value the check can name. The offsets come from `shaders::layout`, which
/// derives them from the shader source; nothing here states one.
#[cfg(test)]
mod tests {
    use super::{
        ChildOp, MaskPlacement, MaskPlan, Region, blit_placement_bytes, composite_params_bytes,
        globals_bytes, reduce_params_bytes,
    };
    use crate::shaders;
    use crate::shaders::layout::{Lane, check};

    /// The globals both instanced lanes read. Two shaders declare the struct, textually
    /// identically, and both are checked: a copy that drifts is a lane drawn at the
    /// wrong origin, which is the defect ADR 0028 shipped with once already.
    #[test]
    fn the_region_globals_are_the_two_lanes_globals() {
        let bytes = globals_bytes(Region {
            x: 11,
            y: 12,
            width: 13,
            height: 14,
        });
        for source in [shaders::RECT, shaders::COVERAGE] {
            check(
                source,
                "Globals",
                &bytes,
                &[
                    ("target_size", Lane::Vec2([13.0, 14.0])),
                    ("origin", Lane::Vec2([11.0, 12.0])),
                ],
            );
        }
    }

    /// The composite pass's uniform (§11.4.5), whose four leading words are the one
    /// place in the crate where scalars of different types sit side by side.
    #[test]
    fn the_composite_uniform_is_composites_params() {
        let op = ChildOp {
            layer: 0,
            mode: 7,
            alpha: 0.25,
            clip_rect: [1.0, 2.0, 3.0, 4.0],
            residue_rect: [5.0, 6.0, 7.0, 8.0],
            residue_origin: [9.0, 10.0],
            compose: 2,
            mask: None,
            isolated: false,
        };
        let placement = MaskPlacement {
            origin: [21.0, 22.0],
            size: [23.0, 24.0],
            outside: 0.5,
        };
        let bytes = composite_params_bytes(
            &op,
            Region {
                x: 31,
                y: 32,
                width: 0,
                height: 0,
            },
            Region {
                x: 41,
                y: 42,
                width: 0,
                height: 0,
            },
            Region {
                x: 51,
                y: 52,
                width: 53,
                height: 54,
            },
            placement,
        );
        check(
            shaders::COMPOSITE,
            "Params",
            &bytes,
            &[
                ("mode", Lane::Word(7)),
                ("alpha", Lane::Float(0.25)),
                // `isolated: false` is §11.4.4's non-isolated group, which is the 1.
                ("non_isolated", Lane::Word(1)),
                ("compose", Lane::Word(2)),
                ("clip", Lane::Vec4([1.0, 2.0, 3.0, 4.0])),
                ("residue", Lane::Vec4([0.0, 0.0, 9.0, 10.0])),
                ("residue_rect", Lane::Vec4([5.0, 6.0, 7.0, 8.0])),
                ("origin", Lane::Vec2([31.0, 32.0])),
                ("child_origin", Lane::Vec2([51.0, 52.0])),
                ("child_size", Lane::Vec2([53.0, 54.0])),
                ("backdrop_origin", Lane::Vec2([41.0, 42.0])),
                ("mask_rect", Lane::Vec4([21.0, 22.0, 23.0, 24.0])),
                ("mask_outside", Lane::Vec4([0.5, 0.0, 0.0, 0.0])),
            ],
        );
    }

    /// The reduce pass's uniform (§11.5), and with it the only array a uniform in this
    /// crate holds: §11.6.5.1's 256-byte transfer table, packed four bytes to a word.
    #[test]
    fn the_reduce_uniform_is_reduces_params() {
        // A table whose every byte differs from its neighbours, so a word swapped
        // inside it is a difference and not a coincidence.
        let mut table = [0_u8; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = u8::try_from(i).expect("the table is 256 long");
        }
        let plan = MaskPlan {
            root: 0,
            kind_word: 1,
            backdrop: [0.125, 0.25, 0.375],
            table,
        };
        let bytes = reduce_params_bytes(&plan);
        check(
            shaders::REDUCE,
            "Params",
            &bytes,
            &[
                ("kind", Lane::Word(1)),
                // Declared to hold the struct's first `vec4f` at 16, and written by
                // nobody; the shader reads neither.
                ("_pad", Lane::Float(0.0)),
                ("_pad2", Lane::Vec2([0.0, 0.0])),
                // The backdrop's alpha is not written: §11.6.5.1 makes it opaque, and
                // the shader takes only `.rgb`.
                ("backdrop", Lane::Vec4([0.125, 0.25, 0.375, 0.0])),
                ("table", Lane::Bytes(&table)),
            ],
        );
    }

    /// The blit pass's placement (ADR 0038, ADR 0039), whose two `vec2f` are the pair
    /// most easily exchanged in the crate: same type, same width, adjacent.
    #[test]
    fn the_blit_uniform_is_blits_placement() {
        let bytes = blit_placement_bytes([-3.0, -4.0], [5.0, 6.0]);
        check(
            shaders::BLIT,
            "Placement",
            &bytes,
            &[
                ("origin", Lane::Vec2([-3.0, -4.0])),
                ("size", Lane::Vec2([5.0, 6.0])),
            ],
        );
    }
}
