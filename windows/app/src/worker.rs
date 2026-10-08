//! The worker thread: sampling, the policy engine, actions, leak detection.
//! Wakes only on kernel memory events, UI commands, and a coalescible timer.

use crate::config::Config;
use crate::nt::{self, MemCommand};
use crate::pool::PoolMonitor;
use crate::privilege::Privileges;
use crate::procs::ProcTable;
use crate::shared::{
    ActivityRow, Caps, Command, Notice, ProcActionKind, ProcRow, Shared, Snapshot,
};
use crate::sysmem::{SysReading, SysSampler};
use crate::winactions::{self as wa, Lowered, Outcome, ids};
use memmanager_core::actions::{Ledger, RegretTracker, Scheduler};
use memmanager_core::cadence::{cadence, tolerance_ms};
use memmanager_core::leak::eta_hours;
use memmanager_core::ring::Ring;
use memmanager_core::sample::{Driver, Sample, pressure_windows};
use memmanager_core::state::{PressureState, StateMachine};
use std::collections::VecDeque;
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Memory::{
    CreateMemoryResourceNotification, LowMemoryResourceNotification,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentProcess, GetCurrentThread, INFINITE, SetThreadPriority,
    SetWaitableTimerEx, THREAD_PRIORITY_BELOW_NORMAL, TIMER_ALL_ACCESS, WaitForMultipleObjects,
};

const MIB: u64 = 1 << 20;
const HISTORY_EVERY_MS: u64 = 5_000;
/// Cache-hit counters are kept per minute for the last hour.
const HITS_EVERY_MS: u64 = 60_000;
const HITS_KEEP: usize = 61;
/// Activity log length, and how long an app keeps its "trimmed"/"quieted" badge.
const ACTIVITY_KEEP: usize = 40;
const ACTED_BADGE_MS: u64 = 30 * 60_000;

/// One logged event (see `ActivityRow`); results come from the ledger.
struct Act {
    t_ms: u64,
    title: String,
    why: String,
    detail: String,
    manual: bool,
    alert: bool,
    /// Has a ledger entry at `t_ms` with a measured result.
    action: bool,
    /// Page-file bytes in use just before, and ~2 s after, the action.
    pf_before: u64,
    pf_after: Option<u64>,
}
const LOW_MEM_SUPPRESS_MS: u64 = 30_000;
const ACTING_MS: u64 = 1_500;
const COMMIT_WARN_FRACTION: f64 = 0.90;

pub fn self_memory() -> (u64, u64) {
    let mut c = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            c.cb,
        )
    };
    if ok.is_ok() {
        (c.PrivateUsage as u64, c.WorkingSetSize as u64)
    } else {
        (0, 0)
    }
}

pub struct Engine {
    pub shared: Arc<Shared>,
    pub cfg: Config,
    pub sampler: SysSampler,
    pub procs: ProcTable,
    pub sm: StateMachine,
    pub scheduler: Scheduler,
    pub regret: RegretTracker,
    pub ledger: Ledger,
    activity: VecDeque<Act>,
    /// (pid, when, what) for the app-list badges.
    acted: VecDeque<(u32, u64, &'static str)>,
    pub lowered: Lowered,
    prev_sample: Option<Sample>,
    pub last: SysReading,
    pub last_p: f64,
    /// What sets the pressure right now (RAM, virtual memory or cache churn).
    pub driver: Driver,
    history: Ring<f32, 120>,
    /// (apps, cache) fractions, same cadence as `history`.
    history_split: Ring<(f32, f32), 120>,
    last_history_ms: u64,
    /// (t, cache hits, disk reads) running counters, one per minute.
    hits_marks: VecDeque<(u64, u64, u64)>,
    pub privileges: Privileges,
    pub caps: Caps,
    paused_until: Option<u64>,
    popup_visible: bool,
    display_off: bool,
    on_battery: bool,
    game_mode: bool,
    acting_until: u64,
    file_cache_capped: bool,
    pool: PoolMonitor,
    pool_note: Option<String>,
    last_commit_warn_ms: Option<u64>,
    task_registered: bool,
    pub next_sys_ms: u64,
    pub next_proc_ms: u64,
}

impl Engine {
    pub fn new(shared: Arc<Shared>, cfg: Config, privileges: Privileges, now: u64) -> Engine {
        let mut e = Engine {
            shared,
            scheduler: Scheduler::new(&wa::specs(cfg.profile), now),
            cfg,
            sampler: SysSampler::new(),
            procs: ProcTable::new(now),
            sm: StateMachine::new(),
            regret: RegretTracker::new(4096),
            ledger: Ledger::new(),
            activity: VecDeque::new(),
            acted: VecDeque::new(),
            lowered: Lowered::default(),
            prev_sample: None,
            last: SysReading::default(),
            last_p: 0.0,
            driver: Driver::None,
            history: Ring::new(),
            history_split: Ring::new(),
            last_history_ms: 0,
            hits_marks: VecDeque::new(),
            privileges,
            caps: Caps {
                mem_commands: privileges.profile,
                file_cache: privileges.quota,
                combine: privileges.profile,
                pool_tags: true,
                lists: false,
            },
            paused_until: None,
            popup_visible: false,
            display_off: false,
            on_battery: !wa::on_ac_power(),
            game_mode: false,
            acting_until: 0,
            file_cache_capped: false,
            pool: PoolMonitor::new(now),
            pool_note: None,
            last_commit_warn_ms: None,
            task_registered: false,
            next_sys_ms: now,
            next_proc_ms: now,
        };
        e.apply_tier_toggles();
        e
    }

