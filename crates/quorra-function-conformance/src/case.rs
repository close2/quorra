//! What a conformance case is: a program, what it is given, what it must produce, and
//! the clause that says so.
//!
//! The `citation` field is the point of this module. CLAUDE.md principle 5 requires
//! that "a test's expected value must be derivable from the specification, and its
//! comment must say *from where*"; a comment beside a table of numbers is a comment
//! nobody reads next to the number it justifies, so here the citation is a field of the
//! case and a case cannot be written without one.

use crate::table42::Table42;
use quorra_scene::function::{FnOp, FnRange};

/// What a case is about.
///
/// Most cases are about one Table 42 operator. Four concerns are not about any operator
/// at all, and an operator-by-operator corpus is exactly the shape that misses them:
/// §7.10's two clips, §7.10.5.3's rule about how many values a program may leave behind,
/// and a pop from an empty stack, which no clause defines at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subject {
    /// One of Table 42's operators.
    Operator(Table42),
    /// §7.10.1's clip of the input values into `Domain`, which happens before the
    /// program's first instruction.
    DomainClip,
    /// §7.10.1's clip of the output values into `Range`, which happens after its last.
    RangeClip,
    /// §7.10.5.3's rule that the number of values left on the stack shall equal the
    /// number of output variables.
    OutputCount,
    /// A pop from an empty operand stack, which no clause defines and the pinned
    /// vocabulary's decision 6 answers deliberately. Not about any one operator: any
    /// operator that pops can reach it.
    EmptyStackPop,
    /// A ground on which a program is refused before a frame is drawn.
    Refusal,
}

/// A condition PLRM3 names as an error in the operator's own entry.
///
/// These are PostScript's error names, reached here through ISO 32000-2 §7.10.5.2's
/// delegation. **None of them defines a value**: the whole content of an entry that
/// says `undefinedresult` is that the operation has no result, which is why these are
/// their own expectation rather than a number. ISO 32000-2 §8.7.4.5.2 contemplates the
/// same thing at the shading: "If the function is undefined at any point within the
/// declared domain rectangle, an error may occur".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PsError {
    /// PLRM3 `undefinedresult`: the operation has no result for these operands.
    UndefinedResult,
    /// PLRM3 `rangecheck`: an operand is outside the operator's domain.
    RangeCheck,
    /// PLRM3 `typecheck`: an operand is of the wrong type.
    TypeCheck,
    /// PLRM3 `stackunderflow`: an operator named more elements than the stack held.
    ///
    /// A plain *pop* never raises this here — the pinned vocabulary's decision 6 makes
    /// it yield 0 and a [`Report`] instead. It is reachable through `copy` and `roll`,
    /// which read a whole region rather than one value, and extending the zero rule to
    /// those would be a second invention on top of the first.
    StackUnderflow,
    /// PLRM3 `stackoverflow`, and ISO 32000-2 §7.10.5.3's "it shall be an error to
    /// overflow the stack".
    StackOverflow,
    /// ISO 32000-2 §7.10.5.3: "It shall be an error for the number of remaining
    /// operands to differ from the number of output variables specified by **Range**".
    /// The clause states the error without giving it a PostScript name.
    OutputCount,
}

/// How closely a computed value must match the expected one.
///
/// **Neither variant is a claim about the standard.** ISO 32000-2 states no precision
/// and no accuracy requirement for evaluating a function, and PLRM3 defers to the
/// hardware (`doc/research-function-paint-arithmetic.md` §1.7). What a tolerance says
/// is how this corpus's own oracle is compared with its own expectations: exactly where
/// the clause fixes an integer or a rounding, and to a stated absolute error where the
/// expected value is a decimal PLRM3 printed to six digits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tolerance {
    /// The value is fixed by the clause — an integer, a comparison, a rounding — and
    /// any difference at all is a defect.
    Exact,
    /// The clause fixes the mathematical value; the expected number is that value to
    /// the digits the source prints. The bound is the test's instrument.
    Absolute(f32),
}

/// A ground on which a program is refused before a frame is drawn.
///
/// `doc/spike-function-paint.md` §6 found these on constructed programs, and principle 6
/// is why they are refusals rather than approximations. Two of that spike's six grounds
/// are **not** here, because the pinned vocabulary retires them: an operator outside
/// Table 42 and a procedure that is not an `if`/`ifelse` operand are both unrepresentable
/// in [`FnOp`], and "a ground nobody can reach is not a ground".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// The program needs a deeper operand stack than §7.10.5.3's floor of 100 entries.
    OperandStackTooDeep,
    /// A `copy`, `index` or `roll` whose count is not statically resolvable: a
    /// generated shader cannot name a slot it cannot compute.
    NonLiteralStackCount,
    /// Two branches join with different stack depths, so no static slot assignment
    /// describes the join.
    JoinDepthMismatch,
    /// A `not` (or another type-sensitive operator) reached by a value whose type two
    /// branches disagree about, so the one-name-two-operators question has no static
    /// answer.
    AmbiguousOperandType,
    /// A jump whose target is not strictly ahead of it. Forward-only jumps are what
    /// bound execution, and the bound is validated rather than trusted.
    BackwardJump,
    /// A jump whose target is past the end of the program.
    JumpOutOfRange,
    /// The program cannot leave the number of values `Range` declares.
    OutputCountMismatch,
}

