//! UI thread: hidden main window (tray callbacks, broadcasts), the flyout
//! window, the message loop, and the glue to the worker thread.

use crate::config::{ColorPreset, Config};
use crate::palette::{Appearance, state_color};
use crate::privilege::Privileges;
use crate::shared::{
    Command, Notice, ProcActionKind, Shared, WM_APP_EXIT_FOR_UPDATE, WM_APP_NOTICE,
    WM_APP_SNAPSHOT, WM_APP_UPDATE,
};
use crate::task;
use crate::tray::{self, Tray, WM_APP_TRAY};
use crate::ui::popup::{Popup, UiAction};
use crate::update::{self, Status as UpdateStatus};
use crate::updater;
use crate::util::{WStr, fmt_bytes, fmt_hours, now_ms, wide};
use crate::winactions;
use memmanager_core::profile::Profile;
use memmanager_core::state::PressureState;
use std::cell::RefCell;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;
use windows::Win32::Foundation::{
    COLORREF, CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM,
    LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GetProcessHeap, HEAP_FLAGS, HeapCompact};
use windows::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_CONSOLE_DISPLAY_STATE};
use windows::Win32::System::Threading::{CreateMutexW, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::Controls::Dialogs::{
    CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    NIN_BALLOONUSERCLICK, NIN_SELECT, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const WM_APP_SHOW: u32 = WM_APP + 4;
const WM_MOUSELEAVE: u32 = 0x02A3;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const MAIN_CLASS: PCWSTR = w!("MemManager.Main");
const FLYOUT_CLASS: PCWSTR = w!("MemManager.Flyout");
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const PAUSE_MS: u64 = 3_600_000;

mod menu_id {
    pub const OPTIMIZE: usize = 1;
    pub const DEEP_CLEAN: usize = 2;
    pub const PAUSE: usize = 3;
    pub const SETTINGS: usize = 4;
    pub const EXIT: usize = 5;
    pub const OPEN: usize = 6;
    pub const PROFILE0: usize = 10;
    pub const PROC_LOWER: usize = 30;
    pub const PROC_TRIM: usize = 31;
    pub const PROC_CAP: usize = 32;
    pub const PROC_UNCAP: usize = 33;
    pub const PROC_EXCLUDE: usize = 34;
    pub const PROC_IGNORE: usize = 35;
    pub const PROC_END: usize = 36;
}

struct App {
    shared: Arc<Shared>,
    cfg: Config,
    cfg_mtime: Option<SystemTime>,
    tray: Tray,
    popup: Popup,
    ap: Appearance,
    main: HWND,
    taskbar_created: u32,
    worker: Option<std::thread::JoinHandle<()>>,
    started_ms: u64,
    last_update_check: u64,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

/// Runs `f` with the app state unless it is already borrowed (re-entrancy
/// from a modal loop); returns `None` in that case.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

fn get_x(v: usize) -> i32 {
    (v & 0xFFFF) as i16 as i32
}

fn get_y(v: usize) -> i32 {
    ((v >> 16) & 0xFFFF) as i16 as i32
}

fn config_mtime() -> Option<SystemTime> {
    Config::path()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
}

pub fn run_key_enabled() -> bool {
    let sub = WStr::new(RUN_KEY);
    let name = WStr::new("MemManager");
    let mut len = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            sub.pcwstr(),
            name.pcwstr(),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut len),
        )
        .is_ok()
    }
}

fn set_run_key(on: bool) {
    let sub = WStr::new(RUN_KEY);
    let name = WStr::new("MemManager");
    unsafe {
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let v = wide(&format!("\"{}\"", exe.display()));
                let _ = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    sub.pcwstr(),
                    name.pcwstr(),
                    REG_SZ.0,
                    Some(v.as_ptr().cast()),
                    (v.len() * 2) as u32,
                );
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, sub.pcwstr(), name.pcwstr());
        }
    }
}

