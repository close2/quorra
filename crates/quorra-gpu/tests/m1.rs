//! The M1 harness: goldens against a CPU reference, byte-equality gates, and the
//! truth-telling tests of `doc/PLAN.md` M1.
//!
//! # Where the expected values come from
//!
//! ISO 32000-2 does not define anti-aliasing, so there is no clause to derive a golden
//! from; the coverage rule is quorra's own documented choice (`doc/adr/0005`): a
//! pixel's coverage is the exact area of its unit cell inside the rectangle, the
//! source is premultiplied and scaled by that coverage, and compositing is the
//! premultiplied over operator quantised to 8 bits per channel between commands.
//! [`cpu_reference`] below implements exactly that rule from the ADR's arithmetic, so
//! these tests pin *the device against the stated definition* — not against another
//! renderer's output, which principle 5 forbids.
//!
//! # What is byte-exact and what is tolerance-bounded (§4.6, §11.4, ADR 0006)
//!
//! Same scene, same viewport, same adapter → the same bytes: a promise, tested
//! byte-exact per adapter. Across adapters, and against the CPU reference, the answer
//! was **measured and is "no"**: the float→unorm8 store conversion of the
//! fixed-function raster path is implementation-defined per driver — a single opaque
//! rectangle of colour 0.1 stores as 26 on llvmpipe and 25 on RADV — so those gates
//! pin a stated bound instead: ±1 unorm step per blend stage in premultiplied space,
//! which the straight-alpha conversion amplifies by at most 255/α (≤ ±2 on this
//! golden, whose minimum alpha is 128). ADR 0006 records the probes and the design
//! consequence; drift beyond the bound still fails loudly.

// Integration-test files sit outside `#[cfg(test)]`, so `clippy.toml`'s
// allow-unwrap-in-tests does not reach them; this is the same policy, stated here.
// The cast and arithmetic allowances cover the CPU reference, whose indices and
// coordinates are bounded by the 48×32 golden target — every cast is exact there,
// and the arithmetic mirrors the shader's on purpose.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use quorra_gpu::{Device, Options, RenderError, Target, TimingProvenance, Viewport};
use quorra_scene::{Affine, Color, Point, Rect, Scene, SceneBuilder};

/// Every Vulkan adapter on this machine, by name. The determinism promises quoted in
/// the module docs are made for Vulkan adapters (RADV and lavapipe are what the
/// caller's CI relies on); a GL adapter that wgpu also enumerates is out of scope and
/// skipped, with a note, rather than silently included or silently dropped.
fn vulkan_adapters() -> Vec<String> {
    let mut names = Vec::new();
    for name in Device::adapter_names() {
        let device = Device::headless(&Options {
            adapter: Some(name.clone()),
            ..Options::default()
        });
        match device {
            Ok(device) if device.description().contains("Vulkan") => names.push(name),
            Ok(device) => {
                eprintln!(
                    "note: skipping non-Vulkan adapter '{}' ({})",
                    name,
                    device.description()
                );
            }
            Err(error) => eprintln!("note: adapter '{name}' did not yield a device: {error}"),
        }
    }
    names.sort();
    names.dedup();
    assert!(
        !names.is_empty(),
        "no Vulkan adapter available; the M1 gates cannot run at all on this machine"
    );
    names
}

fn device_for(adapter: &str) -> Device {
    Device::headless(&Options {
        adapter: Some(adapter.to_owned()),
        ..Options::default()
    })
    .expect("adapter enumerated moments ago must still construct")
}