impl Refusal {
    /// Every refusal ground, so that a ground without a case is a failing test.
    pub const ALL: [Self; 7] = [
        Self::OperandStackTooDeep,
        Self::NonLiteralStackCount,
        Self::JoinDepthMismatch,
        Self::AmbiguousOperandType,
        Self::BackwardJump,
        Self::JumpOutOfRange,
        Self::OutputCountMismatch,
    ];

    /// Whether running the program once, on the case's own inputs, reaches the ground.
    ///
    /// Three of the seven are visible only to a static walk: a count that came off the
    /// stack is perfectly resolvable *at run time*, a join is only reached down one arm,
    /// and a type ambiguity is a property of two paths rather than of the one taken. For
    /// those, the corpus states the ground and the analyser is what must agree with it;
    /// for the other four the reference evaluator refuses, and the test says so.
    #[must_use]
    pub const fn reached_by_evaluation(self) -> bool {
        match self {
            Self::OperandStackTooDeep
            | Self::BackwardJump
            | Self::JumpOutOfRange
            | Self::OutputCountMismatch => true,
            Self::NonLiteralStackCount | Self::JoinDepthMismatch | Self::AmbiguousOperandType => {
                false
            }
        }
    }
}

/// A deliberate choice, made where the specification defines nothing, that the frame
/// must carry rather than adopt silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Report {
    /// The program popped a value the stack did not hold, and got 0. PostScript raises
    /// `stackunderflow`; ISO 32000-2 defines nothing. The pinned vocabulary's decision 6
    /// takes the caller's reading — their `pi_seven_segment.pdf` depends on it three
    /// times — and requires that a frame relying on it says so.
    EmptyStackPop,
}

/// What a case must produce.
///
/// Five states, and the third and fourth are the ones this corpus exists for. A case
/// that would have to guess is marked as a guess rather than given a number, because an
/// expectation nobody can derive is a curve fit with a test's name on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Expectation {
    /// The clause fixes the output values.
    Outputs {
        /// One value per output component, already clipped into `Range`.
        values: &'static [f32],
        /// How closely they must match, and why that is not a claim about the standard.
        tolerance: Tolerance,
        /// A choice the frame must carry, where the program relied on one.
        report: Option<Report>,
    },
    /// The operator's PLRM3 entry names an error for these operands, and no document
    /// defines a value.
    Error(PsError),
    /// Neither ISO 32000-2 nor PLRM3 defines anything here. The string says what was
    /// read and what it did not say; nothing in the corpus supplies a number.
    Undefined {
        /// What the two documents do and do not say, in one sentence.
        silence: &'static str,
    },
    /// The program is refused before a frame is drawn, on this ground.
    Refused(Refusal),
}

/// One conformance case: a program, what it is given, what it must produce, and where
/// that came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Case {
    /// A stable identifier, `operator/what-it-tests`, used in failure messages.
    pub name: &'static str,
    /// What the case is about.
    pub subject: Subject,
    /// The compiled program, exactly as a caller would hand it to us.
    pub program: &'static [FnOp],
    /// §7.10.5.3's initial operand stack: "The input variables shall constitute the
    /// initial operand stack". Most operator cases take none and push their own
    /// literals, which is a legal §7.10.5 function of zero inputs; a `FunctionPaint` is
    /// the two-input specialisation, and [`Case::two_input_program`] is the adaptor.
    pub inputs: &'static [f32],
    /// §7.10's `Domain`, as `[x_min, x_max, y_min, y_max]`. Inputs are clipped into it
    /// before the first instruction.
    pub domain: [f32; 4],
    /// §7.10's `Range`. Outputs are clipped into it after the last.
    pub range: FnRange,
    /// What the case must produce.
    pub expect: Expectation,
    /// Where the expectation came from: the clause or PLRM3 entry, quoted or named, and
    /// — where the reading is ours rather than the document's — that it is ours.
    pub citation: &'static str,
}

