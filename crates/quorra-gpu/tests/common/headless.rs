//! The headless device this suite renders through, and the pixels it hands back.
//!
//! One responsibility: ask for a device, ask it for a frame, read the frame. Nothing here
//! decides what is drawn — that is [`super::scene`]'s business — and nothing here asserts.

use quorra_gpu::{Device, Frame, Options, Target, Viewport};
use quorra_scene::{Affine, Scene};

/// The software adapter, as everywhere in this suite: deterministic, always present.
///
/// Pinned by name rather than read from `QUORRA_ADAPTER`, because a test that asserts a
/// byte is asserting it about a rasteriser. The `function_*.rs` files ask the other way
/// round on purpose and keep a device of their own.
pub fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

/// Draw `scene` into a `width` × `height` readback target and hand back its straight-alpha
/// RGBA bytes.
pub fn render(device: &mut Device, scene: &Scene, width: u32, height: u32) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(width, height, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("renders")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// The bytes of a frame already drawn, for the tests that hold the `Frame` first.
pub fn pixels(frame: Frame) -> Vec<u8> {
    frame.into_raster().unwrap().into_pixels()
}
