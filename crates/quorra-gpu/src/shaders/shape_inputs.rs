//! No shape entry point can reach the soft mask (ADR 0066).
//!
//! ISO 32000-2 Table 57 gives one flag for two parameters:
//!
//! > alpha source … A flag specifying whether the current soft mask and alpha constant
//! > parameters shall be interpreted as shape values ( true ) or opacity values
//! > ( false ). … Initial value: false .
//!
//! A `quorra_scene::Scene` carries no such flag, so both are opacity: §11.6.4.3's `qm`
//! and §11.6.4.4's `qk`. `fs_shape` computes §11.4.6's `f`, and neither belongs in it.
//!
//! **Why this is a text gate and not five device tests.** The property is *the same
//! statement* about five lanes — `rect`, `coverage`, `image`, `shading` and
//! `function_lane` each define an `fs_shape` — and it is an absence, which a rendered
//! frame can only witness one fixture at a time. `tests/mask_shape_or_opacity.rs` draws
//! the pixels the clause requires; this module is what makes the claim about *every*
//! lane, exactly, including the two whose fixtures are expensive to build. It was a
//! five-lane defect when it was found (all five multiplied the mask into the shape,
//! against ADR 0025's own prose), which is the argument for gating it in one place.
//!
//! The reachability is over module-scope calls, which is the whole of a WGSL shader's
//! call graph: there are no function pointers, no recursion, and no dynamic dispatch.
//! `function_lane.wgsl` is a template whose generated half is appended at compile time;
//! that half is built from `function_ops.wgsl`'s operators, which bind no texture at
//! all, so it cannot reach a mask the template does not hand it.

use super::ALL;
use super::wgsl::{function_names, function_text};

/// The entry point that must not read the mask, and the helper that reads it.
///
/// `soft_mask_at` rather than `soft_mask_value`: the wrapper is what a lane calls, and
/// naming the wrapper makes the gate fail on the call a reader would actually add.
const SHAPE: &str = "fs_shape";
const MASK: &str = "soft_mask_at";

/// The entry point that must *still* reach it — the control, without which "the shape
/// does not read the mask" would also pass on a shader that had stopped masking at all.
const SOURCE: &str = "fs_main";

/// How many lanes define a shape entry point. A count, so that deleting or renaming one
/// `fs_shape` fails here rather than leaving the others to satisfy the gate vacuously.
const LANES_WITH_A_SHAPE_PASS: usize = 5;

/// The shortest chain of module-scope calls from `from` to `target`, or `None` when no
/// chain exists.
///
/// A *path* rather than a set, because "these reach the mask" without saying through
/// what is a bug report and not a test failure — the same standard
/// [`copies`](super::copies) holds itself to when it prints both drifted texts. A call
/// is a declared name followed by `(`, searched inside a function's own body; that is
/// the whole of a WGSL call graph, which has no function pointers and no recursion.
///
/// Breadth-first over `seen`, whose second field is each entry's predecessor, so the
/// chain is recovered by walking back from the hit.
fn call_path<'a>(
    shader: &str,
    source: &'a str,
    from: &'a str,
    target: &str,
) -> Option<Vec<&'a str>> {
    let declared = function_names(source);
    let mut seen: Vec<(&str, usize)> = vec![(from, usize::MAX)];
    let mut next = 0usize;
    while let Some((name, _)) = seen.get(next).copied() {
        let here = next;
        next = next.saturating_add(1);
        let Some(body) = function_text(shader, source, name) else {
            continue;
        };
        for callee in &declared {
            if *callee == name
                || !calls(body, callee)
                || seen.iter().any(|(seen_name, _)| seen_name == callee)
            {
                continue;
            }
            seen.push((callee, here));
            if *callee == target {
                return Some(chain_to(&seen, seen.len().saturating_sub(1)));
            }
        }
    }
    None
}

/// Whether `body` calls `callee`: the name followed by `(`, and **not preceded by an
/// identifier character**.
///
/// The second half is what keeps `soft_mask_at` from being found inside a hypothetical
/// `outer_soft_mask_at`. A false edge would fail the shape gate spuriously, which is the
/// harmless direction — but it would let the *control* pass vacuously, which is not.
fn calls(body: &str, callee: &str) -> bool {
    let opening = format!("{callee}(");
    body.match_indices(&opening).any(|(at, _)| {
        body.get(..at)
            .and_then(|head| head.chars().next_back())
            .is_none_or(|before| !before.is_alphanumeric() && before != '_')
    })
}

/// The chain from the walk's root to `at`, following each entry's predecessor.
fn chain_to<'a>(seen: &[(&'a str, usize)], at: usize) -> Vec<&'a str> {
    let mut chain = Vec::new();
    let mut cursor = at;
    while let Some((name, parent)) = seen.get(cursor).copied() {
        chain.push(name);
        if parent == usize::MAX {
            break;
        }
        cursor = parent;
    }
    chain.reverse();
    chain
}

/// Every lane's `fs_shape` computes §11.4.6's `f`, and no path out of it reaches
/// §11.6.4.3's mask.
#[test]
fn a_shape_pass_cannot_reach_the_soft_mask() {
    let mut lanes = 0usize;
    let mut offenders = Vec::new();
    for (shader, source) in ALL {
        if function_text(shader, source, SHAPE).is_none() {
            continue;
        }
        lanes = lanes.saturating_add(1);
        if let Some(path) = call_path(shader, source, SHAPE, MASK) {
            offenders.push(format!("{shader}: {}", path.join(" -> ")));
        }
    }
    assert!(
        offenders.is_empty(),
        "Table 57's alpha source flag is `false` unless a scene says otherwise, and no \
         scene can: §11.6.4.3's mask is opacity, so it may not weight §11.4.6's \
         replacement. These reach it from `{SHAPE}`: {offenders:?}"
    );
    assert_eq!(
        lanes, LANES_WITH_A_SHAPE_PASS,
        "{lanes} shaders define `{SHAPE}`, not the {LANES_WITH_A_SHAPE_PASS} lanes that \
         are supposed to — one was deleted, renamed, or added without updating this gate"
    );
}

/// **The control.** The same shaders' `fs_main` *does* reach the mask, so the assertion
/// above is about where the mask is applied and not about a lane that stopped applying
/// it (`doc/HANDOVER.md`: a gate whose assertion is an absence needs a control).
#[test]
fn the_source_pass_still_reads_the_soft_mask() {
    let mut silent = Vec::new();
    for (shader, source) in ALL {
        if function_text(shader, source, SHAPE).is_none() {
            continue;
        }
        if call_path(shader, source, SOURCE, MASK).is_none() {
            silent.push(*shader);
        }
    }
    assert!(
        silent.is_empty(),
        "every lane's `{SOURCE}` must still multiply §11.6.4.3's mask into the source \
         alpha — these do not, which would make the shape gate vacuous: {silent:?}"
    );
}
