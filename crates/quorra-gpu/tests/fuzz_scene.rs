//! Structured fuzzing of the scene boundary (CLAUDE.md principle 3: "fuzz the scene
//! boundary from the first commit" — this is the first commit with a boundary).
//!
//! A deterministic xorshift generator drives thousands of random upload, builder and
//! release sequences — hostile values included: NaN, infinities, 1e30 coordinates,
//! unordered rects, singular transforms, deep nesting, foreign ids, images whose
//! dimensions and byte count disagree, ramps out of order, meshes anchored at
//! `i32::MIN`, shadings whose sweep is a point — and asserts one property: **the
//! boundary never panics and never accepts silently.** Every call returns `Ok` or a
//! typed error; every finished scene can be costed and rendered (to a drawn frame or
//! a typed refusal) without a crash.
//!
//! # What the vocabulary covers
//!
//! The generator beside this file — `fuzz_scene/rng.rs` for the value spread,
//! `fuzz_scene/generator.rs` for the vocabulary — reaches every *kind* of input the
//! boundary has rather than every value: uploads of all four resource families and each way each
//! one is refused; `rect`, `fill`, `stroke`, `clip`, `image` and `mask` commands; all
//! three paints (solid, §8.7.4.5's axial and radial sweeps over an uploaded ramp, and a
//! pre-rasterised mesh); every field of `GroupSpec`, including the isolation, knockout
//! and staged-compose refusals; nesting to and past `MAX_GROUP_DEPTH`; both coverage
//! lanes; viewports from zero-size to oversized, with damage lists; and the resource
//! lifecycle — release mid-scene, release-then-reference, and double release, each of
//! which must be a *typed* error rather than a wrong picture.
//!
//! Three choices keep the walk productive rather than merely loud. Colours, alphas,
//! transforms and group specs are drawable most of the time, because a scene refused
//! at its first call never reaches the lanes the hostile geometry is aimed at; the
//! scene's total operation count is a budget threaded through the recursion, because
//! a group that is usually *accepted* multiplies rather than terminates; and the
//! hostile resource lifecycle is a property of a seed rather than of a reference,
//! because one unknown id refuses a whole frame. About seven frames in ten draw.
//!
//! Deliberately not `cargo-fuzz`: coverage-guided fuzzing needs a nightly toolchain
//! and this tree pins stable (`rust-toolchain.toml`). A deterministic generator in
//! the ordinary suite runs everywhere the suite runs, on every push; a nightly-based
//! coverage-guided harness can live outside the pinned tree later if the boundary
//! grows past what this covers. Every crasher found here or there becomes a named
//! regression test.

// Test-file lint policy as in m1.rs; the arithmetic here is the fuzzer's own bounded
// index/seed math, not boundary code.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::manual_is_multiple_of
)]

// Cargo compiles every `tests/*.rs` into its own test binary, so the two halves of the
// generator live in a subdirectory — which cargo ignores, having no `main.rs` — and are
// named by path rather than by position.
#[path = "fuzz_scene/generator.rs"]
mod generator;
#[path = "fuzz_scene/rng.rs"]
mod rng;

use generator::{Gen, Pool, random_ops, upload_resources};
use quorra_gpu::{Coverage, Device, DeviceError, Options, Target, Viewport};
use quorra_scene::{Affine, MAX_GROUP_DEPTH, Rect, Scene, SceneBuilder};

/// Scenes per run. Each one uploads, builds, renders and drains. Two thousand of them
/// measured 2.0–2.1 s optimised on llvmpipe with the machine quiet, 2.6–9.9 s at load
/// average 13, and 9.9 s unoptimised inside a full `cargo test --workspace`. CI runs the
/// suite unoptimised, where the next slowest test in this crate is 6.5 s, so that is the
/// number the seed count is set by. These are wall clocks and wall clocks lie under
/// load (ADR 0040) — the spread above is the lie, measured.
const SEEDS: u64 = 2_000;

/// The device's resource budget for this test. Small on purpose:
/// `ResourceBudgetExceeded` is as much of the upload boundary as admission is, and a
/// fuzzer that only ever fits inside the default budget exercises half of `charge`.
const RESOURCE_BUDGET: u64 = 32 * 1024;

