//! The tables the spike prints, and the two of them that are measurements.
//!
//! Kept apart from `main` because they answer different questions: `main` builds
//! pipelines and draws, this states what came out. Two of these do their own
//! measuring — [`cpu_reference`] evaluates the whole page on the processor, and
//! [`cross_adapter`] compares what two adapters drew — and they live here because
//! their result is a row in a table rather than a pipeline anyone draws with.

use std::time::Duration;

use crate::measure;
use crate::program;
use crate::walk::{self, Mode};
use crate::{AdapterRasters, Case, SIZES, SLOTS};

/// Do the two adapters draw the same bytes?
///
/// §4.6 of the brief and `CLAUDE.md`'s environment note both rest on the answer being
/// yes for the current backend — it is what lets the caller's CI use a software
/// rasteriser. A new paint does not inherit that promise; it has to be measured.
pub(crate) fn cross_adapter(rasters: &[AdapterRasters]) {
    let [(first_name, first), (second_name, second)] = rasters else {
        return;
    };
    println!("\n## {first_name} against {second_name}, byte for byte\n");
    for (label, left) in first {
        let Some((_, right)) = second.iter().find(|(other, _)| other == label) else {
            continue;
        };
        let found = measure::agreement(left, right);
        println!(
            "  {label:<24} exact {:>9}  off-by-one {:>7}  differing {:>7}  worst {:>3}",
            found.exact, found.off_by_one, found.differing, found.worst
        );
    }
}

pub(crate) fn load() {
    let uptime = std::process::Command::new("uptime")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_default();
    println!("load at start: {}", uptime.trim());
}

pub(crate) fn programs(cases: &[Case]) {
    println!("\n## the two programs, compiled\n");
    println!(
        "{:<15} {:>6} {:>5} {:>6} {:>9} {:>11} {:>8} {:>10}",
        "program", "bytes", "ops", "depth", "branches", "underflows", "forward", "input-dep"
    );
    for case in cases {
        println!(
            "{:<15} {:>6} {:>5} {:>6} {:>9} {:>11} {:>8} {:>10}",
            case.name,
            case.bytes,
            case.program.ops.len(),
            case.interpreted.max_depth,
            case.interpreted.branches,
            case.interpreted.underflows,
            case.program.verify_forward_only(),
            format!(
                "{} of {}",
                case.program
                    .ops
                    .len()
                    .saturating_sub(case.interpreted.invariant),
                case.program.ops.len()
            )
        );
    }
    for case in cases {
        match &case.generated {
            Ok(facts) => {
                let wgsl = facts.wgsl.as_deref().unwrap_or_default();
                println!(
                    "  {}: generated shader is {} bytes, {} lines",
                    case.name,
                    wgsl.len(),
                    wgsl.lines().count()
                );
            }
            Err(why) => println!("  {}: shape (ii) refuses — {why}", case.name),
        }
    }
}

/// What each shape declines, and on what ground.
///
/// `QUORRA_FUNCTION_PAINT.md` §5.2 asks for "a refusal by name … for any program the
/// device declines". These are constructed programs, one per ground, so the grounds
/// are demonstrated rather than merely listed — a ground nobody can reach is not a
/// ground. Both witnesses pass all of them.
pub(crate) fn refusals() {
    println!("\n## what would be refused, and on what stated ground\n");
    let probes: [(&str, &str); 6] = [
        ("an operator outside Table 42", "{ 2 1 frobnicate }"),
        (
            "a stack deeper than the shader has slots",
            "{ 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 \
              1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 }",
        ),
        ("`index` whose count is computed", "{ add cvi index }"),
        (
            "an `ifelse` whose arms leave different depths",
            "{ 0.5 gt { 1 2 } { 3 } ifelse }",
        ),
        (
            "`not` on a value two branches disagreed about",
            "{ 0.5 gt { 1 } { 0.5 } ifelse not }",
        ),
        ("a procedure that is not a conditional's", "{ { 1 } }"),
    ];
    for (ground, source) in probes {
        let outcome = program::compile(source, 2, 3)
            .and_then(|program| walk::walk(&program, SLOTS, Mode::Generate))
            .map(|_| "accepted".to_string());
        match outcome {
            Ok(state) => println!("  {ground:<46} {state}"),
            Err(why) => println!("  {ground:<46} refused: {why}"),
        }
    }
    println!(
        "  {:<46} accepted, and the caller's own witness needs it",
        "a program that pops an empty operand stack"
    );
}

/// The processor's own answer, once: the cost anchor and the correctness oracle.
pub(crate) fn cpu_reference(cases: &[Case]) -> Vec<(Duration, Vec<u8>)> {
    println!("\n## the same arithmetic on the processor, one thread, no allocation\n");
    let (_, width, height) = SIZES[0];
    let mut out = Vec::new();
    for case in cases {
        let (elapsed, pixels) = measure::cpu_grid(&case.interpreted.ops, width, height);
        let pixels_count = (width as usize) * (height as usize);
        println!(
            "{:<15} {:>9.1} ms for {} px  ({:.0} ns/px)",
            case.name,
            elapsed.as_secs_f64() * 1e3,
            pixels_count,
            elapsed.as_nanos() as f64 / pixels_count as f64
        );
        out.push((elapsed, pixels));
    }
    out
}
