// Shape (i): one shader, a switch over the instruction list, the program as data.
//
// The property that makes this legal WGSL and not a hazard is the caller's
// (`QUORRA_FUNCTION_PAINT.md` §4): every jump is forward, so the instruction count
// bounds the execution. The loop below is therefore a `for` of exactly `u.counts.x`
// iterations — the bound is structural, not a guess at a safe maximum, and a program
// that tried to loop could not express it.
//
// `%SLOTS%` is the operand-stack depth, substituted by the host. It is a compile-time
// constant because a WGSL array must have one, which is the interpreter's single real
// constraint: the shader is compiled once for one depth, and a program needing more
// is refused by name before the frame.

%OPCODES%

const STACK_SLOTS: u32 = %SLOTS%u;

@group(0) @binding(1) var<storage, read> program: array<vec2<u32>>;

// Per-invocation state. Private rather than a function-local array so `push` and
// `pop` can be functions; the storage class is the same either way.
var<private> stack: array<f32, %SLOTS%>;
var<private> sp: u32;

fn push(value: f32) {
    if (sp < STACK_SLOTS) {
        stack[sp] = value;
        sp = sp + 1u;
    }
}

// A pop of an empty stack yields 0. ISO 32000-2 does not define it and PostScript
// would raise `stackunderflow`; the caller's evaluator returns 0.0, and the corpus
// witness `pi_seven_segment.pdf` *depends* on that — see the spike's write-up.
fn pop() -> f32 {
    if (sp > 0u) {
        sp = sp - 1u;
        return stack[sp];
    }
    return 0.0;
}

fn at(index: u32) -> f32 {
    if (index < sp) {
        return stack[index];
    }
    return 0.0;
}

fn reverse_range(first: u32, last: u32) {
    var lo = first;
    var hi = last;
    while (lo + 1u < hi) {
        let t = stack[lo];
        stack[lo] = stack[hi - 1u];
        stack[hi - 1u] = t;
        lo = lo + 1u;
        hi = hi - 1u;
    }
}

// `n j roll`, by three reversals rather than a temporary array: rotating the top n
// right by j is reverse(all), reverse(first j), reverse(rest). A scratch array of
// STACK_SLOTS floats per invocation is what this avoids.
fn roll(count: i32, by: i32) {
    if (count <= 0 || u32(count) > sp) {
        return;
    }
    let n = u32(count);
    var j = by % count;
    if (j < 0) {
        j = j + count;
    }
    let base = sp - n;
    reverse_range(base, sp);
    reverse_range(base, base + u32(j));
    reverse_range(base + u32(j), sp);
}

fn copy_top(count: i32) {
    if (count <= 0 || u32(count) > sp) {
        return;
    }
    let base = sp - u32(count);
    for (var k = 0u; k < u32(count); k = k + 1u) {
        push(stack[base + k]);
    }
}

fn evaluate(x: f32, y: f32) -> vec3<f32> {
    sp = 0u;
    push(x);
    push(y);

    var pc = 0u;
    // The bound. `u.counts.x` is the instruction count and no path revisits an
    // address, so this cannot end early for any reason but the `break`.
    for (var step = 0u; step < u.counts.x; step = step + 1u) {
        if (pc >= u.counts.x) {
            break;
        }
        let instruction = program[pc];
        let arg = instruction.y;
        var next = pc + 1u;

        switch (instruction.x) {
            case OP_PUSH_REAL: { push(bitcast<f32>(arg)); }
            case OP_PUSH_INT: { push(f32(bitcast<i32>(arg))); }
            case OP_PUSH_BOOL: { push(f32(arg)); }

            case OP_ABS: { push(ps_abs(pop())); }
            case OP_ADD: { let b = pop(); push(ps_add(pop(), b)); }
            case OP_ATAN: { let b = pop(); push(ps_atan(pop(), b)); }
            case OP_CEILING: { push(ps_ceiling(pop())); }
            case OP_COS: { push(ps_cos(pop())); }
            case OP_CVI: { push(ps_cvi(pop())); }
            case OP_CVR: { push(ps_cvr(pop())); }
            case OP_DIV: { let b = pop(); push(ps_div(pop(), b)); }
            case OP_EXP: { let b = pop(); push(ps_exp(pop(), b)); }
            case OP_FLOOR: { push(ps_floor(pop())); }
            case OP_IDIV: { let b = pop(); push(ps_idiv(pop(), b)); }
            case OP_LN: { push(ps_ln(pop())); }
            case OP_LOG: { push(ps_log(pop())); }
            case OP_MOD: { let b = pop(); push(ps_mod(pop(), b)); }
            case OP_MUL: { let b = pop(); push(ps_mul(pop(), b)); }
            case OP_NEG: { push(ps_neg(pop())); }
            case OP_ROUND: { push(ps_round(pop())); }
            case OP_SIN: { push(ps_sin(pop())); }
            case OP_SQRT: { push(ps_sqrt(pop())); }
            case OP_SUB: { let b = pop(); push(ps_sub(pop(), b)); }
            case OP_TRUNCATE: { push(ps_truncate(pop())); }

            case OP_AND: { let b = pop(); push(ps_and(pop(), b)); }
            case OP_BITSHIFT: { let b = pop(); push(ps_bitshift(pop(), b)); }
            case OP_EQ: { let b = pop(); push(ps_eq(pop(), b)); }
            case OP_GE: { let b = pop(); push(ps_ge(pop(), b)); }
            case OP_GT: { let b = pop(); push(ps_gt(pop(), b)); }
            case OP_LE: { let b = pop(); push(ps_le(pop(), b)); }
            case OP_LT: { let b = pop(); push(ps_lt(pop(), b)); }
            case OP_NE: { let b = pop(); push(ps_ne(pop(), b)); }
            case OP_NOT: { push(ps_not(pop())); }
            case OP_BIT_NOT: { push(ps_bit_not(pop())); }
            case OP_OR: { let b = pop(); push(ps_or(pop(), b)); }
            case OP_XOR: { let b = pop(); push(ps_xor(pop(), b)); }

            case OP_COPY: { copy_top(i32(pop())); }
            case OP_DUP: { let a = pop(); push(a); push(a); }
            case OP_EXCH: { let b = pop(); let a = pop(); push(b); push(a); }
            case OP_INDEX: {
                let n = i32(pop());
                if (n < 0 || u32(n) >= sp) { push(0.0); } else { push(at(sp - 1u - u32(n))); }
            }
            case OP_POP: { let discarded = pop(); }
            case OP_ROLL: { let j = i32(pop()); roll(i32(pop()), j); }

            case OP_JUMP_IF_FALSE: { if (pop() == 0.0) { next = arg; } }
            case OP_JUMP: { next = arg; }
            default: {}
        }
        pc = next;
    }

    // The program's outputs are what it left, top last: ISO 32000-2 §7.10.5.1 says
    // the results are "the values remaining on the stack", in order.
    let blue = pop();
    let green = pop();
    let red = pop();
    return vec3<f32>(red, green, blue);
}