/// Runs this executable elevated with `params`. Returns the process handle.
fn run_elevated(params: &str) -> Option<HANDLE> {
    let exe = std::env::current_exe().ok()?;
    let file = WStr::new(&exe.to_string_lossy());
    let verb = WStr::new("runas");
    let p = WStr::new(params);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.pcwstr(),
        lpFile: file.pcwstr(),
        lpParameters: p.pcwstr(),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei).ok().map(|_| sei.hProcess) }
}

impl App {
    fn on_snapshot(&mut self) {
        self.maybe_auto_check();
        let snap = self.shared.snapshot();
        self.tray.update(&snap, &self.cfg, &self.ap);
        self.popup.snap = snap;
        if self.popup.visible {
            self.popup.relayout();
        }
    }

    fn on_update_status(&mut self) {
        self.popup.update = self.shared.update_status();
        self.popup.invalidate();
        if self.popup.visible {
            self.popup.relayout();
        }
    }

    /// Daily automatic check (after a short delay at start).
    fn maybe_auto_check(&mut self) {
        if !self.cfg.auto_update {
            return;
        }
        let now = now_ms();
        let due = if self.last_update_check == 0 {
            now.saturating_sub(self.started_ms) >= update::FIRST_CHECK_DELAY_MS
        } else {
            now.saturating_sub(self.last_update_check) >= update::CHECK_EVERY_MS
        };
        if due {
            self.start_update_check(false);
        }
    }

    fn start_update_check(&mut self, manual: bool) {
        if self.popup.update.busy() {
            return;
        }
        self.last_update_check = now_ms();
        let had_offer = matches!(self.popup.update, UpdateStatus::Available(_));
        self.shared.set_update(UpdateStatus::Checking);
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new()
            .name("memmanager-update".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let st = match updater::check(&updater::current_version()) {
                    Ok(Some(o)) => {
                        if !manual && !had_offer {
                            shared.notify(Notice::Info {
                                title: format!("MemManager {} is available", o.version),
                                text: "Open MemManager to install it. It takes a few seconds and keeps your settings.".into(),
                            });
                        }
                        UpdateStatus::Available(o)
                    }
                    Ok(None) => UpdateStatus::UpToDate { at_ms: now_ms() },
                    Err(e) => UpdateStatus::Failed(e),
                };
                shared.set_update(st);
            });
    }

    fn start_update_install(&mut self) {
        let UpdateStatus::Available(offer) = self.popup.update.clone() else {
            return;
        };
        self.shared
            .set_update(UpdateStatus::Installing(offer.version.clone()));
        let shared = self.shared.clone();
        let _ = std::thread::Builder::new()
            .name("memmanager-update".into())
            .stack_size(256 * 1024)
            .spawn(move || match updater::install(&offer) {
                Ok(()) => shared.post(WM_APP_EXIT_FOR_UPDATE),
                Err(e) => shared.set_update(UpdateStatus::Failed(e)),
            });
    }

    fn on_notices(&mut self) {
        for n in self.shared.take_notices() {
            match n {
                Notice::Leak {
                    name,
                    slope_mib_h,
                    eta_h,
                    runaway,
                } => {
                    let (title, text) = if runaway {
                        (
                            format!("{name} is allocating memory very fast"),
                            "Its memory use is climbing by the second. Open MemManager to limit or restart it.".to_string(),
                        )
                    } else {
                        let eta = eta_h.map_or(String::new(), |h| {
                            format!(" At this rate memory runs out in about {}.", fmt_hours(h))
                        });
                        (
                            format!("{name} looks like it's leaking memory"),
                            format!(
                                "It has grown steadily by about {slope_mib_h:.0} MB per hour.{eta} Restarting it frees the memory."
                            ),
                        )
                    };
                    self.tray.balloon(&title, &text, true);
                }
                Notice::Info { title, text } => self.tray.balloon(&title, &text, false),
                Notice::Warning { title, text } => self.tray.balloon(&title, &text, true),
            }
        }
    }

    fn refresh_appearance(&mut self) {
        self.ap = tray::appearance();
        self.popup.set_appearance(self.ap);
        self.tray.invalidate();
        let snap = self.shared.snapshot();
        self.tray.update(&snap, &self.cfg, &self.ap);
    }

    fn apply_config(&mut self, cfg: Config) {
        if cfg == self.cfg {
            return;
        }
        self.cfg = cfg;
        let _ = self.cfg.save();
        self.cfg_mtime = config_mtime();
        self.shared
            .send(Command::SetConfig(Box::new(self.cfg.clone())));
        self.popup.set_config(self.cfg.clone());
        self.tray.invalidate();
        let snap = self.shared.snapshot();
        self.tray.update(&snap, &self.cfg, &self.ap);
    }

    fn reload_config_if_changed(&mut self) {
        let m = config_mtime();
        if m.is_some() && m != self.cfg_mtime {
            self.cfg_mtime = m;
            let cfg = Config::load();
            if cfg != self.cfg {
                self.cfg = cfg.clone();
                self.shared.send(Command::SetConfig(Box::new(cfg.clone())));
                self.popup.set_config(cfg);
                self.tray.invalidate();
            }
        }
    }

    fn show_popup(&mut self) {
        self.reload_config_if_changed();
        self.popup.run_key = run_key_enabled();
        self.popup.snap = self.shared.snapshot();
        let anchor = self.tray.anchor();
        self.popup.show(anchor);
        self.shared.send(Command::PopupVisible(true));
    }

    fn hide_popup(&mut self) {
        if self.popup.visible {
            self.popup.hide();
            self.shared.send(Command::PopupVisible(false));
            unsafe {
                if let Ok(h) = GetProcessHeap() {
                    HeapCompact(h, HEAP_FLAGS(0));
                }
            }
            winactions::trim_self();
        }
    }

    fn toggle_pause(&mut self) {
        let paused = self.shared.snapshot().paused;
        self.shared
            .send(Command::Pause(if paused { None } else { Some(PAUSE_MS) }));
    }
}