    fn apply_tier_toggles(&mut self) {
        for a in self.scheduler.actions.iter_mut() {
            let tier = wa::tier_of(a.spec.id);
            a.enabled = tier == 0 || self.cfg.auto_tiers[(tier - 1) as usize];
        }
    }

    pub fn set_config(&mut self, cfg: Config, now: u64) {
        let profile_changed = cfg.profile != self.cfg.profile;
        self.cfg = cfg;
        if profile_changed {
            // Rebuild buckets for the new profile, keep penalties.
            let old = std::mem::take(&mut self.scheduler.actions);
            self.scheduler = Scheduler::new(&wa::specs(self.cfg.profile), now);
            for a in self.scheduler.actions.iter_mut() {
                if let Some(o) = old.iter().find(|o| o.spec.id == a.spec.id) {
                    a.penalty = o.penalty;
                    a.last_run_ms = o.last_run_ms;
                }
            }
            if self.file_cache_capped
                && self.cfg.profile != memmanager_core::profile::Profile::Aggressive
            {
                wa::set_file_cache_cap(None);
                self.file_cache_capped = false;
            }
        }
        self.apply_tier_toggles();
    }

    pub fn paused(&self, now: u64) -> bool {
        self.paused_until.is_some_and(|t| now < t)
    }

    pub fn handle(&mut self, c: Command, now: u64) -> bool {
        match c {
            Command::OptimizeNow => {
                self.execute(ids::OPTIMIZE_NOW, now, true);
            }
            Command::DeepClean => {
                self.execute(ids::DEEP_CLEAN, now, true);
            }
            Command::SetConfig(c) => self.set_config(*c, now),
            Command::Pause(d) => self.paused_until = d.map(|ms| now + ms),
            Command::PopupVisible(v) => {
                self.popup_visible = v;
                if v {
                    self.next_sys_ms = now;
                    self.next_proc_ms = now;
                }
            }
            Command::ForegroundChanged => {
                let fg = wa::foreground_pid();
                self.lowered.restore(fg);
            }
            Command::Display(on) => {
                self.display_off = !on;
                if on {
                    self.next_sys_ms = now;
                }
            }
            Command::Battery(b) => self.on_battery = b,
            Command::Proc { pid, kind } => self.proc_action(pid, kind, now),
            Command::TaskRegistered(r) => self.task_registered = r,
            Command::Quit => return false,
        }
        true
    }

