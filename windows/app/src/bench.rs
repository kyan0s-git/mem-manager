//! Reproducible memory-bloat benchmark: creates realistic bloat on the live
//! system, runs each MemManager action through the real code paths, and
//! measures what it freed and what it cost afterwards (re-touch latency and
//! hard page faults). Run elevated: `memmanager.exe --bench [--out=PATH]`.

use crate::nt::{self, MemCommand};
use crate::privilege;
use crate::sysmem::{SysReading, SysSampler};
use crate::util::{json_str, now_ms, wide};
use crate::winactions;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::core::PCWSTR;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const CHILDREN: u32 = 3;

struct Event(HANDLE);

impl Event {
    fn named(name: &str) -> Option<Event> {
        let w = wide(name);
        unsafe {
            CreateEventW(None, false, false, PCWSTR(w.as_ptr()))
                .ok()
                .map(Event)
        }
    }
    fn set(&self) {
        unsafe {
            let _ = SetEvent(self.0);
        }
    }
    fn wait(&self, ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, ms) == WAIT_OBJECT_0 }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Parent PID passed via the environment so event names are unique per run.
fn parent_hint() -> u32 {
    std::env::var("MEMMANAGER_BENCH_PARENT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(std::process::id)
}

fn ev_name(parent: u32, id: u32, what: &str) -> String {
    format!("Local\\MemManagerBench.{parent}.{id}.{what}")
}

fn retouch_file(id: u32) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("memmanager-bench-{}-{id}.ms", parent_hint()))
}

/// Child: allocate and touch `mib` MiB, then serve re-touch requests until told to exit.
pub fn child(mib: u64, id: u32) -> i32 {
    let size = (mib * MIB) as usize;
    let mut buf = vec![0u8; size];
    for (i, chunk) in buf.chunks_mut(4096).enumerate() {
        // Distinct page contents (not combinable), like real heap data.
        let tag = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ id as u64;
        chunk[..8].copy_from_slice(&tag.to_le_bytes());
    }
    let (Some(ready), Some(retouch), Some(done), Some(exit)) = (
        Event::named(&ev_name(parent_hint(), id, "ready")),
        Event::named(&ev_name(parent_hint(), id, "retouch")),
        Event::named(&ev_name(parent_hint(), id, "done")),
        Event::named(&ev_name(parent_hint(), id, "exit")),
    ) else {
        return 2;
    };
    ready.set();
    loop {
        let r = unsafe { WaitForMultipleObjects(&[retouch.0, exit.0], false, 600_000) };
        if r == WAIT_OBJECT_0 {
            let t = Instant::now();
            let mut sum = 0u64;
            for chunk in buf.chunks(4096) {
                sum = sum.wrapping_add(chunk[0] as u64);
            }
            std::hint::black_box(sum);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let _ = std::fs::write(retouch_file(id), format!("{ms:.3}"));
            done.set();
        } else {
            return 0;
        }
    }
}

struct Kid {
    child: Child,
    id: u32,
    retouch: Event,
    done: Event,
    exit: Event,
}

impl Kid {
    fn working_set(&self) -> u64 {
        let mut c = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        let h = HANDLE(self.child.as_raw_handle());
        unsafe {
            if GetProcessMemoryInfo(h, &mut c, c.cb).is_ok() {
                c.WorkingSetSize as u64
            } else {
                0
            }
        }
    }

    fn retouch_ms(&self) -> f64 {
        let _ = std::fs::remove_file(retouch_file(self.id));
        self.retouch.set();
        if !self.done.wait(120_000) {
            return -1.0;
        }
        std::fs::read_to_string(retouch_file(self.id))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(-1.0)
    }
}

impl Drop for Kid {
    fn drop(&mut self) {
        self.exit.set();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = std::fs::remove_file(retouch_file(self.id));
    }
}

fn hard_faults() -> u64 {
    nt::perf_info().map_or(0, |p| p.page_read_count as u64)
}

fn sample(s: &mut SysSampler) -> SysReading {
    s.sample(now_ms()).unwrap_or_default()
}

