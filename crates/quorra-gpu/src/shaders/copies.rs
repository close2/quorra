//! The WGSL helpers that promise to be copies are held to it.
//!
//! WGSL has no `#include`. A helper needed by six shaders is therefore written six
//! times, and each copy carries a comment saying the copies are kept textually the
//! same — a promise that nothing enforced, so the copies could drift apart silently
//! and the divergence would show up as one lane masking differently from the others
//! on some page nobody has yet. This module is the enforcement: it walks
//! [`super::ALL`] — the same sources `pipeline.rs` compiles — cuts each promised
//! function out of every shader that defines it (with [`super::wgsl`]'s extractor,
//! shared with the shape-input gate), and requires the texts to be equal byte for byte.
//!
//! Two failure modes, both deliberate:
//!
//! - **Drift.** Two copies differ; the message names the two shaders and prints both
//!   texts, because "a helper drifted" without saying where is a bug report, not a
//!   test failure.
//! - **Disappearance.** Fewer copies than expected. A count is asserted so that
//!   deleting or renaming one copy fails here rather than leaving the remaining
//!   copies to agree with each other vacuously.
//!
//! And one guard on the guard: every comment in every shader that makes the sameness
//! promise must sit above a function this module knows about. A new copied helper that
//! promises sameness without being listed below fails the same test that would have
//! caught it drifting.
//!
//! What is *not* here: `fs_shape`, which four shaders define, and `coverage_at`, which
//! two do. Those bodies are genuinely different — a rectangle's shape is its coverage,
//! an image's is its sampled alpha, a shading's depends on whether the paint marks the
//! pixel at all (ADR 0011), and the coverage lane's `coverage_at` samples an atlas the
//! rectangle lane has no need of. Neither claims to be a copy: `coverage_at`'s comment
//! says "same formula as rect.wgsl", which is a claim about ADR 0005's arithmetic and
//! not about the text. Only a stated promise is guarded, because only a stated promise
//! is a thing a reader is entitled to rely on.
//!
//! **Why this is a unit test and not `tests/shader_copies.rs`.** It was one, with an
//! `include_str!` list of its own, and that list silently fell one shader behind
//! `super`'s: `function_lane.wgsl` (ADR 0053) defines a sixth `soft_mask_value` and
//! makes the promise, and the gate went on comparing five and asserting there were
//! five. An integration test cannot reach a private module, so the choice was between
//! publishing the list and moving the gate to where the list already is. ADR 0059 took
//! the second.

use super::ALL;
use super::wgsl::function_text;

/// The functions that promise to be copies, and how many copies must exist.
///
/// `soft_mask_value` is ADR 0037's mask lookup: the six shaders that sample a soft
/// mask are the rectangle and coverage lanes, the image, shading and function passes,
/// and the compositor. Its per-shader wrapper `soft_mask_at` is *not* listed, and must
/// not be: each lane reads the placement from a different uniform, and pushing that
/// difference into the wrapper is what leaves this function copyable at all.
const PROMISED_COPIES: &[(&str, usize)] = &[("soft_mask_value", 6)];

/// The sentence a copied helper's comment carries.
const PROMISE: &str = "the copies are kept textually the same";

/// A shader's text with its comment fences and line breaks taken out, so that a
/// sentence spanning three comment lines is one sentence to search for.
fn prose_flow(source: &str) -> String {
    source
        .split_whitespace()
        .filter(|token| *token != "//")
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every copy of every promised helper is byte-identical to every other copy.
#[test]
fn promised_helpers_are_textually_identical() {
    for (name, expected) in PROMISED_COPIES {
        let copies: Vec<(&str, &str)> = ALL
            .iter()
            .filter_map(|(shader, source)| {
                function_text(shader, source, name).map(|text| (*shader, text))
            })
            .collect();

        assert_eq!(
            copies.len(),
            *expected,
            "`{name}` is defined in {} shaders ({}), not the {expected} that are \
             supposed to carry it — a copy was deleted, renamed, or added without \
             updating PROMISED_COPIES",
            copies.len(),
            copies
                .iter()
                .map(|(shader, _)| *shader)
                .collect::<Vec<_>>()
                .join(", "),
        );

        let (first_shader, first_text) = copies[0];
        for (shader, text) in &copies[1..] {
            assert_eq!(
                first_text, *text,
                "`{name}` has drifted between {first_shader} and {shader}.\n\
                 --- {first_shader}\n{first_text}\n--- {shader}\n{text}",
            );
        }
    }
}

/// Nothing promises sameness without being guarded above.
#[test]
fn every_sameness_promise_is_guarded() {
    let guarded: Vec<&str> = PROMISED_COPIES.iter().map(|(name, _)| *name).collect();
    let mut promises = 0usize;

    for (shader, source) in ALL {
        let flow = prose_flow(source);
        for (at, _) in flow.match_indices(PROMISE) {
            promises = promises.saturating_add(1);
            // The promise is made in the comment block directly above the function it
            // is about, so the next `fn` names it.
            let rest = flow.get(at..).unwrap_or_default();
            let declared = rest.find("fn ").map(|from| rest.split_at(from).1);
            let name = declared
                .unwrap_or_else(|| {
                    panic!("{shader} promises sameness with no function below it");
                })
                .trim_start_matches("fn ")
                .split('(')
                .next()
                .unwrap_or_default();
            assert!(
                guarded.contains(&name),
                "{shader} promises that `{name}` is kept textually the same as its \
                 copies, but nothing checks it — add it to PROMISED_COPIES",
            );
        }
    }

    let expected: usize = PROMISED_COPIES.iter().map(|(_, count)| *count).sum();
    assert_eq!(
        promises, expected,
        "{promises} shaders make the sameness promise, {expected} functions' worth of \
         copies are guarded — a copy lost its comment or gained one",
    );
}