    fn proc_action(&mut self, pid: u32, kind: ProcActionKind, now: u64) {
        let Some(p) = self.procs.map.get(&pid) else {
            return;
        };
        let (name, ct, ws) = (p.name.clone(), p.create_time, p.working_set);
        let ok = match kind {
            ProcActionKind::LowerPriority => {
                self.lowered.lower(pid, ct, wa::MEMORY_PRIORITY_VERY_LOW)
            }
            ProcActionKind::CapToCurrent => wa::cap_working_set(pid, Some(ws as usize)),
            ProcActionKind::Uncap => wa::cap_working_set(pid, None),
            ProcActionKind::Trim => wa::trim(pid),
            ProcActionKind::End => end_process(pid),
        };
        let verb = match kind {
            ProcActionKind::LowerPriority => "Lowered memory priority of",
            ProcActionKind::CapToCurrent => "Limited RAM of",
            ProcActionKind::Uncap => "Removed RAM limit of",
            ProcActionKind::Trim => "Trimmed",
            ProcActionKind::End => "Ended",
        };
        let (title, detail) = if ok {
            (format!("{verb} {name}"), String::new())
        } else {
            (
                format!("Couldn't change {name}"),
                "Windows denied access".to_string(),
            )
        };
        self.log(Act {
            t_ms: now,
            title,
            why: "You chose this in the app list".into(),
            detail,
            manual: true,
            alert: false,
            action: false,
            pf_before: 0,
            pf_after: None,
        });
        self.next_proc_ms = now;
    }

    /// One system tick. `event` = kernel low-memory signal.
    pub fn tick_system(&mut self, now: u64, event: bool) {
        let Some(r) = self.sampler.sample(now) else {
            return;
        };
        self.caps.lists = r.has_lists;
        if self.regret.is_pending() || self.prev_sample.is_none() {
            // keep page size in sync before the first measurement
        }
        if self.prev_sample.is_none() {
            self.regret = RegretTracker::new(self.sampler.page_size());
        }
        let wp = pressure_windows(
            &r.sample,
            self.prev_sample.as_ref(),
            self.cfg.profile.free_target_frac(),
        );
        let p = wp.p();
        self.last_p = p;
        self.driver = wp.driver();
        let state = self.sm.step(now, p, event);
        if self.regret.observe(
            now,
            r.sample.hard_faults_cum,
            r.sample.fast_avail_bytes,
            &mut self.ledger,
        ) {
            if let Some(e) = self.ledger.last() {
                if let Some(a) = self.scheduler.get_mut(e.action_id) {
                    let pen = a.apply_regret(e.refault_ratio);
                    if let Some(last) = self.ledger.last_mut() {
                        last.penalty_after = pen;
                    }
                }
            }
        }
        self.prev_sample = Some(r.sample);
        // Page-file use ~2 s after each action: how much "freed" RAM went to disk.
        for a in self.activity.iter_mut().rev().take(4) {
            if a.action && a.pf_after.is_none() && now.saturating_sub(a.t_ms) >= 2_000 {
                a.pf_after = Some(r.pagefile_used);
            }
        }
        self.last = r;
        if self.last_history_ms == 0 || now.saturating_sub(self.last_history_ms) >= HISTORY_EVERY_MS
        {
            self.history.push(r.used_fraction() as f32);
            let sp = r.split();
            self.history_split
                .push((sp.apps_fraction() as f32, sp.cache_fraction() as f32));
            self.last_history_ms = now;
        }
        if self
            .hits_marks
            .back()
            .is_none_or(|m| now.saturating_sub(m.0) >= HITS_EVERY_MS)
        {
            self.hits_marks
                .push_back((now, r.cache_hits_cum, r.disk_reads_cum));
            while self.hits_marks.len() > HITS_KEEP {
                self.hits_marks.pop_front();
            }
        }
        self.game_mode = self.cfg.game_mode && wa::game_mode_active();

        if let Some(note) = self.pool.tick(
            now,
            (r.paged_pool_pages as u64) + (r.nonpaged_pool_pages as u64),
            self.sampler.page_size(),
        ) {
            if self.cfg.notify_warnings {
                self.shared.notify(Notice::Warning {
                    title: "Kernel memory leak suspected".into(),
                    text: note.clone(),
                });
            }
            self.log(Act {
                t_ms: now,
                title: "Warned: kernel memory keeps growing".into(),
                why: note.lines().next().unwrap_or_default().to_string(),
                detail: "Usually a driver; MemManager can't free it".into(),
                manual: false,
                alert: true,
                action: false,
                pf_before: 0,
                pf_after: None,
            });
            self.pool_note = Some(note);
        }

        if r.commit_fraction() >= COMMIT_WARN_FRACTION
            && self.cfg.notify_warnings
            && self
                .last_commit_warn_ms
                .is_none_or(|t| now.saturating_sub(t) >= 3_600_000)
        {
            self.last_commit_warn_ms = Some(now);
            let top = self.procs.top(1);
            let who = top.first().map(|p| {
                format!(
                    " Largest: {} ({}).",
                    p.name,
                    crate::util::fmt_bytes(p.private)
                )
            });
            let why = format!(
                "{:.0}% of the commit limit is in use; apps may fail to allocate memory.",
                r.commit_fraction() * 100.0
            );
            self.shared.notify(Notice::Warning {
                title: "Commit charge is almost full".into(),
                text: format!("{why}{}", who.clone().unwrap_or_default()),
            });
            self.log(Act {
                t_ms: now,
                title: "Warned: commit limit almost full".into(),
                why,
                detail: who.unwrap_or_default().trim().to_string(),
                manual: false,
                alert: true,
                action: false,
                pf_before: 0,
                pf_after: None,
            });
        }

        self.maybe_file_cache_cap(&r);
        if !self.paused(now) {
            // RAM actions only help when RAM (not the commit limit) is short:
            // trimming or purging never lowers commit charge.
            self.maybe_act(
                now,
                state,
                wp.phys >= wp.commit,
                wp.phys.max(wp.churn) > 0.0,
            );
        }
    }