struct Scenario {
    id: &'static str,
    name: &'static str,
    action: &'static str,
    freed: i64,
    cost: String,
    details: Vec<(&'static str, String)>,
    skipped: Option<String>,
}

impl Scenario {
    fn json(&self, total: u64) -> String {
        let details: Vec<String> = self
            .details
            .iter()
            .map(|(k, v)| format!("{}:{}", json_str(k), v))
            .collect();
        format!(
            "{{\"id\":{},\"name\":{},\"action\":{},\"freed_bytes\":{},\"freed_pct_ram\":{:.2},\"cost\":{},\"skipped\":{},\"details\":{{{}}}}}",
            json_str(self.id),
            json_str(self.name),
            json_str(self.action),
            self.freed,
            self.freed as f64 * 100.0 / total.max(1) as f64,
            json_str(&self.cost),
            self.skipped.as_deref().map_or("null".to_string(), json_str),
            details.join(",")
        )
    }
}

fn read_file_ms(path: &std::path::Path) -> f64 {
    let t = Instant::now();
    if let Ok(mut f) = std::fs::File::open(path) {
        let mut buf = vec![0u8; 4 << 20];
        while let Ok(n) = f.read(&mut buf) {
            if n == 0 {
                break;
            }
        }
    }
    t.elapsed().as_secs_f64() * 1000.0
}

fn gib(b: i64) -> String {
    format!("{:.2}", b as f64 / GIB as f64)
}

pub fn run(out: Option<String>) -> i32 {
    let privs = privilege::enable_all();
    let mut s = SysSampler::new();
    let r0 = sample(&mut s);
    let total = r0.total;
    let mut scen: Vec<Scenario> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    if !privs.profile {
        failures.push("not elevated: memory-list commands unavailable".into());
    }

    // ---- spawn idle children holding private memory ---------------------
    let idle_total = (2 * GIB).min(total * 15 / 100);
    let per_child_mib = (idle_total / CHILDREN as u64 / MIB).max(64);
    let exe = std::env::current_exe().unwrap_or_default();
    let parent = std::process::id();
    let mut kids: Vec<Kid> = Vec::new();
    for id in 0..CHILDREN {
        let (Some(ready), Some(retouch), Some(done), Some(exit)) = (
            Event::named(&ev_name(parent, id, "ready")),
            Event::named(&ev_name(parent, id, "retouch")),
            Event::named(&ev_name(parent, id, "done")),
            Event::named(&ev_name(parent, id, "exit")),
        ) else {
            failures.push("could not create events".into());
            break;
        };
        match Command::new(&exe)
            .arg(format!("--bench-child={per_child_mib}"))
            .arg(format!("--bench-id={id}"))
            .env("MEMMANAGER_BENCH_PARENT", parent.to_string())
            .spawn()
        {
            Ok(child) => {
                if !ready.wait(120_000) {
                    failures.push(format!("child {id} did not become ready"));
                }
                kids.push(Kid {
                    child,
                    id,
                    retouch,
                    done,
                    exit,
                });
            }
            Err(e) => failures.push(format!("spawn child: {e}")),
        }
    }
    std::thread::sleep(Duration::from_secs(2));
    let warm_ms: f64 = kids.iter().map(|k| k.retouch_ms()).sum();

    // ---- A: selective trim of idle apps --------------------------------
    {
        let before = sample(&mut s);
        let ws_before: u64 = kids.iter().map(|k| k.working_set()).sum();
        let trimmed = kids
            .iter()
            .filter(|k| winactions::trim(k.child.id()))
            .count();
        std::thread::sleep(Duration::from_secs(2));
        let after = sample(&mut s);
        let ws_after: u64 = kids.iter().map(|k| k.working_set()).sum();
        let freed = before.in_use as i64 - after.in_use as i64;

        // ---- C: flush the modified list the trim produced -------------
        let mod_before = sample(&mut s);
        let mut c = Scenario {
            id: "C",
            name: "Modified (dirty) pages from trimmed apps",
            action: "Flush modified list (tier 4)",
            freed: 0,
            cost: String::new(),
            details: vec![],
            skipped: None,
        };
        if privs.profile {
            let st = nt::mem_command(MemCommand::FlushModifiedList);
            std::thread::sleep(Duration::from_secs(2));
            let mod_after = sample(&mut s);
            c.freed = mod_before.modified as i64 - mod_after.modified as i64;
            c.cost = "pagefile writes; pages become reusable cache".into();
            c.details = vec![
                ("modified_before", mod_before.modified.to_string()),
                ("modified_after", mod_after.modified.to_string()),
                ("status", format!("\"0x{:08X}\"", st as u32)),
            ];
        } else {
            c.skipped = Some("needs elevation".into());
        }

        // Cost of the trim: re-touch everything.
        let hf0 = hard_faults();
        let retouch: f64 = kids.iter().map(|k| k.retouch_ms()).sum();
        let hf = hard_faults().saturating_sub(hf0);
        scen.push(Scenario {
            id: "A",
            name: "Idle apps holding private memory",
            action: "Selective trim of idle apps (tier 4)",
            freed,
            cost: format!(
                "re-touch {retouch:.0} ms vs {warm_ms:.0} ms warm; {hf} hard page faults",
            ),
            details: vec![
                (
                    "bloat_bytes",
                    (per_child_mib * MIB * CHILDREN as u64).to_string(),
                ),
                ("processes_trimmed", trimmed.to_string()),
                ("working_set_before", ws_before.to_string()),
                ("working_set_after", ws_after.to_string()),
                ("retouch_ms_warm", format!("{warm_ms:.1}")),
                ("retouch_ms_after", format!("{retouch:.1}")),
                ("hard_faults_during_retouch", hf.to_string()),
            ],
            skipped: None,
        });
        scen.push(c);
    }

    // ---- B: file cache (standby) ---------------------------------------
    let file_bytes = (2 * GIB).min(total * 20 / 100);
    let path = std::env::temp_dir().join(format!("memmanager-bench-{parent}.dat"));
    {
        let ok = (|| -> std::io::Result<()> {
            let mut f = std::fs::File::create(&path)?;
            let mut chunk = vec![0u8; 4 << 20];
            let mut written = 0u64;
            let mut seed = 0x1234_5678_u64;
            while written < file_bytes {
                for b in chunk.iter_mut().step_by(512) {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    *b = (seed >> 33) as u8;
                }
                f.write_all(&chunk)?;
                written += chunk.len() as u64;
            }
            f.sync_all()
        })();
        if let Err(e) = ok {
            failures.push(format!("temp file: {e}"));
        }
        let warm_read = read_file_ms(&path);
        let before = sample(&mut s);
        let mut low = Scenario {
            id: "B1",
            name: "File cache after reading a large file",
            action: "Clear low-priority cache (tier 2)",
            freed: 0,
            cost: String::new(),
            details: vec![],
            skipped: None,
        };
        let mut full = Scenario {
            id: "B2",
            name: "File cache after reading a large file",
            action: "Clear standby cache (tier 3)",
            freed: 0,
            cost: String::new(),
            details: vec![],
            skipped: None,
        };
        if privs.profile {
            let st1 = nt::mem_command(MemCommand::PurgeLowPriorityStandbyList);
            std::thread::sleep(Duration::from_secs(1));
            let mid = sample(&mut s);
            low.freed = before.standby as i64 - mid.standby as i64;
            low.cost = "none: only cache Windows was about to discard".into();
            low.details = vec![
                ("standby_before", before.standby.to_string()),
                ("standby_prio0_before", before.standby_prio0.to_string()),
                ("standby_after", mid.standby.to_string()),
                ("status", format!("\"0x{:08X}\"", st1 as u32)),
            ];
            let st2 = nt::mem_command(MemCommand::PurgeStandbyList);
            std::thread::sleep(Duration::from_secs(1));
            let after = sample(&mut s);
            let cold_read = read_file_ms(&path);
            full.freed = mid.standby as i64 - after.standby as i64;
            full.cost = format!(
                "re-reading the file took {cold_read:.0} ms vs {warm_read:.0} ms from cache"
            );
            full.details = vec![
                ("file_bytes", file_bytes.to_string()),
                ("standby_before", mid.standby.to_string()),
                ("standby_after", after.standby.to_string()),
                ("free_zero_before", mid.free_zero.to_string()),
                ("free_zero_after", after.free_zero.to_string()),
                ("read_ms_cached", format!("{warm_read:.1}")),
                ("read_ms_after_purge", format!("{cold_read:.1}")),
                ("status", format!("\"0x{:08X}\"", st2 as u32)),
            ];
        } else {
            low.skipped = Some("needs elevation".into());
            full.skipped = Some("needs elevation".into());
        }
        scen.push(low);
        scen.push(full);
    }

    // ---- D: deep clean (everything at once) ------------------------------
    {
        let _ = read_file_ms(&path); // re-populate the cache
        for k in &kids {
            k.retouch_ms(); // make the children resident again
        }
        let mut d = Scenario {
            id: "D",
            name: "All of the above at once",
            action: "Deep clean (manual)",
            freed: 0,
            cost: String::new(),
            details: vec![],
            skipped: None,
        };
        if privs.profile {
            let hf_base0 = hard_faults();
            std::thread::sleep(Duration::from_secs(10));
            let base_hf = hard_faults().saturating_sub(hf_base0);
            let before = sample(&mut s);
            let avail_before = before.free_zero + before.standby;
            nt::mem_command(MemCommand::EmptyWorkingSets);
            if privs.quota {
                nt::flush_file_cache();
            }
            nt::mem_command(MemCommand::FlushModifiedList);
            nt::mem_command(MemCommand::PurgeStandbyList);
            std::thread::sleep(Duration::from_secs(1));
            let after = sample(&mut s);
            let hf0 = hard_faults();
            let retouch: f64 = kids.iter().map(|k| k.retouch_ms()).sum();
            std::thread::sleep(Duration::from_secs(10));
            let after_hf = hard_faults().saturating_sub(hf0);
            d.freed = after.free_zero as i64 - before.free_zero as i64;
            d.cost = format!(
                "{after_hf} hard faults in the next 10 s (baseline {base_hf}); apps re-touch {retouch:.0} ms"
            );
            d.details = vec![
                ("in_use_before", before.in_use.to_string()),
                ("in_use_after", after.in_use.to_string()),
                ("free_zero_before", before.free_zero.to_string()),
                ("free_zero_after", after.free_zero.to_string()),
                ("available_before", avail_before.to_string()),
                ("hard_faults_baseline_10s", base_hf.to_string()),
                ("hard_faults_after_10s", after_hf.to_string()),
            ];
        } else {
            d.skipped = Some("needs elevation".into());
        }
        scen.push(d);
    }

    // ---- E: page combining -----------------------------------------------
    {
        let mut e = Scenario {
            id: "E",
            name: "Identical pages across the system",
            action: "Combine pages (tier 5)",
            freed: 0,
            cost: "CPU scan only".into(),
            details: vec![],
            skipped: None,
        };
        if privs.profile {
            let (st, pages) = nt::combine_pages();
            e.freed = (pages as u64 * s.page_size()) as i64;
            e.details = vec![
                ("pages_combined", pages.to_string()),
                ("status", format!("\"0x{:08X}\"", st as u32)),
            ];
            if !nt::ok(st) {
                e.skipped = Some(format!("not supported here ({})", nt::status_name(st)));
            }
        } else {
            e.skipped = Some("needs elevation".into());
        }
        scen.push(e);
    }

    drop(kids);
    let _ = std::fs::remove_file(&path);

    let mut lines = Vec::new();
    for id in ["A", "C", "B1", "B2", "D", "E"] {
        let Some(sc) = scen.iter().find(|s| s.id == id) else {
            continue;
        };
        lines.push(sc.json(total));
        eprintln!(
            "{:<3} {:<42} freed {} GiB  | {}",
            sc.id,
            sc.action,
            gib(sc.freed),
            sc.skipped.clone().unwrap_or_else(|| sc.cost.clone())
        );
    }
    let report = format!(
        "{{\"os\":\"windows\",\"build\":{},\"total_ram\":{},\"elevated\":{},\"scenarios\":[{}],\"failures\":[{}]}}",
        nt::os_build(),
        total,
        privs.elevated,
        lines.join(","),
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
    if failures.is_empty() { 0 } else { 1 }
}
