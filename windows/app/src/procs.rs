//! Process table: one `SystemProcessInformation` snapshot per tick, per-process
//! idle tracking and bounded leak histories for the top candidates.

use crate::config::Config;
use crate::nt;
use crate::util::{display_name, image_key};
use memmanager_core::leak::{self, LeakResult};
use memmanager_core::ring::Ring;
use std::collections::HashMap;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

pub const HIST_LEN: usize = 360; // 2 h at 20 s
pub const HIST_SPACING_MS: u64 = 20_000;
pub const TOP_K: usize = 24;
const HIST_SLACK: usize = 8;
const FAST_GROWTH_BYTES: u64 = 512 << 20;
const MIN_AGE_FOR_HISTORY_MIN: f64 = 10.0;

/// Never touched by any action.
pub const CRITICAL: &[&str] = &[
    "system",
    "registry",
    "memory compression",
    "secure system",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "svchost.exe",
    "dwm.exe",
    "audiodg.exe",
    "fontdrvhost.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "securityhealthservice.exe",
    "explorer.exe",
    "sihost.exe",
    "ctfmon.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "textinputhost.exe",
    "runtimebroker.exe",
    "taskhostw.exe",
    "conhost.exe",
    "dllhost.exe",
    "spoolsv.exe",
    "wudfhost.exe",
    "memmanager.exe",
];

pub type Hist = Ring<(f32, f32), HIST_LEN>;

pub struct Proc {
    pub pid: u32,
    pub parent_pid: u32,
    pub create_time: i64,
    pub key: String,
    pub name: String,
    pub session: u32,
    pub private: u64,
    pub prev_private: u64,
    pub working_set: u64,
    pub cpu: u64,
    pub last_cpu_change_ms: u64,
    seen_gen: u32,
    pub hist: Option<Box<Hist>>,
    last_hist_ms: u64,
    pub leak: LeakResult,
    pub leak_notified_ms: Option<u64>,
    pub notified_eta_h: f64,
}

impl Proc {
    pub fn idle_minutes(&self, now_ms: u64) -> f64 {
        now_ms.saturating_sub(self.last_cpu_change_ms) as f64 / 60_000.0
    }

    pub fn is_critical(&self) -> bool {
        self.pid <= 4 || CRITICAL.contains(&self.key.as_str())
    }
}

#[derive(Default)]
pub struct ProcTable {
    pub map: HashMap<u32, Proc>,
    buf: Vec<u64>,
    generation: u32,
    epoch_ms: u64,
    pub self_pid: u32,
}