    fn maybe_file_cache_cap(&mut self, r: &SysReading) {
        let aggressive = self.cfg.profile == memmanager_core::profile::Profile::Aggressive
            && self.cfg.auto_tiers[4];
        if !aggressive || !self.caps.file_cache {
            return;
        }
        let frac = r.system_cache as f64 / r.total.max(1) as f64;
        if !self.file_cache_capped && frac >= 0.25 {
            self.file_cache_capped = wa::set_file_cache_cap(Some((r.total as f64 * 0.15) as usize));
        } else if self.file_cache_capped && frac < 0.10 {
            wa::set_file_cache_cap(None);
            self.file_cache_capped = false;
        }
    }

    fn maybe_act(&mut self, now: u64, state: PressureState, physical: bool, ram_short: bool) {
        if self.regret.is_pending() {
            return;
        }
        let r = self.last;
        let s = self.sm.smoothed();
        let total = r.total.max(1) as f64;
        let caps = self.caps;
        let game = self.game_mode;
        let f_target = self.cfg.profile.free_target_frac() * total;
        let trim_idle = self.cfg.profile.trim_idle_min() as f64;
        let fg = wa::foreground_pid();
        let has_idle = !self
            .procs
            .idle_candidates(now, trim_idle, 50 * MIB, fg, &self.cfg)
            .is_empty();
        let user_idle = wa::user_idle_ms() >= 5 * 60_000;
        let on_ac = !self.on_battery;
        let max_tier = self.cfg.profile.max_auto_tier();
        let pick = self.scheduler.pick(state, s, now, max_tier, |id| match id {
            ids::DEPRIORITIZE => !game && has_idle,
            ids::PURGE_LOW => caps.mem_commands && caps.lists && r.standby_prio0 >= 256 * MIB,
            ids::GAME_PURGE => {
                game && caps.mem_commands
                    && (r.free_zero as f64) < (1u64 << 30).max((0.06 * total) as u64) as f64
                    && r.standby >= 1 << 30
            }
            ids::PURGE_STANDBY => {
                !game
                    && caps.mem_commands
                    && (r.sample.fast_avail_bytes as f64) < f_target
                    && r.low_repurpose_rate >= 0.0005 * total
            }
            ids::TRIM_IDLE => !game && physical && has_idle,
            ids::FLUSH_MODIFIED => {
                caps.mem_commands && ram_short && (r.modified as f64) >= 0.05 * total
            }
            ids::COMBINE => caps.combine && user_idle && on_ac,
            _ => false,
        });
        if let Some(id) = pick {
            self.execute(id, now, false);
        }
    }

