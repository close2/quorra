#![allow(
    clippy::float_cmp,
    clippy::unwrap_used,
    reason = "the pinned hashes and the emitted text are exact values, and the one float \
              comparison here is the *premise* of a test: that `==` cannot tell +0.0 from -0.0"
)]

//! The generated WGSL: what it says, that it always says the same thing, and that the hash is
//! an identity rather than a summary.
//!
//! The golden below is deliberately the whole emitted function for one small program rather
//! than a grep for a substring. It is the file that fails when the emitter changes, and its
//! failure message is where the instruction to bump `GENERATOR_REVISION` lives — because the
//! hash covers the program, the component count and the revision, *not* the text, and that is
//! the one seam between the two that discipline rather than a type has to hold.

mod function_support;

use function_support::programs::{self, ODD_RGB, UNIT_GRAY, UNIT_RGB, WIDE_GRAY, WIDE_RGB};
use quorra_gpu::function::{ENTRY_POINT, OPERATORS, ProgramHash, analyse, generate};
use quorra_scene::{FnOp, FnRange};

/// The whole emitted function for `x`, `y`, `x*y` in `DeviceRGB`.
///
/// Read it as documentation of the contract: the domain test first and it discards rather than
/// clamps, then the point into slots 0 and 1, one `var` per operand-stack slot, every operator
/// carrying its Table 42 citation, the stack operators as read-then-write blocks, and the
/// range clip at the end.
const GOLDEN_COORDINATES: &str = "\
// Generated from a 5-instruction ISO 32000-2 §7.10.5 type 4 function.
// Do not edit: `quorra_gpu::function::generate` emits this, and it is a pure
// function of the program, so an edit here is lost on the next frame.
//
// Operand stack: 4 slot(s), reached by a walk that computed the depth rather
// than trusting one. Every `copy`/`index`/`roll` count and both readings of
// Table 42's `not` are already resolved, so nothing below is dynamic.
//
// `x` and `y` are in the shading's own space; `domain` is
// (min x, max x, min y, max y). The result's `.a` is coverage: 1 inside the
// domain rectangle, and whatever `background` carries outside it.
fn quorra_function_evaluate(
    x: f32,
    y: f32,
    domain: vec4<f32>,
    range_low: vec3<f32>,
    range_high: vec3<f32>,
    background: vec4<f32>,
) -> vec4<f32> {
    // ISO 32000-2 §8.7.4.5.2: points outside the transformed domain rectangle
    // \"shall be painted with the shading's background colour (Background); if
    // the shading dictionary has no Background entry, such points shall be left
    // unpainted\". Discarded, never clamped: clamping would smear the edge
    // colour at full alpha over everything between the domain and the clip.
    if (!(x >= domain.x && x <= domain.y && y >= domain.z && y <= domain.w)) {
        return background;
    }
    var s0: f32 = 0.0;
    var s1: f32 = 0.0;
    var s2: f32 = 0.0;
    var s3: f32 = 0.0;
    // The point, in the shading's own space, into the two slots the walk
    // reserved for it. Already known to be inside the domain rectangle.
    s0 = x;
    s1 = y;
    s2 = 1.0f;
    { // ISO 32000-2 Table 42, stack operator: read every source, then write
        let t0 = s0;
        s2 = t0;
    }
    s3 = 1.0f;
    { // ISO 32000-2 Table 42, stack operator: read every source, then write
        let t0 = s1;
        s3 = t0;
    }
    s2 = ps_mul(s2, s3); // ISO 32000-2 Table 42: mul
    // ISO 32000-2 §7.10.1: \"output values produced by the function shall be
    // clipped to the range\".
    return vec4<f32>(ps_clip(s0, range_low[0], range_high[0]), ps_clip(s1, range_low[1], range_high[1]), ps_clip(s2, range_low[2], range_high[2]), 1.0);
}
";

/// The emitted text, pinned. If this fails because the emitter changed on purpose, update the
/// constant **and** bump `quorra_gpu::function::GENERATOR_REVISION` — a persisted pipeline
/// cache keys on the revision, and a key whose meaning drifted is a stale shader drawn as a
/// fresh one.
#[test]
fn the_emitted_function_is_what_the_golden_says() {
    let analysis = analyse(programs::COORDINATES).unwrap();
    let shader = generate(&analysis, UNIT_RGB).unwrap();
    assert_eq!(shader.function(), GOLDEN_COORDINATES);
}

