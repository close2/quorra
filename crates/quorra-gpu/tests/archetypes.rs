//! Seven page shapes, priced by counters that cannot drift.
//!
//! # Why this file exists
//!
//! Until it, every performance fixture in this tree was invented: `perf_gate.rs` draws
//! 5 933 **rectangles** because the brief's dense page has 5 933 glyphs. Measured over
//! the caller's 995-page corpus, **not one page emits a single `Command::Rect`** — the
//! command that fixture uses is one no document sends, and since ADR 0047 a document's
//! rectangles reach the same lane as fills instead — and glyph reuse is **1.33
//! placements per distinct outline at the median**, not the 55 that fixture assumes. A
//! gate built on those assumptions cannot see a regression on the shapes documents
//! actually have, which is how a twentyfold zoom cliff and a sixteenfold sheet lived
//! here undetected until the caller's own gate or an example found them.
//!
//! `doc/corpus-profile.md` holds the measurement, its date and how to redo it.
//!
//! # Where the pages live, and why not here
//!
//! **The pages themselves are `quorra-pages`** (ADR 0060). They were defined in this
//! file until 2026-08-17, and four examples carried private copies of the generator —
//! so re-cutting a page meant editing it in five files, and the round that missed one
//! left `examples/retained.rs` asserting a row ADR 0057 had moved. It panicked at its
//! own signature gate on `main` for two days, because `cargo test` neither builds nor
//! runs an example. A dev-dependency reaches a test *and* an example; nothing inside
//! this crate does.
//!
//! What stayed here is what this file is for: **rendering each page and comparing its
//! counters against the row recorded beside it.**
//!
//! # What is gated, and why counters rather than clocks
//!
//! Every `Counters` field is an exact function of the scene and the viewport, so a
//! baseline of them compares by **equality on any machine** — no thresholds, no
//! flakiness under load, and a regression names itself. `tiles` would have caught the
//! atlas cliff, `bytes_uploaded` the sheet committed at the device's maximum width, and
//! `layer_textures` the frame that priced a pair per plan. The clocks are printed and
//! gated far more loosely, because a wall clock on a loaded runner is context rather
//! than evidence (CLAUDE.md principle 2).
//!
//! Every archetype renders on a **fresh device**, because every frame the caller's gate
//! measures is a cold-atlas frame — the warm second render is the exception on a page
//! turn, not the rule, and measuring it is how a fixture flatters itself.

// Test-file lint policy as in m1.rs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects
)]

use std::time::{Duration, Instant};

use quorra_gpu::{Counters, Device, Options, Target, Viewport};
use quorra_pages::{
    ARCHETYPES, Archetype, Recorded, clip_of, curve_clip, outline_side, position, scene,
};
use quorra_scene::{Affine, ImageId, OutlineId, Scene};

const WIDTH: u32 = 1191;
const HEIGHT: u32 = 1684;

/// The archetype's resources, uploaded in the order `quorra_pages::outlines` states.
fn upload(device: &mut Device, shape: &Archetype) -> (Vec<OutlineId>, Option<ImageId>) {
    let outlines = quorra_pages::outlines(shape)
        .iter()
        .map(|path| device.upload_outline(path).unwrap())
        .collect();
    let image = quorra_pages::image_spec(shape)
        .map(|spec| device.upload_image(&spec).unwrap())
        .or(None);
    (outlines, image)
}

/// Build the archetype's scene on this device.
fn build(device: &mut Device, shape: &Archetype) -> Scene {
    let (outlines, image) = upload(device, shape);
    scene(shape, &outlines, image).expect("an archetype builds")
}

/// A device with nothing in its caches, which is what every frame of a page turn is.
fn cold_device() -> Device {
    let device = Device::headless(&Options {
        adapter: Some("llvmpipe".into()),
        ..Options::default()
    })
    .expect("llvmpipe is present wherever this suite runs");
    device.wait_until_warm();
    device
}

fn render(device: &mut Device, scene: &Scene) -> (Counters, Duration) {
    let started = Instant::now();
    let frame = device
        .render(
            scene,
            &Viewport::full(WIDTH, HEIGHT, Affine::IDENTITY),
            Target::Readback,
        )
        .expect("an archetype must draw");
    let counters = frame.counters();
    let raster = frame.into_raster().expect("Readback carries its pixels");
    assert_eq!(raster.pixels().len(), (WIDTH * HEIGHT * 4) as usize);
    (counters, started.elapsed())
}

