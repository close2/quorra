//! The headless device this suite renders through, and the pixels it hands back.
//!
//! One responsibility: ask for a device, ask it for a frame, read the frame. Nothing here
//! decides what is drawn, and nothing here asserts.

use quorra_gpu::Frame;

/// The bytes of a frame already drawn, for the tests that hold the `Frame` first.
pub fn pixels(frame: Frame) -> Vec<u8> {
    frame.into_raster().unwrap().into_pixels()
}
