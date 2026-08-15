// ISO 32000-2 Table 42, one WGSL function per operator.
//
// Both shapes of the spike call these: the interpreter from its switch, the generated
// shader as the right-hand side of an assignment. That is deliberate — it means a
// difference between the two shapes cannot be a difference of arithmetic, so the
// measurement compares dispatch and nothing else.
//
// Values are f32 throughout. §7.10.5.1 gives the calculator integers, reals and
// booleans; the integers are carried in f32 and the type is resolved at compile time
// (see `walk.rs`). `true` is 1.0 and `false` is 0.0, which is what makes `and`, `or`
// and `xor` need only their bitwise reading.
//
// Where ISO 32000-2 states an error condition rather than a value — division by zero,
// `ln` of a non-positive number — this spike yields 0.0 and says so here, because the
// caller's CPU evaluator does the same and a fragment shader has nowhere to raise to.
// A production paint would decide that deliberately; §5.2 of their document is where
// it would be argued.

fn ps_to_int(a: f32) -> i32 {
    // Saturating: a program that reaches ±2^31 has already lost exactness in f32, and
    // a wrapped integer would be a plausible-looking wrong colour (CLAUDE.md §6).
    return i32(clamp(trunc(a), -2147483648.0, 2147483647.0));
}

fn ps_abs(a: f32) -> f32 { return abs(a); }
fn ps_add(a: f32, b: f32) -> f32 { return a + b; }

// `num den atan` — the angle in degrees, in [0, 360).
fn ps_atan(a: f32, b: f32) -> f32 {
    let degrees = atan2(a, b) * 57.29577951308232;
    return select(degrees, degrees + 360.0, degrees < 0.0);
}

fn ps_ceiling(a: f32) -> f32 { return ceil(a); }
fn ps_cos(a: f32) -> f32 { return cos(a * 0.017453292519943295); }
fn ps_cvi(a: f32) -> f32 { return trunc(a); }
fn ps_cvr(a: f32) -> f32 { return a; }
fn ps_div(a: f32, b: f32) -> f32 { return select(a / b, 0.0, b == 0.0); }
fn ps_exp(a: f32, b: f32) -> f32 { return pow(a, b); }
fn ps_floor(a: f32) -> f32 { return floor(a); }

fn ps_idiv(a: f32, b: f32) -> f32 {
    let d = trunc(b);
    return select(trunc(trunc(a) / d), 0.0, d == 0.0);
}

fn ps_ln(a: f32) -> f32 { return select(0.0, log(a), a > 0.0); }
fn ps_log(a: f32) -> f32 { return select(0.0, log2(a) * 0.30102999566398120, a > 0.0); }

fn ps_mod(a: f32, b: f32) -> f32 {
    let d = trunc(b);
    return select(trunc(a) % d, 0.0, d == 0.0);
}

fn ps_mul(a: f32, b: f32) -> f32 { return a * b; }
fn ps_neg(a: f32) -> f32 { return -a; }

// Half away from zero, which is what PostScript's `round` does; WGSL's own `round`
// is half to even, so it is not the operator this is.
fn ps_round(a: f32) -> f32 { return sign(a) * floor(abs(a) + 0.5); }

fn ps_sin(a: f32) -> f32 { return sin(a * 0.017453292519943295); }
fn ps_sqrt(a: f32) -> f32 { return select(0.0, sqrt(a), a > 0.0); }
fn ps_sub(a: f32, b: f32) -> f32 { return a - b; }
fn ps_truncate(a: f32) -> f32 { return trunc(a); }

fn ps_and(a: f32, b: f32) -> f32 { return f32(ps_to_int(a) & ps_to_int(b)); }
fn ps_or(a: f32, b: f32) -> f32 { return f32(ps_to_int(a) | ps_to_int(b)); }
fn ps_xor(a: f32, b: f32) -> f32 { return f32(ps_to_int(a) ^ ps_to_int(b)); }

fn ps_bitshift(a: f32, b: f32) -> f32 {
    let value = ps_to_int(a);
    let shift = ps_to_int(b);
    if (shift >= 32 || shift <= -32) { return 0.0; }
    if (shift >= 0) { return f32(value << u32(shift)); }
    return f32(value >> u32(-shift));
}

// Exact equality, which is what Table 42 says. The caller's CPU evaluator compares
// with an absolute f32 epsilon instead; that difference is recorded in the spike's
// write-up rather than copied here (CLAUDE.md principle 5).
fn ps_eq(a: f32, b: f32) -> f32 { return f32(a == b); }
fn ps_ne(a: f32, b: f32) -> f32 { return f32(a != b); }
fn ps_ge(a: f32, b: f32) -> f32 { return f32(a >= b); }
fn ps_gt(a: f32, b: f32) -> f32 { return f32(a > b); }
fn ps_le(a: f32, b: f32) -> f32 { return f32(a <= b); }
fn ps_lt(a: f32, b: f32) -> f32 { return f32(a < b); }

// The two readings of `not`: logical for a boolean, one's complement for an integer.
// Which one an occurrence means is decided before the frame, by `walk::analyse`.
fn ps_not(a: f32) -> f32 { return f32(a == 0.0); }
fn ps_bit_not(a: f32) -> f32 { return f32(~ps_to_int(a)); }
