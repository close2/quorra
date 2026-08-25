//! The question a compute coverage lane stands or falls on: **does the CPU rasteriser's
//! exact trapezoid arithmetic, ported statement for statement to WGSL, produce the same
//! coverage bytes — against the CPU, between two runs on one adapter, and between two
//! adapters?**
//!
//! `raster/fill.rs` is the library's byte-identity provider today: coverage is rasterised
//! on the host precisely so that every adapter composites the same bytes (its module doc,
//! ADR 0006/0008). A device-side lane can only keep that property if the shader's
//! arithmetic is reproducible. The known hazards, named before the port was written:
//!
//! - **Fused multiply-add.** `raster/` bans `mul_add` (`tests/mul_add_hazard.rs`), but
//!   WGSL permits an implementation to contract `a * b + c`, and naga does not decorate
//!   against it — so `x + dy * dxdy` may round differently per compiler.
//! - **`round` ties.** Rust's `f32::round` is ties-away-from-zero; WGSL's `round` is
//!   ties-to-even. Coverage is non-negative, so `floor(x + 0.5)` matches Rust here and is
//!   what the shader uses.
//! - **`rem_euclid`.** For non-negative `x`, `x - 2·⌊x·0.5⌋` is exact in f32 (the halving
//!   and the doubling are exact, and the subtraction is exact by Sterbenz), so it equals
//!   `rem_euclid(2.0)`; the shader uses that form rather than WGSL's `%`, whose defined
//!   `trunc(x/y)` shape rounds twice.
//! - **Summation order.** The accumulator's cell totals depend on deposit order. The port
//!   keeps it by construction: one invocation owns one row, walks every edge in list
//!   order, and deposits into a private row array — the same per-cell order the CPU's
//!   edge-major walk produces, because `fill.rs` interpolates each slab from the edge's
//!   top rather than incrementally.
//!
//! The mirror of `fill_mask` below is a copy, not a call — `raster` is `pub(crate)` and
//! this spike changes nothing in the library to exist (the `function_paint` rule). If
//! `fill.rs` changes arithmetic, this file's mirror must follow; the cross-check that
//! catches drift is the `mosaic` case, whose expected bytes would move.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "test scaffolding mirroring fill.rs's own allow set (including its argument \
              bundle), one generated-shader string, and an LCG seed quoted as-is; a panic \
              is the test failing"
)]

use quorra_gpu::wgpu;

/// Region size the shader is generated for: one workgroup, one invocation per row.
const W: usize = 64;
const H: usize = 64;

// ---------------------------------------------------------------------------
// The CPU mirror of `raster/fill.rs`, edge-list form (subpath closing is done by the
// caller so the shader can consume a flat `vec4` list).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    NonZero,
    EvenOdd,
}

fn fill_mask_mirror(edges: &[[f32; 4]], rule: Rule) -> Vec<u8> {
    let (w, h) = (W, H);
    let mut acc = vec![0.0_f32; (w + 1) * h];
    let (fw, fh) = (w as f32, h as f32);
    for e in edges {
        accumulate_edge(&mut acc, w, fw, fh, e[0], e[1], e[2], e[3]);
    }
    let mut coverage = vec![0_u8; w * h];
    for y in 0..h {
        let mut running = 0.0_f32;
        for x in 0..w {
            running += acc[y * (w + 1) + x];
            let cov = match rule {
                Rule::NonZero => running.abs().min(1.0),
                Rule::EvenOdd => {
                    let m = running.abs().rem_euclid(2.0);
                    1.0 - (m - 1.0).abs()
                }
            };
            coverage[y * w + x] = (cov * 255.0).round() as u8;
        }
    }
    coverage
}

