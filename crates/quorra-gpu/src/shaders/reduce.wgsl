// Soft-mask reduction, on the device (ISO 32000-2 §11.5; RENDER_LIBRARY.md §4.2).
//
// The rendered mask group arrives as a premultiplied layer; this pass turns each
// pixel into one mask byte by §11.5.2 (alpha) or §11.5.3 (luminosity over an opaque
// backdrop), then §11.6.5.1's transfer table.
//
// # The byte-agreement contract
//
// The caller's `pdf_render::SoftMask::value` is the shared definition of what these
// bytes mean, and this shader is deliberately a second implementation of exactly its
// arithmetic: straight-alpha channel bytes recovered by the integer demultiply rule
// of ADR 0005, source-over onto the opaque backdrop via fma in the same operation
// order, the clause's own 0.30/0.59/0.11 coefficients, round-then-clamp, then the
// table. The output writes f32(byte)/255 to an R8 target, which lands exactly on
// level `byte` on every driver (the value is within 1e-7 of the level, far inside
// any rounding mode's reach) — so the reduction is byte-deterministic even though
// ADR 0006 rules out determinism for blended stores. `tests/m6.rs` holds all 256
// bytes of both rules against a transcription of the caller's function.
//
// §11.5.3's order is the order: composite onto the backdrop FIRST, then take the
// luminosity of the result. The other order is a different number wherever the
// group is partly transparent, and it produces a plausible picture.

struct Params {
    // 0 = §11.5.2 Alpha, 1 = §11.5.3 Luminosity.
    kind: u32,
    _pad: f32,
    _pad2: vec2f,
    // §11.6.5.1's /BC, resolved device RGB; alpha ignored (the clause makes the
    // backdrop fully opaque).
    backdrop: vec4f,
    // §11.6.5.1's /TR as 256 bytes, packed four per u32, identity when absent.
    table: array<vec4<u32>, 16>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var src_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) position: vec4f,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index >> 1u) * 4 - 1);
    out.position = vec4f(x, y, 0.0, 1.0);
    return out;
}

// The transfer table lookup: byte i lives in word i/4, byte lane i%4.
fn transfer(value: u32) -> u32 {
    let word = params.table[value / 16u][(value / 4u) % 4u];
    return (word >> ((value % 4u) * 8u)) & 0xFFu;
}

// ADR 0005's integer demultiply: straight = (premul·255 + a/2) / a, clamped.
fn demultiply(premul_byte: u32, alpha_byte: u32) -> u32 {
    return min((premul_byte * 255u + alpha_byte / 2u) / alpha_byte, 255u);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4f {
    let pixel = textureLoad(src_tex, vec2i(in.position.xy), 0);
    // Unorm loads are exact multiples of 1/255: scaling recovers the stored bytes.
    let a_byte = u32(round(pixel.a * 255.0));
    var derived: u32;
    if params.kind == 0u {
        // §11.5.2: "The mask value at each point shall then be derived from the
        // alpha of the group." Colours ignored.
        derived = a_byte;
    } else if a_byte == 0u {
        // Fully transparent over the opaque backdrop is the backdrop; mirror the
        // caller's arithmetic at alpha = 0 exactly (over() collapses to backdrop).
        let l = fma(0.30, params.backdrop.r, fma(0.59, params.backdrop.g, 0.11 * params.backdrop.b));
        derived = u32(clamp(round(l * 255.0), 0.0, 255.0));
    } else {
        // §11.5.3: source-over onto the fully opaque backdrop, then the luminosity
        // of the result — in the caller's own operation order (fma for fma).
        let alpha = f32(a_byte) / 255.0;
        let r = f32(demultiply(u32(round(pixel.r * 255.0)), a_byte)) / 255.0;
        let g = f32(demultiply(u32(round(pixel.g * 255.0)), a_byte)) / 255.0;
        let b = f32(demultiply(u32(round(pixel.b * 255.0)), a_byte)) / 255.0;
        let over_r = fma(r, alpha, params.backdrop.r * (1.0 - alpha));
        let over_g = fma(g, alpha, params.backdrop.g * (1.0 - alpha));
        let over_b = fma(b, alpha, params.backdrop.b * (1.0 - alpha));
        let l = fma(0.30, over_r, fma(0.59, over_g, 0.11 * over_b));
        derived = u32(clamp(round(l * 255.0), 0.0, 255.0));
    }
    let value = transfer(derived);
    // f32(value)/255 sits on the R8 level exactly; the store cannot move it.
    return vec4f(f32(value) / 255.0, 0.0, 0.0, 1.0);
}
