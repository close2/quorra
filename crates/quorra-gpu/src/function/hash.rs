//! The content hash a generated pipeline is cached by.
//!
//! One responsibility: **a number that is the same for two programs that generate the same
//! shader, and different otherwise.** `doc/adr/0053` makes the paint "a generated shader
//! cached by the program's hash", and this is that hash; the cache itself is not built here.
//!
//! # Why this is not [`crate::keyhash::KeyHasher`], and not `DefaultHasher`
//!
//! `KeyHasher` is a *table* hasher — it exists to spread bucket indices for the frame's hot
//! maps, and its contract is speed plus avalanche, not stability. The standard library's
//! `DefaultHasher` is explicitly not stable across releases and is randomly seeded in a
//! `HashMap`. This one is asked for something neither offers: **the same program hashes to
//! the same number in every process, on every machine, for as long as the generator is
//! unchanged** — because the number keys a pipeline cache that may be persisted, and a
//! persisted key whose meaning drifts is a stale pipeline drawn as a fresh one.
//!
//! FNV-1a over a canonical byte encoding, which is a few lines whose behaviour a reader can
//! verify by inspection. It is not a cryptographic hash and must never be used where an
//! adversary chooses the keys; here the keys come from the caller's own compiler, and a
//! collision costs a wrong shader — which is why the collision domain is kept small by
//! hashing the *inputs to generation* rather than a summary of them.
//!
//! # The one discipline this asks for, and the test that enforces it
//!
//! The hash covers the program, the `Range`'s component count and
//! [`GENERATOR_REVISION`] — **not the emitted text**, because a cache lookup that had to
//! generate the text first would not be a cache. So a change to the emitter that alters the
//! output without altering any of those three would leave two different shaders sharing a
//! key.
//!
//! That is why [`GENERATOR_REVISION`] exists and why `tests/function_generate.rs` pins the
//! emitted source of a golden program: any edit to the emitter breaks that test, and its
//! failure message is where the instruction to bump the revision lives. The operator library
//! needs no such discipline — its text is hashed directly, because it is a compile-time
//! constant and costs one pass over 8 kB.

use std::fmt;

use quorra_scene::FnOp;

use super::OPERATORS;

/// Bumped whenever [`super::generate`] emits different WGSL for the same inputs.
///
/// A number rather than a date so that it orders, and a constant rather than a git revision
/// so that it is a decision someone takes rather than a fact about a checkout.
pub const GENERATOR_REVISION: u32 = 1;

/// The identity of a generated shader: equal exactly when the same WGSL would be emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramHash(u64);

impl ProgramHash {
    /// The hash as the number a cache keys on.
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ProgramHash {
    /// Sixteen hexadecimal digits, zero-padded, so that two hashes sort and compare as text
    /// the way they do as numbers.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// The FNV-1a state. One multiply and one xor per byte, with the 64-bit parameters from the
/// algorithm's own definition.
struct Fnv(u64);

impl Fnv {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn byte(&mut self, value: u8) {
        // Wrapping is the arithmetic FNV-1a specifies, not an overflow being tolerated.
        self.0 = (self.0 ^ u64::from(value)).wrapping_mul(Self::PRIME);
    }

    fn bytes(&mut self, values: &[u8]) {
        for value in values {
            self.byte(*value);
        }
    }

    fn word(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }
}

/// The hash of a program, independent of the `Range` any shading gives it.
pub(super) fn hash(program: &[FnOp]) -> ProgramHash {
    let mut state = Fnv::new();
    state.word(GENERATOR_REVISION);
    state.bytes(OPERATORS.as_bytes());
    // The length goes in as well as the instructions: without it, a program and the same
    // program with a trailing zero-encoded instruction would be indistinguishable if one
    // were ever added.
    state.word(u32::try_from(program.len()).unwrap_or(u32::MAX));
    for op in program {
        let (code, payload) = encode(*op);
        state.byte(code);
        state.word(payload);
    }
    ProgramHash(state.0)
}

/// The shader's hash: a program's, mixed with the component count that decides how many
/// values the emitted function reads and how many channels it fills.
pub(super) fn with_components(program: ProgramHash, components: usize) -> ProgramHash {
    let mut state = Fnv(program.0);
    state.word(u32::try_from(components).unwrap_or(u32::MAX));
    ProgramHash(state.0)
}

/// One instruction as an opcode and a 32-bit payload.
///
/// [`FnOp`] cannot derive `Hash` or `Eq`, because `PushReal` holds an `f32`. That is why the
/// payload below is `f32::to_bits` rather than any comparison, and why **`0.0` and `-0.0`
/// hash differently, deliberately**: they are two different literals, they print as two
/// different WGSL texts, and a hash that merged them would hand one program the other's
/// shader. The opposite failure — a hash that varied for one program — would be a silent
/// permanent cache miss rather than a compile error, which is why
/// `tests/function_generate.rs` pins a literal value for a fixed program.
///
/// The opcodes are written out rather than taken from a discriminant so that reordering
/// [`FnOp`]'s variants — a change with no other consequence — cannot silently invalidate
/// every persisted cache key.
fn encode(op: FnOp) -> (u8, u32) {
    match op {
        FnOp::PushReal(value) => (0, value.to_bits()),
        FnOp::PushInt(value) => (1, value.cast_unsigned()),
        FnOp::PushBool(value) => (2, u32::from(value)),
        FnOp::Abs => (3, 0),
        FnOp::Add => (4, 0),
        FnOp::Atan => (5, 0),
        FnOp::Ceiling => (6, 0),
        FnOp::Cos => (7, 0),
        FnOp::Cvi => (8, 0),
        FnOp::Cvr => (9, 0),
        FnOp::Div => (10, 0),
        FnOp::Exp => (11, 0),
        FnOp::Floor => (12, 0),
        FnOp::Idiv => (13, 0),
        FnOp::Ln => (14, 0),
        FnOp::Log => (15, 0),
        FnOp::Mod => (16, 0),
        FnOp::Mul => (17, 0),
        FnOp::Neg => (18, 0),
        FnOp::Round => (19, 0),
        FnOp::Sin => (20, 0),
        FnOp::Sqrt => (21, 0),
        FnOp::Sub => (22, 0),
        FnOp::Truncate => (23, 0),
        FnOp::And => (24, 0),
        FnOp::Bitshift => (25, 0),
        FnOp::Eq => (26, 0),
        FnOp::Ge => (27, 0),
        FnOp::Gt => (28, 0),
        FnOp::Le => (29, 0),
        FnOp::Lt => (30, 0),
        FnOp::Ne => (31, 0),
        FnOp::Not => (32, 0),
        FnOp::Or => (33, 0),
        FnOp::Xor => (34, 0),
        FnOp::Copy => (35, 0),
        FnOp::Dup => (36, 0),
        FnOp::Exch => (37, 0),
        FnOp::Index => (38, 0),
        FnOp::Pop => (39, 0),
        FnOp::Roll => (40, 0),
        FnOp::JumpUnless { target } => (41, target),
        FnOp::Jump { target } => (42, target),
    }
}