/// A frame's counters as the row `quorra-pages` records, field by named field.
///
/// **The mapping is written out rather than collected into an array**, because a
/// positional row is a mapping that can be written wrongly in silence and a field name
/// is not. `quorra-pages` cannot own this function: `Counters` lives here, and that
/// crate must stay free of an adapter.
fn recorded(counters: &Counters) -> Recorded {
    Recorded {
        commands: u64::from(counters.commands),
        commands_culled: u64::from(counters.commands_culled),
        distinct_outlines: u64::from(counters.distinct_outlines),
        atlas_distinct_keys: u64::from(counters.atlas_distinct_keys),
        clip_distinct_regions: u64::from(counters.clip_distinct_regions),
        tiles: u64::from(counters.tiles),
        layer_textures: u64::from(counters.layer_textures),
        clip_residue_regions: u64::from(counters.clip_residue_regions),
        clip_residue_tiles: u64::from(counters.clip_residue_tiles),
        coverage_texels: counters.coverage.texels,
    }
}

/// The gate: what each archetype costs, in quantities that cannot flake.
///
/// The recorded rows are `quorra-pages`' — each page's doc comment explains its own row
/// line by line, which is where the explanation belongs now that the page and its price
/// are one item. A baseline nobody can account for is a baseline nobody can defend.
///
/// **Every archetype is measured before any of them is judged.** A loop that asserts as
/// it goes reports the first row that moved and hides the rest, which is the wrong shape
/// for a signature: a change that moves one row and a change that moves five are
/// different changes, and the second must not read as the first.
#[test]
fn the_archetypes_cost_what_they_are_recorded_to_cost() {
    let mut moved = Vec::new();
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (counters, elapsed) = render(&mut device, &scene);
        let Some(expected) = shape.recorded else {
            panic!("no row recorded for {}", shape.name)
        };
        let actual = recorded(&counters);
        eprintln!("{:14} {actual:?} in {elapsed:?}", shape.name);
        if actual != expected {
            moved.push(format!(
                "{}:\n    got      {actual:?}\n    recorded {expected:?}",
                shape.name
            ));
        }
    }
    assert!(
        moved.is_empty(),
        "the archetype signature moved. Every field is an exact function of the scene \
         and the viewport, so this is a change in what the library does — explain it, \
         then record it in `quorra-pages`:\n  {}",
        moved.join("\n  ")
    );
}

/// **A curve clip clips the marks under it** — asserted as an interaction, in the two
/// quantities that say the interaction happened, on both sides of the library's boundary.
///
/// The trap this exists for is written down in `doc/notes-tiling-bound.md` §3 and cost
/// two ADRs: a fixture whose subject is an *interaction* needs a gate that fails when the
/// interaction stops happening, and the signature above was not one. It counted 40 and
/// 600 tiles for two pages whose clips and marks did not overlap at all, because until
/// ADR 0057 a mark whose chain admitted nothing still got a mark-sized tile — so the
/// count survived the property it was standing in for.
///
/// Two assertions, each from a different side:
///
/// - **from the generator's own arithmetic**, with nothing of the crate in it: every one
///   of the `clipped` commands has a mark box that meets the box of the clip that clips
///   it. This is the property `quorra_pages::curve_clip` is written to hold.
/// - **from the counters**: `tiles == clipped`. A mark whose chain admits no pixel is not
///   rasterised (ADR 0057), so the library agreeing that there are exactly `clipped`
///   tiles is the same statement measured through the encode. It is exact and
///   adapter-independent, like every other row of the signature.
///
/// The residue lane's own numbers — how many chains were rasterised once over a region
/// and how many per tile — are in each page's recorded row, which compares by equality;
/// what is asserted here is only that a rasterisation happened at all and that none of
/// them is unaccounted for.
#[test]
fn a_curve_clip_clips_the_marks_that_draw_under_it() {
    for shape in ARCHETYPES {
        if shape.rect_clips || shape.clips == 0 || shape.clipped == 0 {
            continue;
        }
        let meeting = (0..shape.clipped)
            .filter(|index| a_mark_meets_its_clip(shape, *index))
            .count() as u32;
        assert_eq!(
            meeting, shape.clipped,
            "{}: the fixture's own arithmetic says only {meeting} of its {} clipped \
             commands have a mark that meets the clip clipping it — a clip that admits \
             nothing exercises nothing, whatever the tile count says",
            shape.name, shape.clipped,
        );

        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (counters, _) = render(&mut device, &scene);
        assert_eq!(
            counters.tiles, shape.clipped,
            "{}: {} clipped commands and {} coverage tiles. Every mark under a curve clip \
             rasterises one and nothing else on this page does, so a difference is a \
             clipped mark whose chain admits no pixel of it (ADR 0057)",
            shape.name, shape.clipped, counters.tiles,
        );
        assert!(
            counters.clip_residue_regions + counters.clip_residue_tiles <= shape.clipped,
            "{}: {} residue rasterisations for {} clipped commands — a chain is \
             rasterised once over its region or once per asking tile (ADR 0049), never \
             both",
            shape.name,
            counters.clip_residue_regions + counters.clip_residue_tiles,
            shape.clipped,
        );
        assert!(
            counters.clip_residue_regions + counters.clip_residue_tiles > 0,
            "{}: the page draws under curve clips and rasterised no residue at all",
            shape.name,
        );
    }
}

