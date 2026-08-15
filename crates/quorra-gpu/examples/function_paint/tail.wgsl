// The placement: a full-viewport triangle whose fragments each evaluate the program
// once. `evaluate` is defined above — by the interpreter or by the generated shader —
// and WGSL has no forward declarations, which is why this file is last.

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle covering the clip volume: (-1,-1), (3,-1), (-1,3).
    let x = f32(index & 1u) * 4.0 - 1.0;
    let y = f32((index >> 1u) & 1u) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    // §10.7.4's centre rule: `at.xy` is already the pixel centre, so the value a
    // device pixel carries is the function at that centre and nothing is interpolated.
    let s = u.inv.x * at.x + u.inv.z * at.y + u.off.x;
    let t = u.inv.y * at.x + u.inv.w * at.y + u.off.y;
    // §8.7.4.5.2: inputs are clipped to Domain.
    let x = clamp(s, u.domain.x, u.domain.y);
    let y = clamp(t, u.domain.z, u.domain.w);
    return vec4<f32>(evaluate(x, y), 1.0);
}