/// The frame's target, from a zero-size window to one four times the usual side, with a
/// damage list a quarter of the time.
///
/// Zero size is legitimate for `Readback` and yields an empty raster; a non-finite
/// viewport transform and a malformed damage rect are both refusals that name
/// themselves (§4.7), and both are reachable from here. The damage list is filled in
/// place rather than returned because a `Viewport` borrows it.
fn fuzz_viewport(generator: &mut Gen, damage: &mut Vec<Rect>) -> (u32, u32, Affine) {
    let (width, height) = match generator.rng.next() % 8 {
        0 => (0, 0),
        1 => (1, 1),
        2 => (37, 5),
        3 => (128, 96),
        _ => (32, 32),
    };
    let transform = if generator.rng.one_in(16) {
        generator.rng.affine()
    } else {
        Affine::IDENTITY
    };
    if generator.rng.one_in(4) {
        for _ in 0..=(generator.rng.next() % 3) {
            // Mostly usable regions, because a list refused for one bad rectangle
            // never exercises the damage path it was built to exercise.
            damage.push(if generator.rng.one_in(8) {
                generator.rng.rect()
            } else {
                generator.rng.tame_rect()
            });
        }
    }
    (width, height, transform)
}

/// Release everything this seed still holds, then ask for each dead id again: a double
/// release is a typed error, and the budget is back to zero once the last live
/// resource is gone.
fn drain_resources(device: &mut Device, pool: &Pool, seed: u64) {
    for &id in &pool.live {
        let released = device.release(id);
        assert!(
            released.is_ok(),
            "seed {seed}: releasing a live {id:?} gave {released:?}"
        );
    }
    for &id in &pool.dead {
        let again = device.release(id);
        assert!(
            matches!(again, Err(DeviceError::UnknownResource { id: named }) if named == id),
            "seed {seed}: a second release of {id:?} was not a typed refusal"
        );
    }
    assert_eq!(
        device.resource_bytes_in_use(),
        0,
        "seed {seed}: the resource budget did not come back"
    );
}

/// What a finished scene must be able to say about itself, whatever it was built from:
/// the depth bound held, and the cost agrees with the scene it was measured from.
fn check_cost(scene: &Scene, seed: u64) {
    let cost = scene.cost();
    assert!(
        cost.group_depth <= MAX_GROUP_DEPTH,
        "seed {seed}: the depth bound leaked"
    );
    assert_eq!(
        cost.masks,
        scene.masks().len(),
        "seed {seed}: the cost disagrees with the scene about its masks"
    );
    assert_eq!(
        cost.clips,
        scene.clips().len(),
        "seed {seed}: the cost disagrees with the scene about its clips"
    );
}

/// The one property, over two thousand seeded scenes: no panic, no silent acceptance,
/// and every finished scene either renders or is refused with a typed error.
#[test]
fn the_boundary_never_panics_and_never_accepts_silently() {
    let mut device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        max_resource_bytes: RESOURCE_BUDGET,
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");

    for seed in 1..=SEEDS {
        let mut generator = Gen::new(seed);
        // Both coverage producers, alternating: they are two implementations of the
        // same bytes (ADR 0016), and a fixed choice fuzzes one of them.
        device.set_coverage(if seed % 2 == 0 {
            Coverage::Gpu
        } else {
            Coverage::Cpu
        });
        upload_resources(&mut generator, &mut device);

        let mut builder = SceneBuilder::new();
        // The page group is not a knockout group, so nothing at the root is an element of
        // one (§11.4.6, ADR 0069).
        random_ops(&mut generator, &mut device, &mut builder, 0, false);
        let scene = builder.finish();
        check_cost(&scene, seed);

        let mut damage = Vec::new();
        let (width, height, transform) = fuzz_viewport(&mut generator, &mut damage);
        let viewport = Viewport {
            width,
            height,
            transform,
            damage: &damage,
        };
        match device.render(&scene, &viewport, Target::Readback) {
            Ok(frame) => {
                let raster = frame.into_raster().expect("readback frames carry rasters");
                assert_eq!(
                    raster.pixels().len(),
                    usize::try_from(width * height * 4).unwrap(),
                    "seed {seed}"
                );
            }
            Err(error) => {
                // A refusal is legitimate; what matters is that it is typed and the
                // device remains usable.
                let _ = error.to_string();
            }
        }

        drain_resources(&mut device, &generator.pool, seed);
    }
}
