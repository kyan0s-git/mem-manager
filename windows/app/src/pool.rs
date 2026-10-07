//! Kernel pool leak detection and pool-tag attribution
//! (docs/research/leak-detection.md §4).

use crate::nt::{self, PoolTag};
use memmanager_core::leak;
use memmanager_core::profile::LeakThresholds;
use memmanager_core::ring::Ring;
use std::collections::HashMap;

const SAMPLE_EVERY_MS: u64 = 60_000;
const SECOND_SNAPSHOT_AFTER_MS: u64 = 5 * 60_000;
const REPORT_COOLDOWN_MS: u64 = 24 * 3_600_000;
const THRESHOLDS: LeakThresholds = LeakThresholds {
    slope_mib_h: 50.0,
    abs_mib: 200.0,
    rel: 0.25,
};

/// Common tags → owner. Unknown tags fall back to a driver-file scan.
const KNOWN: &[(&str, &str)] = &[
    (
        "NtFC",
        "NTFS file contexts (often a file-handle leak in a sync or backup client)",
    ),
    ("NtfF", "NTFS file records"),
    ("Ntfx", "NTFS"),
    ("MmSt", "Memory manager section objects (mapped files)"),
    ("MmCa", "Memory manager control areas"),
    ("File", "File objects (handle leak in some process)"),
    ("Even", "Event objects (handle leak in some process)"),
    ("Thre", "Thread objects"),
    ("Proc", "Process objects (zombie processes)"),
    ("Toke", "Security tokens"),
    ("Irp ", "I/O request packets (driver)"),
    ("Ddk ", "Generic driver allocation (untagged driver)"),
    ("Wfpn", "Windows Filtering Platform (firewall / VPN)"),
    ("NDnd", "NDIS network driver"),
    ("Nb07", "NetBIOS"),
    ("TcpE", "TCP/IP endpoints"),
    ("smNp", "ReadyBoost / store manager"),
    ("Vi54", "Video memory manager (GPU driver)"),
    ("CM31", "Registry (configuration manager)"),
    ("AleE", "Windows Filtering Platform (ALE)"),
];

pub struct PoolMonitor {
    hist: Ring<(f32, f32), 240>,
    epoch_ms: u64,
    last_sample_ms: u64,
    first_snapshot: Option<(u64, HashMap<u32, PoolTag>)>,
    last_report_ms: Option<u64>,
    pub enabled: bool,
}

impl PoolMonitor {
    pub fn new(now_ms: u64) -> Self {
        PoolMonitor {
            hist: Ring::new(),
            epoch_ms: now_ms,
            last_sample_ms: 0,
            first_snapshot: None,
            last_report_ms: None,
            enabled: true,
        }
    }

    /// Feed every system tick. Returns a report when a leak has been attributed.
    pub fn tick(&mut self, now_ms: u64, pool_pages: u64, page_size: u64) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if let Some((t0, snap)) = &self.first_snapshot {
            if now_ms.saturating_sub(*t0) >= SECOND_SNAPSHOT_AFTER_MS {
                let before = snap.clone();
                self.first_snapshot = None;
                return self.attribute(&before, now_ms);
            }
            return None;
        }
        if self.last_sample_ms != 0 && now_ms.saturating_sub(self.last_sample_ms) < SAMPLE_EVERY_MS
        {
            return None;
        }
        self.last_sample_ms = now_ms;
        let t_h = now_ms.saturating_sub(self.epoch_ms) as f32 / 3_600_000.0;
        let mib = (pool_pages * page_size) as f64 / memmanager_core::MIB;
        self.hist.push((t_h, mib as f32));
        let cooled = self
            .last_report_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= REPORT_COOLDOWN_MS);
        if !cooled {
            return None;
        }
        let series: Vec<(f64, f64)> = self
            .hist
            .iter()
            .map(|(t, y)| (t as f64, y as f64))
            .collect();
        let r = leak::analyze(&series, THRESHOLDS, false, true);
        if r.leak {
            match nt::pool_tags() {
                Ok(tags) => {
                    let map = tags
                        .into_iter()
                        .map(|t| (u32::from_le_bytes(t.tag), t))
                        .collect();
                    self.first_snapshot = Some((now_ms, map));
                }
                Err(_) => self.enabled = false,
            }
        }
        None
    }

    fn attribute(&mut self, before: &HashMap<u32, PoolTag>, now_ms: u64) -> Option<String> {
        let after = nt::pool_tags().ok()?;
        let mut growth: Vec<(i64, PoolTag)> = after
            .into_iter()
            .map(|t| {
                let prev = before
                    .get(&u32::from_le_bytes(t.tag))
                    .map_or(0, |p| p.used());
                (t.used() as i64 - prev as i64, t)
            })
            .filter(|(d, _)| *d > 0)
            .collect();
        growth.sort_unstable_by_key(|g| std::cmp::Reverse(g.0));
        if growth.is_empty() {
            return None;
        }
        self.last_report_ms = Some(now_ms);
        let lines: Vec<String> = growth
            .iter()
            .take(3)
            .map(|(d, t)| {
                let tag = t.tag_str();
                format!(
                    "{tag} +{} ({}) — {}",
                    crate::util::fmt_bytes(*d as u64),
                    crate::util::fmt_bytes(t.used()),
                    owner_of(&tag)
                )
            })
            .collect();
        Some(format!(
            "Kernel memory is growing and can only be reclaimed by a reboot or a driver update. Top tags:\n{}",
            lines.join("\n")
        ))
    }
}

pub fn owner_of(tag: &str) -> String {
    if let Some((_, o)) = KNOWN.iter().find(|(t, _)| *t == tag) {
        return (*o).to_string();
    }
    match scan_drivers(tag) {
        Some(d) => format!("found in {d}"),
        None => "unknown driver".into(),
    }
}

/// Searches `%SystemRoot%\System32\drivers\*.sys` (< 16 MB each) for the tag bytes.
fn scan_drivers(tag: &str) -> Option<String> {
    let root = std::env::var_os("SystemRoot")?;
    let dir = std::path::Path::new(&root).join("System32").join("drivers");
    let needle = tag.as_bytes();
    if needle.len() != 4 {
        return None;
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("sys"))
        {
            continue;
        }
        if entry.metadata().map_or(true, |m| m.len() > 16 << 20) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.windows(4).any(|w| w == needle) {
                found.push(path.file_name()?.to_string_lossy().into_owned());
                if found.len() >= 3 {
                    break;
                }
            }
        }
    }
    (!found.is_empty()).then(|| found.join(", "))
}