/// §4.6 wants a frame to be a function of its inputs; a pipeline cache keyed by a hash needs
/// the stronger form. Same program, same bytes, every time.
#[test]
fn the_same_program_generates_byte_identical_wgsl() {
    for witness in programs::all() {
        let first = generate(&analyse(&witness.program).unwrap(), witness.range).unwrap();
        for _ in 0..4 {
            let again = generate(&analyse(&witness.program).unwrap(), witness.range).unwrap();
            assert_eq!(first.module(), again.module(), "{}", witness.name);
            assert_eq!(first.hash(), again.hash(), "{}", witness.name);
        }
    }
}

/// The hash is a fixed number, not merely a consistent one.
///
/// `FnOp` cannot derive `Hash`, so this hash is hand-written over `f32::to_bits` — and the
/// failure mode of getting it wrong is a *silent permanent cache miss*, which no compiler and
/// no other test in this file would notice. Pinning the value is the only thing that does.
///
/// If this fails after a deliberate change to `GENERATOR_REVISION`, the operator library or
/// the byte encoding, update the constant. If it fails for any other reason, the hash is not
/// stable and the cache is not a cache.
#[test]
fn the_hash_of_a_fixed_program_is_a_fixed_number() {
    let analysis = analyse(programs::COORDINATES).unwrap();
    assert_eq!(
        analysis.program_hash().value(),
        0xbe43_2b4e_eced_86e0,
        "program hash is {}",
        analysis.program_hash()
    );
    assert_eq!(
        analysis.shader_hash(UNIT_RGB).value(),
        0x4783_1147_d097_1a93,
        "shader hash is {}",
        analysis.shader_hash(UNIT_RGB)
    );
}

/// `0.0` and `-0.0` are two different literals that print as two different WGSL texts, so
/// merging them would hand one program the other's shader. `f32 == f32` says they are equal;
/// `f32::to_bits` says they are not, and the hash uses the second on purpose.
#[test]
fn positive_and_negative_zero_are_two_programs() {
    let positive = analyse(&[FnOp::PushReal(0.0)]).unwrap();
    let negative = analyse(&[FnOp::PushReal(-0.0)]).unwrap();
    assert_eq!(
        0.0_f32, -0.0_f32,
        "the premise: `==` cannot tell them apart"
    );
    assert_ne!(positive.program_hash(), negative.program_hash());
    assert_ne!(
        generate(&positive, UNIT_GRAY).unwrap().function(),
        generate(&negative, UNIT_GRAY).unwrap().function()
    );
}

/// And two programs that differ produce two hashes that differ — including in the one place a
/// summary would collide: the *value* of a literal, which changes the shader and nothing else
/// about the program's shape.
#[test]
fn the_hash_separates_programs_that_generate_different_shaders() {
    let mut seen: std::collections::HashMap<ProgramHash, &'static str> =
        std::collections::HashMap::new();
    for witness in programs::all() {
        let analysis = analyse(&witness.program).unwrap();
        let shader = generate(&analysis, witness.range).unwrap();
        if let Some(other) = seen.insert(shader.hash(), witness.name) {
            panic!("{} and {other} share a hash", witness.name);
        }
    }

    let one = generate(&analyse(&[FnOp::PushReal(0.25)]).unwrap(), UNIT_GRAY).unwrap();
    let other = generate(&analyse(&[FnOp::PushReal(0.26)]).unwrap(), UNIT_GRAY).unwrap();
    assert_ne!(one.hash(), other.hash());
    assert_ne!(one.function(), other.function());
}

/// The component count changes the emitted function, so it has to change the hash. The
/// range's *numbers* do not, because they are runtime parameters.
#[test]
fn the_hash_covers_the_component_count_and_not_the_bounds() {
    let analysis = analyse(programs::OUT_OF_RANGE).unwrap();
    let unit = generate(&analysis, UNIT_RGB).unwrap();
    let odd = generate(&analysis, ODD_RGB).unwrap();
    assert_eq!(unit.hash(), odd.hash());
    assert_eq!(unit.function(), odd.function());

    let grey = generate(&analysis, UNIT_GRAY).unwrap();
    assert_ne!(grey.hash(), unit.hash());
    assert_ne!(grey.function(), unit.function());
}

/// A literal is printed in Rust's shortest round-tripping form, so the text names exactly the
/// `f32` the caller compiled — no precision invented, none lost.
#[test]
fn a_literal_round_trips_through_the_emitted_text() {
    for value in [
        0.1_f32,
        -6.5,
        1e30,
        1e-30,
        f32::MIN_POSITIVE,
        -0.0,
        16_777_217.0,
    ] {
        let analysis = analyse(&[FnOp::PushReal(value)]).unwrap();
        let source = generate(&analysis, WIDE_GRAY).unwrap();
        let printed = source
            .function()
            .lines()
            .find_map(|line| line.trim().strip_prefix("s2 = ")?.strip_suffix("f;"))
            .unwrap_or_else(|| panic!("no literal in\n{}", source.function()));
        assert_eq!(
            printed.parse::<f32>().unwrap().to_bits(),
            value.to_bits(),
            "{value:?} printed as {printed}"
        );
    }
}

