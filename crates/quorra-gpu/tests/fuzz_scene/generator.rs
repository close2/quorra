//! The vocabulary: what a fuzzed scene can be made of.
//!
//! One responsibility — which uploads, commands and lifecycle events exist, and how a
//! scene is walked through them. The numbers they are built from are [`crate::rng`]'s;
//! the property they must satisfy is `fuzz_scene.rs`'s. Reading this module answers
//! "what can happen to the boundary", and nothing else.

// Test-file lint policy as in m1.rs; the arithmetic here is the fuzzer's own bounded
// index/seed math, not boundary code.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use quorra_gpu::Device;
use quorra_scene::{
    BlendMode, ClipId, Compose, FnOp, FnRange, FunctionId, GroupSpec, ImageId, ImageSpec, LineCap,
    LineJoin, MAX_GROUP_DEPTH, MaskId, MaskKind, MeshId, MeshSpec, OutlineId, Paint, Point, RampId,
    ResourceId, SceneBuilder, SceneError, Segment, ShadingKind, Stop, Stroke, Transfer,
};

use crate::rng::Rng;

/// Commands one scene may attempt, across every nesting level. A budget rather than a
/// depth cap because an accepted group is a *branch*: with group specs valid most of
/// the time the recursion is supercritical, and only a shared counter bounds it.
const OPS_PER_SCENE: u32 = 160;

/// What one seed uploaded and what its scene has defined.
///
/// Released ids stay in the reference vectors on purpose: a command naming a resource
/// this device no longer holds must be a typed `Unknown*` refusal at render time, and
/// that is only fuzzed if the fuzzer keeps saying the dead name.
#[derive(Debug, Default)]
pub(crate) struct Pool {
    outlines: Vec<OutlineId>,
    images: Vec<ImageId>,
    ramps: Vec<RampId>,
    meshes: Vec<MeshId>,
    functions: Vec<FunctionId>,
    clips: Vec<ClipId>,
    masks: Vec<MaskId>,
    /// Uploaded and not yet released: releasing one must succeed.
    pub(crate) live: Vec<ResourceId>,
    /// Released: releasing one again must be `DeviceError::UnknownResource`.
    pub(crate) dead: Vec<ResourceId>,
    /// Whether this seed is a **dangling-reference** seed: one that fabricates foreign
    /// ids and releases resources out from under its own half-built scene.
    ///
    /// A scene-level flag rather than a per-reference one, because a single unknown id
    /// refuses the *whole* frame: at one reference in ten nearly every seed would be a
    /// refusal seed, and the lanes that draw images, sweeps and meshes would never see
    /// a frame at all. One seed in three carries the hostile lifecycle; the rest draw.
    dangling: bool,
}

/// The generator's state for one scene: the stream, what it has uploaded, and how many
/// more operations it may attempt.
#[derive(Debug)]
pub(crate) struct Gen {
    pub(crate) rng: Rng,
    pub(crate) pool: Pool,
    budget: u32,
}

impl Gen {
    pub(crate) fn new(seed: u64) -> Self {
        let mut rng = Rng::seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let dangling = rng.one_in(3);
        Self {
            rng,
            pool: Pool {
                dangling,
                ..Pool::default()
            },
            budget: OPS_PER_SCENE,
        }
    }

    /// Take one operation from the scene's budget, or refuse to recurse further.
    fn spend(&mut self) -> bool {
        if self.budget == 0 {
            return false;
        }
        self.budget -= 1;
        true
    }
}

/// An identifier of the given family: one this seed really produced, or — where
/// `may_fabricate` allows it — one out of the air. A foreign id is a caller bug the
/// boundary must name rather than dereference. `None` where the pool is empty and this
/// seed does not fabricate, so that a family nothing uploaded simply goes unmentioned
/// instead of turning every seed into a refused frame.
fn pick<T: Copy>(
    rng: &mut Rng,
    from: &[T],
    fabricate: fn(u32) -> T,
    may_fabricate: bool,
) -> Option<T> {
    if may_fabricate && (from.is_empty() || rng.one_in(6)) {
        return Some(fabricate(u32::try_from(rng.next() % 1_000).unwrap()));
    }
    if from.is_empty() {
        return None;
    }
    Some(from[usize::try_from(rng.next()).unwrap_or(0) % from.len()])
}

