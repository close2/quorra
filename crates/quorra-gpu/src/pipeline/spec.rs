//! What each pipeline *is*: one [`Kind`] per lane and pass, and the descriptor it
//! turns into.
//!
//! One place, so that adding a pass means adding an arm here and nothing else, and so
//! that `pipeline.rs` is the store's laziness and its warm-up rather than a table of
//! entry-point names. The three blend states below are the whole of ADR 0010's algebra
//! as `wgpu` factors, named once and referred to by name from the arms that use them.

use super::{Layouts, Modules};

/// Which pipeline: the two lanes in their three blend styles, plus the compositor's
/// own passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    /// Analytic rectangles, premultiplied over (`rect.wgsl` `fs_main`, ADR 0005).
    RectOver,
    /// Rectangle shape-erase for knockout (`fs_shape`, factors `ZERO`/`1-alpha`).
    RectErase,
    /// Rectangle additive deposit for knockout (`fs_main`, factors `ONE`/`ONE`).
    RectAdd,
    /// Coverage quads, premultiplied over (`coverage.wgsl` `fs_main`).
    CoverOver,
    /// Coverage shape-erase for knockout.
    CoverErase,
    /// Coverage additive deposit for knockout.
    CoverAdd,
    /// One image quad, premultiplied over (`image.wgsl` `fs_main`, ADR 0011).
    ImageOver,
    /// Image shape-erase for knockout (`fs_shape`).
    ImageErase,
    /// Image additive deposit for knockout.
    ImageAdd,
    /// One shading or mesh quad, premultiplied over (`shading.wgsl` `fs_main`,
    /// ISO 32000-2 §8.7.4.5).
    ShadedOver,
    /// Shading shape-erase for knockout (`fs_shape`).
    ShadedErase,
    /// Shading additive deposit for knockout.
    ShadedAdd,
    /// One finished layer onto its backdrop under a §11.3.5 blend mode
    /// (`composite.wgsl`; REPLACE, arithmetic wholly in-shader).
    Composite,
    /// Soft-mask reduction to R8 (`reduce.wgsl`; §11.5, byte-agreed).
    Reduce,
    /// Root layer to target, unchanged (`blit.wgsl`).
    Blit,
    /// Outline triangles accumulating a winding number (`winding.wgsl` `fs_winding`;
    /// additive, ADR 0016). The GPU coverage lane's first pass.
    Winding,
    /// Winding to coverage under a fill rule (`winding.wgsl` `fs_resolve`; REPLACE).
    WindingResolve,
}

/// Premultiplied over: `(ONE, ONE_MINUS_SRC_ALPHA)` on both channels. ADR 0010.
const OVER: wgpu::BlendState = both(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha);
/// Knockout's shape-erase: scale the backdrop by `1 − shape`, with shape in the source
/// alpha. ADR 0010.
const ERASE: wgpu::BlendState = both(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrcAlpha);
/// Knockout's deposit, and the winding lane's accumulation: `(ONE, ONE)`. ADR 0010.
const ADD: wgpu::BlendState = both(wgpu::BlendFactor::One, wgpu::BlendFactor::One);

/// One pair of factors on colour and alpha alike.
const fn both(src: wgpu::BlendFactor, dst: wgpu::BlendFactor) -> wgpu::BlendState {
    let component = wgpu::BlendComponent {
        src_factor: src,
        dst_factor: dst,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component,
        alpha: component,
    }
}

/// The analytic rectangle lane's instance: bounds and colour.
const RECT_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
/// The winding resolve's instance: one tile's rectangle and its sheet origin.
const RESOLVE_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2];
/// One outline triangle's vertex: position, sheet origin, and its ±1 channel mask.
const WINDING_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
/// The coverage lane's instance: placement, atlas rectangle, colour.
const COVER_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2, 1 => Float32x2, 2 => Float32x4,
    3 => Float32x4, 4 => Float32x4
];

/// Everything that distinguishes one [`Kind`] from another, in one place, so
/// [`PipelineStore::compile`] is the descriptor and nothing else.
pub(crate) struct Spec<'a> {
    pub(crate) label: &'static str,
    pub(crate) shader: &'a wgpu::ShaderModule,
    pub(crate) layout: &'a wgpu::PipelineLayout,
    /// The vertex entry point. Every lane shares `vs_main`; the winding pass
    /// has two stages of its own in one module.
    pub(crate) vertex: &'static str,
    pub(crate) entry: &'static str,
    pub(crate) blend: Option<wgpu::BlendState>,
    pub(crate) buffer: Option<(u64, &'static [wgpu::VertexAttribute])>,
    /// How the buffer advances. The lanes draw one quad per instance; the
    /// winding pass draws real triangles, one vertex at a time.
    pub(crate) step: wgpu::VertexStepMode,
    /// Four-corner strip quads; `false` for the full-screen triangle passes.
    pub(crate) strip: bool,
}

