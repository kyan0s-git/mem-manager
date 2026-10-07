//! Headless self-test: exercises the real engine against the live system and
//! prints a JSON report (CI runs this on windows-latest).
//!
//! `memmanager.exe --selftest[=SECONDS] [--out=PATH]`

use crate::config::{Config, IconMetric, IconStyle};
use crate::icon::{self, IconSpec};
use crate::nt;
use crate::palette::Rgba;
use crate::privilege;
use crate::shared::Shared;
use crate::util::{json_str, now_ms};
use crate::winactions::{self as wa, ids};
use crate::worker::{Engine, self_memory};
use memmanager_core::state::PressureState;
use std::fmt::Write as _;
use std::sync::Arc;

/// Private-bytes ceiling for the headless engine (no UI resources loaded).
const BUDGET_PRIVATE: u64 = 8 << 20;

pub fn run(seconds: u64, out: Option<String>) -> i32 {
    let privileges = privilege::enable_all();
    let shared = Arc::new(match Shared::new() {
        Ok(s) => s,
        Err(_) => return 10,
    });
    let start = now_ms();
    let mut eng = Engine::new(shared, Config::default(), privileges, start);
    let mut failures: Vec<String> = Vec::new();
    let mut samples = String::new();

    // 1. Sample the system and processes at 1 Hz.
    let deadline = start + seconds.max(3) * 1000;
    let mut ticks = 0;
    while now_ms() < deadline {
        let t = now_ms();
        eng.tick_procs(t);
        eng.tick_system(t, false);
        let r = eng.last;
        ticks += 1;
        let _ = write!(
            samples,
            "{}{{\"t_ms\":{},\"used\":{:.4},\"fast_avail\":{},\"standby\":{},\"modified\":{},\"commit\":{},\"commit_limit\":{},\"p\":{:.4},\"s\":{:.4},\"state\":\"{}\",\"hard_fault_rate\":{:.1}}}",
            if ticks > 1 { "," } else { "" },
            t - start,
            r.used_fraction(),
            r.sample.fast_avail_bytes,
            r.standby,
            r.modified,
            r.commit,
            r.commit_limit,
            eng.last_p,
            eng.sm.smoothed(),
            eng.sm.state().name(),
            r.hard_fault_rate
        );
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
    let r = eng.last;
    if r.total == 0 || r.commit_limit == 0 {
        failures.push("system sampling returned zeros".into());
    }
    if eng.procs.map.len() < 10 {
        failures.push(format!(
            "process snapshot too small ({})",
            eng.procs.map.len()
        ));
    }

    // 2. Exercise a tier-2 purge when privileged (proves the NT command path).
    let mut purge_json = String::from("null");
    if privileges.profile {
        let before = eng.last.sample.fast_avail_bytes;
        let t0 = now_ms();
        let o = eng.execute(ids::PURGE_LOW, t0, true);
        let took = now_ms() - t0;
        std::thread::sleep(std::time::Duration::from_millis(1500));
        eng.tick_system(now_ms(), false);
        let after = eng.last.sample.fast_avail_bytes;
        let st = o.statuses.first().map_or(0, |s| s.1);
        purge_json = format!(
            "{{\"status\":\"0x{:08X}\",\"ok\":{},\"took_ms\":{},\"fast_avail_before\":{},\"fast_avail_after\":{}}}",
            st as u32, o.ok, took, before, after
        );
        if !o.ok {
            failures.push(format!(
                "purge low-priority standby failed: {}",
                nt::status_name(st)
            ));
        }
        // Memory priority round-trip on ourselves (documented API path).
        let me = std::process::id();
        if !wa::set_memory_priority(me, wa::MEMORY_PRIORITY_LOW) {
            failures.push("SetProcessInformation(ProcessMemoryPriority) failed".into());
        }
    }

    // 3. Icon rasterizer for every style and state.
    for style in IconStyle::ALL {
        for st in PressureState::ALL {
            let spec = IconSpec {
                size: 32,
                fraction: 0.62,
                state: st,
                color: Rgba::rgb(0x0078D4),
                track: Rgba::with_a(0xFFFFFF, 0.3),
                paused: false,
                limited: false,
            };
            let ok = match style {
                IconStyle::Ring => icon::draw_ring(&spec).max_alpha() > 0.9,
                IconStyle::Bar => icon::draw_bar(&spec).max_alpha() > 0.9,
                IconStyle::Number => {
                    crate::tray::render_icon(32, "62", &spec, IconMetric::Used).is_some()
                }
            };
            if !ok {
                failures.push(format!("icon {} / {} failed", style.name(), st.name()));
            }
        }
    }

    // 4. Pool tags (read-only).
    let pool = nt::pool_tags().map(|t| t.len()).unwrap_or(0);

    let (private, ws) = self_memory();
    if private > BUDGET_PRIVATE {
        failures.push(format!(
            "private bytes {} exceed budget {}",
            private, BUDGET_PRIVATE
        ));
    }

    let caps = eng.caps;
    let top: Vec<String> = eng
        .procs
        .top(5)
        .iter()
        .map(|p| {
            format!(
                "{{\"name\":{},\"private\":{}}}",
                json_str(&p.name),
                p.private
            )
        })
        .collect();
    let report = format!(
        "{{\"elevated\":{},\"privileges\":{{\"profile\":{},\"quota\":{}}},\"caps\":{{\"lists\":{},\"mem_commands\":{},\"file_cache\":{},\"combine\":{}}},\
\"total\":{},\"kernel_paged\":{},\"kernel_nonpaged\":{},\"processes\":{},\"compressed_store\":{},\"pool_tags\":{},\"top\":[{}],\"purge\":{},\
\"self\":{{\"private\":{},\"working_set\":{}}},\"samples\":[{}],\"failures\":[{}]}}",
        privileges.elevated,
        privileges.profile,
        privileges.quota,
        caps.lists,
        caps.mem_commands,
        caps.file_cache,
        caps.combine,
        r.total,
        r.kernel_paged,
        r.kernel_nonpaged,
        eng.procs.map.len(),
        eng.procs.compressed_store(),
        pool,
        top.join(","),
        purge_json,
        private,
        ws,
        samples,
        failures
            .iter()
            .map(|f| json_str(f))
            .collect::<Vec<_>>()
            .join(",")
    );
    if let Some(p) = out {
        let _ = std::fs::write(p, &report);
    }
    println!("{report}");
    eng.shutdown();
    if failures.is_empty() { 0 } else { 1 }
}