/// Things that must run without the app borrowed (they pump messages).
enum Deferred {
    None,
    TrayMenu(POINT),
    ProcMenu(u32),
    PickColor(u8),
    DeepClean,
    EnableCleaning,
    RemoveTask,
    EditExclusions,
    ConfirmOptimize(String),
    InstallUpdate,
}

/// Opens a web page as the signed-in user, never elevated: Explorer hands the
/// URL to the default browser in the user's normal session.
fn open_url(url: &str) {
    let _ = std::process::Command::new("explorer.exe").arg(url).spawn();
}

fn handle_ui_action(a: UiAction) -> Deferred {
    with_app(|app| match a {
        UiAction::Close => {
            app.hide_popup();
            Deferred::None
        }
        UiAction::Resize => {
            app.popup.relayout();
            Deferred::None
        }
        UiAction::OptimizeNow => {
            app.shared.send(Command::OptimizeNow);
            Deferred::None
        }
        UiAction::TogglePause => {
            app.toggle_pause();
            Deferred::None
        }
        UiAction::SetConfig(c) => {
            app.apply_config(*c);
            Deferred::None
        }
        UiAction::SetRunKey(on) => {
            set_run_key(on);
            app.popup.run_key = run_key_enabled();
            app.popup.invalidate();
            Deferred::None
        }
        UiAction::ProcMenu(pid) => Deferred::ProcMenu(pid),
        UiAction::PickColor(i) => Deferred::PickColor(i),
        UiAction::DeepClean => Deferred::DeepClean,
        UiAction::EnableCleaning => Deferred::EnableCleaning,
        UiAction::RemoveTask => Deferred::RemoveTask,
        UiAction::EditExclusions => Deferred::EditExclusions,
        UiAction::ConfirmOptimize(text) => Deferred::ConfirmOptimize(text),
        UiAction::OpenExplainer => {
            let mut c = app.cfg.clone();
            c.explained = true;
            app.apply_config(c);
            open_url(crate::values::EXPLAINER_URL);
            app.hide_popup();
            Deferred::None
        }
        UiAction::CheckUpdate => {
            app.start_update_check(true);
            Deferred::None
        }
        UiAction::InstallUpdate => Deferred::InstallUpdate,
    })
    .unwrap_or(Deferred::None)
}