fn accumulate_edge(acc: &mut [f32], w: usize, fw: f32, fh: f32, x0: f32, y0: f32, x1: f32, y1: f32) {
    if y0 == y1 {
        return;
    }
    let (dir, top_x, top_y, bot_x, bot_y) = if y0 < y1 {
        (1.0_f32, x0, y0, x1, y1)
    } else {
        (-1.0, x1, y1, x0, y0)
    };
    let (top_x, top_y) = if top_y < 0.0 {
        (
            top_x + (bot_x - top_x) * (0.0 - top_y) / (bot_y - top_y),
            0.0,
        )
    } else {
        (top_x, top_y)
    };
    let (bot_x, bot_y) = if bot_y > fh {
        (top_x + (bot_x - top_x) * (fh - top_y) / (bot_y - top_y), fh)
    } else {
        (bot_x, bot_y)
    };
    if bot_y <= top_y {
        return;
    }
    let dxdy = (bot_x - top_x) / (bot_y - top_y);
    if !dxdy.is_finite() {
        return;
    }
    let mut y = top_y.floor().max(0.0);
    while y < bot_y {
        let row = y as usize;
        if row >= acc.len() / (w + 1) {
            break;
        }
        let entry_y = top_y.max(y);
        let exit_y = bot_y.min(y + 1.0);
        let entry_x = top_x + (entry_y - top_y) * dxdy;
        let exit_x = top_x + (exit_y - top_y) * dxdy;
        deposit_slab(
            &mut acc[row * (w + 1)..(row + 1) * (w + 1)],
            fw,
            dir,
            entry_x,
            entry_y,
            exit_x,
            exit_y,
        );
        y += 1.0;
    }
}

fn deposit_slab(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    if xs >= 0.0 && xs <= fw && xe >= 0.0 && xe <= fw {
        deposit_inside(row, fw, dir, xs, ys, xe, ye);
        return;
    }
    let (dx, dy) = (xe - xs, ye - ys);
    let mut cuts = [(0.0_f32, 0.0_f32); 2];
    let mut count = 0;
    if dx != 0.0 {
        for border in [0.0_f32, fw] {
            let t = (border - xs) / dx;
            if t > 0.0 && t < 1.0 {
                cuts[count] = (t, border);
                count += 1;
            }
        }
        if count == 2 && cuts[1].0 < cuts[0].0 {
            cuts.swap(0, 1);
        }
    }
    let (mut px, mut py) = (xs, ys);
    for (t, border) in cuts.iter().take(count).copied() {
        let (nx, ny) = (border, ys + dy * t);
        deposit_inside(row, fw, dir, px, py, nx, ny);
        (px, py) = (nx, ny);
    }
    deposit_inside(row, fw, dir, px, py, xe, ye);
}

