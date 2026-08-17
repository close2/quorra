//! **The proof of ADR 0056**: a page rendered on one thread while the window is
//! presented from another, and the pixels read back to show where the affine put them.
//!
//! This is the surface path's second smoke test and it exists for the reason §10.4 of
//! the brief gives for the first one — "no gate we have turns a page and every defect of
//! three consecutive sessions lived there". A `Presenter` that compiles and is `Send` is
//! not evidence of anything; a window whose pixels are where a non-identity affine says
//! they are, produced while a `&mut Device` was busy on another thread, is.
//!
//! Run headless, which is what CI does:
//!
//! ```text
//! xvfb-run -a cargo run -p quorra-gpu --example present_thread
//! ```
//!
//! `QUORRA_PRESENT_ADAPTER` picks the adapter. `xwd` (Debian/Ubuntu: `x11-apps`) must be
//! present — the example reads its own window back and fails loudly without it.
//!
//! # What it asserts, in the order it does
//!
//! 1. A device built for a surface hands out **one** presenter, and a second ask gets
//!    `None`.
//! 2. That presenter refuses to present **by name** before it is told a size, and again
//!    when the size it is told has no pixels in it — rather than configuring a swapchain
//!    for a window nobody described.
//! 3. The device — now on another thread — refuses `Target::Surface` with
//!    `PresenterDetached`, and goes on rendering into a host texture.
//! 4. Presents happen **while that render is in flight**, on the main thread, counted.
//! 5. The finished page goes on the window under a 2× placement offset by (64, 32),
//!    with the chrome over it at the identity, and every sampled pixel is where those
//!    two affines put it — including the strip the page does not reach, which is the
//!    assertion that would pass at the identity and must not, and the page's own first
//!    row and column, which is the rectangle ADR 0058 draws seen from both sides.
//! 6. The same page presented under `ImageFilter::Linear` — the sampler branch, whose
//!    normalised coordinates nothing else here exercises — still lands where the affine
//!    says, and alone on the window, because a slice is what is shown and not what was.
//! 7. The presenter is refused by a device that did not hand it out, and comes back
//!    intact inside the refusal.
//! 8. Attached to its own device, `Target::Surface` draws again, and the window shows a
//!    picture the presenter could not have produced.
//! 9. `PresentCost` says what it should: two layers, a swapchain configured once, and
//!    **no pipeline compiled** — the presenting pass was in the warm set (ADR 0043,
//!    ADR 0056), which is the end-to-end half of "detaching compiles nothing".

// An example's arithmetic is window coordinates, byte offsets inside a file it just
// read, and small counts — all bounded and all exact in the types they use, where the
// library's own lints are stricter because its inputs are not. `expect` and `panic` are
// the policy every example in this tree follows: a proof that cannot run must fail
// loudly rather than report success.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

mod fixture;
mod xwd;

use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use quorra_gpu::{Device, Layer, Options, PresentCost, Presenter, RenderError, Target, Viewport};
use quorra_scene::{Affine, Color, ImageFilter};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::{Window, WindowId};

/// The window's title, which is also how `xwd` finds it.
const TITLE: &str = "quorra presenter thread";

/// The page's placement on the window: two device pixels per page texel, offset so that
/// a strip of the window is left uncovered. Both halves matter — the scale is what a
/// filter has to answer for, and the offset is what makes "the page is everywhere"
/// distinguishable from "the page is where the affine says".
const SCALE: f32 = 2.0;
const OFFSET: (f32, f32) = (64.0, 32.0);

/// ADR 0006's store-conversion bound, which is what a colour is allowed to differ by
/// between what the scene stated and what the window shows.
const TOLERANCE: u8 = 2;

/// How long to keep presenting the same picture before reading the window back. The X
/// server is on the other side of a socket; this is the one place a wall clock appears
/// in this example, and it is a wait rather than a measurement.
const SETTLE: Duration = Duration::from_millis(300);

