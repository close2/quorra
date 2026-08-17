//! The surface-path smoke test: a real window, real presents, and pixels a person
//! (or `xwd`) can look at.
//!
//! §10.4 of the brief is blunt about why this class of test exists: "no gate we have
//! turns a page and every defect of three consecutive sessions lived there" — the
//! window path is where untested claims go to die. This example creates a window,
//! renders a known scene through `Target::Surface` for a fixed number of frames, and
//! exits 0 only if every present succeeded.
//!
//! Run headless: `Xvfb :77 & DISPLAY=:77 cargo run -p quorra-gpu --example
//! window_smoke` — lavapipe presents to Xvfb through the X fallback path, which is
//! the same arrangement the caller's CI uses. `QUORRA_SMOKE_ADAPTER` picks the
//! adapter; `QUORRA_SMOKE_HOLD_MS` keeps the window up after the counted frames so an
//! external probe (`xdotool` + `xwd`) can read the pixels back.
//!
//! The expected pixels: the window clears to transparent and draws one full-window
//! rectangle of (0.2, 0.4, 0.8, 1.0) with a (0.8, 0.2, 0.2, 1.0) square inset at its
//! centre — unorm 51/102/204 and 204/51/51, ±2 for the store-conversion bound of
//! ADR 0006.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_precision_loss)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{Affine, Color, Point, Rect, Scene, SceneBuilder};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

const FRAMES: u32 = 5;

fn scene(width: f32, height: f32) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(width, height)),
            Affine::IDENTITY,
            Color::new(0.2, 0.4, 0.8, 1.0),
            None,
            None,
        )
        .unwrap();
    let (cx, cy) = (width / 2.0, height / 2.0);
    builder
        .rect(
            Rect::new(
                Point::new(cx - 50.0, cy - 50.0),
                Point::new(cx + 50.0, cy + 50.0),
            ),
            Affine::IDENTITY,
            Color::new(0.8, 0.2, 0.2, 1.0),
            None,
            None,
        )
        .unwrap();
    builder.finish()
}

struct Smoke {
    window: Option<Arc<Window>>,
    device: Option<Device>,
    presented: u32,
    hold: Option<Duration>,
    holding_since: Option<Instant>,
}

impl ApplicationHandler for Smoke {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("quorra window smoke")
                        .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
                )
                .expect("window creation on the test display"),
        );
        let options = Options {
            adapter: std::env::var("QUORRA_SMOKE_ADAPTER").ok(),
            ..Options::default()
        };
        let device = Device::for_surface(Arc::clone(&window), &options).expect("surface device");
        eprintln!("window smoke on {}", device.description());
        self.device = Some(device);
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => {
                let (Some(window), Some(device)) = (self.window.as_ref(), self.device.as_mut())
                else {
                    return;
                };
                let size = window.inner_size();
                let viewport = Viewport::full(size.width, size.height, Affine::IDENTITY);
                let frame_scene = scene(size.width as f32, size.height as f32);
                match device.render(&frame_scene, &viewport, Target::Surface) {
                    Ok(frame) => {
                        if self.presented < FRAMES {
                            self.presented = self.presented.saturating_add(1);
                            eprintln!(
                                "frame {}/{FRAMES}: execute {:?} ({:?})",
                                self.presented,
                                frame.timings().execute,
                                frame.timings().execute_provenance,
                            );
                        }
                    }
                    Err(error) => {
                        // A refused frame fails the smoke test loudly.
                        eprintln!("present refused: {error}");
                        std::process::exit(1);
                    }
                }
                if self.presented < FRAMES {
                    window.request_redraw();
                } else if let Some(hold) = self.hold {
                    let since = *self.holding_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= hold {
                        event_loop.exit();
                    } else {
                        // Keep presenting while an external probe reads the window.
                        window.request_redraw();
                    }
                } else {
                    event_loop.exit();
                }
            }
            WindowEvent::CloseRequested => event_loop.exit(),
            _ => {}
        }
    }
}

/// `--check` is accepted and changes nothing: this example is already the smallest run
/// that reaches its own assertion — five frames presented to a real surface — and CI has
/// run it since ADR 0043. It is accepted so that every example takes the same invocation
/// (ADR 0060).
fn main() {
    if std::env::args().any(|arg| arg == "--check") {
        println!("check: this example is already the smallest run that asserts what it asserts");
    }
    let hold = std::env::var("QUORRA_SMOKE_HOLD_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis);
    let event_loop = EventLoop::new().expect("an event loop needs a display; set DISPLAY");
    let mut smoke = Smoke {
        window: None,
        device: None,
        presented: 0,
        hold,
        holding_since: None,
    };
    event_loop.run_app(&mut smoke).expect("event loop");
    assert_eq!(smoke.presented, FRAMES, "not every frame presented");
    println!("presented {FRAMES} frames");
}