impl<'a> Spec<'a> {
    // One match arm per pipeline kind: irreducible dispatch, each arm a handful of
    // lines (clippy.toml's stated exception style).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn of(kind: Kind, layouts: &'a Layouts, modules: &'a Modules) -> Self {
        match kind {
            Kind::RectOver | Kind::RectErase | Kind::RectAdd => Spec {
                label: "quorra rect",
                shader: &modules.rect,
                layout: &layouts.lane_pipe,
                vertex: "vs_main",
                entry: if kind == Kind::RectErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::RectErase => ERASE,
                    Kind::RectAdd => ADD,
                    _ => OVER,
                }),
                buffer: Some((crate::encode::RECT_INSTANCE_STRIDE, &RECT_ATTRIBUTES)),
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::CoverOver | Kind::CoverErase | Kind::CoverAdd => Spec {
                label: "quorra coverage",
                shader: &modules.cover,
                layout: &layouts.lane_pipe,
                vertex: "vs_main",
                entry: if kind == Kind::CoverErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::CoverErase => ERASE,
                    Kind::CoverAdd => ADD,
                    _ => OVER,
                }),
                buffer: Some((crate::encode::QUAD_INSTANCE_STRIDE, &COVER_ATTRIBUTES)),
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::ImageOver | Kind::ImageErase | Kind::ImageAdd => Spec {
                label: "quorra image",
                shader: &modules.image,
                layout: &layouts.image_pipe,
                vertex: "vs_main",
                entry: if kind == Kind::ImageErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::ImageErase => ERASE,
                    Kind::ImageAdd => ADD,
                    _ => OVER,
                }),
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::ShadedOver | Kind::ShadedErase | Kind::ShadedAdd => Spec {
                label: "quorra shading",
                shader: &modules.shading,
                layout: &layouts.shading_pipe,
                vertex: "vs_main",
                entry: if kind == Kind::ShadedErase {
                    "fs_shape"
                } else {
                    "fs_main"
                },
                blend: Some(match kind {
                    Kind::ShadedErase => ERASE,
                    Kind::ShadedAdd => ADD,
                    _ => OVER,
                }),
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
            Kind::Composite => Spec {
                label: "quorra composite",
                shader: &modules.composite,
                layout: &layouts.composite_pipe,
                vertex: "vs_main",
                entry: "fs_main",
                blend: None, // REPLACE: the shader computes §11.3.6 wholly itself
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: false,
            },
            Kind::Reduce => Spec {
                label: "quorra reduce",
                shader: &modules.reduce,
                layout: &layouts.reduce_pipe,
                vertex: "vs_main",
                entry: "fs_main",
                blend: None,
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: false,
            },
            Kind::Blit => Spec {
                label: "quorra blit",
                shader: &modules.blit,
                layout: &layouts.blit_pipe,
                vertex: "vs_main",
                entry: "fs_main",
                blend: None,
                buffer: None,
                step: wgpu::VertexStepMode::Instance,
                strip: false,
            },
            // Additive, and that is the whole mechanism: what lands in the sheet is
            // the sum of every triangle's ±1, which is the winding number (ADR 0016).
            Kind::Winding => Spec {
                label: "quorra winding",
                shader: &modules.winding,
                layout: &layouts.winding_pipe,
                vertex: "vs_winding",
                entry: "fs_winding",
                blend: Some(ADD),
                buffer: Some((crate::outline::WindingVertex::STRIDE, &WINDING_ATTRIBUTES)),
                step: wgpu::VertexStepMode::Vertex,
                strip: false,
            },
            // Additive as well, for a different reason: each pass carries four of the
            // frame's samples and the sheet sums the groups.
            Kind::WindingResolve => Spec {
                label: "quorra winding resolve",
                shader: &modules.winding,
                layout: &layouts.resolve_pipe,
                vertex: "vs_resolve",
                entry: "fs_resolve",
                blend: Some(ADD),
                buffer: Some((crate::pane::TILE_STRIDE, &RESOLVE_ATTRIBUTES)),
                step: wgpu::VertexStepMode::Instance,
                strip: true,
            },
        }
    }
}