/// `--check` is accepted and changes nothing: this example is already an assertion
/// harness that runs its whole set once, and CI has run it since ADR 0056. It is
/// accepted so that every example takes the same invocation (ADR 0060).
fn main() {
    if std::env::args().any(|arg| arg == "--check") {
        eprintln!("check: this example is already the smallest run that asserts what it asserts");
    }
    let mut display = Display::open();
    let options = Options {
        adapter: std::env::var("QUORRA_PRESENT_ADAPTER").ok(),
        ..Options::default()
    };
    let mut device =
        Device::for_surface(Arc::clone(&display.window), &options).expect("surface device");
    eprintln!("present_thread on {}", device.description());
    // Waited for on purpose: it is what makes assertion 9 mean something. The warm set
    // of a device built for a surface includes the presenting pass, so a present that
    // reports a compile after this line is a regression in ADR 0056's warm-set decision.
    device.wait_until_warm();

    let (window_width, window_height) = fixture::WINDOW;
    let chrome = fixture::layer_texture(&device, fixture::WINDOW, "chrome");
    let page = fixture::layer_texture(&device, fixture::PAGE, "page");
    device
        .render(
            &fixture::chrome(),
            &Viewport::full(window_width, window_height, Affine::IDENTITY),
            Target::Texture(&chrome),
        )
        .expect("the chrome renders into a host texture");

    let mut presenter = device
        .detach_presenter()
        .expect("a surface device hands one out");
    assert!(
        device.detach_presenter().is_none(),
        "a device hands out one presenter, and the second ask gets nothing"
    );
    assert!(
        presenter.last().is_none(),
        "a presenter that has presented nothing says so"
    );
    match presenter.present(&[]) {
        Err(RenderError::PresenterUnsized) => {}
        other => panic!("a presenter with no size must refuse by name, got {other:?}"),
    }
    // A minimised window: a state rather than an error, and the present that meets it is
    // refused by name rather than configuring a swapchain with no pixels in it.
    presenter.resize(0, window_height);
    match presenter.present(&[]) {
        Err(RenderError::ZeroSizeTarget { target: "Surface" }) => {}
        other => panic!("a window with no pixels must refuse by name, got {other:?}"),
    }
    presenter.resize(window_width, window_height);

    let device = render_on_another_thread_while_presenting(device, &page, &chrome, &mut presenter);

    let placement = Affine::scale(SCALE, SCALE).then(Affine::translate(OFFSET.0, OFFSET.1));
    let layers = [
        Layer {
            texture: &page,
            placement,
            filter: ImageFilter::Nearest,
        },
        Layer {
            texture: &chrome,
            placement: Affine::IDENTITY,
            filter: ImageFilter::Nearest,
        },
    ];
    present_until_settled(&mut display, &mut presenter, &layers);
    check_the_affine_landed();
    check_the_cost(&presenter.last().expect("two layers were just presented"));
    check_the_other_filter_runs(&mut display, &mut presenter, &page, placement);

    let mut device = give_it_back(device, presenter, &options);
    draw_through_the_surface_again(&mut display, &mut device);

    println!("present_thread: the window is where the affine says, on both paths");
}

/// Steps 3 and 4: the device goes to another thread, refuses `Target::Surface` by name,
/// renders the page into a host texture — and the main thread presents while it does.
fn render_on_another_thread_while_presenting(
    mut device: Device,
    page: &wgpu::Texture,
    chrome: &wgpu::Texture,
    presenter: &mut Presenter,
) -> Device {
    let (started, has_started) = mpsc::channel();
    let (finished, has_finished) = mpsc::channel();
    let page_for_render = page.clone();
    let render = thread::spawn(move || {
        let viewport = Viewport::full(fixture::PAGE.0, fixture::PAGE.1, Affine::IDENTITY);
        match device.render(&fixture::page(), &viewport, Target::Surface) {
            Err(RenderError::PresenterDetached) => {}
            other => panic!("a detached device must refuse Target::Surface by name, got {other:?}"),
        }
        started.send(()).expect("the main thread is waiting");
        let began = Instant::now();
        device
            .render(
                &fixture::page(),
                &viewport,
                Target::Texture(&page_for_render),
            )
            .expect("the page renders into a host texture while the window is presented");
        finished
            .send(began.elapsed())
            .expect("the main thread is waiting");
        device
    });

    has_started.recv().expect("the render thread started");
    let chrome_only = [Layer {
        texture: chrome,
        placement: Affine::IDENTITY,
        filter: ImageFilter::Nearest,
    }];
    let mut presents = 0_u32;
    let render_took = loop {
        presenter
            .present(&chrome_only)
            .expect("presenting while the device renders elsewhere");
        presents += 1;
        if let Ok(took) = has_finished.try_recv() {
            break took;
        }
    };
    let device = render.join().expect("the render thread finished");
    // The number this example exists for, printed rather than only asserted: how many
    // pictures the window received during one `Device::render` — against the one the
    // caller's arrangement can produce today, where the render holds the only
    // `&mut Device` and nothing may present until it returns. Both figures are wall
    // clocks on a machine running a test suite, so they are a ratio worth reading and
    // not a measurement worth quoting.
    println!("presents while one render of {render_took:?} was in flight: {presents}");
    assert!(
        presents >= 2,
        "the point of the split is presenting during a render; only {presents} got through"
    );
    device
}