fn deposit_inside(row: &mut [f32], fw: f32, dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {
    let xs = xs.clamp(0.0, fw);
    let xe = xe.clamp(0.0, fw);
    let (mut px, mut py) = (xs, ys);
    loop {
        let boundary = if xe > px {
            let b = px.floor() + 1.0;
            if b < xe { Some(b) } else { None }
        } else if xe < px {
            let b = px.ceil() - 1.0;
            if b > xe { Some(b) } else { None }
        } else {
            None
        };
        let (nx, ny) = match boundary {
            Some(b) => {
                let t = (b - xs) / (xe - xs);
                (b, ys + (ye - ys) * t)
            }
            None => (xe, ye),
        };
        let d = dir * (ny - py);
        if d != 0.0 {
            let xm = 0.5 * (px + nx);
            let cell = (xm.floor().max(0.0) as usize).min(row.len().saturating_sub(2));
            let frac = xm - cell as f32;
            row[cell] += d * (1.0 - frac);
            row[cell + 1] += d * frac;
        }
        if boundary.is_none() {
            break;
        }
        (px, py) = (nx, ny);
    }
}

// ---------------------------------------------------------------------------
// The WGSL port: one invocation per row, edges walked in list order into a private
// row accumulator, then the serial prefix sum — the CPU's own per-cell deposit order.
// ---------------------------------------------------------------------------

fn shader() -> String {
    format!(
        r"
const W: u32 = {W}u;
const H: u32 = {H}u;
const FW: f32 = {W}.0;
const FH: f32 = {H}.0;

@group(0) @binding(0) var<storage, read> edges: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> coverage: array<u32>;
@group(0) @binding(2) var<uniform> rule: vec4<u32>;

var<private> row_acc: array<f32, {ACC}>;

fn deposit_inside(dir: f32, xs_in: f32, ys: f32, xe_in: f32, ye: f32) {{
    let xs = clamp(xs_in, 0.0, FW);
    let xe = clamp(xe_in, 0.0, FW);
    var px = xs;
    var py = ys;
    loop {{
        var has_boundary = false;
        var b = 0.0;
        if (xe > px) {{
            b = floor(px) + 1.0;
            has_boundary = b < xe;
        }} else if (xe < px) {{
            b = ceil(px) - 1.0;
            has_boundary = b > xe;
        }}
        var nx = xe;
        var ny = ye;
        if (has_boundary) {{
            let t = (b - xs) / (xe - xs);
            nx = b;
            ny = ys + (ye - ys) * t;
        }}
        let d = dir * (ny - py);
        if (d != 0.0) {{
            let xm = 0.5 * (px + nx);
            let cell = min(u32(max(floor(xm), 0.0)), W - 1u);
            let frac = xm - f32(cell);
            row_acc[cell] += d * (1.0 - frac);
            row_acc[cell + 1u] += d * frac;
        }}
        if (!has_boundary) {{
            break;
        }}
        px = nx;
        py = ny;
    }}
}}

fn deposit_slab(dir: f32, xs: f32, ys: f32, xe: f32, ye: f32) {{
    if (xs >= 0.0 && xs <= FW && xe >= 0.0 && xe <= FW) {{
        deposit_inside(dir, xs, ys, xe, ye);
        return;
    }}
    let dx = xe - xs;
    let dy = ye - ys;
    var cut_t = array<f32, 2>(0.0, 0.0);
    var cut_b = array<f32, 2>(0.0, 0.0);
    var count = 0u;
    if (dx != 0.0) {{
        for (var i = 0u; i < 2u; i += 1u) {{
            let border = select(0.0, FW, i == 1u);
            let t = (border - xs) / dx;
            if (t > 0.0 && t < 1.0) {{
                cut_t[count] = t;
                cut_b[count] = border;
                count += 1u;
            }}
        }}
        if (count == 2u && cut_t[1] < cut_t[0]) {{
            let tt = cut_t[0]; cut_t[0] = cut_t[1]; cut_t[1] = tt;
            let bb = cut_b[0]; cut_b[0] = cut_b[1]; cut_b[1] = bb;
        }}
    }}
    var px = xs;
    var py = ys;
    for (var i = 0u; i < count; i += 1u) {{
        let nx = cut_b[i];
        let ny = ys + dy * cut_t[i];
        deposit_inside(dir, px, py, nx, ny);
        px = nx;
        py = ny;
    }}
    deposit_inside(dir, px, py, xe, ye);
}}

@compute @workgroup_size(64)
fn rasterize_rows(@builtin(global_invocation_id) id: vec3<u32>) {{
    let my_row = id.x;
    if (my_row >= H) {{ return; }}
    for (var i = 0u; i <= W; i += 1u) {{
        row_acc[i] = 0.0;
    }}
    let fy = f32(my_row);
    let n = arrayLength(&edges);
    for (var e = 0u; e < n; e += 1u) {{
        let edge = edges[e];
        let x0 = edge.x; let y0 = edge.y; let x1 = edge.z; let y1 = edge.w;
        if (y0 == y1) {{ continue; }}
        var dir = 1.0;
        var top_x = x0; var top_y = y0; var bot_x = x1; var bot_y = y1;
        if (y0 >= y1) {{
            dir = -1.0;
            top_x = x1; top_y = y1; bot_x = x0; bot_y = y0;
        }}
        if (top_y < 0.0) {{
            top_x = top_x + (bot_x - top_x) * (0.0 - top_y) / (bot_y - top_y);
            top_y = 0.0;
        }}
        if (bot_y > FH) {{
            bot_x = top_x + (bot_x - top_x) * (FH - top_y) / (bot_y - top_y);
            bot_y = FH;
        }}
        if (bot_y <= top_y) {{ continue; }}
        let dxdy = (bot_x - top_x) / (bot_y - top_y);
        // fill.rs guards `!dxdy.is_finite()`; WGSL may assume no NaN/Inf, so the guard
        // is stated as the magnitude test the comment there derives (slope past f32::MAX
        // means a slab under 2.4e-11 of a pixel). Test inputs stay far from the bound.
        if (abs(dxdy) > 3.0e38) {{ continue; }}
        // The CPU walks y from floor(top_y).max(0) to bot_y in steps of 1; this row
        // participates exactly when it lies in that integer sequence.
        if (fy < max(floor(top_y), 0.0) || fy >= bot_y) {{ continue; }}
        let entry_y = max(top_y, fy);
        let exit_y = min(bot_y, fy + 1.0);
        let entry_x = top_x + (entry_y - top_y) * dxdy;
        let exit_x = top_x + (exit_y - top_y) * dxdy;
        deposit_slab(dir, entry_x, entry_y, exit_x, exit_y);
    }}
    var running = 0.0;
    for (var x = 0u; x < W; x += 1u) {{
        running += row_acc[x];
        var cov = 0.0;
        if (rule.x == 0u) {{
            cov = min(abs(running), 1.0);
        }} else {{
            let a = abs(running);
            let m = a - 2.0 * floor(a * 0.5);
            cov = 1.0 - abs(m - 1.0);
        }}
        // Rust's `round` is ties-away-from-zero; coverage is non-negative, so this is it.
        coverage[my_row * W + x] = u32(floor(cov * 255.0 + 0.5));
    }}
}}
",
        ACC = W + 1,
    )
}

// ---------------------------------------------------------------------------
// Device plumbing, `function_support/compute.rs`'s shape.
// ---------------------------------------------------------------------------

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    name: String,
}

