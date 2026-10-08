//! State shared between the UI thread and the worker thread.

use crate::config::Config;
use crate::privilege::Privileges;
use crate::sysmem::SysReading;
use crate::update::Status as UpdateStatus;
use memmanager_core::state::PressureState;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const WM_APP_SNAPSHOT: u32 = WM_APP + 2;
pub const WM_APP_NOTICE: u32 = WM_APP + 3;
/// The update status changed (`Shared::update_status`).
pub const WM_APP_UPDATE: u32 = WM_APP + 10;
/// An update is being installed: exit so it can replace this copy.
pub const WM_APP_EXIT_FOR_UPDATE: u32 = WM_APP + 11;

#[derive(Clone, Debug, Default)]
pub struct ProcRow {
    pub pid: u32,
    pub name: String,
    pub key: String,
    pub private: u64,
    pub working_set: u64,
    /// Theil–Sen slope of private bytes, MiB/h (0 until enough history).
    pub slope_mib_h: f64,
    pub leak: bool,
    pub runaway: bool,
    pub lowered: bool,
    pub critical: bool,
    /// Minutes without CPU activity.
    pub idle_min: f64,
}

#[derive(Clone, Debug, Default)]
pub struct LedgerRow {
    pub t_ms: u64,
    pub label: &'static str,
    pub detail: String,
    pub manual: bool,
    pub freed: u64,
    pub refault_ratio: f64,
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    pub lists: bool,
    pub mem_commands: bool,
    pub file_cache: bool,
    pub combine: bool,
    pub pool_tags: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub t_ms: u64,
    pub reading: SysReading,
    pub compressed: u64,
    pub s: f64,
    pub state: PressureState,
    /// Used fraction, one point per ~5 s, oldest first (≤ 120 points).
    pub history: Vec<f32>,
    /// (apps, cache) fractions at the same points as `history`.
    pub history_split: Vec<(f32, f32)>,
    /// Page faults answered from cache, and pages read from disk, over `hits_window_ms`.
    pub cache_hits: u64,
    pub disk_reads: u64,
    pub hits_window_ms: u64,
    pub top: Vec<ProcRow>,
    /// Newest first.
    pub ledger: Vec<LedgerRow>,
    pub privileges: Privileges,
    pub caps: Caps,
    pub game_mode: bool,
    pub paused: bool,
    pub acting: bool,
    pub pool_note: Option<String>,
    pub self_private: u64,
    pub self_working_set: u64,
    pub task_registered: bool,
    pub lowered_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcActionKind {
    LowerPriority,
    CapToCurrent,
    Uncap,
    Trim,
    End,
}

#[derive(Debug)]
pub enum Command {
    OptimizeNow,
    DeepClean,
    SetConfig(Box<Config>),
    /// Pause automatic actions for this many ms (`None` resumes).
    Pause(Option<u64>),
    PopupVisible(bool),
    ForegroundChanged,
    Display(bool),
    Battery(bool),
    Proc {
        pid: u32,
        kind: ProcActionKind,
    },
    TaskRegistered(bool),
    Quit,
}

#[derive(Clone, Debug)]
pub enum Notice {
    Leak {
        name: String,
        slope_mib_h: f64,
        eta_h: Option<f64>,
        runaway: bool,
    },
    Info {
        title: String,
        text: String,
    },
    Warning {
        title: String,
        text: String,
    },
}

pub struct Shared {
    pub snapshot: Mutex<Snapshot>,
    commands: Mutex<VecDeque<Command>>,
    notices: Mutex<VecDeque<Notice>>,
    update: Mutex<UpdateStatus>,
    cmd_event: isize,
    main_hwnd: AtomicIsize,
}

impl Shared {
    pub fn new() -> windows::core::Result<Shared> {
        let ev = unsafe { CreateEventW(None, false, false, None)? };
        Ok(Shared {
            snapshot: Mutex::new(Snapshot::default()),
            commands: Mutex::new(VecDeque::new()),
            notices: Mutex::new(VecDeque::new()),
            update: Mutex::new(UpdateStatus::Idle),
            cmd_event: ev.0 as isize,
            main_hwnd: AtomicIsize::new(0),
        })
    }

    pub fn cmd_event(&self) -> HANDLE {
        HANDLE(self.cmd_event as *mut _)
    }

    pub fn set_main_hwnd(&self, hwnd: HWND) {
        self.main_hwnd.store(hwnd.0 as isize, Ordering::Release);
    }

    pub fn post(&self, msg: u32) {
        let h = self.main_hwnd.load(Ordering::Acquire);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), msg, WPARAM(0), LPARAM(0));
            }
        }
    }

    pub fn send(&self, c: Command) {
        if let Ok(mut q) = self.commands.lock() {
            q.push_back(c);
        }
        unsafe {
            let _ = SetEvent(self.cmd_event());
        }
    }

    pub fn take_commands(&self) -> Vec<Command> {
        self.commands
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }

    pub fn publish(&self, snap: Snapshot) {
        if let Ok(mut s) = self.snapshot.lock() {
            *s = snap;
        }
        self.post(WM_APP_SNAPSHOT);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn notify(&self, n: Notice) {
        if let Ok(mut q) = self.notices.lock() {
            q.push_back(n);
        }
        self.post(WM_APP_NOTICE);
    }

    pub fn set_update(&self, s: UpdateStatus) {
        if let Ok(mut u) = self.update.lock() {
            *u = s;
        }
        self.post(WM_APP_UPDATE);
    }

    pub fn update_status(&self) -> UpdateStatus {
        self.update.lock().map(|u| u.clone()).unwrap_or_default()
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        self.notices
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }
}