/// Step 5's pixels: every sampled point, and why it is that colour.
fn check_the_affine_landed() {
    let shot = xwd::capture(TITLE);
    assert_eq!(
        shot.size(),
        (fixture::WINDOW.0 as usize, fixture::WINDOW.1 as usize),
        "the window `xwd` found is not the size this example asked for"
    );
    let page_pixel = |x: f32, y: f32| -> (usize, usize) {
        // Where a page texel's centre lands on the window, which is the placement
        // applied forwards — the assertion is that the shader's inverse agrees.
        (
            (x * SCALE + OFFSET.0) as usize,
            (y * SCALE + OFFSET.1) as usize,
        )
    };
    // Outside the page and outside the chrome: the clear, which is transparent (§3) and
    // which an opaque X visual shows as black. **This is the assertion that fails at the
    // identity**, where the page would cover the window's top-left corner.
    same(
        shot.at(8, 8),
        [0, 0, 0],
        "the strip the page does not reach",
    );
    // The page's field, at a page texel outside its mark.
    let (x, y) = page_pixel(68.0, 84.0);
    same(shot.at(x, y), rgb(fixture::FIELD), "the page's field");
    // The page's mark, whose position on the window is the whole question.
    let (x, y) = page_pixel(130.0, 90.0);
    same(shot.at(x, y), rgb(fixture::MARK), "the page's mark");
    // One pixel to the left of the mark's left edge, which at any other scale or offset
    // would be inside it: 100 × 2 + 64 = 264, so 263 is field and 264 is mark.
    same(
        shot.at(263, 152),
        rgb(fixture::FIELD),
        "just left of the mark",
    );
    same(
        shot.at(264, 152),
        rgb(fixture::MARK),
        "the mark's left edge",
    );
    // The chrome, over the page, at the identity.
    same(shot.at(620, 460), rgb(fixture::CHROME), "the chrome's mark");
    check_the_pages_own_edges(&shot);
}

/// **ADR 0058's seam**: the page's first covered pixel and the one beside it.
///
/// The present pass draws each layer's own rectangle rather than the whole window, so
/// there is now a boundary in the *geometry* where before there was only one in the
/// fragment stage's arithmetic — and a rectangle one pixel too small is a page missing
/// its outermost row, which no interior sample can see. These four points are that
/// boundary from both sides: the placement puts page texel (0, 0) at device (64, 32), so
/// column 64 and row 32 are the page's own first pixel and column 63 and row 31 are the
/// window's clear.
///
/// It is the same argument the mark's left edge makes about the *affine*, made about the
/// rectangle that is now drawn: the interior assertions above pass under a bound that is
/// too small by a pixel, and these do not.
fn check_the_pages_own_edges(shot: &xwd::Shot) {
    same(shot.at(63, 100), [0, 0, 0], "one pixel left of the page");
    same(
        shot.at(64, 100),
        rgb(fixture::FIELD),
        "the page's left edge",
    );
    same(shot.at(200, 31), [0, 0, 0], "one pixel above the page");
    same(shot.at(200, 32), rgb(fixture::FIELD), "the page's top edge");
}

/// The filter's other branch, run rather than only compiled.
///
/// `ImageFilter::Linear` goes through the hardware sampler, whose coordinates are
/// normalised where the shader's arithmetic is in texels — a division by the layer's
/// extent that nothing else in this example exercises, and that a wrong denominator
/// would turn into a picture rather than an error. Sampling well inside a region is what
/// makes the assertion about *where* rather than about interpolation: at a 2× placement
/// the interior of the mark is the mark's colour under either filter, and reading the
/// field there instead would mean the sampler was pointed somewhere else entirely.
fn check_the_other_filter_runs(
    display: &mut Display,
    presenter: &mut Presenter,
    page: &wgpu::Texture,
    placement: Affine,
) {
    let smoothed = [Layer {
        texture: page,
        placement,
        filter: ImageFilter::Linear,
    }];
    present_until_settled(display, presenter, &smoothed);
    let shot = xwd::capture(TITLE);
    same(shot.at(324, 212), rgb(fixture::MARK), "the mark, sampled");
    same(shot.at(200, 200), rgb(fixture::FIELD), "the field, sampled");
    // And the chrome is gone, because this present did not carry it: a slice is what is
    // on the window, not what was on it last time.
    same(
        shot.at(620, 460),
        rgb(fixture::FIELD),
        "where the chrome was",
    );
}

