//! The two scenes this proof presents, and the textures they are rendered into.
//!
//! Both are chosen so that a *wrong* placement fails an assertion rather than passing
//! one: the page's mark is where the affine puts it and nowhere else, the chrome's mark
//! is in the corner the page cannot reach, and the window has a strip the page does not
//! cover so that "the page moved" and "the page is everywhere" are different pictures.

use quorra_gpu::Device;
use quorra_scene::{Affine, Color, Point, Rect, Scene, SceneBuilder};

/// The window, and therefore the chrome layer.
pub(crate) const WINDOW: (u32, u32) = (640, 480);
/// The page layer, half the window in each direction so that a 2× placement fits it.
pub(crate) const PAGE: (u32, u32) = (320, 240);

/// The page's field, and the colour a point outside its mark must have.
pub(crate) const FIELD: Color = Color {
    r: 0.2,
    g: 0.4,
    b: 0.8,
    a: 1.0,
};
/// The page's mark: a square whose position in the window is what the affine decides.
pub(crate) const MARK: Color = Color {
    r: 0.8,
    g: 0.2,
    b: 0.2,
    a: 1.0,
};
/// The chrome's mark, in the window's bottom-right corner.
pub(crate) const CHROME: Color = Color {
    r: 0.0,
    g: 0.8,
    b: 0.2,
    a: 1.0,
};

/// The page mark's rectangle, in the page texture's own pixels.
pub(crate) const MARK_RECT: (f32, f32, f32, f32) = (100.0, 60.0, 160.0, 120.0);
/// The chrome mark's rectangle, in the window's pixels.
pub(crate) const CHROME_RECT: (f32, f32, f32, f32) = (600.0, 440.0, 640.0, 480.0);

/// How many one-pixel rectangles the page carries beyond its two marks.
///
/// **They exist to make the encode take long enough to present over**, which is the
/// whole arrangement under test: a frame of the caller's real page is 57.7 to 913.1 ms
/// of processor time, and a fixture that encodes in a microsecond would prove nothing
/// about presenting *while* one runs. They are drawn in the field's own colour, in a
/// band no assertion samples, so they change no pixel this example checks.
const FILLER: u32 = 18_000;
/// The band the filler is drawn in, clear of every sampled point.
const FILLER_BAND: (f32, f32) = (180.0, 240.0);

/// A texture this device can render into **and** present: both usages, because they are
/// two different things and a layer needs both (`LayerProblem::NotSampleable`).
pub(crate) fn layer_texture(
    device: &Device,
    (width, height): (u32, u32),
    label: &str,
) -> wgpu::Texture {
    let (gpu, _) = device.wgpu();
    gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The page: a field, a mark inside it, and enough work to be worth presenting over.
pub(crate) fn page() -> Scene {
    let mut builder = SceneBuilder::new();
    let (width, height) = (PAGE.0 as f32, PAGE.1 as f32);
    rect(&mut builder, (0.0, 0.0, width, height), FIELD);
    rect(&mut builder, MARK_RECT, MARK);
    for index in 0..FILLER {
        let x = (index % PAGE.0) as f32;
        let y = FILLER_BAND.0 + f32::from((index / PAGE.0) as u16 % 60);
        assert!(y < FILLER_BAND.1);
        rect(&mut builder, (x, y, x + 1.0, y + 1.0), FIELD);
    }
    builder.finish()
}

/// The chrome: transparent, except one mark in the corner.
///
/// Transparent everywhere else on purpose — it is the layer that proves a slice
/// composes rather than replaces, and a chrome that covered the window would prove
/// nothing about what is under it.
pub(crate) fn chrome() -> Scene {
    let mut builder = SceneBuilder::new();
    rect(&mut builder, CHROME_RECT, CHROME);
    builder.finish()
}

/// What the window shows once the presenter is attached back: a full-window field with
/// the chrome's colour in the same corner, drawn through `Target::Surface` — a picture
/// the presenter could not have left behind, so seeing it is the proof that the device
/// has its surface again.
pub(crate) fn through_the_surface() -> Scene {
    let mut builder = SceneBuilder::new();
    let (width, height) = (WINDOW.0 as f32, WINDOW.1 as f32);
    rect(&mut builder, (0.0, 0.0, width, height), MARK);
    rect(&mut builder, CHROME_RECT, CHROME);
    builder.finish()
}

fn rect(builder: &mut SceneBuilder, (x0, y0, x1, y1): (f32, f32, f32, f32), color: Color) {
    builder
        .rect(
            Rect::new(Point::new(x0, y0), Point::new(x1, y1)),
            Affine::IDENTITY,
            color,
            None,
            None,
        )
        .expect("a rectangle inside the coordinate limit");
}
