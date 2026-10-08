//! System-wide sampling: memory lists, commit, fault counters → core `Sample`.

use crate::nt::{self, MemoryLists};
use memmanager_core::sample::Sample;
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

/// Everything the policy and the UI need from one system tick.
#[derive(Clone, Copy, Debug, Default)]
pub struct SysReading {
    pub sample: Sample,
    pub has_lists: bool,
    pub lists: MemoryLists,
    pub total: u64,
    /// Task Manager "In use".
    pub in_use: u64,
    pub modified: u64,
    pub standby: u64,
    pub free_zero: u64,
    pub standby_prio0: u64,
    pub commit: u64,
    pub commit_limit: u64,
    /// Page-file bytes in use / total (all page files). 0/0 without a page file.
    pub pagefile_used: u64,
    pub pagefile_total: u64,
    pub kernel_paged: u64,
    pub kernel_nonpaged: u64,
    pub system_cache: u64,
    /// Repurposes per second at priorities 0..=2 (low-value cache being recycled), bytes/s.
    pub low_repurpose_rate: f64,
    /// Hard faults per second (pages read from disk).
    pub hard_fault_rate: f64,
    /// Running count of page faults answered from the standby/modified lists
    /// ("transition faults": data came back from memory instead of disk).
    pub cache_hits_cum: u64,
    /// Running count of pages read from disk.
    pub disk_reads_cum: u64,
    pub paged_pool_pages: u32,
    pub nonpaged_pool_pages: u32,
}

impl SysReading {
    /// Apps / speed-up cache / free (values.rs).
    pub fn split(&self) -> crate::values::Split {
        crate::values::Split::new(self.total, self.standby, self.free_zero)
    }
    pub fn used_fraction(&self) -> f64 {
        self.in_use as f64 / self.total.max(1) as f64
    }
    pub fn commit_fraction(&self) -> f64 {
        self.commit as f64 / self.commit_limit.max(1) as f64
    }
}

#[derive(Default)]
pub struct SysSampler {
    page_size: u64,
    hard_cum: u64,
    hits_cum: u64,
    last_page_read: Option<u32>,
    last_transition: Option<u32>,
    last_low_repurposed: Option<u64>,
    last_t: u64,
    pub lists_ok: Option<bool>,
}

impl SysSampler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page_size(&self) -> u64 {
        self.page_size.max(4096)
    }

    pub fn sample(&mut self, t_ms: u64) -> Option<SysReading> {
        let mut ms = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        unsafe { GlobalMemoryStatusEx(&mut ms).ok()? };
        let mut pi = PERFORMANCE_INFORMATION {
            cb: size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        unsafe { GetPerformanceInfo(&mut pi, pi.cb).ok()? };
        let page = pi.PageSize as u64;
        self.page_size = page;
        let total = ms.ullTotalPhys;

        let mut r = SysReading {
            total,
            commit: pi.CommitTotal as u64 * page,
            commit_limit: pi.CommitLimit as u64 * page,
            kernel_paged: pi.KernelPaged as u64 * page,
            kernel_nonpaged: pi.KernelNonpaged as u64 * page,
            system_cache: pi.SystemCache as u64 * page,
            ..Default::default()
        };

        if let Ok((used, total)) = nt::pagefile_pages() {
            r.pagefile_used = used * page;
            r.pagefile_total = total * page;
        }

        let lists = nt::memory_lists();
        self.lists_ok = Some(lists.is_ok());
        let dt = if self.last_t > 0 {
            t_ms.saturating_sub(self.last_t) as f64 / 1000.0
        } else {
            0.0
        };
        match lists {
            Ok(l) => {
                r.has_lists = true;
                r.lists = l;
                r.modified = l.modified as u64 * page;
                r.standby = l.standby() as u64 * page;
                r.free_zero = (l.free + l.zero) as u64 * page;
                r.standby_prio0 = l.standby_by_priority[0] as u64 * page;
                let accounted = r.modified + r.standby + r.free_zero + l.bad as u64 * page;
                r.in_use = total.saturating_sub(accounted);
                let low: u64 = l.repurposed_by_priority[..3]
                    .iter()
                    .map(|&v| v as u64)
                    .sum();
                if let (Some(prev), true) = (self.last_low_repurposed, dt > 0.0) {
                    r.low_repurpose_rate = low.saturating_sub(prev) as f64 * page as f64 / dt;
                }
                self.last_low_repurposed = Some(low);
            }
            Err(_) => {
                // Monitor-only fallback: "available" includes standby.
                r.in_use = total.saturating_sub(ms.ullAvailPhys);
                r.free_zero = ms.ullAvailPhys;
                r.standby = (pi.SystemCache as u64 * page).min(ms.ullAvailPhys);
            }
        }

        let churn: u64 = if r.has_lists {
            r.lists.repurposed_by_priority[5..]
                .iter()
                .map(|&v| v as u64)
                .sum()
        } else {
            0
        };
        if let Ok(p) = nt::perf_info() {
            if let Some(prev) = self.last_page_read {
                let d = p.page_read_count.wrapping_sub(prev) as u64;
                self.hard_cum += d;
                if dt > 0.0 {
                    r.hard_fault_rate = d as f64 / dt;
                }
            }
            self.last_page_read = Some(p.page_read_count);
            if let Some(prev) = self.last_transition {
                self.hits_cum += p.transition_count.wrapping_sub(prev) as u64;
            }
            self.last_transition = Some(p.transition_count);
            r.paged_pool_pages = p.paged_pool_pages;
            r.nonpaged_pool_pages = p.non_paged_pool_pages;
        }
        self.last_t = t_ms;
        r.cache_hits_cum = self.hits_cum;
        r.disk_reads_cum = self.hard_cum;

        let fast_avail = if r.has_lists {
            r.free_zero + r.standby_prio0
        } else {
            ms.ullAvailPhys
        };
        r.sample = Sample {
            t_ms,
            page_size: page,
            total_bytes: total,
            fast_avail_bytes: fast_avail,
            cache_bytes: r.standby,
            commit_bytes: r.commit,
            commit_limit: r.commit_limit,
            churn_pages_cum: churn,
            hard_faults_cum: self.hard_cum,
            ..Default::default()
        };
        Some(r)
    }
}
