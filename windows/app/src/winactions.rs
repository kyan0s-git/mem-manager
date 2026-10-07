//! Windows action executors and the platform action table
//! (docs/architecture/windows.md §3).

use crate::nt::{self, MemCommand};
use crate::util::wide;
use memmanager_core::actions::ActionSpec;
use memmanager_core::profile::Profile;
use memmanager_core::state::PressureState;
use std::collections::{HashMap, HashSet};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    MMDeviceEnumerator, eMultimedia, eRender,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers,
    GetDriveTypeW, GetLogicalDrives, OPEN_EXISTING,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Memory::{
    SETPROCESSWORKINGSETSIZEEX_FLAGS, SetProcessWorkingSetSizeEx, SetSystemFileCacheSize,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    GetCurrentProcess, MEMORY_PRIORITY_INFORMATION, OpenProcess,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
    PROCESS_POWER_THROTTLING_STATE, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    PROCESS_SET_QUOTA, ProcessMemoryPriority, ProcessPowerThrottling, SetProcessInformation,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use windows::core::PCWSTR;

pub mod ids {
    pub const DEPRIORITIZE: u8 = 1;
    pub const PURGE_LOW: u8 = 2;
    pub const PURGE_STANDBY: u8 = 3;
    pub const TRIM_IDLE: u8 = 4;
    pub const FLUSH_MODIFIED: u8 = 5;
    pub const COMBINE: u8 = 6;
    pub const GAME_PURGE: u8 = 7;
    pub const OPTIMIZE_NOW: u8 = 20;
    pub const DEEP_CLEAN: u8 = 21;
}

pub fn action_label(id: u8) -> &'static str {
    match id {
        ids::DEPRIORITIZE => "Quieted idle background apps",
        ids::PURGE_LOW => "Cleared low-priority cache",
        ids::PURGE_STANDBY => "Cleared standby cache",
        ids::TRIM_IDLE => "Trimmed idle apps",
        ids::FLUSH_MODIFIED => "Wrote out modified pages",
        ids::COMBINE => "Combined duplicate pages",
        ids::GAME_PURGE => "Game mode: cleared standby cache",
        ids::OPTIMIZE_NOW => "Optimize now",
        ids::DEEP_CLEAN => "Deep clean",
        _ => "Action",
    }
}

/// Tier (1..=5) of an automatic action, used for the per-tier toggles.
pub fn tier_of(id: u8) -> u8 {
    match id {
        ids::DEPRIORITIZE => 1,
        ids::PURGE_LOW => 2,
        ids::PURGE_STANDBY | ids::GAME_PURGE => 3,
        ids::TRIM_IDLE | ids::FLUSH_MODIFIED => 4,
        ids::COMBINE => 5,
        _ => 0,
    }
}

pub fn specs(profile: Profile) -> Vec<ActionSpec> {
    let (sb_cap, sb_refill) = profile.standby_bucket();
    let s = |id, tier, min_state, cooldown_s, cap, refill| ActionSpec {
        id,
        tier,
        min_state,
        cooldown_s,
        bucket_capacity: cap,
        refill_per_h: refill,
    };
    vec![
        s(
            ids::DEPRIORITIZE,
            1,
            PressureState::Elevated,
            60.0,
            1e9,
            1e9,
        ),
        s(ids::PURGE_LOW, 2, PressureState::Elevated, 30.0, 10.0, 60.0),
        s(ids::GAME_PURGE, 3, PressureState::Normal, 20.0, 30.0, 180.0),
        s(
            ids::PURGE_STANDBY,
            3,
            PressureState::High,
            120.0,
            sb_cap,
            sb_refill,
        ),
        s(ids::TRIM_IDLE, 4, PressureState::High, 120.0, 6.0, 12.0),
        s(ids::FLUSH_MODIFIED, 4, PressureState::High, 300.0, 2.0, 4.0),
        s(ids::COMBINE, 5, PressureState::Normal, 3600.0, 1.0, 1.0),
    ]
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub const MEMORY_PRIORITY_VERY_LOW: u32 = 1;
pub const MEMORY_PRIORITY_LOW: u32 = 2;
pub const MEMORY_PRIORITY_NORMAL: u32 = 5;

fn set_memory_priority_handle(h: HANDLE, prio: u32) -> bool {
    let info = MEMORY_PRIORITY_INFORMATION {
        MemoryPriority: windows::Win32::System::Threading::MEMORY_PRIORITY(prio),
    };
    unsafe {
        SetProcessInformation(
            h,
            ProcessMemoryPriority,
            (&info as *const MEMORY_PRIORITY_INFORMATION).cast(),
            size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
        .is_ok()
    }
}

pub fn set_memory_priority(pid: u32, prio: u32) -> bool {
    let Ok(h) = (unsafe {
        OpenProcess(
            PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
    }) else {
        return false;
    };
    let h = OwnedHandle(h);
    set_memory_priority_handle(h.0, prio)
}

/// Removes as many pages as possible from one process's working set.
pub fn trim(pid: u32) -> bool {
    let Ok(h) = (unsafe {
        OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
    }) else {
        return false;
    };
    let h = OwnedHandle(h);
    unsafe { EmptyWorkingSet(h.0).is_ok() }
}

/// Hard working-set cap (user action on a runaway). `None` removes the cap.
pub fn cap_working_set(pid: u32, max_bytes: Option<usize>) -> bool {
    const QUOTA_LIMITS_HARDWS_MIN_DISABLE: u32 = 0x2;
    const QUOTA_LIMITS_HARDWS_MAX_ENABLE: u32 = 0x4;
    const QUOTA_LIMITS_HARDWS_MAX_DISABLE: u32 = 0x8;
    let Ok(h) = (unsafe {
        OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
    }) else {
        return false;
    };
    let h = OwnedHandle(h);
    let (min, max, flags) = match max_bytes {
        Some(m) => (
            1 << 20,
            m.max(16 << 20),
            QUOTA_LIMITS_HARDWS_MIN_DISABLE | QUOTA_LIMITS_HARDWS_MAX_ENABLE,
        ),
        None => (
            usize::MAX,
            usize::MAX,
            QUOTA_LIMITS_HARDWS_MIN_DISABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE,
        ),
    };
    unsafe {
        SetProcessWorkingSetSizeEx(h.0, min, max, SETPROCESSWORKINGSETSIZEEX_FLAGS(flags)).is_ok()
    }
}

/// Our own process: EcoQoS, ignore timer resolution, low memory priority.
pub fn make_self_efficient() {
    unsafe {
        let me = GetCurrentProcess();
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: 1,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        };
        let _ = SetProcessInformation(
            me,
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        set_memory_priority_handle(me, MEMORY_PRIORITY_LOW);
    }
}

/// Trims our own working set (after the flyout closes).
pub fn trim_self() {
    unsafe {
        let _ = SetProcessWorkingSetSizeEx(
            GetCurrentProcess(),
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        );
    }
}

pub fn foreground_pid() -> u32 {
    let mut pid = 0u32;
    unsafe {
        let hwnd = GetForegroundWindow();
        if !hwnd.is_invalid() {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
    }
    pid
}

/// Full-screen D3D app, presentation mode, or a "busy" full-screen app.
pub fn game_mode_active() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_BUSY || s == QUNS_PRESENTATION_MODE,
        Err(_) => false,
    }
}

pub fn user_idle_ms() -> u64 {
    let mut li = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    unsafe {
        if GetLastInputInfo(&mut li).as_bool() {
            GetTickCount().wrapping_sub(li.dwTime) as u64
        } else {
            0
        }
    }
}

pub fn on_ac_power() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).is_ok() && s.ACLineStatus != 0 }
}

