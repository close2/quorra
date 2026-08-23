//! The binding tables every pipeline is built against — and the half of a pipeline
//! that cannot be refused.
//!
//! A bind-group layout and a pipeline layout are descriptions: `wgpu` checks them
//! against nothing but themselves, and no shader source reaches them. That is why they
//! live apart from [`Modules`](super::Modules), which parse WGSL and therefore *can* be
//! refused by an adapter. The split is the one the store's fallibility follows: asking
//! for a layout is infallible, asking for a pipeline is not.
//!
//! Every entry here is the table its shader in `src/shaders/` assumes, entry for entry.
//! `min_binding_size` on each uniform makes a layout that disagrees with its WGSL a
//! validation error rather than a wrong picture.

/// A uniform buffer binding, with the size the shader's struct is — `min_binding_size`
/// makes a layout that disagrees with its WGSL a validation error rather than a wrong
/// picture.
fn uniform_entry(
    binding: u32,
    size: u64,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

/// A sampled texture read by `textureLoad` alone: exact fetches, no filtering (§4.6).
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The image's texture, the one filterable binding in the crate: linear filtering is the
/// placement's resolved decision (§4.5), so the hardware sampler must be usable on it.
fn filterable_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// The sampler that goes with it.
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// A uniform both stages read: a quad's, whose vertex stage places it and whose fragment
/// stage shades it from the same numbers.
const QUAD_UNIFORM: wgpu::ShaderStages =
    wgpu::ShaderStages::VERTEX.union(wgpu::ShaderStages::FRAGMENT);

/// The bind-group layouts alone, grouped so [`Layouts::new`] stays a composition of
/// named parts.
struct BindLayouts {
    globals: wgpu::BindGroupLayout,
    textures: wgpu::BindGroupLayout,
    image: wgpu::BindGroupLayout,
    shading: wgpu::BindGroupLayout,
    function: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
    reduce: wgpu::BindGroupLayout,
    blit: wgpu::BindGroupLayout,
    present: wgpu::BindGroupLayout,
    sampled: wgpu::BindGroupLayout,
    winding: wgpu::BindGroupLayout,
}

/// Every binding table and pipeline layout the store hands out, made once per device.
pub(crate) struct Layouts {
    pub(crate) globals: wgpu::BindGroupLayout,
    /// Group 1 of both lanes: atlas, scratch, soft mask.
    pub(crate) textures: wgpu::BindGroupLayout,
    /// Image quad: params, image (filterable), sampler, soft mask, scratch.
    pub(crate) image: wgpu::BindGroupLayout,
    /// Shading quad: params, paint (ramp or mesh), scratch, soft mask.
    pub(crate) shading: wgpu::BindGroupLayout,
    /// Function quad: params, scratch, soft mask — and **no paint texture**, which is the
    /// whole of what a device-evaluated colour changes about a shading's bindings
    /// (ADR 0053).
    pub(crate) function: wgpu::BindGroupLayout,
    /// Composite pass: params, backdrop, src, soft mask, scratch.
    pub(crate) composite: wgpu::BindGroupLayout,
    /// Reduce pass: params (with the transfer table), src.
    pub(crate) reduce: wgpu::BindGroupLayout,
    /// Blit pass: src, and where in it to read (ADR 0038).
    pub(crate) blit: wgpu::BindGroupLayout,
    /// Present pass: the placement's inverse, the layer (filterable), the sampler
    /// (ADR 0056).
    pub(crate) present: wgpu::BindGroupLayout,
    /// The winding resolve's one sampled texture.
    pub(crate) sampled: wgpu::BindGroupLayout,
    /// Winding and resolve passes: the sheet's globals, read by both stages.
    pub(crate) winding: wgpu::BindGroupLayout,
    pub(crate) lane_pipe: wgpu::PipelineLayout,
    pub(crate) image_pipe: wgpu::PipelineLayout,
    pub(crate) shading_pipe: wgpu::PipelineLayout,
    pub(crate) function_pipe: wgpu::PipelineLayout,
    pub(crate) composite_pipe: wgpu::PipelineLayout,
    pub(crate) reduce_pipe: wgpu::PipelineLayout,
    pub(crate) blit_pipe: wgpu::PipelineLayout,
    pub(crate) present_pipe: wgpu::PipelineLayout,
    pub(crate) winding_pipe: wgpu::PipelineLayout,
    pub(crate) resolve_pipe: wgpu::PipelineLayout,
}

impl Layouts {
    /// Every layout, in one place.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layouts = bind_layouts(device);
        let pipe_layout = |label: &str, groups: &[&wgpu::BindGroupLayout]| {
            let refs: Vec<Option<&wgpu::BindGroupLayout>> =
                groups.iter().map(|g| Some(*g)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &refs,
                immediate_size: 0,
            })
        };
        let lane_pipe = pipe_layout("quorra lane", &[&layouts.globals, &layouts.textures]);
        let image_pipe = pipe_layout("quorra image", &[&layouts.image]);
        let shading_pipe = pipe_layout("quorra shading", &[&layouts.shading]);
        let function_pipe = pipe_layout("quorra function", &[&layouts.function]);
        let composite_pipe = pipe_layout("quorra composite", &[&layouts.composite]);
        let reduce_pipe = pipe_layout("quorra reduce", &[&layouts.reduce]);
        let blit_pipe = pipe_layout("quorra blit", &[&layouts.blit]);
        let present_pipe = pipe_layout("quorra present", &[&layouts.present]);
        let winding_pipe = pipe_layout("quorra winding", &[&layouts.winding]);
        let resolve_pipe = pipe_layout(
            "quorra winding resolve",
            &[&layouts.winding, &layouts.sampled],
        );

        Self {
            globals: layouts.globals,
            textures: layouts.textures,
            image: layouts.image,
            shading: layouts.shading,
            function: layouts.function,
            composite: layouts.composite,
            reduce: layouts.reduce,
            blit: layouts.blit,
            present: layouts.present,
            sampled: layouts.sampled,
            winding: layouts.winding,
            lane_pipe,
            image_pipe,
            shading_pipe,
            function_pipe,
            composite_pipe,
            reduce_pipe,
            blit_pipe,
            present_pipe,
            winding_pipe,
            resolve_pipe,
        }
    }
}