/// The domain every case carries unless it is about the domain clip: the unit square,
/// which is what both of the caller's witnesses declare.
pub const UNIT_DOMAIN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// The range an operator case carries: wide enough that §7.10's output clip is a
/// no-op, so that the case measures its operator and nothing else. The clip has its own
/// cases, with ranges that bite.
pub const OPEN_GRAY: FnRange = FnRange::Gray([f32::MIN, f32::MAX]);

/// As [`OPEN_GRAY`], for the stack operators, which are easiest to state as a program
/// leaving three values.
pub const OPEN_RGB: FnRange = FnRange::Rgb([
    [f32::MIN, f32::MAX],
    [f32::MIN, f32::MAX],
    [f32::MIN, f32::MAX],
]);

impl Case {
    /// A case whose clause fixes its outputs exactly.
    #[must_use]
    pub const fn exact(
        name: &'static str,
        subject: Subject,
        program: &'static [FnOp],
        values: &'static [f32],
        citation: &'static str,
    ) -> Self {
        Self {
            name,
            subject,
            program,
            inputs: &[],
            domain: UNIT_DOMAIN,
            range: OPEN_GRAY,
            expect: Expectation::Outputs {
                values,
                tolerance: Tolerance::Exact,
                report: None,
            },
            citation,
        }
    }

    /// A case whose expected value is a decimal the source printed, compared to a
    /// stated absolute bound.
    #[must_use]
    pub const fn near(
        name: &'static str,
        subject: Subject,
        program: &'static [FnOp],
        values: &'static [f32],
        bound: f32,
        citation: &'static str,
    ) -> Self {
        let mut case = Self::exact(name, subject, program, values, citation);
        case.expect = Expectation::Outputs {
            values,
            tolerance: Tolerance::Absolute(bound),
            report: None,
        };
        case
    }

    /// A case the operator's own entry names an error for.
    #[must_use]
    pub const fn error(
        name: &'static str,
        subject: Subject,
        program: &'static [FnOp],
        error: PsError,
        citation: &'static str,
    ) -> Self {
        let mut case = Self::exact(name, subject, program, &[], citation);
        case.expect = Expectation::Error(error);
        case
    }

    /// A case where the two documents define nothing, and the corpus says so.
    #[must_use]
    pub const fn undefined(
        name: &'static str,
        subject: Subject,
        program: &'static [FnOp],
        silence: &'static str,
        citation: &'static str,
    ) -> Self {
        let mut case = Self::exact(name, subject, program, &[], citation);
        case.expect = Expectation::Undefined { silence };
        case
    }

    /// A case a device must refuse before it draws.
    #[must_use]
    pub const fn refused(
        name: &'static str,
        program: &'static [FnOp],
        ground: Refusal,
        citation: &'static str,
    ) -> Self {
        let mut case = Self::exact(name, Subject::Refusal, program, &[], citation);
        case.expect = Expectation::Refused(ground);
        case
    }

    /// The same case with a non-empty initial operand stack.
    #[must_use]
    pub const fn with_inputs(mut self, inputs: &'static [f32]) -> Self {
        self.inputs = inputs;
        self
    }

    /// The same case with a `Domain` that is not the unit square.
    #[must_use]
    pub const fn with_domain(mut self, domain: [f32; 4]) -> Self {
        self.domain = domain;
        self
    }

    /// The same case with a `Range` of its own.
    #[must_use]
    pub const fn with_range(mut self, range: FnRange) -> Self {
        self.range = range;
        self
    }

    /// The same case, with the frame required to carry a report.
    #[must_use]
    pub const fn with_report(mut self, carried: Report) -> Self {
        if let Expectation::Outputs {
            values, tolerance, ..
        } = self.expect
        {
            self.expect = Expectation::Outputs {
                values,
                tolerance,
                report: Some(carried),
            };
        }
        self
    }

    /// The case's program as a two-input function, which is the shape a
    /// `FunctionPaint` has and therefore the shape a device runs.
    ///
    /// A device pushes `(x, y)` and nothing else, so a case declaring fewer than two
    /// inputs needs the surplus discarded first — and prepending instructions moves
    /// every jump target, which is the part that is easy to get wrong and is why this
    /// lives here rather than in each caller.
    #[must_use]
    pub fn two_input_program(&self) -> Vec<FnOp> {
        let surplus = 2usize.saturating_sub(self.inputs.len());
        let offset = u32::try_from(surplus).unwrap_or(u32::MAX);
        let mut out = vec![FnOp::Pop; surplus];
        out.extend(self.program.iter().map(|op| match *op {
            FnOp::Jump { target } => FnOp::Jump {
                target: target.saturating_add(offset),
            },
            FnOp::JumpUnless { target } => FnOp::JumpUnless {
                target: target.saturating_add(offset),
            },
            other => other,
        }));
        out
    }
}