fn adapters() -> Vec<Gpu> {
    let instance = quorra_gpu::create_instance();
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .into_iter()
        .filter_map(|adapter| {
            let name = adapter.get_info().name;
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("coverage determinism"),
                    required_limits: adapter.limits(),
                    ..Default::default()
                }))
                .ok()?;
            Some(Gpu {
                device,
                queue,
                name,
            })
        })
        .collect()
}

impl Gpu {
    fn run(&self, edges: &[[f32; 4]], rule: Rule) -> Vec<u8> {
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("coverage determinism"),
                source: wgpu::ShaderSource::Wgsl(shader().into()),
            });
        let edge_bytes: Vec<u8> = edges
            .iter()
            .flat_map(|e| e.iter().flat_map(|v| v.to_le_bytes()))
            .collect();
        let make = |bytes: &[u8], usage| {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: bytes.len().max(16) as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let input = make(&edge_bytes, wgpu::BufferUsages::STORAGE);
        let rule_word: u32 = match rule {
            Rule::NonZero => 0,
            Rule::EvenOdd => 1,
        };
        let rule_bytes: Vec<u8> = [rule_word, 0, 0, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let rule_buffer = make(&rule_bytes, wgpu::BufferUsages::UNIFORM);
        let out_bytes = (W * H * 4) as u64;
        let output = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: out_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: out_bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("coverage determinism"),
                layout: None,
                module: &module,
                entry_point: Some("rasterize_rows"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: rule_buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((H as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, out_bytes);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device should answer the map");
        let mapped = slice.get_mapped_range().expect("the readback should map");
        let out = mapped
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]).min(255) as u8)
            .collect();
        drop(mapped);
        readback.unmap();
        out
    }
}

// ---------------------------------------------------------------------------
// Inputs: the shapes that exercise every branch, deterministically.
// ---------------------------------------------------------------------------

/// A small deterministic generator (an LCG) so the geometry is the same on every run
/// and machine without a dependency.
struct Lcg(u64);
impl Lcg {
    fn next_f32(&mut self, low: f32, high: f32) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let unit = ((self.0 >> 33) as f32) / (u32::MAX >> 1) as f32;
        low + unit * (high - low)
    }
}

fn close_polygon(points: &[(f32, f32)], edges: &mut Vec<[f32; 4]>) {
    let n = points.len();
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        edges.push([a.0, a.1, b.0, b.1]);
    }
}

