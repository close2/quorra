//! One scene, several threads, at the same time — and the same page from each of them.
//!
//! # Where the question comes from
//!
//! hayro #1343, by way of the caller's `doc/HAYRO_ISSUES_FOR_QUORRA.md` §7: somebody
//! integrating `hayro-syntax` into a commercial CAD application under concurrent page
//! interpretation found that resolving objects from several threads on one shared document
//! **silently yields nulls** and occasionally panics — three distinct races, named and
//! located. The caller's reading of why it generalises:
//!
//! > Parallel page interpretation over one immutable document is a thing every serious
//! > embedder eventually wants, and #1343 is a catalogue of what breaks when the caching
//! > layer under an "immutable" document is not itself linearisable.
//!
//! The document is not ours. The **scene** is, and it is the object an embedder would
//! share the same way: `RENDER_LIBRARY.md` §2.3 requires it to be `Send + Sync` and cheap
//! to clone, and `doc/adr/0001` makes that structural — a [`Scene`] is an `Arc` around
//! immutable data, with no interior mutability and no cache under it to be unlinearisable.
//!
//! # Why this is not only a compile-time assertion
//!
//! `tests/retained_handle.rs` asserts `Send + Sync` for [`Scene`] with an empty generic
//! function, which is the right way to state a trait bound and says nothing at all about
//! two threads actually rendering from one scene at once. A trait bound is a claim about
//! types; #1343 is a bug about *use*. So this file shares one scene by reference across
//! four threads that render it concurrently on four devices, and requires the reference
//! frame back from every one of them.
//!
//! Sharing **by reference** rather than by clone is deliberate: a clone needs only `Send`,
//! and it is `Sync` that says the object may be read from two threads at once — which is
//! the shape #1343 is about.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, Target, Viewport};
use quorra_scene::{
    Affine, BlendMode, Color, Compose, FillRule, GroupSpec, OutlineId, Paint, Point, Scene,
    SceneBuilder, Segment,
};

const SIDE: u32 = 120;

/// How many threads read the one scene at once. Four is more than the two a race needs and
/// few enough that four software devices fit in a test run.
const READERS: usize = 4;

/// How many threads a rendering device may use for its own geometry, so that the shared
/// read happens while each reader is itself inside a `std::thread::scope`.
const ENCODE_THREADS: usize = 4;

fn blob(lobes: u32, radius: f32) -> Vec<Segment> {
    let point = |angle: f32, r: f32| Point::new(r * angle.cos(), r * angle.sin());
    let mut path = vec![Segment::MoveTo(point(0.0, radius))];
    for step in 0..lobes {
        let from = f32::from(step as u16) / lobes as f32 * std::f32::consts::TAU;
        let to = f32::from(step as u16 + 1) / lobes as f32 * std::f32::consts::TAU;
        let (a, b) = (point(from, radius), point(to, radius * 0.8));
        path.push(Segment::CubicTo {
            c1: Point::new(a.x + (b.x - a.x) * 0.3, a.y + (b.y - a.y) * 0.1),
            c2: Point::new(a.x + (b.x - a.x) * 0.7, a.y + (b.y - a.y) * 0.9),
            to: b,
        });
    }
    path.push(Segment::Close);
    path
}

fn device(threads: usize) -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        encode_threads: threads,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

/// The outlines a page is written in, uploaded to one device.
///
/// **Uploaded resources are the device's and a scene names them by identifier**, so a
/// device that is to draw a scene it did not see built must be given the same resources in
/// the same order. That is a real part of the contract rather than a convenience of this
/// test: it is what lets one viewport-free scene (§2.3) be rendered by devices that have
/// never met, and it is why `shapes` is a separate step from `page`.
struct Shapes {
    curve: OutlineId,
    small: OutlineId,
}

fn shapes(device: &mut Device) -> Shapes {
    Shapes {
        curve: device.upload_outline(&blob(24, 26.0)).unwrap(),
        small: device.upload_outline(&blob(6, 7.0)).unwrap(),
    }
}