fn filetime_now() -> i64 {
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

impl ProcTable {
    pub fn new(now_ms: u64) -> Self {
        Self {
            epoch_ms: now_ms,
            self_pid: std::process::id(),
            ..Default::default()
        }
    }

    fn t_hours(&self, now_ms: u64) -> f32 {
        (now_ms.saturating_sub(self.epoch_ms)) as f32 / 3_600_000.0
    }

    /// Refreshes the table. Returns the PIDs whose leak analysis changed to `leak`
    /// or `runaway` this tick.
    pub fn update(&mut self, now_ms: u64, cfg: &Config) -> Result<Vec<u32>, i32> {
        nt::query_processes(&mut self.buf)?;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let map = &mut self.map;
        nt::for_each_process(&self.buf, |e| {
            if e.pid == 0 {
                return;
            }
            let fresh = match map.get(&e.pid) {
                Some(p) => p.create_time != e.create_time,
                None => true,
            };
            if fresh {
                let image = if e.name.is_empty() {
                    if e.pid == 4 {
                        "System".to_string()
                    } else {
                        String::new()
                    }
                } else {
                    String::from_utf16_lossy(e.name)
                };
                map.insert(
                    e.pid,
                    Proc {
                        pid: e.pid,
                        parent_pid: e.parent_pid,
                        create_time: e.create_time,
                        key: image_key(&image),
                        name: display_name(&image),
                        session: e.session_id,
                        private: e.private_bytes,
                        prev_private: e.private_bytes,
                        working_set: e.working_set,
                        cpu: e.cpu_time,
                        last_cpu_change_ms: now_ms,
                        seen_gen: generation,
                        hist: None,
                        last_hist_ms: 0,
                        leak: LeakResult::default(),
                        leak_notified_ms: None,
                        notified_eta_h: f64::INFINITY,
                    },
                );
            } else if let Some(p) = map.get_mut(&e.pid) {
                p.prev_private = p.private;
                p.private = e.private_bytes;
                p.working_set = e.working_set;
                if e.cpu_time != p.cpu {
                    p.cpu = e.cpu_time;
                    p.last_cpu_change_ms = now_ms;
                }
                p.seen_gen = generation;
            }
        });
        self.map.retain(|_, p| p.seen_gen == generation);
        Ok(self.update_histories(now_ms, cfg))
    }

    fn update_histories(&mut self, now_ms: u64, cfg: &Config) -> Vec<u32> {
        // Candidates: top K by private bytes plus fast growers.
        let mut ranked: Vec<(u64, u32)> = self
            .map
            .values()
            .filter(|p| p.pid > 4 && p.key != "memory compression" && p.key != "registry")
            .map(|p| (p.private, p.pid))
            .collect();
        ranked.sort_unstable_by(|a, b| b.cmp(a));
        let mut want: Vec<u32> = ranked.iter().take(TOP_K).map(|r| r.1).collect();
        for p in self.map.values() {
            if p.private.saturating_sub(p.prev_private) >= FAST_GROWTH_BYTES
                && !want.contains(&p.pid)
            {
                want.push(p.pid);
            }
        }
        // Drop histories beyond the budget, smallest first.
        let mut with_hist: Vec<(u64, u32)> = self
            .map
            .values()
            .filter(|p| p.hist.is_some() && !want.contains(&p.pid))
            .map(|p| (p.private, p.pid))
            .collect();
        let budget = (TOP_K + HIST_SLACK).saturating_sub(want.len());
        if with_hist.len() > budget {
            with_hist.sort_unstable();
            for (_, pid) in with_hist.iter().take(with_hist.len() - budget) {
                if let Some(p) = self.map.get_mut(pid) {
                    p.hist = None;
                    p.leak = LeakResult::default();
                }
            }
        }

        let now_ft = filetime_now();
        let t_h = self.t_hours(now_ms);
        let th = cfg.profile.leak_thresholds();
        let mut flagged = Vec::new();
        let mut series = [(0.0f64, 0.0f64); HIST_LEN];
        for pid in want {
            let Some(p) = self.map.get_mut(&pid) else {
                continue;
            };
            let age_min = (now_ft - p.create_time) as f64 / 600_000_000.0;
            if age_min < MIN_AGE_FOR_HISTORY_MIN && p.hist.is_none() {
                continue;
            }
            let h = p.hist.get_or_insert_with(|| Box::new(Hist::new()));
            if p.last_hist_ms != 0 && now_ms.saturating_sub(p.last_hist_ms) < HIST_SPACING_MS {
                continue;
            }
            p.last_hist_ms = now_ms;
            h.push((t_h, (p.private as f64 / memmanager_core::MIB) as f32));
            let n = h.len();
            for (dst, (t, y)) in series.iter_mut().zip(h.iter()) {
                *dst = (t as f64, y as f64);
            }
            let was = p.leak.leak || p.leak.runaway;
            p.leak = leak::analyze(&series[..n], th, cfg.is_heavy(&p.key), true);
            if (p.leak.leak || p.leak.runaway) && !was {
                flagged.push(pid);
            }
        }
        flagged
    }

    /// Processes sorted by private bytes, largest first.
    pub fn top(&self, n: usize) -> Vec<&Proc> {
        let mut v: Vec<&Proc> = self
            .map
            .values()
            .filter(|p| p.pid > 4 && p.key != "memory compression")
            .collect();
        v.sort_unstable_by_key(|p| std::cmp::Reverse(p.private));
        v.truncate(n);
        v
    }

    /// Working set of the "Memory Compression" process (the compression store).
    pub fn compressed_store(&self) -> u64 {
        self.map
            .values()
            .find(|p| p.key == "memory compression")
            .map_or(0, |p| p.working_set)
    }

    /// Idle, user-session, non-critical, non-excluded processes that are not
    /// the foreground app (or its parent/children), largest first.
    pub fn idle_candidates(
        &self,
        now_ms: u64,
        idle_min: f64,
        min_ws: u64,
        foreground: u32,
        cfg: &Config,
    ) -> Vec<&Proc> {
        let fg_parent = self.map.get(&foreground).map_or(0, |p| p.parent_pid);
        let mut v: Vec<&Proc> = self
            .map
            .values()
            .filter(|p| {
                p.session != 0
                    && p.pid != self.self_pid
                    && p.pid != foreground
                    && p.parent_pid != foreground
                    && (fg_parent == 0 || p.pid != fg_parent)
                    && !p.is_critical()
                    && !cfg.is_excluded(&p.key)
                    && p.working_set >= min_ws
                    && p.idle_minutes(now_ms) >= idle_min
            })
            .collect();
        v.sort_unstable_by(|a, b| {
            let sa = a.working_set as f64 * a.idle_minutes(now_ms).sqrt();
            let sb = b.working_set as f64 * b.idle_minutes(now_ms).sqrt();
            sb.total_cmp(&sa)
        });
        v
    }
}