    /// Runs one action now and records it in the ledger.
    pub fn execute(&mut self, id: u8, now: u64, manual: bool) -> Outcome {
        let state = self.sm.state();
        let before = self.last;
        let why = self.reason(id, manual);
        let mut names: Vec<String> = Vec::new();
        self.regret.begin(
            now,
            id,
            manual,
            state,
            before.sample.hard_faults_cum,
            before.sample.fast_avail_bytes,
            &mut self.ledger,
        );
        if let Some(a) = self.scheduler.get_mut(id) {
            a.mark_run(now);
        }
        let mut out = Outcome::default();
        let trim_idle = self.cfg.profile.trim_idle_min() as f64;
        match id {
            ids::DEPRIORITIZE => {
                let prio = if state >= PressureState::High {
                    wa::MEMORY_PRIORITY_VERY_LOW
                } else {
                    wa::MEMORY_PRIORITY_LOW
                };
                names = self.deprioritize(now, trim_idle, prio, 8);
                out.ok = !names.is_empty();
                out.detail = format!("{} app(s) moved to low memory priority", names.len());
            }
            ids::PURGE_LOW => {
                out.ok = wa::run_mem_command(
                    MemCommand::PurgeLowPriorityStandbyList,
                    "purge_low_priority_standby",
                    &mut out,
                );
            }
            ids::PURGE_STANDBY | ids::GAME_PURGE => {
                out.ok =
                    wa::run_mem_command(MemCommand::PurgeStandbyList, "purge_standby", &mut out);
            }
            ids::TRIM_IDLE => {
                names = self.trim_idle(now, trim_idle, 5);
                out.ok = !names.is_empty();
                out.detail = format!("{} idle app(s) trimmed", names.len());
            }
            ids::FLUSH_MODIFIED => {
                out.ok =
                    wa::run_mem_command(MemCommand::FlushModifiedList, "flush_modified", &mut out);
            }
            ids::COMBINE => {
                let (st, pages) = nt::combine_pages();
                out.statuses.push(("combine_pages", st));
                out.ok = nt::ok(st);
                if st == nt::STATUS_INVALID_INFO_CLASS {
                    self.caps.combine = false;
                }
                out.detail = format!(
                    "{} combined",
                    crate::util::fmt_bytes(pages as u64 * self.sampler.page_size())
                );
            }
            ids::OPTIMIZE_NOW => {
                names = self.deprioritize(now, trim_idle.min(10.0), wa::MEMORY_PRIORITY_LOW, 32);
                let n1 = names.len();
                let mut ok = n1 > 0;
                if self.caps.mem_commands {
                    ok |= wa::run_mem_command(
                        MemCommand::PurgeLowPriorityStandbyList,
                        "purge_low_priority_standby",
                        &mut out,
                    );
                    if (before.modified as f64) >= 0.05 * before.total as f64 {
                        wa::run_mem_command(
                            MemCommand::FlushModifiedList,
                            "flush_modified",
                            &mut out,
                        );
                    }
                }
                let trimmed = self.trim_idle(now, trim_idle.min(10.0), 5);
                let n2 = trimmed.len();
                for t in trimmed {
                    if !names.contains(&t) {
                        names.push(t);
                    }
                }
                ok |= n2 > 0;
                out.ok = ok;
                out.detail = format!(
                    "{n1} app(s) quieted, {n2} idle app(s) trimmed, low-priority cache cleared"
                );
            }
            ids::DEEP_CLEAN => {
                let mut ok = false;
                if self.caps.mem_commands {
                    ok |= wa::run_mem_command(
                        MemCommand::EmptyWorkingSets,
                        "empty_working_sets",
                        &mut out,
                    );
                }
                if self.caps.file_cache {
                    let st = nt::flush_file_cache();
                    out.statuses.push(("flush_file_cache", st));
                    ok |= nt::ok(st);
                }
                let vols = wa::flush_volumes();
                if self.caps.mem_commands {
                    ok |= wa::run_mem_command(
                        MemCommand::FlushModifiedList,
                        "flush_modified",
                        &mut out,
                    );
                    ok |= wa::run_mem_command(
                        MemCommand::PurgeStandbyList,
                        "purge_standby",
                        &mut out,
                    );
                }
                if self.caps.combine {
                    let (st, _) = nt::combine_pages();
                    out.statuses.push(("combine_pages", st));
                }
                out.ok = ok;
                out.detail = format!(
                    "working sets, file cache, {vols} volume(s), modified and standby lists"
                );
            }
            _ => {}
        }
        if !out.ok && out.detail.is_empty() {
            let failed: Vec<String> = out
                .statuses
                .iter()
                .filter(|(_, st)| !nt::ok(*st))
                .map(|(n, st)| format!("{n}: {}", nt::status_name(*st)))
                .collect();
            out.detail = failed.join(", ");
        }
        let detail = if !names.is_empty() {
            crate::values::apps_list(&names)
        } else if !out.ok {
            out.detail.clone()
        } else {
            String::new()
        };
        self.log(Act {
            t_ms: now,
            title: wa::action_label(id).into(),
            why,
            detail,
            manual,
            alert: false,
            action: true,
            pf_before: before.pagefile_used,
            pf_after: None,
        });
        self.acting_until = now + ACTING_MS;
        // Measure the immediate effect right away (the 2 s "after" sample comes later).
        if manual {
            self.next_sys_ms = now + 2_000;
            if self.cfg.notify_results {
                self.shared.notify(Notice::Info {
                    title: wa::action_label(id).into(),
                    text: out.detail.clone(),
                });
            }
        }
        out
    }