/// The golden scene: fractional edges on all sides, overlap with partial alpha, an
/// edge-crossing rectangle, an empty rectangle, and a y-flip in the viewport — every
/// M1 behaviour in one 48×32 frame.
fn golden_scene() -> Scene {
    let mut builder = SceneBuilder::new();
    // Opaque red, fractional edges on every side.
    builder
        .rect(
            Rect::new(Point::new(2.25, 3.5), Point::new(20.75, 17.125)),
            Affine::IDENTITY,
            Color::new(1.0, 0.0, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    // Half-transparent blue overlapping the red, exercising the over operator at
    // fractional coverage.
    builder
        .rect(
            Rect::new(Point::new(10.5, 8.25), Point::new(30.0, 24.5)),
            Affine::IDENTITY,
            Color::new(0.0, 0.25, 1.0, 0.5),
            None,
            None,
        )
        .unwrap();
    // Crosses the right and bottom target edges: clipped by the viewport, no wrap.
    builder
        .rect(
            Rect::new(Point::new(40.0, 28.0), Point::new(60.0, 40.0)),
            Affine::IDENTITY,
            Color::new(0.0, 1.0, 0.0, 0.75),
            None,
            None,
        )
        .unwrap();
    // A scaled and translated rect, through the command transform.
    builder
        .rect(
            Rect::new(Point::new(1.0, 1.0), Point::new(3.0, 2.0)),
            Affine::scale(4.0, 4.0).then(Affine::translate(20.0, 2.0)),
            Color::new(0.5, 0.5, 0.0, 1.0),
            None,
            None,
        )
        .unwrap();
    // Empty: draws nothing, legitimately.
    builder
        .rect(
            Rect::new(Point::new(5.0, 5.0), Point::new(5.0, 30.0)),
            Affine::IDENTITY,
            Color::new(1.0, 1.0, 1.0, 1.0),
            None,
            None,
        )
        .unwrap();
    builder.finish()
}

const GOLDEN_W: u32 = 48;
const GOLDEN_H: u32 = 32;

/// The golden viewport carries the y-flip, as §3 of the brief places it: scene y-up,
/// device y-down.
fn golden_viewport() -> Viewport<'static> {
    Viewport::full(
        GOLDEN_W,
        GOLDEN_H,
        Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: 0.0,
            f: 32.0,
        },
    )
}