fn set_modal(on: bool) {
    with_app(|a| a.popup.modal = on);
}

fn run_deferred(d: Deferred) {
    match d {
        Deferred::None => {}
        Deferred::TrayMenu(pt) => tray_menu(pt),
        Deferred::ProcMenu(pid) => proc_menu(pid),
        Deferred::PickColor(i) => pick_color(i),
        Deferred::DeepClean => deep_clean(),
        Deferred::EnableCleaning => enable_cleaning(),
        Deferred::RemoveTask => remove_task(),
        Deferred::EditExclusions => edit_exclusions(),
        Deferred::ConfirmOptimize(text) => {
            if message_box(
                popup_hwnd(),
                &text,
                "Optimize now",
                MB_OKCANCEL | MB_ICONINFORMATION,
            ) == IDOK
            {
                with_app(|a| a.shared.send(Command::OptimizeNow));
            }
        }
        Deferred::InstallUpdate => install_update(),
    }
}

fn install_update() {
    let Some(UpdateStatus::Available(o)) = with_app(|a| a.popup.update.clone()) else {
        return;
    };
    let how = match updater::kind() {
        updater::Kind::Installed => {
            "Windows may ask for permission to run the installer. MemManager closes, updates and starts again with your settings."
        }
        updater::Kind::Portable => {
            "MemManager replaces its program file in this folder and starts again with your settings."
        }
    };
    let text = format!(
        "Install MemManager {} now?\n\n{how}\n\nThe download is checked against its published SHA-256 first.",
        o.version
    );
    if message_box(
        popup_hwnd(),
        &text,
        "Update MemManager",
        MB_OKCANCEL | MB_ICONINFORMATION,
    ) == IDOK
    {
        with_app(|a| a.start_update_install());
    }
}

fn message_box(owner: HWND, text: &str, title: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let t = WStr::new(text);
    let c = WStr::new(title);
    set_modal(true);
    let r = unsafe {
        MessageBoxW(
            Some(owner),
            t.pcwstr(),
            c.pcwstr(),
            style | MB_SETFOREGROUND,
        )
    };
    set_modal(false);
    r
}

fn popup_hwnd() -> HWND {
    with_app(|a| a.popup.hwnd).unwrap_or_default()
}

fn deep_clean() {
    let owner = popup_hwnd();
    let r = message_box(
        owner,
        "Deep clean empties every app's working set and all memory caches at once.\n\n\
         It frees the most RAM right now, but apps will be slow for a few seconds while they read their \
         data back in, and changed data they held is written to the page file (virtual memory) first. \
         MemManager's automatic optimization is usually the better choice.\n\nRun deep clean now?",
        "Deep clean",
        MB_OKCANCEL | MB_ICONWARNING,
    );
    if r == IDOK {
        with_app(|a| a.shared.send(Command::DeepClean));
    }
}

fn enable_cleaning() {
    let owner = popup_hwnd();
    set_modal(true);
    let h = run_elevated("--register-task --launch");
    set_modal(false);
    if let Some(h) = h {
        unsafe {
            let _ = CloseHandle(h);
        }
        // The elevated helper relaunches MemManager once this instance exits.
        with_app(|a| a.hide_popup());
        unsafe { PostQuitMessage(0) };
    } else {
        let _ = owner;
    }
}

fn remove_task() {
    set_modal(true);
    let h = run_elevated("--unregister-task");
    if let Some(h) = h {
        unsafe {
            let _ = WaitForSingleObject(h, 20_000);
            let _ = CloseHandle(h);
        }
    }
    set_modal(false);
    let registered = task::is_registered();
    with_app(|a| a.shared.send(Command::TaskRegistered(registered)));
}