/// Step 9: what the presenter says the last present cost.
fn check_the_cost(cost: &PresentCost) {
    assert_eq!(cost.layers, 2, "two layers were presented");
    assert!(
        cost.compiled.is_none(),
        "the presenting pass is in the warm set of a device built for a surface \
         (ADR 0043, ADR 0056); this present compiled it inline instead, in {:?}",
        cost.compiled
    );
    assert!(
        !cost.reconfigured,
        "the swapchain was configured by the first present and must not be replaced again \
         while the window's size has not changed"
    );
}

/// Step 7 and the attach: a device that did not hand this presenter out refuses it, and
/// hands it back rather than consuming it — then its own device takes it.
fn give_it_back(device: Device, presenter: Presenter, options: &Options) -> Device {
    let mut stranger = Device::headless(options).expect("a second device");
    let refused = stranger
        .attach_presenter(presenter)
        .expect_err("a headless device did not hand this presenter out");
    eprintln!("a foreign attach was refused: {refused}");
    let presenter = refused.into_presenter();
    let mut device = device;
    device
        .attach_presenter(presenter)
        .expect("the device that handed it out takes it back");
    device
}

/// Step 8: `Target::Surface` draws again, and the window proves it.
fn draw_through_the_surface_again(display: &mut Display, device: &mut Device) {
    let viewport = Viewport::full(fixture::WINDOW.0, fixture::WINDOW.1, Affine::IDENTITY);
    let scene = fixture::through_the_surface();
    let until = Instant::now() + SETTLE;
    while Instant::now() < until {
        display.pump();
        device
            .render(&scene, &viewport, Target::Surface)
            .expect("Target::Surface works again once the presenter is back");
    }
    let shot = xwd::capture(TITLE);
    // A full-window field the presenter never had a layer for: the last present left the
    // top-left corner black, so this pixel is the one that says who drew last.
    same(
        shot.at(8, 8),
        rgb(fixture::MARK),
        "the surface frame's field",
    );
    same(shot.at(620, 460), rgb(fixture::CHROME), "its corner");
}

/// Present the same picture until the X server has had time to show it.
fn present_until_settled(display: &mut Display, presenter: &mut Presenter, layers: &[Layer<'_>]) {
    let until = Instant::now() + SETTLE;
    while Instant::now() < until {
        display.pump();
        presenter
            .present(layers)
            .expect("presenting the finished page");
    }
}

/// A colour as the window should show it: straight 8-bit RGB, which for an opaque
/// rectangle is what §3's premultiplied storage holds anyway.
fn rgb(color: Color) -> [u8; 3] {
    [
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8,
    ]
}

/// Equal within ADR 0006's bound, naming what was being checked when it is not.
fn same(got: [u8; 3], want: [u8; 3], what: &str) {
    let near = got
        .iter()
        .zip(want)
        .all(|(a, b)| a.abs_diff(b) <= TOLERANCE);
    assert!(
        near,
        "{what}: the window shows {got:?}, the scene says {want:?}"
    );
}

/// The window, and the event loop kept alive beside it.
///
/// `pump_app_events` rather than `run_app` because this example is a script and not an
/// application: the order of its steps is the thing being proved, and a state machine
/// spread over redraw callbacks would hide it. The platform caveat is winit's own —
/// pumping is supported on X11, which is where this runs.
struct Display {
    event_loop: EventLoop<()>,
    opener: Opener,
    window: Arc<Window>,
}

struct Opener {
    window: Option<Arc<Window>>,
}

impl ApplicationHandler for Opener {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title(TITLE)
                    .with_inner_size(winit::dpi::PhysicalSize::new(
                        fixture::WINDOW.0,
                        fixture::WINDOW.1,
                    )),
            )
            .expect("window creation on the test display");
        self.window = Some(Arc::new(window));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
        }
    }
}

impl Display {
    fn open() -> Self {
        let mut event_loop = EventLoop::new().expect("an event loop needs a display; set DISPLAY");
        let mut opener = Opener { window: None };
        let until = Instant::now() + Duration::from_secs(10);
        while opener.window.is_none() {
            event_loop.pump_app_events(Some(Duration::from_millis(10)), &mut opener);
            assert!(
                Instant::now() < until,
                "the display produced no window in ten seconds"
            );
        }
        let window = Arc::clone(
            opener
                .window
                .as_ref()
                .expect("the loop above waited for it"),
        );
        Self {
            event_loop,
            opener,
            window,
        }
    }

    /// Let the window system say what it has to say. Nothing here depends on the
    /// events; a window that is never pumped is a window the server stops believing in.
    fn pump(&mut self) {
        self.event_loop
            .pump_app_events(Some(Duration::ZERO), &mut self.opener);
    }
}