/// A page with a group, overlapping translucent marks and a curve clip: enough of the
/// scene vocabulary that a reader which mis-read any part of it would draw a different
/// page rather than a blank one.
fn page(shapes: &Shapes) -> Scene {
    let (curve, small) = (shapes.curve, shapes.small);
    let mut builder = SceneBuilder::new();
    let clip = builder
        .clip(
            curve,
            Affine::translate(60.0, 60.0),
            FillRule::NonZero,
            None,
        )
        .unwrap();
    for index in 0..24_u32 {
        let shade = f32::from((index % 5) as u16) / 5.0;
        let at = Affine::translate(
            30.0 + f32::from((index % 6) as u16) * 9.0,
            30.0 + f32::from((index / 6) as u16) * 9.0,
        );
        builder
            .fill(
                small,
                at,
                FillRule::NonZero,
                Paint::Solid(Color::new(shade, 0.3, 1.0 - shade, 0.7)),
                (index % 4 == 0).then_some(clip),
                BlendMode::Normal,
                Compose::SrcOver,
                None,
            )
            .unwrap();
    }
    builder
        .group(
            GroupSpec {
                alpha: 0.55,
                blend: BlendMode::Multiply,
                clip: None,
                knockout: false,
                mask: None,
                isolated: true,
                compose: Compose::SrcOver,
            },
            |body| {
                for index in 0..6_u32 {
                    body.fill(
                        curve,
                        Affine::translate(45.0 + f32::from(index as u16) * 6.0, 70.0),
                        FillRule::NonZero,
                        Paint::Solid(Color::new(0.1, 0.8, 0.4, 0.6)),
                        None,
                        BlendMode::Normal,
                        Compose::SrcOver,
                        None,
                    )?;
                }
                Ok(())
            },
        )
        .unwrap();
    builder.finish()
}

fn draw(device: &mut Device, scene: &Scene) -> Vec<u8> {
    device
        .render(
            scene,
            &Viewport::full(SIDE, SIDE, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("the fixture is inside every budget")
        .into_raster()
        .unwrap()
        .into_pixels()
}

/// §2.3's three claims about a [`Scene`], each as the kind of statement it is.
///
/// `Send` and `Sync` are trait bounds and are stated as bounds. **Cheap to clone** is not a
/// bound and is usually left as prose, so it is stated here as the fact that makes it true:
/// a `Scene` is one pointer wide, so a clone copies a pointer and bumps a refcount. A scene
/// that grew an inline field would still compile, still be `Send + Sync`, and no longer be
/// cheap to clone — and this is the only thing in the tree that would say so.
#[test]
fn a_scene_is_send_sync_and_one_pointer_wide() {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<Scene>();
    assert_eq!(
        size_of::<Scene>(),
        size_of::<usize>(),
        "§2.3 asks for a scene that is cheap to clone, and ADR 0001 holds it with an Arc \
         around immutable data: one pointer, so a clone is a pointer copy and a refcount"
    );
}

/// **Four threads render one scene at the same time and every one of them draws the
/// reference page** — hayro #1343's shape, asked of the object an embedder would share.
///
/// The scene is shared by `&`, which needs `Sync`; each thread has its own `Device` and its
/// own encode threads, so the concurrency is real at both levels. A scene with a cache
/// under it, or with any interior mutability, is what #1343 is a catalogue of, and it would
/// show up here as one reader's page differing from the rest — the *silent* failure their
/// reporter had, not a panic.
#[test]
fn one_scene_renders_concurrently_on_several_devices() {
    let mut first = device(ENCODE_THREADS);
    let scene = page(&shapes(&mut first));
    let reference = draw(&mut first, &scene);
    assert!(
        reference.iter().skip(3).step_by(4).any(|&a| a > 0),
        "the shared page draws something, or every comparison below is between blanks"
    );

    let shared = &scene;
    let expected = &reference;
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..READERS)
            .map(|reader| {
                scope.spawn(move || {
                    let mut device = device(ENCODE_THREADS);
                    let _ = shapes(&mut device);
                    let drawn = draw(&mut device, shared);
                    assert!(
                        drawn == *expected,
                        "reader {reader} drew a different page from the same scene"
                    );
                })
            })
            .collect();
        for reader in readers {
            reader.join().expect("a reader thread panicked");
        }
    });
}

/// A scene outlives the device that named its resources, and a clone of it is the same
/// page.
///
/// The other half of "cheap to clone": an embedder holding one scene per resident page
/// hands clones to workers, and a clone must be the same scene rather than a snapshot of
/// one. Rendered on a second device from a clone made on a third thread.
#[test]
fn a_cloned_scene_drawn_elsewhere_is_the_same_page() {
    let mut first = device(1);
    let scene = page(&shapes(&mut first));
    let reference = draw(&mut first, &scene);

    let clone = std::thread::spawn(move || scene.clone())
        .join()
        .expect("cloning a scene cannot panic");
    let mut second = device(ENCODE_THREADS);
    let _ = shapes(&mut second);
    assert!(
        draw(&mut second, &clone) == reference,
        "a clone of a scene, made on another thread and drawn on another device, is not \
         the page the original drew"
    );
}