/// PIDs with an active audio session on the default render device.
/// Requires COM to be initialised on the calling thread.
pub fn audible_pids() -> HashSet<u32> {
    let mut out = HashSet::new();
    let _ = (|| -> windows::core::Result<()> {
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None)?;
            let sessions = mgr.GetSessionEnumerator()?;
            for i in 0..sessions.GetCount()? {
                let ctl = sessions.GetSession(i)?;
                if ctl.GetState()? != AudioSessionStateActive {
                    continue;
                }
                let ctl2: IAudioSessionControl2 = windows::core::Interface::cast(&ctl)?;
                if let Ok(pid) = ctl2.GetProcessId() {
                    out.insert(pid);
                }
            }
        }
        Ok(())
    })();
    out
}

/// Aggressive profile: hard cap for the system file cache. `None` removes it.
pub fn set_file_cache_cap(max_bytes: Option<usize>) -> bool {
    const FILE_CACHE_MAX_HARD_ENABLE: u32 = 0x1;
    const FILE_CACHE_MAX_HARD_DISABLE: u32 = 0x2;
    unsafe {
        match max_bytes {
            Some(m) => SetSystemFileCacheSize(16 << 20, m, FILE_CACHE_MAX_HARD_ENABLE).is_ok(),
            None => SetSystemFileCacheSize(0, 0, FILE_CACHE_MAX_HARD_DISABLE).is_ok(),
        }
    }
}

/// Flushes dirty cached data on every fixed volume. Returns how many succeeded.
pub fn flush_volumes() -> u32 {
    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    let mut n = 0;
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = wide(&format!("{letter}:\\"));
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        let path = wide(&format!("\\\\.\\{letter}:"));
        let h = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };
        if let Ok(h) = h {
            let h = OwnedHandle(h);
            if unsafe { FlushFileBuffers(h.0) }.is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Outcome of one executed action (for the ledger text and notifications).
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub ok: bool,
    pub detail: String,
    pub statuses: Vec<(&'static str, i32)>,
}

/// Processes whose memory priority we lowered, so we can restore them.
#[derive(Default)]
pub struct Lowered {
    map: HashMap<u32, i64>,
}

impl Lowered {
    pub const MAX: usize = 256;

    pub fn contains(&self, pid: u32) -> bool {
        self.map.contains_key(&pid)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn lower(&mut self, pid: u32, create_time: i64, prio: u32) -> bool {
        if self.map.len() >= Self::MAX && !self.map.contains_key(&pid) {
            return false;
        }
        if set_memory_priority(pid, prio) {
            self.map.insert(pid, create_time);
            true
        } else {
            false
        }
    }

    /// Restores one process (it became foreground) if we lowered it.
    pub fn restore(&mut self, pid: u32) {
        if self.map.remove(&pid).is_some() {
            set_memory_priority(pid, MEMORY_PRIORITY_NORMAL);
        }
    }

    /// Forgets processes that exited (or whose PID was reused).
    pub fn prune(&mut self, alive: impl Fn(u32, i64) -> bool) {
        self.map.retain(|&pid, &mut ct| alive(pid, ct));
    }

    pub fn restore_all(&mut self) {
        for (pid, _) in self.map.drain() {
            set_memory_priority(pid, MEMORY_PRIORITY_NORMAL);
        }
    }
}

pub fn run_mem_command(cmd: MemCommand, name: &'static str, out: &mut Outcome) -> bool {
    let st = nt::mem_command(cmd);
    out.statuses.push((name, st));
    nt::ok(st)
}
