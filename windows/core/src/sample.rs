//! System sample model and the pressure score `P` (policy-engine §2–3).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub t_ms: u64,
    pub page_size: u64,
    pub total_bytes: u64,
    /// Windows: free + zero + standby[prio 0]. macOS: free (incl. speculative).
    pub fast_avail_bytes: u64,
    /// Windows: standby (all priorities). macOS: external + purgeable.
    pub cache_bytes: u64,
    /// Windows only.
    pub commit_bytes: u64,
    pub commit_limit: u64,
    /// Windows: Σ RepurposedPagesByPriority[5..7]. macOS: decompressions.
    pub churn_pages_cum: u64,
    /// Windows: PageReadCount. macOS: pageins.
    pub hard_faults_cum: u64,
    /// macOS: 1 | 2 | 4 from `kern.memorystatus_vm_pressure_level`. Windows: 0.
    pub kernel_level: u32,
    /// macOS: `kern.memorystatus_level`. Windows: 0.
    pub free_pct: u32,
    /// macOS: `vm.swapusage.xsu_used`. Windows: 0.
    pub swap_used_bytes: u64,
}

fn clamp01(x: f64) -> f64 {
    x.clamp(0.0, 1.0)
}

fn dt_s(cur: &Sample, prev: Option<&Sample>) -> Option<(f64, Sample)> {
    let prev = *prev?;
    let dt = cur.t_ms.saturating_sub(prev.t_ms) as f64 / 1000.0;
    (dt > 0.0).then_some((dt, prev))
}

/// Sub-scores of the Windows pressure score, for display and debugging.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WinPressure {
    pub phys: f64,
    pub commit: f64,
    pub churn: f64,
}

impl WinPressure {
    pub fn p(&self) -> f64 {
        self.phys.max(self.commit).max(self.churn)
    }
}

pub fn pressure_windows(cur: &Sample, prev: Option<&Sample>, free_target_frac: f64) -> WinPressure {
    let total = cur.total_bytes.max(1) as f64;
    let f_target = free_target_frac * total;
    let phys = clamp01((f_target - cur.fast_avail_bytes as f64) / f_target);
    let commit = if cur.commit_limit > 0 {
        clamp01((cur.commit_bytes as f64 / cur.commit_limit as f64 - 0.70) / 0.25)
    } else {
        0.0
    };
    let churn = match dt_s(cur, prev) {
        Some((dt, prev)) => {
            let pages = cur.churn_pages_cum.saturating_sub(prev.churn_pages_cum) as f64;
            let rate = pages * cur.page_size as f64 / total / dt;
            clamp01(rate / 0.002)
        }
        None => 0.0,
    };
    WinPressure {
        phys,
        commit,
        churn,
    }
}

/// Floor contributed by the kernel level (dispatch values 1/2/4).
pub fn mac_level_floor(level: u32) -> f64 {
    match level {
        4 => 0.90,
        2 => 0.60,
        _ => 0.0,
    }
}

pub fn pressure_macos(cur: &Sample, prev: Option<&Sample>) -> f64 {
    let level_floor = mac_level_floor(cur.kernel_level);
    let avail = clamp01((60.0 - cur.free_pct as f64) / 60.0);
    let swap_growth = match dt_s(cur, prev) {
        Some((dt, prev)) => {
            let grew = cur.swap_used_bytes.saturating_sub(prev.swap_used_bytes) as f64;
            clamp01(grew / dt / (64.0 * crate::MIB))
        }
        None => 0.0,
    };
    level_floor.max(0.8 * avail).max(swap_growth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_subscores() {
        let gib = 1u64 << 30;
        let prev = Sample {
            t_ms: 0,
            page_size: 4096,
            total_bytes: 16 * gib,
            ..Default::default()
        };
        let cur = Sample {
            t_ms: 10_000,
            fast_avail_bytes: gib / 2,
            commit_bytes: 20 * gib,
            commit_limit: 24 * gib,
            churn_pages_cum: 0,
            ..prev
        };
        let p = pressure_windows(&cur, Some(&prev), 0.06);
        // F_target = 0.96 GiB, avail 0.5 GiB ⇒ phys ≈ 0.479
        assert!((p.phys - (0.96 - 0.5) / 0.96).abs() < 1e-9);
        // commit 83.3% ⇒ (0.8333-0.70)/0.25 ≈ 0.533
        assert!((p.commit - (20.0 / 24.0 - 0.70) / 0.25).abs() < 1e-9);
        assert_eq!(p.churn, 0.0);
        assert!((p.p() - p.commit).abs() < 1e-12);
    }

    #[test]
    fn macos_floor_and_swap() {
        let prev = Sample {
            t_ms: 0,
            free_pct: 70,
            kernel_level: 1,
            ..Default::default()
        };
        let mut cur = Sample { t_ms: 1000, ..prev };
        assert_eq!(pressure_macos(&cur, Some(&prev)), 0.0);
        cur.kernel_level = 2;
        assert_eq!(pressure_macos(&cur, Some(&prev)), 0.6);
        cur.kernel_level = 1;
        cur.swap_used_bytes = (64.0 * crate::MIB) as u64;
        assert!((pressure_macos(&cur, Some(&prev)) - 1.0).abs() < 1e-12);
    }
}
