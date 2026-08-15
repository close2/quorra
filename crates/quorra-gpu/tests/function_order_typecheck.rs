//! What `gt`, `ge`, `lt` and `le` answer for a **boolean** operand — pinned, not endorsed.
//!
//! # This file asserts a behaviour it does not defend
//!
//! PLRM3's entry for the four order comparisons takes `num₁ num₂`, and its own sentence is
//! the one `doc/notes-function-wiring.md` §2.3 quotes:
//!
//! > If the operands are of other types … a `typecheck` error occurs.
//!
//! We do not raise it. On the operand stack a boolean *is* the `f32` `1.0` or `0.0`, the
//! lowering emits the numeric comparison, and the shader answers with a value where the
//! entry has none. That is a deliberate hold rather than an oversight — `doc/notes-function-wiring.md`
//! §2.3 records why: it is the same shape as every other guarded error in
//! `function_ops.wgsl` (a zero divisor, a negative `sqrt`), and ADR 0053 §3.2 already has
//! the guard value open as a contract question with the caller. Changing one member of that
//! family without the others would be a third reading of the same silence.
//!
//! **So this file exists to make the hold visible.** The conformance corpus states these
//! cases as `Expectation::Error`, which means it asserts *no value* for them: today's
//! answers were therefore reachable by nothing at all, and a later change — to a refusal at
//! admission, to a guard of 0.0, to anything — would have moved them silently. With this
//! file, the change breaks a test whose comment says what it is breaking, which is the
//! difference between a decision and a drift.
//!
//! Nothing here is derived from ISO 32000-2, and none of it may be cited as though it were.
//! Every assertion is of the form *"today the device computes X"*, and the clause is quoted
//! beside it to say what it would have to compute to be right.
//!
//! # Where the values are read
//!
//! Through the compute harness, at full `f32` precision: the question is what the program
//! computes, and ADR 0006's 8-bit store would put a rounding step between the shader and
//! the assertion for no gain (`function_device.rs`'s header states the same reason at
//! length). A comparison leaves a *boolean* on the stack, and §7.10.5.3 forbids a boolean
//! output — "It shall be an error … for any of them to be objects other than numbers" — so
//! every program below converts it with `{1}{0} ifelse`, which is the idiom the corpus's
//! own relational cases use.

// Test-file lint policy as in m1.rs; the values here are exact 1.0 and 0.0 out of an
// `ifelse`, so an epsilon would weaken the pin into one three behaviours pass.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

mod function_support;

use function_support::compute::{Compute, Shading};
use quorra_scene::{FnOp, FnRange};

/// `x y pop pop <tail> {1} {0} ifelse` — the program whose one output is 1 when `tail`
/// leaves `true` and 0 when it leaves `false`.
///
/// The two `pop`s discard the shading's own inputs, so the answer is a constant and the
/// point it is evaluated at cannot matter; §7.10.5.3 requires the output count to equal the
/// `Range`'s component count, which the single value below satisfies for `FnRange::Gray`.
fn decided(tail: &[FnOp]) -> Vec<FnOp> {
    let mut program = vec![FnOp::Pop, FnOp::Pop];
    program.extend_from_slice(tail);
    let branch = program.len();
    // An index into a program of a handful of instructions; the conversion cannot fail and
    // says so rather than truncating silently.
    let at = |offset: usize| {
        u32::try_from(branch.saturating_add(offset)).expect("a program of a few instructions")
    };
    // `JumpUnless` to the else arm; the then arm jumps past the end, which is how a
    // trailing `ifelse` lowers (`FnOp::JumpUnless`'s own documentation).
    program.push(FnOp::JumpUnless { target: at(3) });
    program.push(FnOp::PushInt(1));
    program.push(FnOp::Jump { target: at(4) });
    program.push(FnOp::PushInt(0));
    program
}