/// Every bind-group layout: the binding tables the shaders in `src/shaders/` assume,
/// entry for entry. The entries themselves are named above.
fn bind_layouts(device: &wgpu::Device) -> BindLayouts {
    let make = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        })
    };
    BindLayouts {
        globals: make(
            "quorra globals",
            // Both stages, since ADR 0036: the vertex stage subtracts the
            // attachment's device origin before mapping to clip space and the
            // fragment stage adds it back to recover the device pixel it is
            // shading. Declaring it vertex-only is a validation error rather than a
            // wrong picture, which is the good kind — wgpu refused every pipeline
            // that read it from a fragment stage.
            &[uniform_entry(0, 16, QUAD_UNIFORM)],
        ),
        textures: make(
            "quorra lane sources",
            // Binding 3 is the mask's placement (ADR 0037), which belongs to the
            // mask rather than to the pass: `Globals` is written once per region and
            // one region's pass draws batches under different masks.
            &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                uniform_entry(3, 32, wgpu::ShaderStages::FRAGMENT),
            ],
        ),
        image: make(
            "quorra image",
            &[
                uniform_entry(0, 144, QUAD_UNIFORM),
                filterable_entry(1),
                sampler_entry(2),
                texture_entry(3),
                texture_entry(4),
            ],
        ),
        shading: make(
            "quorra shading",
            &[
                uniform_entry(0, 176, QUAD_UNIFORM),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
            ],
        ),
        // 208 bytes: the shading lane's numbers with the sweep's geometry replaced by
        // §8.7.4.5.2's domain rectangle, §7.10.1's range bounds and the background —
        // and one texture fewer, because the colour is computed rather than sampled.
        function: make(
            "quorra function",
            &[
                uniform_entry(0, 208, QUAD_UNIFORM),
                texture_entry(1),
                texture_entry(2),
            ],
        ),
        composite: make(
            "quorra composite",
            &[
                uniform_entry(0, 144, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
            ],
        ),
        reduce: make(
            "quorra reduce",
            &[
                uniform_entry(0, 288, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
            ],
        ),
        // The source origin (ADR 0038) is fragment-only: the pass is a full-screen
        // triangle whose vertex stage knows nothing about where it reads.
        blit: make(
            "quorra blit",
            &[
                texture_entry(0),
                uniform_entry(1, 16, wgpu::ShaderStages::FRAGMENT),
            ],
        ),
        // The present pass (ADR 0056). Filterable, unlike every other sampled texture in
        // this crate except the image's: a layer is put on the surface under an affine
        // the host chose, so the sample point does not land on texel centres and the
        // hardware sampler must be usable on it. 64 bytes: the placement's inverse (six
        // coefficients across two vec4s, the second carrying the filter), the rectangle
        // the vertex stage draws, then the layer's extent in texels. **Both stages**,
        // since ADR 0058: the vertex stage reads the rectangle and the fragment stage
        // everything else, which is the same `QUAD_UNIFORM` shape the lanes use.
        present: make(
            "quorra present",
            &[
                uniform_entry(0, 64, QUAD_UNIFORM),
                filterable_entry(1),
                sampler_entry(2),
            ],
        ),
        // The winding resolve's source. Identical in shape to `blit` before ADR 0038
        // gave that one an origin, and it was the same layout for exactly that
        // reason — a coincidence of shape, not a shared responsibility.
        sampled: make("quorra sampled texture", &[texture_entry(0)]),
        // Both stages read it: the vertex stage for the sheet's size and the band
        // it is drawing, the resolve fragment for the fill rule, the sample grid and
        // the same band.
        // 48 bytes: the sheet size, this draw's sample offset, the channel mask it
        // accumulates into, and the band's origin and height (ADR 0027).
        winding: make("quorra winding", &[uniform_entry(0, 48, QUAD_UNIFORM)]),
    }
}