/// Clip and mask ids fabricate on every seed, unlike resource ids: an unknown one is a
/// *builder* refusal that costs one command, not a frame.
fn maybe_clip(rng: &mut Rng, pool: &Pool) -> Option<ClipId> {
    if rng.one_in(3) {
        None
    } else {
        pick(rng, &pool.clips, ClipId, true)
    }
}

fn maybe_mask(rng: &mut Rng, pool: &Pool) -> Option<MaskId> {
    if rng.one_in(4) {
        pick(rng, &pool.masks, MaskId, true)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Uploads
// ---------------------------------------------------------------------------

/// An image spec from across the whole shape of §4.7's check: consistent small ones a
/// device stores, a zero dimension, a byte count that disagrees with the dimensions in
/// both directions, dimensions no allocation could ever match, and one large enough to
/// be refused by the seed's stated budget. Nothing here allocates more than 36 KiB.
fn image_spec(rng: &mut Rng) -> ImageSpec {
    let choice = rng.next() % 10;
    let byte = u8::try_from(rng.next() % 256).unwrap();
    let width = 1 + u32::try_from(rng.next() % 4).unwrap();
    let height = 1 + u32::try_from(rng.next() % 4).unwrap();
    let filled = |width: u32, height: u32, len: usize| ImageSpec {
        width,
        height,
        data: Arc::from(vec![byte; len].as_slice()),
    };
    match choice {
        0 => filled(0, height, 0),
        1 => filled(width, 0, 0),
        2 => filled(2, 2, 15),
        3 => filled(2, 2, 17),
        4 => filled(u32::MAX, u32::MAX, 4),
        5 => filled(60_000, 60_000, 0),
        // Consistent, and 36 KiB against the 32 KiB budget the harness states: the
        // refusal that names all three numbers, with nothing stored.
        6 => filled(96, 96, 96 * 96 * 4),
        _ => filled(width, height, usize::try_from(width * height * 4).unwrap()),
    }
}

/// Ramp stops, ascending by construction most of the time — an ordered ramp is the only
/// kind that gets past the upload and into the shading lane — and empty, unordered or
/// out of range the rest of the time, which is the other three `Ramp*` refusals.
fn ramp_stops(rng: &mut Rng) -> Vec<Stop> {
    let count = rng.next() % 5;
    let hostile = rng.one_in(4);
    (0..count)
        .map(|i| Stop {
            offset: if hostile {
                rng.f32()
            } else if count > 1 {
                (i as f32) / ((count - 1) as f32)
            } else {
                0.0
            },
            color: rng.color(),
        })
        .collect()
}

/// An outline: uploadable three times in four, and drawn from the hostile spread
/// otherwise, which is where `OutlineNonFinite` and `OutlineCoordinateTooLarge` live.
/// A path that never uploads leaves a pool of fabricated ids behind it, and a frame
/// refused for an unknown outline draws none of the rest of the scene.
fn random_outline(rng: &mut Rng) -> Vec<Segment> {
    let hostile = rng.one_in(4);
    let point = |rng: &mut Rng| {
        if hostile {
            Point::new(rng.f32(), rng.f32())
        } else {
            Point::new(rng.tame(), rng.tame())
        }
    };
    let mut segments = vec![Segment::MoveTo(point(rng))];
    for _ in 0..(rng.next() % 6) {
        segments.push(match rng.next() % 3 {
            0 => Segment::LineTo(point(rng)),
            1 => Segment::CubicTo {
                c1: point(rng),
                c2: point(rng),
                to: point(rng),
            },
            _ => Segment::Close,
        });
    }
    segments
}

/// A short §7.10.5 program: mostly nonsense, which is exactly the input
/// `Device::upload_function` exists to refuse.
///
/// CLAUDE.md principle 3 asks for the *scene boundary* to be fuzzed from the first commit,
/// and a compiled function program is the newest thing to arrive across it: it comes from
/// another process's parser, it is a jump graph, and every one of the analyser's budgets is
/// stated over document-derived arithmetic. Most of these are refused — a backward jump, a
/// type two branches disagree about, an output count no `Range` matches — and the ones that
/// are not get a generated shader compiled for them, which is the other half of what this
/// fuzzes.
///
/// Bounded at eight instructions on purpose: an admitted program costs a shader compile per
/// distinct program, and the point here is the boundary rather than the throughput.
fn random_program(rng: &mut Rng) -> Vec<FnOp> {
    let length = 1 + (rng.next() % 8) as usize;
    (0..length).map(|_| random_op(rng)).collect()
}

/// One instruction, drawn from the whole of Table 42 plus the two jumps the caller's
/// compiler emits for `if` and `ifelse`.
#[allow(clippy::cast_possible_truncation)] // a jump target is a u32 by construction
fn random_op(rng: &mut Rng) -> FnOp {
    match rng.next() % 45 {
        0 => FnOp::PushReal(rng.f32()),
        1 => FnOp::PushInt(rng.i32()),
        2 => FnOp::PushBool(rng.one_in(2)),
        3 => FnOp::Abs,
        4 => FnOp::Add,
        5 => FnOp::Atan,
        6 => FnOp::Ceiling,
        7 => FnOp::Cos,
        8 => FnOp::Cvi,
        9 => FnOp::Cvr,
        10 => FnOp::Div,
        11 => FnOp::Exp,
        12 => FnOp::Floor,
        13 => FnOp::Idiv,
        14 => FnOp::Ln,
        15 => FnOp::Log,
        16 => FnOp::Mod,
        17 => FnOp::Mul,
        18 => FnOp::Neg,
        19 => FnOp::Round,
        20 => FnOp::Sin,
        21 => FnOp::Sqrt,
        22 => FnOp::Sub,
        23 => FnOp::Truncate,
        24 => FnOp::And,
        25 => FnOp::Bitshift,
        26 => FnOp::Eq,
        27 => FnOp::Ge,
        28 => FnOp::Gt,
        29 => FnOp::Le,
        30 => FnOp::Lt,
        31 => FnOp::Ne,
        32 => FnOp::Not,
        33 => FnOp::Or,
        34 => FnOp::Xor,
        35 => FnOp::Copy,
        36 => FnOp::Dup,
        37 => FnOp::Exch,
        38 => FnOp::Index,
        39 => FnOp::Pop,
        40 => FnOp::Roll,
        41 | 42 => FnOp::JumpUnless {
            target: (rng.next() % 12) as u32,
        },
        _ => FnOp::Jump {
            target: (rng.next() % 12) as u32,
        },
    }
}

/// Fill the seed's pool from all five upload paths. Every refusal is typed and leaves
/// the device usable, which is what the rest of the seed then depends on.
pub(crate) fn upload_resources(generator: &mut Gen, device: &mut Device) {
    for _ in 0..=(generator.rng.next() % 3) {
        let segments = random_outline(&mut generator.rng);
        if let Ok(id) = device.upload_outline(&segments) {
            generator.pool.outlines.push(id);
            generator.pool.live.push(id.into());
        }
    }
    for _ in 0..=(generator.rng.next() % 3) {
        let spec = image_spec(&mut generator.rng);
        if let Ok(id) = device.upload_image(&spec) {
            generator.pool.images.push(id);
            generator.pool.live.push(id.into());
        }
    }
    for _ in 0..=(generator.rng.next() % 3) {
        let stops = ramp_stops(&mut generator.rng);
        if let Ok(id) = device.upload_ramp(&stops) {
            generator.pool.ramps.push(id);
            generator.pool.live.push(id.into());
        }
    }
    for _ in 0..=(generator.rng.next() % 3) {
        let mesh = MeshSpec {
            left: generator.rng.i32(),
            top: generator.rng.i32(),
            image: image_spec(&mut generator.rng),
        };
        if let Ok(id) = device.upload_mesh(&mesh) {
            generator.pool.meshes.push(id);
            generator.pool.live.push(id.into());
        }
    }
    // One program at most, and only in some seeds: an admitted one costs a generated
    // shader compile, and this file's job is the boundary rather than the throughput.
    if generator.rng.one_in(3) {
        let program = random_program(&mut generator.rng);
        if let Ok(id) = device.upload_function(&program) {
            generator.pool.functions.push(id);
            generator.pool.live.push(id.into());
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// A sweep in the shading's own space: axial and radial, with degenerate geometry
/// reached deliberately — a zero-length axis and two coincident equal circles are both
/// a division by zero in the parametric solve of §8.7.4.5.2/.3.
fn shading(rng: &mut Rng) -> ShadingKind {
    let extend = (rng.one_in(2), rng.one_in(2));
    let start = rng.point();
    let degenerate = rng.one_in(4);
    let end = if degenerate { start } else { rng.point() };
    if rng.one_in(2) {
        ShadingKind::Axial { start, end, extend }
    } else {
        let start_radius = if rng.one_in(3) { 0.0 } else { rng.coord() };
        ShadingKind::Radial {
            start,
            start_radius,
            end,
            end_radius: if degenerate {
                start_radius
            } else {
                rng.coord()
            },
            extend,
        }
    }
}

/// A paint: a ramp or a mesh where this seed has one to name, and a solid colour where
/// it has not — the fallback keeps a seed that uploaded no shading resource drawing
/// rather than refusing.
fn paint(rng: &mut Rng, pool: &Pool) -> Paint {
    let chosen = match rng.next() % 7 {
        0 | 1 => pick(rng, &pool.ramps, RampId, pool.dangling).map(|ramp| Paint::Shading {
            ramp,
            kind: shading(rng),
            transform: rng.affine(),
        }),
        2 => pick(rng, &pool.meshes, MeshId, pool.dangling).map(Paint::Mesh),
        // ADR 0053's paint. The `Range` is drawn independently of the program, so most of
        // these are the §7.10.5.3 output-count refusal — which is a typed refusal of the
        // *frame* and therefore as much of the boundary as a drawn one.
        3 => pick(rng, &pool.functions, FunctionId, pool.dangling).map(|program| Paint::Function {
            program,
            domain: rng.rect(),
            matrix: rng.affine(),
            range: if rng.one_in(2) {
                FnRange::Gray([rng.tame(), rng.tame()])
            } else {
                FnRange::Rgb([
                    [rng.tame(), rng.tame()],
                    [rng.tame(), rng.tame()],
                    [rng.tame(), rng.tame()],
                ])
            },
            background: if rng.one_in(2) {
                None
            } else {
                Some(rng.color())
            },
        }),
        _ => None,
    };
    chosen.unwrap_or_else(|| Paint::Solid(rng.color()))
}

fn op_fill(
    generator: &mut Gen,
    builder: &mut SceneBuilder,
    clip: Option<ClipId>,
    mask: Option<MaskId>,
) {
    let Some(outline) = pick(
        &mut generator.rng,
        &generator.pool.outlines,
        OutlineId,
        generator.pool.dangling,
    ) else {
        return;
    };
    let paint = paint(&mut generator.rng, &generator.pool);
    let _ = builder.fill(
        outline,
        generator.rng.affine(),
        generator.rng.fill_rule(),
        paint,
        clip,
        generator.rng.blend(),
        generator.rng.compose(),
        mask,
    );
}

fn op_stroke(
    generator: &mut Gen,
    builder: &mut SceneBuilder,
    clip: Option<ClipId>,
    mask: Option<MaskId>,
) {
    let Some(outline) = pick(
        &mut generator.rng,
        &generator.pool.outlines,
        OutlineId,
        generator.pool.dangling,
    ) else {
        return;
    };
    let paint = paint(&mut generator.rng, &generator.pool);
    let stroke = Stroke {
        // Widths are scene-space and non-negative since ADR 0085 (zero is §8.4.3.2's
        // thinnest line); a quarter of these are arbitrary bits, most of which are
        // negative or non-finite and so `InvalidStroke`.
        width: if generator.rng.one_in(4) {
            generator.rng.f32()
        } else {
            generator.rng.unit() * 4.0 + 0.25
        },
        adjust: false,
        cap: LineCap::Round,
        join: LineJoin::Bevel,
        miter_limit: if generator.rng.one_in(4) {
            generator.rng.f32()
        } else {
            4.0
        },
    };
    let _ = builder.stroke(
        outline,
        generator.rng.affine(),
        stroke,
        paint,
        clip,
        generator.rng.blend(),
        mask,
    );
}

fn op_image(
    generator: &mut Gen,
    builder: &mut SceneBuilder,
    clip: Option<ClipId>,
    mask: Option<MaskId>,
) {
    let Some(image) = pick(
        &mut generator.rng,
        &generator.pool.images,
        ImageId,
        generator.pool.dangling,
    ) else {
        return;
    };
    let _ = builder.image(
        image,
        generator.rng.affine(),
        generator.rng.alpha(),
        generator.rng.filter(),
        clip,
        generator.rng.blend(),
        mask,
    );
}

fn op_clip(generator: &mut Gen, builder: &mut SceneBuilder, parent: Option<ClipId>) {
    let Some(outline) = pick(
        &mut generator.rng,
        &generator.pool.outlines,
        OutlineId,
        generator.pool.dangling,
    ) else {
        return;
    };
    let transform = generator.rng.affine();
    let rule = generator.rng.fill_rule();
    if let Ok(id) = builder.clip(outline, transform, rule, parent) {
        generator.pool.clips.push(id);
    }
}

fn op_group(
    generator: &mut Gen,
    device: &mut Device,
    builder: &mut SceneBuilder,
    depth: usize,
    clip: Option<ClipId>,
    mask: Option<MaskId>,
) {
    let spec = GroupSpec {
        alpha: generator.rng.alpha(),
        blend: generator.rng.blend(),
        clip,
        knockout: generator.rng.one_in(3),
        mask,
        // A quarter of the groups ask for §11.4.4's backdrop, including inside
        // knockout groups and under blend modes the builder must refuse — the refusal
        // is as much of the boundary as the acceptance is.
        isolated: !generator.rng.one_in(4),
        compose: generator.rng.compose(),
    };
    let knockout = spec.knockout;
    let _ = builder.group(spec, |inner| {
        random_ops(generator, device, inner, depth + 1, knockout);
        Ok(())
    });
}

/// A soft-mask definition (§11.5): both reduction rules, and `/TR` absent, identity and
/// arbitrary. The body is ordinary scene content, which is what the clause makes it.
fn op_mask(generator: &mut Gen, device: &mut Device, builder: &mut SceneBuilder, depth: usize) {
    let kind = if generator.rng.one_in(2) {
        MaskKind::Alpha
    } else {
        MaskKind::Luminosity {
            backdrop: generator.rng.color(),
        }
    };
    let transfer = match generator.rng.next() % 3 {
        0 => None,
        1 => Some(Transfer::identity()),
        _ => {
            let mut table = [0_u8; 256];
            let phase = u8::try_from(generator.rng.next() % 256).unwrap();
            for (i, slot) in table.iter_mut().enumerate() {
                *slot = u8::try_from(i).unwrap().wrapping_add(phase);
            }
            Some(Transfer(table))
        }
    };
    // §11.6.5 renders the mask group on its own, so a knockout group outside this call is
    // not above the mask's content — the same reset `SceneBuilder::mask` makes.
    let defined = builder.mask(kind, transfer, |inner| {
        random_ops(generator, device, inner, depth + 1, false);
        Ok(())
    });
    if let Ok(id) = defined {
        generator.pool.masks.push(id);
    }
}

/// Release a live resource in the middle of building. The scene keeps referencing it,
/// so the render that follows must refuse by name (`RenderError::Unknown*`) rather
/// than draw a page with a hole where the resource was. Confined to the
/// dangling-reference seeds, for the reason [`Pool::dangling`] gives.
fn op_release(generator: &mut Gen, device: &mut Device) {
    if !generator.pool.dangling || generator.pool.live.is_empty() {
        return;
    }
    let index = usize::try_from(generator.rng.next()).unwrap_or(0) % generator.pool.live.len();
    let id = generator.pool.live.swap_remove(index);
    device
        .release(id)
        .expect("a resource this seed uploaded and has not released yet");
    generator.pool.dead.push(id);
}

/// Nest plain groups until the builder refuses. **Two** things about these groups can be
/// refused, and the assertion pins each to its own condition rather than admitting either
/// anywhere: the depth bound, which must happen at [`MAX_GROUP_DEPTH`] and *only* there,
/// and §11.4.6's element rule (ADR 0069), which must happen when the chain's first link is
/// an element of a knockout group and *only* then.
///
/// The `depth` this is called with is the fuzzer's own count of open frames, so the
/// assertion also checks that count against the builder's; `element_of_knockout` is the
/// same question about the frame the first link lands in, and the builder answers it from
/// its own stack. Only the first link can be one — every link below it is an element of an
/// ordinary group, whatever encloses the chain.
fn nest_chain(builder: &mut SceneBuilder, depth: usize, remaining: u32, element_of_knockout: bool) {
    let spec = GroupSpec {
        alpha: 1.0,
        blend: BlendMode::Normal,
        clip: None,
        knockout: false,
        mask: None,
        isolated: true,
        compose: Compose::SrcOver,
    };
    let result = builder.group(spec, |inner| {
        if remaining > 0 {
            nest_chain(inner, depth + 1, remaining - 1, false);
        }
        Ok(())
    });
    match result {
        Ok(()) => assert!(
            depth < MAX_GROUP_DEPTH && !element_of_knockout,
            "a group opened at depth {depth}, element of a knockout group: \
             {element_of_knockout}"
        ),
        // Checked before the depth bound is, so it wins wherever both hold.
        Err(SceneError::KnockoutElementGroupUnsupported) => assert!(
            element_of_knockout,
            "at depth {depth} the builder applied §11.4.6's element rule outside a \
             knockout group"
        ),
        Err(error) => assert!(
            matches!(error, SceneError::GroupTooDeep { limit } if limit == MAX_GROUP_DEPTH)
                && depth >= MAX_GROUP_DEPTH
                && !element_of_knockout,
            "at depth {depth} the builder refused with {error}"
        ),
    }
}

/// Draw the whole vocabulary into `builder`, recursing through group and mask bodies
/// until the scene's operation budget runs out.
///
/// `element_of_knockout` is what §11.4.6 makes of the commands landing here — whether the
/// group *immediately* enclosing them is a knockout group. It is the fuzzer's own copy of
/// the question `SceneBuilder` answers from its frame stack, which is what lets
/// [`nest_chain`] hold one answer against the other rather than accepting either.
pub(crate) fn random_ops(
    generator: &mut Gen,
    device: &mut Device,
    builder: &mut SceneBuilder,
    depth: usize,
    element_of_knockout: bool,
) {
    // A page's worth at the top level, a handful inside each group or mask body; the
    // scene's budget is what actually stops the recursion.
    let ops = if depth == 0 {
        12 + (generator.rng.next() % 24)
    } else {
        2 + (generator.rng.next() % 8)
    };
    for _ in 0..ops {
        if !generator.spend() {
            return;
        }
        let clip = maybe_clip(&mut generator.rng, &generator.pool);
        let mask = maybe_mask(&mut generator.rng, &generator.pool);
        match generator.rng.next() % 16 {
            0..=2 => {
                let rect = generator.rng.rect();
                let transform = generator.rng.affine();
                let color = generator.rng.color();
                let _ = builder.rect(rect, transform, color, clip, mask);
            }
            3..=5 => op_fill(generator, builder, clip, mask),
            6 => op_stroke(generator, builder, clip, mask),
            7 | 8 => op_clip(generator, builder, clip),
            9 | 10 => op_image(generator, builder, clip, mask),
            11 | 12 => op_group(generator, device, builder, depth, clip, mask),
            13 => op_mask(generator, device, builder, depth),
            14 => op_release(generator, device),
            _ => nest_chain(
                builder,
                depth,
                4 + u32::try_from(generator.rng.next() % 16).unwrap(),
                element_of_knockout,
            ),
        }
    }
}