/// The Entwurf shape: a mosaic of abutting quads with jittered shared corners, plus
/// stars for self-crossings, plus geometry that leaves the region on every side, plus
/// near-horizontal slivers.
fn test_edges() -> Vec<[f32; 4]> {
    let mut lcg = Lcg(0x5DEECE66D);
    let mut edges = Vec::new();
    // Mosaic of abutting quads over a jittered lattice: shared corners, sub-pixel jitter.
    let cell = 7.3_f32;
    let mut corners = Vec::new();
    for gy in 0..10 {
        let mut row = Vec::new();
        for gx in 0..10 {
            row.push((
                gx as f32 * cell + lcg.next_f32(-1.4, 1.4) - 2.0,
                gy as f32 * cell + lcg.next_f32(-1.4, 1.4) - 2.0,
            ));
        }
        corners.push(row);
    }
    for gy in 0..9 {
        for gx in 0..9 {
            close_polygon(
                &[
                    corners[gy][gx],
                    corners[gy][gx + 1],
                    corners[gy + 1][gx + 1],
                    corners[gy + 1][gx],
                ],
                &mut edges,
            );
        }
    }
    // Stars: many self-crossings, both winding parities.
    for star in 0..6 {
        let cx = lcg.next_f32(8.0, 56.0);
        let cy = lcg.next_f32(8.0, 56.0);
        let mut points = Vec::new();
        for k in 0..7 {
            let angle = (k * 3) as f32 * (std::f32::consts::TAU / 7.0) + star as f32;
            let r = lcg.next_f32(4.0, 14.0);
            points.push((cx + r * angle.cos(), cy + r * angle.sin()));
        }
        close_polygon(&points, &mut edges);
    }
    // Geometry crossing every border: triangles far outside, cut by the slab path.
    close_polygon(&[(-30.0, 10.0), (90.0, 22.0), (30.0, -25.0)], &mut edges);
    close_polygon(&[(-15.0, 70.0), (80.0, 90.0), (40.0, 30.0)], &mut edges);
    // Near-horizontal slivers: tiny dy, huge dxdy — the steep-slope arithmetic.
    for _ in 0..24 {
        let y = lcg.next_f32(0.0, 64.0);
        let dy = lcg.next_f32(1.0e-5, 3.0e-3);
        let x0 = lcg.next_f32(-10.0, 70.0);
        let x1 = lcg.next_f32(-10.0, 70.0);
        close_polygon(&[(x0, y), (x1, y + dy), (x1, y + 8.0), (x0, y + 8.0)], &mut edges);
    }
    // Sub-pixel shards, §10.7.4's population.
    for _ in 0..40 {
        let x = lcg.next_f32(0.0, 63.0);
        let y = lcg.next_f32(0.0, 63.0);
        close_polygon(
            &[
                (x, y),
                (x + lcg.next_f32(0.05, 0.6), y + lcg.next_f32(0.02, 0.3)),
                (x + lcg.next_f32(0.05, 0.5), y + lcg.next_f32(0.3, 0.9)),
            ],
            &mut edges,
        );
    }
    edges
}

fn diff_report(a: &[u8], b: &[u8]) -> (usize, u32) {
    let mut count = 0;
    let mut max = 0_u32;
    for (x, y) in a.iter().zip(b) {
        let d = u32::from(x.abs_diff(*y));
        if d > 0 {
            count += 1;
            max = max.max(d);
        }
    }
    (count, max)
}

/// The probe: every adapter on the machine, both rules, three comparisons each —
/// two runs of one dispatch, the shader against the CPU mirror, and every adapter
/// against every other. **All three are asserted at byte identity**, because that is
/// what they measured when this was written (RADV, llvmpipe and radeonsi: zero pixels
/// of 4 096 differ, both rules, 588 edges through every branch — the finding ADR 0079
/// rests on). The honest bound on the claim: those three share Mesa's compiler; a
/// vendor compiler that fuses `a * b + c` (WGSL permits it, naga does not forbid it)
/// would fail here, and that failure arriving with named pixels on a new adapter is
/// this test doing its job for the compute-lane design rather than an inconvenience.
#[test]
fn ported_coverage_is_reproducible_and_reports_cross_adapter_identity() {
    let gpus = adapters();
    assert!(!gpus.is_empty(), "no adapter at all on this machine");
    let edges = test_edges();
    println!("{} edges over {W}x{H}", edges.len());
    for rule in [Rule::NonZero, Rule::EvenOdd] {
        let rule_name = match rule {
            Rule::NonZero => "non-zero",
            Rule::EvenOdd => "even-odd",
        };
        let cpu = fill_mask_mirror(&edges, rule);
        let mut outputs = Vec::new();
        for gpu in &gpus {
            let once = gpu.run(&edges, rule);
            let again = gpu.run(&edges, rule);
            assert_eq!(
                once, again,
                "{}: two runs of one dispatch disagree ({rule_name})",
                gpu.name
            );
            let (pixels, max) = diff_report(&cpu, &once);
            println!(
                "{rule_name} on {}: vs CPU mirror {pixels} pixel(s) differ, max {max}",
                gpu.name
            );
            assert_eq!(
                (pixels, max),
                (0, 0),
                "{}: the WGSL port diverged from the CPU arithmetic ({rule_name})",
                gpu.name
            );
            outputs.push((gpu.name.clone(), once));
        }
        for pair in outputs.windows(2) {
            let (pixels, max) = diff_report(&pair[0].1, &pair[1].1);
            println!(
                "{rule_name}: {} vs {}: {pixels} pixel(s) differ, max {max}",
                pair[0].0, pair[1].0
            );
            assert_eq!(
                (pixels, max),
                (0, 0),
                "{} and {} disagree ({rule_name})",
                pair[0].0,
                pair[1].0
            );
        }
    }
}