/// ISO 32000-2 §8.7.4.5.2's rule for a point outside the domain rectangle is in the emitted
/// text, it discards, and **it does not clamp**. The input clamp revision 1 of the pinned
/// vocabulary asked for is asserted absent, because its absence is the correction.
#[test]
fn the_domain_is_a_test_rather_than_a_clamp() {
    let analysis = analyse(programs::COORDINATES).unwrap();
    let function = generate(&analysis, UNIT_RGB)
        .unwrap()
        .function()
        .to_string();

    assert!(function.contains("ISO 32000-2 §8.7.4.5.2:"));
    assert!(function.contains("return background;"));
    assert!(
        function
            .contains("if (!(x >= domain.x && x <= domain.y && y >= domain.z && y <= domain.w))")
    );
    assert!(
        !function.contains("ps_clip(x"),
        "the input must not be clamped into the domain:\n{function}"
    );
    assert!(!function.contains("ps_clip(y"));
    assert!(function.contains("s0 = x;"));
    assert!(function.contains("s1 = y;"));
}

/// §7.10.1's *output* clip is still a clamp, still required, and carries its clause.
#[test]
fn the_output_is_clamped_with_its_clause() {
    let analysis = analyse(programs::OUT_OF_RANGE).unwrap();
    let function = generate(&analysis, ODD_RGB).unwrap().function().to_string();
    assert_eq!(function.matches("range_low[").count(), 3);
    assert_eq!(function.matches("range_high[").count(), 3);
    assert_eq!(function.matches("ISO 32000-2 §7.10.1:").count(), 1);
}

/// `DeviceGray` reads one component, clips it against the one pair its `Range` carries, and
/// paints it in all three channels.
#[test]
fn a_grey_range_clips_one_component_and_replicates_it() {
    let analysis = analyse(programs::CONSTANT_GREY).unwrap();
    let function = generate(&analysis, FnRange::Gray([0.1, 0.9]))
        .unwrap()
        .function()
        .to_string();
    assert!(function.contains("let grey = ps_clip(s2, range_low[0], range_high[0]);"));
    assert!(function.contains("return vec4<f32>(grey, grey, grey, 1.0);"));
    assert_eq!(function.matches("range_low[").count(), 1);
}

/// Every operator that reaches the shader carries its Table 42 citation, which is CLAUDE.md
/// principle 5 applied to WGSL: a shader is write-only code unless the rule it implements is
/// stated beside it.
#[test]
fn every_emitted_operator_cites_table_42() {
    for witness in programs::all() {
        let analysis = analyse(&witness.program).unwrap();
        let shader = generate(&analysis, witness.range).unwrap();
        for line in shader.function().lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('s') && trimmed.contains("ps_") && !trimmed.contains("ps_clip") {
                assert!(
                    trimmed.contains("ISO 32000-2 Table 42:"),
                    "{}: {trimmed}",
                    witness.name
                );
            }
        }
    }
}

/// The module is the operator library plus the function, and the entry point is the name the
/// composing lane will call. Both are asserted so that a rename cannot happen quietly.
#[test]
fn the_module_is_the_library_plus_the_function() {
    let analysis = analyse(programs::ARITHMETIC_ONLY).unwrap();
    let shader = generate(&analysis, WIDE_RGB).unwrap();
    assert!(shader.module().starts_with(OPERATORS));
    assert!(shader.module().ends_with(shader.function()));
    assert!(shader.function().contains(ENTRY_POINT));
    assert_eq!(ENTRY_POINT, "quorra_function_evaluate");
}

/// A `Permute` reads every source before writing any target. `exch` is the case that proves
/// it: two sequential moves would leave both slots holding one value.
#[test]
fn a_permutation_reads_every_source_before_it_writes() {
    let analysis = analyse(&[FnOp::Dup, FnOp::Exch]).unwrap();
    let function = generate(&analysis, WIDE_RGB)
        .unwrap()
        .function()
        .to_string();
    let block = function
        .split("{ // ISO 32000-2 Table 42, stack operator")
        .nth(2)
        .unwrap_or_else(|| panic!("no second stack block in\n{function}"));
    let write = block.find("s1 = t").unwrap_or(usize::MAX);
    let last_read = block.rfind("let t").unwrap_or(0);
    assert!(last_read < write, "a write precedes a read in\n{block}");
}