fn edit_exclusions() {
    let Some(path) = Config::path() else { return };
    with_app(|a| {
        if !path.exists() {
            let _ = a.cfg.save();
        }
        a.hide_popup();
    });
    let _ = std::process::Command::new("notepad.exe").arg(path).spawn();
}

fn pick_color(i: u8) {
    let Some((owner, mut cfg, ap)) = with_app(|a| (a.popup.hwnd, a.cfg.clone(), a.ap)) else {
        return;
    };
    if cfg.preset != ColorPreset::Custom {
        let tb = Appearance { ..ap };
        for (k, st) in PressureState::ALL.iter().enumerate() {
            cfg.custom_colors[k] = state_color(cfg.preset, &cfg.custom_colors, *st, &tb).to_hex();
        }
    }
    let cur = cfg.custom_colors[i as usize];
    let bgr = |rgb: u32| ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF);
    let mut custom = [COLORREF(0x00FF_FFFF); 16];
    for (k, c) in cfg.custom_colors.iter().enumerate() {
        custom[k] = COLORREF(bgr(*c));
    }
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: owner,
        rgbResult: COLORREF(bgr(cur)),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN | CC_ANYCOLOR,
        ..Default::default()
    };
    set_modal(true);
    let ok = unsafe { ChooseColorW(&mut cc).as_bool() };
    set_modal(false);
    if ok {
        cfg.custom_colors[i as usize] = bgr(cc.rgbResult.0);
        cfg.preset = ColorPreset::Custom;
        with_app(|a| {
            a.apply_config(cfg);
            if a.popup.visible {
                unsafe {
                    let _ = SetForegroundWindow(a.popup.hwnd);
                }
            }
        });
    }
}

fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str) {
    let t = WStr::new(text);
    unsafe {
        let _ = AppendMenuW(menu, flags, id, t.pcwstr());
    }
}

fn track(menu: HMENU, owner: HWND, pt: POINT) -> usize {
    set_modal(true);
    let id = unsafe {
        let _ = SetForegroundWindow(owner);
        let r = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            owner,
            None,
        );
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        r.0 as usize
    };
    set_modal(false);
    id
}

fn tray_menu(pt: POINT) {
    let Some((main, snap, cfg)) = with_app(|a| (a.main, a.shared.snapshot(), a.cfg.clone())) else {
        return;
    };
    let menu = unsafe { CreatePopupMenu() };
    let Ok(menu) = menu else { return };
    let can_clean = snap.privileges.profile;
    append(menu, MF_STRING, menu_id::OPEN, "Open MemManager");
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING, menu_id::OPTIMIZE, "Optimize now");
    append(
        menu,
        if can_clean {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        },
        menu_id::DEEP_CLEAN,
        "Deep clean…",
    );
    append(
        menu,
        MF_STRING,
        menu_id::PAUSE,
        if snap.paused {
            "Resume automatic optimization"
        } else {
            "Pause for 1 hour"
        },
    );
    if let Ok(sub) = unsafe { CreatePopupMenu() } {
        for (i, p) in Profile::ALL.iter().enumerate() {
            let checked = if *p == cfg.profile {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
            append(sub, MF_STRING | checked, menu_id::PROFILE0 + i, p.label());
        }
        let t = WStr::new("Profile");
        unsafe {
            let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, t.pcwstr());
        }
    }
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING, menu_id::SETTINGS, "Settings");
    append(menu, MF_STRING, menu_id::EXIT, "Exit");
    let id = track(menu, main, pt);
    match id {
        menu_id::OPEN => {
            with_app(|a| a.show_popup());
        }
        menu_id::OPTIMIZE => {
            with_app(|a| a.shared.send(Command::OptimizeNow));
        }
        menu_id::DEEP_CLEAN => deep_clean(),
        menu_id::PAUSE => {
            with_app(|a| a.toggle_pause());
        }
        menu_id::SETTINGS => {
            with_app(|a| {
                a.show_popup();
                a.popup.view = crate::ui::popup::View::Settings;
                a.popup.relayout();
            });
        }
        menu_id::EXIT => unsafe {
            PostQuitMessage(0);
        },
        i if (menu_id::PROFILE0..menu_id::PROFILE0 + 3).contains(&i) => {
            let mut c = cfg;
            c.profile = Profile::ALL[i - menu_id::PROFILE0];
            with_app(|a| a.apply_config(c));
        }
        _ => {}
    }
}