    /// Why an action runs, in plain words, from the reading that triggered it.
    fn reason(&self, id: u8, manual: bool) -> String {
        let r = &self.last;
        let b = crate::util::fmt_bytes;
        if manual {
            return match id {
                ids::DEEP_CLEAN => "You asked for a deep clean".into(),
                _ => "You asked".into(),
            };
        }
        match id {
            ids::DEPRIORITIZE => "Memory got busy; idle background apps now go last in line".into(),
            ids::PURGE_LOW => format!(
                "{} of cache that Windows itself marked least useful",
                b(r.standby_prio0)
            ),
            ids::PURGE_STANDBY => format!(
                "Only {} was quickly free and Windows was already recycling its cache",
                b(r.sample.fast_avail_bytes)
            ),
            ids::GAME_PURGE => format!(
                "A full-screen game is running and free memory fell to {}",
                b(r.free_zero)
            ),
            ids::TRIM_IDLE => "Apps were competing for memory; these were idle and silent".into(),
            ids::FLUSH_MODIFIED => format!(
                "{} of changed pages were waiting to be written out",
                b(r.modified)
            ),
            ids::COMBINE => "PC idle and on power: merging identical pages".into(),
            _ => String::new(),
        }
    }

    fn log(&mut self, a: Act) {
        self.activity.push_back(a);
        while self.activity.len() > ACTIVITY_KEEP {
            self.activity.pop_front();
        }
    }

    fn mark_acted(&mut self, pid: u32, now: u64, what: &'static str) {
        self.acted
            .retain(|(p, t, _)| *p != pid && now.saturating_sub(*t) < ACTED_BADGE_MS);
        self.acted.push_back((pid, now, what));
        while self.acted.len() > 32 {
            self.acted.pop_front();
        }
    }

    /// Lowers the memory priority of idle apps; returns their names.
    fn deprioritize(&mut self, now: u64, idle_min: f64, prio: u32, max: usize) -> Vec<String> {
        let fg = wa::foreground_pid();
        let audible = wa::audible_pids();
        let cands: Vec<(u32, i64)> = self
            .procs
            .idle_candidates(now, idle_min, 50 * MIB, fg, &self.cfg)
            .into_iter()
            .filter(|p| !audible.contains(&p.pid) && !self.lowered.contains(p.pid))
            .take(max)
            .map(|p| (p.pid, p.create_time))
            .collect();
        let mut names = Vec::new();
        for (pid, ct) in cands {
            if self.lowered.lower(pid, ct, prio) {
                self.mark_acted(pid, now, "quieted");
                if let Some(p) = self.procs.map.get(&pid) {
                    names.push(p.name.clone());
                }
            }
        }
        names
    }

    /// Trims idle apps' working sets; returns their names.
    fn trim_idle(&mut self, now: u64, idle_min: f64, max: usize) -> Vec<String> {
        let fg = wa::foreground_pid();
        let audible = wa::audible_pids();
        let cands: Vec<u32> = self
            .procs
            .idle_candidates(now, idle_min, 100 * MIB, fg, &self.cfg)
            .into_iter()
            .filter(|p| !audible.contains(&p.pid))
            .take(max)
            .map(|p| p.pid)
            .collect();
        let mut names = Vec::new();
        for pid in cands {
            if wa::trim(pid) {
                self.mark_acted(pid, now, "trimmed");
                if let Some(p) = self.procs.map.get(&pid) {
                    names.push(p.name.clone());
                }
            }
        }
        names
    }