/// What the device computes for one such program: `true` as 1.0, `false` as 0.0.
fn answer(compute: &Compute, tail: &[FnOp]) -> f32 {
    let program = decided(tail);
    let colours = compute.run(
        &Shading {
            program: &program,
            range: FnRange::Gray([0.0, 1.0]),
            domain: [0.0, 1.0, 0.0, 1.0],
            background: None,
        },
        &[(0.5, 0.5)],
    );
    // `FnRange::Gray` returns the level in all three channels; any one of them is it.
    colours[0][0]
}

/// **The four order comparisons answer a boolean numerically.** PLRM3 raises `typecheck`;
/// we compute `true` as 1 and `false` as 0 and compare those.
///
/// The expectations below are *today's behaviour*, written down so that changing it is a
/// visible decision. Each row is a comparison PLRM3 gives no value at all, and the value in
/// the second column is the one the numeric reading produces.
#[test]
fn an_order_comparison_on_booleans_is_answered_numerically_today() {
    let Some(compute) = Compute::new() else {
        eprintln!("no adapter available; the order-comparison pin did not run");
        return;
    };
    let adapter = &compute.adapter;
    let t = FnOp::PushBool(true);
    let f = FnOp::PushBool(false);

    for (tail, today, reading) in [
        (vec![t, f, FnOp::Gt], 1.0_f32, "1 > 0"),
        (vec![t, f, FnOp::Lt], 0.0, "1 < 0"),
        (vec![t, f, FnOp::Ge], 1.0, "1 >= 0"),
        (vec![t, f, FnOp::Le], 0.0, "1 <= 0"),
        (vec![f, t, FnOp::Gt], 0.0, "0 > 1"),
        (vec![t, t, FnOp::Ge], 1.0, "1 >= 1"),
        (vec![t, t, FnOp::Gt], 0.0, "1 > 1"),
    ] {
        assert_eq!(
            answer(&compute, &tail),
            today,
            "{adapter}: PLRM3 makes this a `typecheck` and we answer it as {reading}; that \
             is ADR 0053 §3.2's open question and this test is its pin, not its defence"
        );
    }
}

/// The same for a **mixed** boolean and number, where the incoherence is easiest to see:
/// `eq` answers by type and the order comparisons answer by value, so today one program can
/// conclude that `true` is neither greater than nor less than `1` and also not equal to it.
///
/// `eq`'s answer is the one that *is* derived from the specification — PLRM3: "Simple
/// objects are equal if their types and values are the same", so a boolean and a number are
/// never equal, and `doc/notes-function-wiring.md` §2.2 records the corpus case that found
/// us answering the opposite. It is asserted here beside the unpinned three so that a
/// reader can see which of the four rows carries a clause and which carry only a behaviour.
#[test]
fn a_boolean_against_a_number_compares_by_type_for_eq_and_by_value_for_the_rest() {
    let Some(compute) = Compute::new() else {
        eprintln!("no adapter available; the order-comparison pin did not run");
        return;
    };
    let adapter = &compute.adapter;
    let t = FnOp::PushBool(true);
    let one = FnOp::PushInt(1);

    // Derived from PLRM3's `eq` entry: different types are never equal.
    assert_eq!(
        answer(&compute, &[t, one, FnOp::Eq]),
        0.0,
        "{adapter}: `true 1 eq` is false because the types differ (PLRM3, `eq`)"
    );
    assert_eq!(
        answer(&compute, &[t, one, FnOp::Ne]),
        1.0,
        "{adapter}: and `ne` is its negation"
    );
    // Not derived from anything: PLRM3 raises `typecheck` for both of these.
    assert_eq!(
        answer(&compute, &[t, one, FnOp::Ge]),
        1.0,
        "{adapter}: today `true 1 ge` is the numeric `1 >= 1`; PLRM3 raises `typecheck`"
    );
    assert_eq!(
        answer(&compute, &[t, one, FnOp::Le]),
        1.0,
        "{adapter}: today `true 1 le` is the numeric `1 <= 1`; PLRM3 raises `typecheck`. \
         With the two rows above, a program can conclude that `true` is both at least and \
         at most `1` while being unequal to it — which is the cost of the hold, written \
         down rather than argued about"
    );
}
