//! Theil–Sen + Mann–Kendall leak and runaway detector (policy-engine §7).
//!
//! Allocation-free: series are decimated to ≤ 64 points and pairwise slopes
//! go into a fixed scratch array. Must agree with
//! `spec/reference/policy_ref.py` on every vector in `spec/test-vectors/`.

use crate::profile::LeakThresholds;

pub const MAX_POINTS: usize = 64;
const MAX_PAIRS: usize = MAX_POINTS * (MAX_POINTS - 1) / 2;
pub const MIN_SAMPLES: usize = 40;
pub const MIN_SPAN_H: f64 = 0.5;
pub const HEAVY_MULTIPLIER: f64 = 2.0;
pub const RUNAWAY_MIB_PER_H: f64 = 1024.0 * 60.0;
const RUNAWAY_WINDOW_H: f64 = 2.0 / 60.0;
const RUNAWAY_MIN_SAMPLES: usize = 5;
const MIN_TAU: f64 = 0.6;
const MIN_Z: f64 = 2.33;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LeakResult {
    pub eligible: bool,
    /// Theil–Sen slope, MiB per hour.
    pub slope: f64,
    pub tau: f64,
    pub z: f64,
    pub plateau: bool,
    pub leak: bool,
    pub runaway: bool,
}

/// Median of `xs` (reorders it). Mean of the two middle values for even counts.
fn median_in_place(xs: &mut [f64]) -> f64 {
    let n = xs.len();
    if n == 0 {
        return 0.0;
    }
    let mid = n / 2;
    let (left, m, _) = xs.select_nth_unstable_by(mid, |a, b| a.total_cmp(b));
    let m = *m;
    if n % 2 == 1 {
        m
    } else {
        let lo = left.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (lo + m) / 2.0
    }
}

/// Theil–Sen slope of `pts` (≤ 64 points); `scratch` holds the pairwise slopes.
fn theil_sen(pts: &[(f64, f64)], scratch: &mut [f64; MAX_PAIRS]) -> f64 {
    let mut k = 0;
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            let (ti, yi) = pts[i];
            let (tj, yj) = pts[j];
            if tj > ti {
                scratch[k] = (yj - yi) / (tj - ti);
                k += 1;
            }
        }
    }
    if k == 0 {
        0.0
    } else {
        median_in_place(&mut scratch[..k])
    }
}

/// Mann–Kendall (τ, Z) with tie correction.
fn mann_kendall(pts: &[(f64, f64)]) -> (f64, f64) {
    let n = pts.len();
    let mut s: i64 = 0;
    for i in 0..n {
        for j in i + 1..n {
            let d = pts[j].1 - pts[i].1;
            s += (d > 0.0) as i64 - (d < 0.0) as i64;
        }
    }
    let mut ys = [0.0f64; MAX_POINTS];
    for (dst, p) in ys.iter_mut().zip(pts) {
        *dst = p.1;
    }
    let ys = &mut ys[..n];
    ys.sort_unstable_by(|a, b| a.total_cmp(b));
    let mut tie_term = 0.0;
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && ys[j] == ys[i] {
            j += 1;
        }
        let t = (j - i) as f64;
        if t > 1.0 {
            tie_term += t * (t - 1.0) * (2.0 * t + 5.0);
        }
        i = j;
    }
    let nf = n as f64;
    let var = (nf * (nf - 1.0) * (2.0 * nf + 5.0) - tie_term) / 18.0;
    let sf = s as f64;
    let z = if s > 0 {
        (sf - 1.0) / var.sqrt()
    } else if s < 0 {
        (sf + 1.0) / var.sqrt()
    } else {
        0.0
    };
    let tau = sf / (nf * (nf - 1.0) / 2.0);
    (tau, z)
}

/// Keeps indices n-1, n-1-k, … (k = ceil(n/64)), oldest first. Returns the count.
fn decimate(series: &[(f64, f64)], out: &mut [(f64, f64); MAX_POINTS]) -> usize {
    let n = series.len();
    if n <= MAX_POINTS {
        out[..n].copy_from_slice(series);
        return n;
    }
    let k = n.div_ceil(MAX_POINTS);
    let count = (n - 1) / k + 1;
    for (slot, idx) in (0..count).rev().zip((0..n).rev().step_by(k)) {
        out[slot] = series[idx];
    }
    count
}

/// Analyses one process series `(t_hours, mib)`, oldest first.
pub fn analyze(
    series: &[(f64, f64)],
    th: LeakThresholds,
    heavy: bool,
    process_age_ok: bool,
) -> LeakResult {
    let mut out = LeakResult::default();
    let mut scratch = [0.0f64; MAX_PAIRS];
    let mult = if heavy { HEAVY_MULTIPLIER } else { 1.0 };

    // §7.8 runaway: raw samples within the last 2 minutes (newest 64 at most).
    if let Some(&(t_end, _)) = series.last() {
        let from = series.len().saturating_sub(MAX_POINTS);
        let tail_start = series[from..]
            .iter()
            .position(|p| p.0 >= t_end - RUNAWAY_WINDOW_H)
            .map_or(series.len(), |i| from + i);
        let tail = &series[tail_start..];
        if tail.len() >= RUNAWAY_MIN_SAMPLES {
            out.runaway = theil_sen(tail, &mut scratch) >= RUNAWAY_MIB_PER_H;
        }
    }

    let n_raw = series.len();
    if n_raw < MIN_SAMPLES || series[n_raw - 1].0 - series[0].0 < MIN_SPAN_H || !process_age_ok {
        return out;
    }
    out.eligible = true;

    let mut buf = [(0.0, 0.0); MAX_POINTS];
    let n = decimate(series, &mut buf);
    let d = &buf[..n];
    let slope = theil_sen(d, &mut scratch);
    let (tau, z) = mann_kendall(d);
    let m = 8usize.max(n.div_ceil(4)).min(n);
    let s_tail = theil_sen(&d[n - m..], &mut scratch);
    let plateau = s_tail < 0.5 * slope;
    let growth = d[n - 1].1 - d[0].1;
    let baseline = d[0].1;
    out.slope = slope;
    out.tau = tau;
    out.z = z;
    out.plateau = plateau;
    out.leak = slope >= th.slope_mib_h * mult
        && growth >= (th.abs_mib * mult).max(th.rel * baseline)
        && tau >= MIN_TAU
        && z > MIN_Z
        && !plateau;
    out
}

/// Hours until `headroom_mib` is consumed at `slope_mib_h` (None if not growing).
pub fn eta_hours(headroom_mib: f64, slope_mib_h: f64) -> Option<f64> {
    (slope_mib_h > 0.0 && headroom_mib.is_finite()).then(|| headroom_mib.max(0.0) / slope_mib_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_even_odd() {
        assert_eq!(median_in_place(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median_in_place(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn decimate_keeps_newest() {
        let s: Vec<(f64, f64)> = (0..241).map(|i| (i as f64, i as f64)).collect();
        let mut out = [(0.0, 0.0); MAX_POINTS];
        let n = decimate(&s, &mut out);
        assert_eq!(n, 61); // k = 4 ⇒ indices 240, 236, …, 0
        assert_eq!(out[n - 1].0, 240.0);
        assert_eq!(out[0].0, 0.0);
    }
}