    pub fn tick_procs(&mut self, now: u64) {
        let Ok(flagged) = self.procs.update(now, &self.cfg) else {
            return;
        };
        let map = &self.procs.map;
        self.lowered
            .prune(|pid, ct| map.get(&pid).is_some_and(|p| p.create_time == ct));
        let headroom_mib =
            self.last.commit_limit.saturating_sub(self.last.commit) as f64 / memmanager_core::MIB;
        let mut alerts = Vec::new();
        for pid in flagged {
            let Some(p) = self.procs.map.get_mut(&pid) else {
                continue;
            };
            if self.cfg.ignores_leaks(&p.key) || p.is_critical() {
                continue;
            }
            let eta = eta_hours(headroom_mib, p.leak.slope);
            let due = match p.leak_notified_ms {
                None => true,
                Some(t) => {
                    now.saturating_sub(t) >= 6 * 3_600_000
                        || eta.is_some_and(|e| e <= p.notified_eta_h / 2.0)
                }
            };
            if due && self.cfg.notify_leaks {
                p.leak_notified_ms = Some(now);
                p.notified_eta_h = eta.unwrap_or(f64::INFINITY);
                let runaway = p.leak.runaway && !p.leak.leak;
                self.shared.notify(Notice::Leak {
                    name: p.name.clone(),
                    slope_mib_h: p.leak.slope,
                    eta_h: eta,
                    runaway,
                });
                alerts.push(Act {
                    t_ms: now,
                    title: if runaway {
                        format!("Warned: {} is allocating very fast", p.name)
                    } else {
                        format!("Warned: {} looks like it's leaking", p.name)
                    },
                    why: format!(
                        "Grew steadily by about {:.0} MB per hour",
                        p.leak.slope.max(0.0)
                    ),
                    detail: "Restarting it frees the memory".into(),
                    manual: false,
                    alert: true,
                    action: false,
                    pf_before: 0,
                    pf_after: None,
                });
            }
        }
        for a in alerts {
            self.log(a);
        }
    }