/// Does mark `index`'s box meet the box of the clip that clips it?
///
/// The clip's box as the *scene* will carry it: `curve_clip`'s transform applied to the
/// ellipse `outline_of` traces. Deliberately not `marks_box` — that is the box
/// `curve_clip` is built from, so testing against it would assert an identity and pass
/// however the clip is placed. (It does: the first version of this gate was tautological
/// and survived the forced defect, and only the counter above caught it.)
fn a_mark_meets_its_clip(shape: &Archetype, index: u32) -> bool {
    let clip = clip_of(shape, index) as u32;
    let placed = curve_clip(shape, clip);
    let clip_side = outline_side(shape, clip % shape.distinct.max(1));
    let (chx, chy) = (placed.a * clip_side * 0.5, placed.d * clip_side * 0.65);
    let at = position(shape, index, shape.side);
    let side = outline_side(shape, index % shape.distinct.max(1));
    let (hx, hy) = (side * 0.5, side * 0.65);
    at.e - hx < placed.e + chx
        && at.e + hx > placed.e - chx
        && at.f - hy < placed.f + chy
        && at.f + hy > placed.f - chy
}

/// The scenes are what the profile says they are: the generator cannot drift from the
/// numbers it was built from without this failing.
#[test]
fn the_generator_builds_the_shape_the_profile_states() {
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let cost = scene.cost();
        // `Cost::commands` walks the tree and counts a group as a node of its own,
        // where `Counters::commands` counts the scene's top level. The two differ by
        // exactly the groups, and both are right about what they measure.
        assert_eq!(
            cost.commands as u32,
            shape.commands + shape.images + shape.groups,
            "{} must hold the commands its profile states",
            shape.name
        );
        assert_eq!(
            cost.clips as u32, shape.clips,
            "{} must define the clip regions its profile states",
            shape.name
        );
        assert!(
            cost.group_depth as u32 <= 1,
            "{}: the corpus reaches depth 2 at most, so a deeper generator is a bug here",
            shape.name
        );
    }
}

/// What the archetypes take on a cold device, printed always and gated loosely.
///
/// **Ignored by default, and that is the honest status.** A wall clock on a loaded
/// machine is context rather than evidence: this file's counter gate passed unchanged
/// under a load average of 32 while this test read sixteen seconds for an archetype
/// recorded at 1.79 — the same code, the same scene, a neighbour compiling something.
/// A gate that fails for that reason teaches people to ignore failures. Run it
/// deliberately, on a quiet machine, when a number is what you want:
/// `cargo test --release -p quorra-gpu --test archetypes -- --ignored --nocapture`.
/// Measured on llvmpipe, release, cold device, **2026-08-17 at load average 4.95, with
/// the curve clips cut around their marks**: median page 20 ms, dense text 44, artwork
/// **148**, image page 29, clip mountain 30, giant 36, drawing 46. Software rasterisation
/// dominates every one of them, which is why the gate is a multiple rather than a bound.
///
/// The same list on 2026-08-12 read 18 / 41 / **160** / 29 / 30 / 27 on a page whose
/// clips met 8 of the 600 marks they clipped — nearly the same numbers for a page doing
/// nearly the same work to no effect, which is what makes the two look comparable when
/// they are not. The same run at load average 380 read artwork at **605 ms**, four times
/// the quiet figure, which is the whole argument for `#[ignore]`.
#[test]
#[ignore = "a wall clock is a measurement here, not a gate; see the doc comment"]
fn no_archetype_takes_absurdly_long() {
    // The rasteriser is a byte loop, so an unoptimised build is an order of magnitude
    // slower and one threshold cannot serve both. Re-measured 2026-08-17 on llvmpipe, cold
    // device, whole frame including readback, load average 5–8, with the curve clips cut
    // around their marks: release, the worst archetype is **artwork at 148 ms**; debug,
    // **dense text at 2.29 s** (artwork 0.76 s). Each build gets ~4× its own worst, and
    // both thresholds are unchanged by the re-cut — artwork does about the work it did
    // before ADR 0057 took its empty tiles away, which is why the release figure moved by
    // 12 ms and not by a factor.
    let limit = if cfg!(debug_assertions) {
        Duration::from_secs(8)
    } else {
        Duration::from_millis(700)
    };
    for shape in ARCHETYPES {
        let mut device = cold_device();
        let scene = build(&mut device, shape);
        let (_, elapsed) = render(&mut device, &scene);
        eprintln!("{:14} cold frame {elapsed:?}", shape.name);
        assert!(
            elapsed < limit,
            "{} took {elapsed:?}, past {limit:?} — the recorded numbers are in this \
             file's doc comment, and none of them is within a factor of five of this",
            shape.name
        );
    }
}
