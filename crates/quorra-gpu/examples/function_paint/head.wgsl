// What every shape of the paint is given: the map from a device pixel to the
// shading's parameter space, and the domain it is clipped to.
//
// ISO 32000-2 §8.7.4.5.2: a type 1 shading is "a function of two variables" over a
// Domain, placed by a Matrix. `inv` and `off` are the inverse of that placement
// composed with the page-to-device transform, so a fragment's own coordinate is the
// only input the paint needs — which is the property §3 of the caller's document
// calls "pure".

struct Uniforms {
    // The 2x2 of the device -> parameter map, column-major: (a, b, c, d).
    inv: vec4<f32>,
    // Its translation.
    off: vec2<f32>,
    // (instruction count, output components). The interpreter reads the first; the
    // generated shader needs neither, and it is why it can be faster.
    counts: vec2<u32>,
    // Domain, as ISO 32000-2 §8.7.4.5.2 orders it: x0 x1 y0 y1.
    domain: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