    pub fn snapshot(&mut self, now: u64) -> Snapshot {
        let (self_private, self_ws) = self_memory();
        let hits0 = self.hits_marks.front().copied().unwrap_or((now, 0, 0));
        // Lists only the flyout shows are built only while it is visible.
        let full = self.popup_visible;
        let acted = |pid: u32| {
            self.acted
                .iter()
                .rev()
                .find(|(p, t, _)| *p == pid && now.saturating_sub(*t) < ACTED_BADGE_MS)
                .map(|(_, _, w)| *w)
        };
        let top = if full {
            self.procs
                .top(6)
                .into_iter()
                .map(|p| ProcRow {
                    pid: p.pid,
                    name: p.name.clone(),
                    key: p.key.clone(),
                    private: p.private,
                    working_set: p.working_set,
                    slope_mib_h: if p.hist.as_ref().is_some_and(|h| h.len() >= 10) {
                        p.leak.slope
                    } else {
                        0.0
                    },
                    leak: p.leak.leak && !self.cfg.ignores_leaks(&p.key),
                    runaway: p.leak.runaway,
                    lowered: self.lowered.contains(p.pid),
                    critical: p.is_critical(),
                    idle_min: p.idle_minutes(now),
                    acted: acted(p.pid),
                })
                .collect()
        } else {
            Vec::new()
        };
        let activity = if full {
            self.activity
                .iter()
                .rev()
                .map(|a| {
                    let e = if a.action {
                        self.ledger.iter().rev().find(|e| e.t_ms == a.t_ms)
                    } else {
                        None
                    };
                    ActivityRow {
                        t_ms: a.t_ms,
                        title: a.title.clone(),
                        why: a.why.clone(),
                        detail: a.detail.clone(),
                        manual: a.manual,
                        alert: a.alert,
                        measured: e.is_some(),
                        freed: e.map_or(0, |e| e.freed_bytes()),
                        refault_ratio: e.map_or(0.0, |e| e.refault_ratio),
                        pending: e.is_some_and(|e| e.pending),
                        pagefile_delta: a
                            .pf_after
                            .map_or(0, |after| after as i64 - a.pf_before as i64),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        Snapshot {
            t_ms: now,
            reading: self.last,
            driver: self.driver,
            compressed: self.procs.compressed_store(),
            s: self.sm.smoothed(),
            state: self.sm.state(),
            history: if full {
                self.history.iter().collect()
            } else {
                Vec::new()
            },
            history_split: if full {
                self.history_split.iter().collect()
            } else {
                Vec::new()
            },
            cache_hits: self.last.cache_hits_cum.saturating_sub(hits0.1),
            disk_reads: self.last.disk_reads_cum.saturating_sub(hits0.2),
            hits_window_ms: now.saturating_sub(hits0.0),
            top,
            activity,
            privileges: self.privileges,
            caps: self.caps,
            game_mode: self.game_mode,
            paused: self.paused(now),
            acting: now < self.acting_until,
            pool_note: self.pool_note.clone(),
            self_private,
            self_working_set: self_ws,
            task_registered: self.task_registered,
            lowered_count: self.lowered.len(),
        }
    }

    pub fn shutdown(&mut self) {
        self.lowered.restore_all();
        if self.file_cache_capped {
            wa::set_file_cache_cap(None);
        }
    }
}

fn end_process(pid: u32) -> bool {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    unsafe {
        match OpenProcess(PROCESS_TERMINATE, false, pid) {
            Ok(h) => {
                let ok = TerminateProcess(h, 1).is_ok();
                let _ = CloseHandle(h);
                ok
            }
            Err(_) => false,
        }
    }
}

/// Worker thread entry point.
pub fn run(shared: Arc<Shared>, cfg: Config, privileges: Privileges) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    let now = crate::util::now_ms();
    let mut eng = Engine::new(shared.clone(), cfg, privileges, now);
    let timer = unsafe { CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0) }.ok();
    let low_mem = unsafe { CreateMemoryResourceNotification(LowMemoryResourceNotification) }.ok();
    let mut low_mem_suppressed_until = 0u64;

    loop {
        let now = crate::util::now_ms();
        let cad = cadence(
            eng.popup_visible,
            eng.sm.state(),
            eng.display_off,
            eng.on_battery,
        );
        let mut wait_ms = INFINITE;
        if let Some(c) = cad {
            let due = eng.next_sys_ms.min(eng.next_proc_ms);
            let delay = due.saturating_sub(now);
            match timer {
                Some(t) => {
                    let due_100ns = -((delay.max(1) as i64) * 10_000);
                    let tol = tolerance_ms(c.system_ms);
                    unsafe {
                        let _ = SetWaitableTimerEx(t, &due_100ns, 0, None, None, None, tol);
                    }
                }
                None => wait_ms = delay as u32,
            }
        }
        let mut handles: Vec<HANDLE> = vec![shared.cmd_event()];
        if let (Some(t), Some(_)) = (timer, cad) {
            handles.push(t);
        }
        let low_idx = match low_mem {
            Some(h) if now >= low_mem_suppressed_until => {
                handles.push(h);
                Some(handles.len() - 1)
            }
            _ => None,
        };
        let wait_ms = if low_idx.is_none() && low_mem.is_some() {
            wait_ms.min(low_mem_suppressed_until.saturating_sub(now) as u32 + 1)
        } else {
            wait_ms
        };
        let r = unsafe { WaitForMultipleObjects(&handles, false, wait_ms) };
        let fired = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;

        let now = crate::util::now_ms();
        let mut keep = true;
        for c in shared.take_commands() {
            keep &= eng.handle(c, now);
        }
        if !keep {
            break;
        }
        if low_idx == Some(fired) {
            low_mem_suppressed_until = now + LOW_MEM_SUPPRESS_MS;
            eng.tick_system(now, true);
            eng.next_sys_ms = now + cad.map_or(10_000, |c| c.system_ms as u64);
        }
        if let Some(c) = cadence(
            eng.popup_visible,
            eng.sm.state(),
            eng.display_off,
            eng.on_battery,
        ) {
            if now >= eng.next_proc_ms {
                eng.tick_procs(now);
                eng.next_proc_ms = now + c.process_ms as u64;
            }
            if now >= eng.next_sys_ms {
                eng.tick_system(now, false);
                eng.next_sys_ms = now + c.system_ms as u64;
            }
        }
        let snap = eng.snapshot(now);
        shared.publish(snap);
    }
    eng.shutdown();
    unsafe {
        if let Some(t) = timer {
            let _ = CloseHandle(t);
        }
        if let Some(h) = low_mem {
            let _ = CloseHandle(h);
        }
    }
}
