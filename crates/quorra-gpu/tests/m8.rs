//! The M8 harness: damage, honoured exactly (ADR 0012).
//!
//! The property under test is the viewport contract: a damage list against a
//! retained `Texture` target means the device may touch **only** those pixels —
//! inside the rectangles the frame must equal a full redraw, outside them the
//! previous contents must survive byte-for-byte. Targets with nothing to patch
//! redraw fully and say so in a `Report`, and malformed lists are refused.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects
)]

use quorra_gpu::{Device, Options, RenderError, ReportKind, Target, Viewport};
use quorra_scene::{Affine, BlendMode, Color, Point, Rect, Scene, SceneBuilder};

/// 64 pixels wide on purpose: 64 × 4 bytes = 256, the buffer-copy row alignment.
const SIZE: u32 = 64;

fn device() -> Device {
    Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs")
}

fn target_texture(device: &Device) -> wgpu::Texture {
    let (gpu, _) = device.wgpu();
    gpu.create_texture(&wgpu::TextureDescriptor {
        label: Some("m8 target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Read a caller-owned texture back through the raw wgpu handles the device
/// exposes for tier-3 hosts.
fn read_texture(device: &Device, texture: &wgpu::Texture) -> Vec<u8> {
    let (gpu, queue) = device.wgpu();
    let buffer = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("m8 readback"),
        size: u64::from(SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = slice.get_mapped_range().expect("mapped").to_vec();
    buffer.unmap();
    pixels
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE + x) * 4) as usize;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// A full-target wash of one colour.
fn wash(color: Color) -> Scene {
    let mut builder = SceneBuilder::new();
    builder
        .rect(
            Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
            Affine::IDENTITY,
            color,
            None,
            None,
        )
        .unwrap();
    builder.finish()
}

/// The damage contract, both halves at once: after patching scene B over scene A
/// with one damage rect, the inside equals a full redraw of B and the outside
/// still equals A — even though B differs from A everywhere (which is exactly what
/// proves the device touched nothing beyond the list).
#[test]
fn damage_patch_touches_only_the_damage() {
    let mut device = device();
    let scene_a = wash(Color::new(1.0, 0.0, 0.0, 1.0));
    let scene_b = wash(Color::new(0.0, 0.0, 1.0, 1.0));
    let full = Viewport::full(SIZE, SIZE, Affine::IDENTITY);

    let patched = target_texture(&device);
    device
        .render(&scene_a, &full, Target::Texture(&patched))
        .expect("baseline frame");
    let damage = [Rect::new(Point::new(16.0, 8.0), Point::new(32.0, 24.0))];
    let frame = device
        .render(
            &scene_b,
            &Viewport {
                width: SIZE,
                height: SIZE,
                transform: Affine::IDENTITY,
                damage: &damage,
            },
            Target::Texture(&patched),
        )
        .expect("patched frame");
    assert!(
        frame.reports().is_empty(),
        "honoured damage carries no report, got {:?}",
        frame.reports()
    );

    let reference = target_texture(&device);
    device
        .render(&scene_b, &full, Target::Texture(&reference))
        .expect("reference frame");

    let got = read_texture(&device, &patched);
    let want_inside = read_texture(&device, &reference);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let inside = (16..32).contains(&x) && (8..24).contains(&y);
            if inside {
                assert_eq!(
                    pixel(&got, x, y),
                    pixel(&want_inside, x, y),
                    "inside damage at ({x}, {y}): must equal a full redraw"
                );
            } else {
                assert_eq!(
                    pixel(&got, x, y),
                    [255, 0, 0, 255],
                    "outside damage at ({x}, {y}): the previous contents must survive"
                );
            }
        }
    }
}

/// Several disjoint rects patch independently; the gap between them is untouched.
#[test]
fn multiple_damage_rects_patch_independently() {
    let mut device = device();
    let scene_a = wash(Color::new(0.0, 1.0, 0.0, 1.0));
    let scene_b = wash(Color::new(0.0, 0.0, 0.0, 1.0));
    let full = Viewport::full(SIZE, SIZE, Affine::IDENTITY);

    let texture = target_texture(&device);
    device
        .render(&scene_a, &full, Target::Texture(&texture))
        .expect("baseline");
    let damage = [
        Rect::new(Point::new(0.0, 0.0), Point::new(8.0, 8.0)),
        Rect::new(Point::new(40.0, 40.0), Point::new(48.0, 56.0)),
    ];
    device
        .render(
            &scene_b,
            &Viewport {
                width: SIZE,
                height: SIZE,
                transform: Affine::IDENTITY,
                damage: &damage,
            },
            Target::Texture(&texture),
        )
        .expect("patched");
    let got = read_texture(&device, &texture);
    assert_eq!(pixel(&got, 4, 4), [0, 0, 0, 255], "first rect patched");
    assert_eq!(pixel(&got, 44, 50), [0, 0, 0, 255], "second rect patched");
    assert_eq!(
        pixel(&got, 20, 20),
        [0, 255, 0, 255],
        "the gap between rects is untouched"
    );
}

/// A layered scene (a group with alpha) patches through the same machinery: the
/// compositor renders scissored, and only the damage rect reaches the target.
#[test]
fn layered_scenes_patch_too() {
    let mut device = device();
    let scene_a = wash(Color::new(1.0, 1.0, 1.0, 1.0));
    let mut b = SceneBuilder::new();
    b.group(
        quorra_scene::GroupSpec {
            alpha: 0.5,
            blend: BlendMode::Normal,
            clip: None,
            knockout: false,
            mask: None,
        },
        |body| {
            body.rect(
                Rect::new(Point::new(0.0, 0.0), Point::new(SIZE as f32, SIZE as f32)),
                Affine::IDENTITY,
                Color::new(0.0, 0.0, 0.0, 1.0),
                None,
                None,
            )
        },
    )
    .unwrap();
    let scene_b = b.finish();
    let full = Viewport::full(SIZE, SIZE, Affine::IDENTITY);

    let patched = target_texture(&device);
    device
        .render(&scene_a, &full, Target::Texture(&patched))
        .expect("baseline");
    let damage = [Rect::new(Point::new(8.0, 8.0), Point::new(24.0, 24.0))];
    device
        .render(
            &scene_b,
            &Viewport {
                width: SIZE,
                height: SIZE,
                transform: Affine::IDENTITY,
                damage: &damage,
            },
            Target::Texture(&patched),
        )
        .expect("patched");

    let reference = target_texture(&device);
    device
        .render(&scene_b, &full, Target::Texture(&reference))
        .expect("reference");

    let got = read_texture(&device, &patched);
    let want = read_texture(&device, &reference);
    assert_eq!(
        pixel(&got, 16, 16),
        pixel(&want, 16, 16),
        "inside: the composited group, exactly as a full redraw makes it"
    );
    assert_eq!(
        pixel(&got, 40, 40),
        [255, 255, 255, 255],
        "outside: the previous frame survives"
    );
}

/// A `Readback` target has no retained contents to patch: the frame draws fully
/// and says so — a `Report`, never a silent choice (§5).
#[test]
fn targets_without_retained_contents_redraw_fully_and_report() {
    let mut device = device();
    let damage = [Rect::new(Point::new(0.0, 0.0), Point::new(8.0, 8.0))];
    let frame = device
        .render(
            &wash(Color::new(0.2, 0.4, 0.6, 1.0)),
            &Viewport {
                width: SIZE,
                height: SIZE,
                transform: Affine::IDENTITY,
                damage: &damage,
            },
            Target::Readback,
        )
        .expect("full redraw");
    let report = frame
        .reports()
        .iter()
        .find(|r| r.kind == ReportKind::DamageNotHonoured)
        .expect("the unhonoured damage is reported");
    assert!(
        report.detail.contains("Readback"),
        "the report names the target kind: {report:?}"
    );
    // And the frame really is the full redraw it claims.
    let pixels = frame.into_raster().unwrap().into_pixels();
    assert_eq!(pixel(&pixels, 60, 60)[3], 255);
}

/// §4.7 at the damage boundary: a NaN or inverted rectangle is refused by index,
/// not guessed at — a wrong guess is exactly the stale frame damage exists to
/// prevent.
#[test]
fn malformed_damage_is_refused_by_index() {
    let mut device = device();
    let texture = target_texture(&device);
    let scene = wash(Color::new(0.0, 0.0, 0.0, 1.0));
    for bad in [
        Rect::new(Point::new(f32::NAN, 0.0), Point::new(8.0, 8.0)),
        Rect::new(Point::new(10.0, 0.0), Point::new(2.0, 8.0)),
    ] {
        let damage = [Rect::new(Point::new(0.0, 0.0), Point::new(4.0, 4.0)), bad];
        match device.render(
            &scene,
            &Viewport {
                width: SIZE,
                height: SIZE,
                transform: Affine::IDENTITY,
                damage: &damage,
            },
            Target::Texture(&texture),
        ) {
            Err(RenderError::InvalidDamage { index }) => assert_eq!(index, 1),
            other => panic!("expected InvalidDamage, got {other:?}"),
        }
    }
}

/// A damage list that clamps to nothing on this target means nothing visible
/// changed — honouring it exactly means touching no pixel at all.
#[test]
fn offscreen_damage_touches_nothing() {
    let mut device = device();
    let scene_a = wash(Color::new(1.0, 0.5, 0.0, 1.0));
    let scene_b = wash(Color::new(0.0, 0.0, 0.0, 1.0));
    let full = Viewport::full(SIZE, SIZE, Affine::IDENTITY);
    let texture = target_texture(&device);
    device
        .render(&scene_a, &full, Target::Texture(&texture))
        .expect("baseline");
    let damage = [Rect::new(
        Point::new(100.0, 100.0),
        Point::new(120.0, 120.0),
    )];
    device
        .render(
            &scene_b,
            &Viewport {
                width: SIZE,
                height: SIZE,
                transform: Affine::IDENTITY,
                damage: &damage,
            },
            Target::Texture(&texture),
        )
        .expect("a no-op patch is a legitimate frame");
    let got = read_texture(&device, &texture);
    for y in 0..SIZE {
        for x in 0..SIZE {
            assert_eq!(
                pixel(&got, x, y),
                [255, 128, 0, 255],
                "({x}, {y}) must be untouched"
            );
        }
    }
}