fn proc_menu(pid: u32) {
    let Some((owner, snap, cfg)) = with_app(|a| (a.popup.hwnd, a.shared.snapshot(), a.cfg.clone()))
    else {
        return;
    };
    let Some(row) = snap.top.iter().find(|r| r.pid == pid).cloned() else {
        return;
    };
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return;
    };
    let ok = if row.critical {
        MF_STRING | MF_GRAYED
    } else {
        MF_STRING
    };
    append(
        menu,
        MF_STRING | MF_GRAYED,
        0,
        &format!("{} · {}", row.name, fmt_bytes(row.private)),
    );
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, ok, menu_id::PROC_LOWER, "Lower memory priority");
    append(menu, ok, menu_id::PROC_TRIM, "Trim its memory now");
    append(
        menu,
        ok,
        menu_id::PROC_CAP,
        &format!(
            "Limit RAM to {} (slows it, doesn't stop leaks)",
            fmt_bytes(row.working_set)
        ),
    );
    append(menu, ok, menu_id::PROC_UNCAP, "Remove RAM limit");
    append(menu, MF_SEPARATOR, 0, "");
    let excluded = cfg.is_excluded(&row.key);
    append(
        menu,
        MF_STRING | if excluded { MF_CHECKED } else { MF_UNCHECKED },
        menu_id::PROC_EXCLUDE,
        "Never optimize this app",
    );
    let ignored = cfg.ignores_leaks(&row.key);
    append(
        menu,
        MF_STRING | if ignored { MF_CHECKED } else { MF_UNCHECKED },
        menu_id::PROC_IGNORE,
        "Don't alert about leaks in this app",
    );
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, ok, menu_id::PROC_END, "End process…");
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    let id = track(menu, owner, pt);
    let send = |k: ProcActionKind| {
        with_app(|a| a.shared.send(Command::Proc { pid, kind: k }));
    };
    match id {
        menu_id::PROC_LOWER => send(ProcActionKind::LowerPriority),
        menu_id::PROC_TRIM => send(ProcActionKind::Trim),
        menu_id::PROC_CAP => send(ProcActionKind::CapToCurrent),
        menu_id::PROC_UNCAP => send(ProcActionKind::Uncap),
        menu_id::PROC_EXCLUDE | menu_id::PROC_IGNORE => {
            let mut c = cfg;
            let list = if id == menu_id::PROC_EXCLUDE {
                &mut c.exclusions
            } else {
                &mut c.ignore_leaks
            };
            if let Some(i) = list.iter().position(|k| *k == row.key) {
                list.remove(i);
            } else {
                list.push(row.key.clone());
            }
            with_app(|a| a.apply_config(c));
        }
        menu_id::PROC_END => {
            let r = message_box(
                owner,
                &format!("End {}? Unsaved work in it will be lost.", row.name),
                "End process",
                MB_OKCANCEL | MB_ICONWARNING,
            );
            if r == IDOK {
                send(ProcActionKind::End);
            }
        }
        _ => {}
    }
    with_app(|a| unsafe {
        let _ = SetForegroundWindow(a.popup.hwnd);
    });
}

extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let tb_created = with_app(|a| a.taskbar_created).unwrap_or(0);
    match msg {
        WM_APP_TRAY => {
            let ev = loword(lp.0 as usize);
            match ev {
                NIN_SELECT | NIN_KEYSELECT => {
                    with_app(|a| {
                        if a.popup.visible {
                            a.hide_popup();
                        } else if now_ms().saturating_sub(a.popup.hidden_at) > 300 {
                            a.show_popup();
                        }
                    });
                }
                NIN_BALLOONUSERCLICK => {
                    with_app(|a| a.show_popup());
                }
                WM_CONTEXTMENU => {
                    let pt = POINT {
                        x: get_x(wp.0),
                        y: get_y(wp.0),
                    };
                    with_app(|a| a.hide_popup());
                    run_deferred(Deferred::TrayMenu(pt));
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_APP_SNAPSHOT => {
            with_app(|a| a.on_snapshot());
            LRESULT(0)
        }
        WM_APP_NOTICE => {
            with_app(|a| a.on_notices());
            LRESULT(0)
        }
        WM_APP_UPDATE => {
            with_app(|a| a.on_update_status());
            LRESULT(0)
        }
        WM_APP_EXIT_FOR_UPDATE => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_APP_SHOW => {
            with_app(|a| a.show_popup());
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE | WM_DPICHANGED
        | WM_DISPLAYCHANGE => {
            with_app(|a| a.refresh_appearance());
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }
        WM_POWERBROADCAST => {
            const PBT_POWERSETTINGCHANGE: usize = 0x8013;
            if wp.0 == PBT_POWERSETTINGCHANGE && lp.0 != 0 {
                let s = unsafe { &*(lp.0 as *const POWERBROADCAST_SETTING) };
                let data = s.Data[0] as u32;
                if s.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
                    with_app(|a| a.shared.send(Command::Display(data != 0)));
                } else if s.PowerSetting == GUID_ACDC_POWER_SOURCE {
                    with_app(|a| a.shared.send(Command::Battery(data != 0)));
                }
            }
            LRESULT(1)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        m if m != 0 && m == tb_created => {
            with_app(|a| {
                a.tray.readd();
                a.tray.invalidate();
                let snap = a.shared.snapshot();
                a.tray.update(&snap, &a.cfg, &a.ap);
            });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

extern "system" fn flyout_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe {
                BeginPaint(hwnd, &mut ps);
            }
            with_app(|a| a.popup.paint());
            unsafe {
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_ACTIVATE => {
            if loword(wp.0) == WA_INACTIVE {
                with_app(|a| {
                    if !a.popup.modal {
                        a.hide_popup();
                    }
                });
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = (get_x(lp.0 as usize), get_y(lp.0 as usize));
            with_app(|a| {
                if !a.popup.tracking_mouse {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    a.popup.tracking_mouse = true;
                }
                a.popup.on_mouse_move(x, y);
            });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|a| a.popup.on_mouse_leave());
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (get_x(lp.0 as usize), get_y(lp.0 as usize));
            with_app(|a| a.popup.on_button_down(x, y));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = (get_x(lp.0 as usize), get_y(lp.0 as usize));
            if let Some(action) = with_app(|a| a.popup.on_button_up(x, y)).flatten() {
                run_deferred(handle_ui_action(action));
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xFFFF) as i16;
            with_app(|a| a.popup.on_wheel(delta));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            if let Some(action) = with_app(|a| a.popup.on_key(wp.0 as u16, shift)).flatten() {
                run_deferred(handle_ui_action(action));
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = loword(wp.0);
            with_app(|a| a.popup.set_dpi(dpi));
            LRESULT(0)
        }
        WM_CLOSE => {
            with_app(|a| a.hide_popup());
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if let Some(s) = SHARED.get() {
        s.send(Command::ForegroundChanged);
    }
}

fn register_class(hinst: HINSTANCE, name: PCWSTR, proc_: WNDPROC, shadow: bool) {
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: if shadow {
            CS_DROPSHADOW
        } else {
            WNDCLASS_STYLES(0)
        },
        lpfnWndProc: proc_,
        hInstance: hinst,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
        // The app icon embedded by build.rs (group id 1), shared with dialogs and Explorer.
        hIcon: unsafe { LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default() },
        lpszClassName: name,
        ..Default::default()
    };
    unsafe {
        RegisterClassExW(&wc);
    }
}

/// The tray application. Returns the process exit code.
pub fn run(privileges: Privileges) -> i32 {
    unsafe {
        // Single instance: a second launch just opens the flyout of the first.
        let mutex = CreateMutexW(None, true, w!("Local\\MemManager.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(h) = FindWindowW(MAIN_CLASS, None) {
                let _ = PostMessageW(Some(h), WM_APP_SHOW, WPARAM(0), LPARAM(0));
            }
            return 0;
        }

        winactions::make_self_efficient();
        let cfg = Config::load();
        let shared = match Shared::new() {
            Ok(s) => Arc::new(s),
            Err(_) => return 1,
        };
        let _ = SHARED.set(shared.clone());

        let hinst: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        register_class(hinst, MAIN_CLASS, Some(main_proc), false);
        register_class(hinst, FLYOUT_CLASS, Some(flyout_proc), true);
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            MAIN_CLASS,
            w!("MemManager"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst),
            None,
        )
        .unwrap_or_default();
        let flyout = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            FLYOUT_CLASS,
            w!("MemManager"),
            WS_POPUP,
            0,
            0,
            360,
            500,
            None,
            None,
            Some(hinst),
            None,
        )
        .unwrap_or_default();
        if main.is_invalid() || flyout.is_invalid() {
            return 1;
        }
        shared.set_main_hwnd(main);

        // Allow messages from lower-integrity processes (Explorer, a second
        // non-elevated instance) to reach the elevated tray window.
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
        for m in [
            taskbar_created,
            WM_APP_SHOW,
            WM_APP_TRAY,
            WM_COMMAND,
            WM_APP_EXIT_FOR_UPDATE,
        ] {
            let _ = ChangeWindowMessageFilterEx(main, m, MSGFLT_ALLOW, None);
        }
        let _ = RegisterPowerSettingNotification(
            HANDLE(main.0),
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let _ = RegisterPowerSettingNotification(
            HANDLE(main.0),
            &GUID_ACDC_POWER_SOURCE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        let ap = tray::appearance();
        let mut tr = Tray::new(main);
        tr.update(&shared.snapshot(), &cfg, &ap);
        let popup = Popup::new(flyout, cfg.clone(), ap);

        let worker = {
            let s = shared.clone();
            let c = cfg.clone();
            std::thread::Builder::new()
                .name("memmanager-worker".into())
                .stack_size(256 * 1024)
                .spawn(move || crate::worker::run(s, c, privileges))
                .ok()
        };
        {
            let s = shared.clone();
            let _ = std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || s.send(Command::TaskRegistered(task::is_registered())));
        }

        APP.with(|cell| {
            *cell.borrow_mut() = Some(App {
                shared: shared.clone(),
                cfg,
                cfg_mtime: config_mtime(),
                tray: tr,
                popup,
                ap,
                main,
                taskbar_created,
                worker,
                started_ms: now_ms(),
                last_update_check: 0,
            });
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = windows::Win32::UI::Accessibility::UnhookWinEvent(hook);
        let app = APP.with(|cell| cell.borrow_mut().take());
        if let Some(mut a) = app {
            a.hide_popup();
            a.tray.remove();
            a.shared.send(Command::Quit);
            if let Some(w) = a.worker.take() {
                let _ = w.join();
            }
        }
        if let Ok(m) = mutex {
            let _ = CloseHandle(m);
        }
        let _ = INFINITE;
        0
    }
}

/// `--register-task [--launch]`: runs elevated (from the "Enable cleaning" button).
pub fn register_task(launch: bool) -> i32 {
    let code = task::register();
    if code == 0 && launch {
        // Wait for the non-elevated instance to exit, then start the elevated one.
        for _ in 0..150 {
            let exists = unsafe {
                windows::Win32::System::Threading::OpenMutexW(
                    windows::Win32::System::Threading::SYNCHRONIZATION_SYNCHRONIZE,
                    false,
                    w!("Local\\MemManager.SingleInstance"),
                )
            };
            match exists {
                Ok(h) => unsafe {
                    let _ = CloseHandle(h);
                },
                Err(_) => break,
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }
    code
}
