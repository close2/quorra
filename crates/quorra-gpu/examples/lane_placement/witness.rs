//! The caller's own §31.2 table, and the two pieces of arithmetic that say what produced
//! it.
//!
//! Nothing here renders. It is published numbers plus the sampled lane's own grid
//! arithmetic, run every time this instrument runs, because the conclusion it reaches is
//! one this tree would otherwise have to assert in prose — and a paragraph is not
//! something a later round can watch fail.
//!
//! The two questions it answers, both about `bug1743245.pdf`'s six graph-paper rules:
//!
//! 1. **Did our two lanes receive the same geometry?** If the sampled lane is a quantiser
//!    and nothing else, then applying [`lattice_mean`] to their *default*-lane column must
//!    reproduce their *sampled*-lane column. It does, to zero.
//! 2. **Is the default lane's offset per command?** If it were, it would not fit one
//!    affine. [`affine_fit`] fits scale and offset to their two columns and prints what is
//!    left over.

use super::fixture::HAIRLINE;

/// Their §31.2's oracle column: the centroid of each of `bug1743245.pdf`'s six rules under
/// `render-cpu`, in device pixels along a raster row.
const ORACLE: [f64; 6] = [33.000, 49.500, 66.000, 82.500, 99.000, 115.500];

/// Their §31.2's default-lane column — `Coverage::Cpu`, the same six rules.
const DEFAULT_LANE: [f64; 6] = [33.122, 49.602, 66.083, 82.567, 99.047, 115.531];

/// Their §31.2's sampled-lane column — `Coverage::Gpu`, the same six rules. Their table
/// records it as "identical to the oracle".
const SAMPLED_LANE: [f64; 6] = [33.000, 49.500, 66.000, 82.500, 99.000, 115.500];

/// Where the sampled lane puts the centroid of an axis-aligned band of `width` device
/// pixels centred at `centre`, on a grid of `samples` samples.
///
/// **Derived from `winding::sample_offsets`, not fitted.** That function lays `n` samples
/// on a `√n × √n` ordered grid and puts the k-th row at `(k + ½)/√n` of a pixel, so across
/// the whole device the sample rows are the lattice `(k + ½)/√n` for integer `k`. A band
/// covers exactly the lattice points inside it; every pixel row it reaches contributes
/// `√n` samples per lattice point it holds, so the coverage-weighted centroid of the whole
/// band is the plain mean of the lattice points it contains. The sampled lane therefore
/// cannot place a band anywhere but on that mean, whatever the geometry says — and half a
/// pitch is the most it can be wrong by.
///
/// Half-open at the top, matching the half-open square region ISO 32000-2 §10.7.4 states
/// the scan-conversion rule over; a lattice point exactly on a band's edge is a
/// measure-zero case either way.
fn lattice_mean(centre: f64, width: f64, samples: u32) -> f64 {
    let side = f64::from(samples.isqrt().max(1));
    let (low, high) = (centre - width / 2.0, centre + width / 2.0);
    // The lattice points in `[low, high)` are the integers `k` with
    // `low·side − ½ ≤ k < high·side − ½`.
    let first = (low * side - 0.5).ceil();
    let last = (high * side - 0.5).ceil() - 1.0;
    if last < first {
        return f64::NAN;
    }
    (f64::midpoint(first, last) + 0.5) / side
}

/// The least-squares affine `to ≈ scale · from + offset`, over paired samples.
///
/// Two numbers rather than six, which is the whole of the test: a per-command quantiser
/// has one free value per command and cannot be summarised, and a difference in the device
/// transform has exactly these two.
fn affine_fit(from: &[f64], to: &[f64]) -> (f64, f64) {
    let count = from.len() as f64;
    let mean_from = from.iter().sum::<f64>() / count;
    let mean_to = to.iter().sum::<f64>() / count;
    let covariance: f64 = from
        .iter()
        .zip(to)
        .map(|(a, b)| (a - mean_from) * (b - mean_to))
        .sum();
    let variance: f64 = from.iter().map(|a| (a - mean_from).powi(2)).sum();
    let scale = covariance / variance;
    (scale, mean_to - scale * mean_from)
}

/// Runs both checks over the caller's published table and prints what they came to.
pub(crate) fn against_the_callers_table(samples: u32) {
    println!("--- phase 4: the caller's own §31.2 numbers, and what fits them ---\n");
    println!(
        "{:>5}  {:>10} {:>10} {:>10}  {:>10} {:>9}",
        "rule", "oracle", "default", "sampled", "lattice", "residual"
    );
    let mut worst_snap = 0.0_f64;
    for index in 0..ORACLE.len() {
        let snapped = lattice_mean(DEFAULT_LANE[index], f64::from(HAIRLINE), samples);
        let residual = snapped - SAMPLED_LANE[index];
        worst_snap = worst_snap.max(residual.abs());
        println!(
            "{index:>5}  {:>10.3} {:>10.3} {:>10.3}  {snapped:>10.3} {residual:>+9.4}",
            ORACLE[index], DEFAULT_LANE[index], SAMPLED_LANE[index]
        );
    }
    println!(
        "\ntheir sampled column is their default column put through this lane's own \
         {samples}-sample grid,\nto {worst_snap:.4} device pixels. The two settings were \
         handed the same geometry."
    );

    let (scale, offset) = affine_fit(&ORACLE, &DEFAULT_LANE);
    let worst_fit = ORACLE
        .iter()
        .zip(&DEFAULT_LANE)
        .map(|(a, b)| (scale.mul_add(*a, offset) - b).abs())
        .fold(0.0_f64, f64::max);
    println!(
        "their default column is their oracle column under one affine — scale {scale:.6}, \
         offset {offset:+.4} px —\nto {worst_fit:.4} device pixels. Two free values fit six \
         commands, so it is not per command.\n"
    );
}