/// The reference rasteriser: the documented coverage and compositing rule of
/// `doc/adr/0005`, implemented independently of the GPU path (same definition, second
/// implementation — which is what makes the comparison a check and not a tautology).
fn cpu_reference(scene: &Scene, viewport: &Viewport<'_>) -> Vec<u8> {
    let width = viewport.width as usize;
    let height = viewport.height as usize;
    // Premultiplied f32 working target, quantised to unorm8 after every command —
    // matching an rgba8unorm attachment, which stores 8 bits between draws.
    let mut target = vec![[0_u8; 4]; width * height];
    for command in scene.commands() {
        let quorra_scene::Command::Rect {
            rect,
            transform,
            color,
            clip,
            mask: _,
        } = *command
        else {
            panic!("the golden scene is rectangle-only");
        };
        assert!(clip.is_none(), "the golden scene is unclipped");
        let to_device = transform.then(viewport.transform);
        assert!(to_device.preserves_axes(), "golden scene is axis-aligned");
        let p0 = to_device.apply(rect.min);
        let p1 = to_device.apply(rect.max);
        let (min_x, max_x) = (p0.x.min(p1.x), p0.x.max(p1.x));
        let (min_y, max_y) = (p0.y.min(p1.y), p0.y.max(p1.y));
        if min_x >= max_x || min_y >= max_y {
            continue;
        }
        let premul = [
            color.r * color.a,
            color.g * color.a,
            color.b * color.a,
            color.a,
        ];
        for y in 0..height {
            for x in 0..width {
                // Identical arithmetic to rect.wgsl: the pixel's cell is
                // [x, x+1) × [y, y+1), coverage is the exact overlap area.
                let (px, py) = (x as f32, y as f32);
                let extent_x = (max_x.min(px + 1.0) - min_x.max(px)).max(0.0);
                let extent_y = (max_y.min(py + 1.0) - min_y.max(py)).max(0.0);
                let coverage = extent_x * extent_y;
                if coverage <= 0.0 {
                    continue;
                }
                let dst = &mut target[y * width + x];
                let src_a = premul[3] * coverage;
                for channel in 0..4 {
                    let src = premul[channel] * coverage;
                    let dst_f = f32::from(dst[channel]) / 255.0;
                    let out = src + dst_f * (1.0 - src_a);
                    // Round to nearest; exact ties cannot occur because k + 1/2
                    // has no representation as an f32 multiple of 1/255.
                    dst[channel] = (out.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
    }
    // The boundary conversion, per the same ADR: straight = (c·255 + a/2) / a.
    let mut out = Vec::with_capacity(width * height * 4);
    for pixel in target {
        let a = pixel[3];
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            for channel in &pixel[..3] {
                let straight = (u32::from(*channel) * 255 + u32::from(a) / 2) / u32::from(a);
                out.push(straight.min(255) as u8);
            }
            out.push(a);
        }
    }
    out
}

/// On mismatch, both rasters land as PNGs a person can look at.
fn write_artifact(name: &str, width: u32, height: u32, pixels: &[u8]) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
    path
}

fn render_golden(device: &mut Device) -> Vec<u8> {
    device
        .render(&golden_scene(), &golden_viewport(), Target::Readback)
        .expect("the golden scene is within every budget")
        .into_raster()
        .expect("a Readback frame carries a raster")
        .into_pixels()
}

/// The stated cross-implementation bound (module docs, ADR 0006): ±1 unorm step per
/// blend stage in premultiplied space, amplified to at most ±2 by the straight-alpha
/// conversion on this golden (minimum alpha 128).
const UNORM_TOLERANCE: i32 = 2;

/// Largest per-byte difference between two rasters, or a panic with PNG artefacts if
/// the shapes differ.
fn max_byte_diff(actual: &[u8], expected: &[u8]) -> i32 {
    assert_eq!(actual.len(), expected.len());
    actual
        .iter()
        .zip(expected)
        .map(|(a, e)| (i32::from(*a) - i32::from(*e)).abs())
        .max()
        .unwrap_or(0)
}

/// The golden: every Vulkan adapter agrees with the CPU reference within the stated
/// unorm-conversion bound.
#[test]
fn golden_matches_cpu_reference_on_every_adapter() {
    let expected = cpu_reference(&golden_scene(), &golden_viewport());
    for adapter in vulkan_adapters() {
        let mut device = device_for(&adapter);
        let actual = render_golden(&mut device);
        let diff = max_byte_diff(&actual, &expected);
        if diff > UNORM_TOLERANCE {
            let got = write_artifact(
                &format!("golden-actual-{adapter}"),
                GOLDEN_W,
                GOLDEN_H,
                &actual,
            );
            let want = write_artifact(
                &format!("golden-expected-{adapter}"),
                GOLDEN_W,
                GOLDEN_H,
                &expected,
            );
            panic!(
                "adapter '{}' differs from the CPU reference by {} unorm steps (bound: {}; \
                 artefacts: {} vs {})",
                device.description(),
                diff,
                UNORM_TOLERANCE,
                got.display(),
                want.display()
            );
        }
    }
}

/// §4.6: same scene, same viewport, same adapter → the same bytes. Twice on one
/// device, and again on a freshly constructed device.
#[test]
fn repeated_renders_are_byte_identical() {
    for adapter in vulkan_adapters() {
        let mut device = device_for(&adapter);
        let first = render_golden(&mut device);
        let second = render_golden(&mut device);
        assert_eq!(
            first, second,
            "two renders on one device diverged ({adapter})"
        );
        let mut fresh = device_for(&adapter);
        let third = render_golden(&mut fresh);
        assert_eq!(first, third, "a fresh device diverged ({adapter})");
    }
}

/// §11.4, answered by measurement and pinned: adapters are NOT byte-identical through
/// the fixed-function raster path — the float→unorm8 store conversion is the driver's
/// (ADR 0006) — so this gate pins the stated bound instead, and fails loudly beyond
/// it. If this design ever claims byte identity again (a shader-owned quantisation,
/// weighed at M6), this test tightens back to `assert_eq`.
#[test]
fn cross_adapter_output_stays_within_the_stated_bound() {
    let adapters = vulkan_adapters();
    if adapters.len() < 2 {
        eprintln!(
            "note: only one Vulkan adapter ({}); the cross-adapter gate compared nothing",
            adapters[0]
        );
        return;
    }
    let rasters: Vec<(String, Vec<u8>)> = adapters
        .iter()
        .map(|name| {
            let mut device = device_for(name);
            (name.clone(), render_golden(&mut device))
        })
        .collect();
    let (reference_name, reference) = &rasters[0];
    for (name, raster) in &rasters[1..] {
        let diff = max_byte_diff(raster, reference);
        assert!(
            diff <= UNORM_TOLERANCE,
            "adapters '{name}' and '{reference_name}' differ by {diff} unorm steps \
             (stated bound: {UNORM_TOLERANCE}, ADR 0006) — something beyond store-conversion \
             rounding diverged"
        );
    }
}

/// A blank scene is a legitimate scene: `Ok`, zero commands, fully transparent pixels.
#[test]
fn blank_scene_renders_ok_and_transparent() {
    for adapter in vulkan_adapters() {
        let mut device = device_for(&adapter);
        let frame = device
            .render(
                &SceneBuilder::new().finish(),
                &Viewport::full(8, 8, Affine::IDENTITY),
                Target::Readback,
            )
            .expect("a blank scene renders");
        assert_eq!(frame.counters().commands, 0);
        assert!(frame.reports().is_empty());
        let raster = frame.into_raster().unwrap();
        assert!(raster.pixels().iter().all(|&b| b == 0));
    }
}

/// A zero-size readback is a legitimate frame; a zero-size texture target is refused
/// by name.
#[test]
fn zero_size_targets() {
    let adapter = &vulkan_adapters()[0];
    let mut device = device_for(adapter);
    let scene = golden_scene();
    let frame = device
        .render(
            &scene,
            &Viewport::full(0, 0, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("zero-size readback is legitimate");
    let raster = frame.into_raster().unwrap();
    assert_eq!((raster.width(), raster.height()), (0, 0));
    assert!(raster.pixels().is_empty());

    let (gpu, _) = device.wgpu();
    let texture = gpu.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    match device.render(
        &scene,
        &Viewport::full(0, 0, Affine::IDENTITY),
        Target::Texture(&texture),
    ) {
        Err(RenderError::ZeroSizeTarget { target: "Texture" }) => {}
        other => panic!("expected ZeroSizeTarget, got {other:?}"),
    }
}

/// Truth-telling: a frame knows which payload it carries, and a caller texture is
/// validated against its contract before anything draws.
#[test]
fn frames_tell_the_truth_and_textures_are_validated() {
    let adapter = &vulkan_adapters()[0];
    let mut device = device_for(adapter);
    let scene = golden_scene();
    let viewport = golden_viewport();

    // A conforming host texture: render succeeds, and the frame refuses to pretend
    // it holds a raster.
    let (gpu, _) = device.wgpu();
    let make_texture = |w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        gpu.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let good = make_texture(
        GOLDEN_W,
        GOLDEN_H,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let wrong_format = make_texture(
        GOLDEN_W,
        GOLDEN_H,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let wrong_size = make_texture(
        GOLDEN_W + 1,
        GOLDEN_H,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let wrong_usage = make_texture(
        GOLDEN_W,
        GOLDEN_H,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureUsages::TEXTURE_BINDING,
    );

    let frame = device
        .render(&scene, &viewport, Target::Texture(&good))
        .expect("a conforming texture renders");
    assert_eq!(frame.timings().readback, std::time::Duration::ZERO);
    match frame.into_raster() {
        Err(RenderError::NotAReadbackFrame) => {}
        other => panic!("a Texture frame must refuse into_raster, got {other:?}"),
    }

    assert!(matches!(
        device.render(&scene, &viewport, Target::Texture(&wrong_format)),
        Err(RenderError::TextureFormat { .. })
    ));
    assert!(matches!(
        device.render(&scene, &viewport, Target::Texture(&wrong_size)),
        Err(RenderError::TextureSize { .. })
    ));
    assert!(matches!(
        device.render(&scene, &viewport, Target::Texture(&wrong_usage)),
        Err(RenderError::TextureUsage)
    ));

    // Surface on a headless device: refused by name.
    assert!(matches!(
        device.render(&scene, &viewport, Target::Surface),
        Err(RenderError::NoSurface)
    ));
}

/// The refusals of §5, through the public API: budget, oblique rectangle, oversized
/// target, non-finite viewport.
#[test]
fn refusals_name_what_they_refuse() {
    let adapter = &vulkan_adapters()[0];
    let scene = golden_scene();
    let viewport = golden_viewport();

    let mut tiny_budget = Device::headless(&Options {
        adapter: Some(adapter.clone()),
        max_frame_bytes: 16,
        ..Options::default()
    })
    .unwrap();
    match tiny_budget.render(&scene, &viewport, Target::Readback) {
        Err(RenderError::FrameBudgetExceeded { needed, budget }) => {
            assert_eq!(budget, 16);
            assert!(needed > budget);
        }
        other => panic!("expected FrameBudgetExceeded, got {other:?}"),
    }

    let mut device = device_for(adapter);
    let limit = device.limits().max_target_size;
    assert!(matches!(
        device.render(
            &scene,
            &Viewport::full(limit + 1, 8, Affine::IDENTITY),
            Target::Readback
        ),
        Err(RenderError::TargetTooLarge { .. })
    ));

    let non_finite = Viewport::full(
        8,
        8,
        Affine {
            e: f32::NAN,
            ..Affine::IDENTITY
        },
    );
    assert!(matches!(
        device.render(&scene, &non_finite, Target::Readback),
        Err(RenderError::NonFiniteViewportTransform)
    ));
}

/// A damage list cannot be honoured until M8: the frame is drawn in full and says so
/// in a report, rather than quietly ignoring the economy it was asked for.
#[test]
fn unhonoured_damage_is_reported_not_silent() {
    let adapter = &vulkan_adapters()[0];
    let mut device = device_for(adapter);
    let damage = [Rect::new(Point::new(0.0, 0.0), Point::new(4.0, 4.0))];
    let viewport = Viewport {
        width: GOLDEN_W,
        height: GOLDEN_H,
        transform: golden_viewport().transform,
        damage: &damage,
    };
    let frame = device
        .render(&golden_scene(), &viewport, Target::Readback)
        .expect("the frame is drawn in full");
    assert!(
        frame
            .reports()
            .iter()
            .any(|r| r.kind == quorra_gpu::ReportKind::DamageNotHonoured),
        "an unhonoured damage list must be reported"
    );
    // And the full-frame pixels are exactly what the same device draws without a
    // damage list — same adapter, so this comparison is byte-exact (§4.6).
    assert_eq!(
        frame.into_raster().unwrap().into_pixels(),
        render_golden(&mut device)
    );
}

/// §7's startup contract: the split is reported, the device is eventually warm, and
/// the warm duration then appears.
#[test]
fn startup_reports_its_three_phases() {
    let adapter = &vulkan_adapters()[0];
    let device = device_for(adapter);
    let startup = device.startup();
    assert!(startup.adapter_enumeration > std::time::Duration::ZERO);
    assert!(startup.device_creation > std::time::Duration::ZERO);
    device.wait_until_warm();
    assert!(device.is_warm());
    assert!(device.startup().pipeline_compilation.is_some());
}

/// The timing provenance is never ambiguous: with timestamp queries the per-pass
/// phases exist; without, the frame says `WallClock` rather than pretending.
#[test]
fn execute_provenance_is_stated() {
    for adapter in vulkan_adapters() {
        let mut device = device_for(&adapter);
        let frame = device
            .render(&golden_scene(), &golden_viewport(), Target::Readback)
            .unwrap();
        match frame.timings().execute_provenance {
            TimingProvenance::TimestampQueries => {
                assert!(
                    frame
                        .timings()
                        .phases
                        .iter()
                        .any(|(name, _)| *name == "content pass"),
                    "timestamped frames carry per-pass phases ({adapter})"
                );
            }
            TimingProvenance::WallClock => {
                eprintln!("note: '{adapter}' has no timestamp queries; execute is a wall clock");
            }
        }
    }
}
