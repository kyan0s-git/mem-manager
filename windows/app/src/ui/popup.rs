//! The flyout: dashboard and settings views drawn with Direct2D
//! (docs/architecture/ui-design.md §2–3).

use super::gfx::{Align, Font, Gfx, Rect};
use crate::config::{ColorPreset, Config, IconMetric, IconStyle};
use crate::palette::{Appearance, Palette, Rgba, state_color};
use crate::shared::Snapshot;
use crate::update::Status as UpdateStatus;
use crate::util::{fmt_ago, fmt_bytes, now_ms};
use crate::values;
use memmanager_core::profile::Profile;
use memmanager_core::state::PressureState;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos,
    ShowWindow,
};

pub const W: f32 = 360.0;
const PAD: f32 = 16.0;
const SETTINGS_MAX_H: f32 = 620.0;
const HEADER_H: f32 = 56.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Settings,
    Pause,
    Optimize,
    Profile(u8),
    Proc(u32),
    EnableCleaning,
    Back,
    Style(u8),
    Metric(u8),
    Preset(u8),
    Swatch(u8),
    Tier(u8),
    GameMode,
    Notify(u8),
    Translucent,
    TaskButton,
    RunKey,
    DeepClean,
    EditExclusions,
    Values(u8),
    AutoUpdate,
    CheckUpdate,
    InstallUpdate,
    Explainer,
    Activity,
}

#[derive(Debug)]
pub enum UiAction {
    Close,
    OptimizeNow,
    TogglePause,
    SetConfig(Box<Config>),
    ProcMenu(u32),
    EnableCleaning,
    RemoveTask,
    PickColor(u8),
    DeepClean,
    EditExclusions,
    SetRunKey(bool),
    Resize,
    /// "Optimize now" while memory is comfortable: confirm with this explanation first.
    ConfirmOptimize(String),
    OpenExplainer,
    CheckUpdate,
    InstallUpdate,
}

/// Optional dashboard rows, shared by layout and paint.
#[derive(Default)]
struct Extras {
    update: Option<String>,
    /// The one-time explainer row and the cache caption (friendly values only).
    explainer: bool,
}

const LINE2_H: f32 = 34.0;
const ROW_H: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Dashboard,
    Settings,
    Activity,
}

pub struct Popup {
    pub hwnd: HWND,
    gfx: Option<Gfx>,
    pub view: View,
    hits: Vec<(Rect, Hit)>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    focus: Option<usize>,
    scroll: f32,
    content_h: f32,
    pub visible: bool,
    pub hidden_at: u64,
    pub dpi: u32,
    pub snap: Snapshot,
    pub cfg: Config,
    pub ap: Appearance,
    pal: Palette,
    pub run_key: bool,
    pub tracking_mouse: bool,
    /// A dialog or menu owned by the flyout is open: don't hide on deactivation.
    pub modal: bool,
    backdrop: bool,
    pub update: UpdateStatus,
}

/// Windows 11 22H2 (build 22621) or later supports system backdrops.
pub fn supports_backdrop() -> bool {
    crate::nt::os_build() >= 22621
}

fn sz_dark(hwnd: HWND, dark: bool) {
    let v: i32 = dark as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&v as *const i32).cast(),
            4,
        );
    }
}

impl Popup {
    pub fn new(hwnd: HWND, cfg: Config, ap: Appearance) -> Popup {
        let mut p = Popup {
            hwnd,
            gfx: None,
            view: View::Dashboard,
            hits: Vec::new(),
            hover: None,
            pressed: None,
            focus: None,
            scroll: 0.0,
            content_h: 0.0,
            visible: false,
            hidden_at: 0,
            dpi: 96,
            snap: Snapshot::default(),
            pal: Palette::new(&ap),
            cfg,
            ap,
            run_key: false,
            tracking_mouse: false,
            modal: false,
            backdrop: false,
            update: UpdateStatus::Idle,
        };
        p.apply_window_style();
        p
    }

    /// Rounded corners, dark mode and (if enabled) the acrylic backdrop.
    pub fn apply_window_style(&mut self) {
        unsafe {
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const core::ffi::c_void,
                4,
            );
            sz_dark(self.hwnd, !self.ap.apps_light);
            self.backdrop = self.cfg.translucent && supports_backdrop() && !self.ap.high_contrast;
            let kind = if self.backdrop {
                DWMSBT_TRANSIENTWINDOW
            } else {
                DWMSBT_NONE
            };
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &kind as *const _ as *const core::ffi::c_void,
                4,
            );
            let m = if self.backdrop {
                MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                }
            } else {
                MARGINS::default()
            };
            let _ = DwmExtendFrameIntoClientArea(self.hwnd, &m);
        }
    }

    pub fn set_appearance(&mut self, ap: Appearance) {
        self.ap = ap;
        self.pal = Palette::new(&ap);
        self.apply_window_style();
        self.invalidate();
    }

    pub fn set_config(&mut self, cfg: Config) {
        let restyle = cfg.translucent != self.cfg.translucent;
        self.cfg = cfg;
        if restyle {
            self.apply_window_style();
        }
        self.invalidate();
    }

    pub fn invalidate(&self) {
        if self.visible {
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
    }

    fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    fn height(&self) -> f32 {
        match self.view {
            View::Dashboard => self.dashboard_height(),
            View::Settings | View::Activity => SETTINGS_MAX_H,
        }
    }

    fn scrollable(&self) -> bool {
        matches!(self.view, View::Settings | View::Activity)
    }

    fn extras(&self) -> Extras {
        let snap = &self.snap;
        let mut e = Extras::default();
        if let UpdateStatus::Available(o) = &self.update {
            e.update = Some(format!("MemManager {} is available", o.version));
        }
        e.explainer = self.cfg.friendly && !self.cfg.explained && snap.reading.total > 0;
        e
    }

    fn dashboard_height(&self) -> f32 {
        let rows = self.snap.top.len().min(5) as f32;
        let mut h = 280.0 + rows * 32.0 + 8.0;
        let e = self.extras();
        if e.update.is_some() {
            h += ROW_H + 4.0;
        }
        if e.explainer {
            h += LINE2_H + ROW_H + 4.0; // cache caption + "Why?" row
        }
        if !self.snap.privileges.profile {
            h += 40.0;
        }
        if self.snap.pool_note.is_some() {
            h += 36.0;
        }
        h + 44.0 + 64.0
    }

    /// Positions the flyout next to `anchor` (tray icon rect, screen px) and shows it.
    pub fn show(&mut self, anchor: Option<RECT>) {
        let pt = anchor.map_or(
            POINT {
                x: i32::MAX / 2,
                y: i32::MAX / 2,
            },
            |r| POINT {
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
            },
        );
        unsafe {
            let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
            let mut mi = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(mon, &mut mi);
            let (mut dx, mut dy) = (96u32, 96u32);
            if GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() {
                self.dpi = dx;
            }
            let s = self.scale();
            let w = (W * s).round() as i32;
            let h = (self.height() * s).round() as i32;
            let work = mi.rcWork;
            let monr = mi.rcMonitor;
            let m = (12.0 * s) as i32;
            let (ax, ay) = if anchor.is_some() {
                (pt.x, pt.y)
            } else {
                (work.right, work.bottom)
            };
            let clamp_x = |x: i32| x.clamp(work.left + m, (work.right - w - m).max(work.left + m));
            let clamp_y = |y: i32| y.clamp(work.top + m, (work.bottom - h - m).max(work.top + m));
            let (x, y) = if work.top > monr.top {
                (clamp_x(ax - w / 2), work.top + m)
            } else if work.left > monr.left {
                (work.left + m, clamp_y(ay - h / 2))
            } else if work.right < monr.right {
                (work.right - w - m, clamp_y(ay - h / 2))
            } else {
                (clamp_x(ax - w / 2), work.bottom - h - m)
            };
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
            if let Some(g) = &self.gfx {
                g.resize(w as u32, h as u32);
                g.set_dpi(self.dpi as f32);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.visible = true;
        self.hover = None;
        self.focus = None;
        self.invalidate();
    }

    /// Re-applies the size after the content height changed.
    pub fn relayout(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            let mut r = RECT::default();
            let _ = windows::Win32::UI::WindowsAndMessaging::GetWindowRect(self.hwnd, &mut r);
            let s = self.scale();
            let w = (W * s).round() as i32;
            let h = (self.height() * s).round() as i32;
            if r.bottom - r.top != h {
                // Keep the edge nearest the taskbar fixed (assume bottom unless near the top).
                let y = if r.top < 200 { r.top } else { r.bottom - h };
                let _ = SetWindowPos(
                    self.hwnd,
                    Some(HWND_TOPMOST),
                    r.left,
                    y,
                    w,
                    h,
                    SWP_NOACTIVATE,
                );
                if let Some(g) = &self.gfx {
                    g.resize(w as u32, h as u32);
                }
            }
        }
        self.invalidate();
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.visible = false;
        self.hidden_at = now_ms();
        self.gfx = None; // release device resources
        self.hits.clear();
        self.view = View::Dashboard;
        self.scroll = 0.0;
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
        if let Some(g) = &self.gfx {
            g.set_dpi(self.dpi as f32);
        }
        self.relayout();
    }

    // ---------------------------------------------------------------- input

    fn to_dip(&self, x: i32, y: i32) -> (f32, f32) {
        let s = self.scale();
        (x as f32 / s, y as f32 / s)
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<Hit> {
        let (x, y) = self.to_dip(x, y);
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, h)| *h)
    }

    pub fn on_mouse_move(&mut self, x: i32, y: i32) {
        let h = self.hit_at(x, y);
        if h != self.hover {
            self.hover = h;
            self.invalidate();
        }
    }

    pub fn on_mouse_leave(&mut self) {
        self.tracking_mouse = false;
        if self.hover.take().is_some() {
            self.invalidate();
        }
    }

    pub fn on_button_down(&mut self, x: i32, y: i32) {
        self.pressed = self.hit_at(x, y);
        self.focus = None;
        self.invalidate();
    }

    pub fn on_button_up(&mut self, x: i32, y: i32) -> Option<UiAction> {
        let h = self.hit_at(x, y);
        let p = self.pressed.take();
        self.invalidate();
        match (h, p) {
            (Some(h), Some(p)) if h == p => self.activate(h),
            _ => None,
        }
    }

    pub fn on_wheel(&mut self, delta: i16) {
        if !self.scrollable() {
            return;
        }
        let viewport = SETTINGS_MAX_H - HEADER_H;
        let max = (self.content_h - viewport).max(0.0);
        self.scroll = (self.scroll - delta as f32 / 120.0 * 48.0).clamp(0.0, max);
        self.invalidate();
    }

    pub fn on_key(&mut self, vk: u16, shift: bool) -> Option<UiAction> {
        const VK_TAB: u16 = 0x09;
        const VK_RETURN: u16 = 0x0D;
        const VK_ESCAPE: u16 = 0x1B;
        const VK_SPACE: u16 = 0x20;
        match vk {
            VK_ESCAPE => {
                if self.scrollable() {
                    self.view = View::Dashboard;
                    self.focus = None;
                    return Some(UiAction::Resize);
                }
                Some(UiAction::Close)
            }
            VK_TAB => {
                let n = self.hits.len();
                if n > 0 {
                    self.focus = Some(match (self.focus, shift) {
                        (None, false) => 0,
                        (None, true) => n - 1,
                        (Some(i), false) => (i + 1) % n,
                        (Some(i), true) => (i + n - 1) % n,
                    });
                    self.ensure_focus_visible();
                    self.invalidate();
                }
                None
            }
            VK_RETURN | VK_SPACE => {
                let h = self.focus.and_then(|i| self.hits.get(i)).map(|(_, h)| *h)?;
                self.activate(h)
            }
            _ => None,
        }
    }

    fn ensure_focus_visible(&mut self) {
        if !self.scrollable() {
            return;
        }
        if let Some((r, _)) = self.focus.and_then(|i| self.hits.get(i)) {
            let top = HEADER_H;
            let bottom = SETTINGS_MAX_H - 8.0;
            if r.y < top {
                self.scroll = (self.scroll - (top - r.y) - 8.0).max(0.0);
            } else if r.bottom() > bottom {
                self.scroll += r.bottom() - bottom + 8.0;
            }
        }
    }

    fn changed(&self, f: impl FnOnce(&mut Config)) -> Option<UiAction> {
        let mut c = self.cfg.clone();
        f(&mut c);
        Some(UiAction::SetConfig(Box::new(c)))
    }

    fn activate(&mut self, h: Hit) -> Option<UiAction> {
        match h {
            Hit::Settings => {
                self.view = View::Settings;
                self.scroll = 0.0;
                self.focus = None;
                Some(UiAction::Resize)
            }
            Hit::Activity => {
                self.view = View::Activity;
                self.scroll = 0.0;
                self.focus = None;
                Some(UiAction::Resize)
            }
            Hit::Back => {
                self.view = View::Dashboard;
                self.focus = None;
                Some(UiAction::Resize)
            }
            Hit::Pause => Some(UiAction::TogglePause),
            Hit::Optimize => Some(
                match values::optimize_preview(
                    self.snap.state,
                    &self.snap.reading.split(),
                    self.cfg.friendly,
                ) {
                    Some(text) => UiAction::ConfirmOptimize(text),
                    None => UiAction::OptimizeNow,
                },
            ),
            Hit::Profile(i) => self.changed(|c| c.profile = Profile::ALL[i as usize]),
            Hit::Proc(pid) => Some(UiAction::ProcMenu(pid)),
            Hit::EnableCleaning => Some(UiAction::EnableCleaning),
            Hit::Style(i) => self.changed(|c| c.icon_style = IconStyle::ALL[i as usize]),
            Hit::Metric(i) => self.changed(|c| c.icon_metric = IconMetric::ALL[i as usize]),
            Hit::Values(i) => self.changed(|c| c.set_friendly(i == 0)),
            Hit::AutoUpdate => self.changed(|c| c.auto_update = !c.auto_update),
            Hit::CheckUpdate => (!self.update.busy()).then_some(UiAction::CheckUpdate),
            Hit::InstallUpdate => {
                matches!(self.update, UpdateStatus::Available(_)).then_some(UiAction::InstallUpdate)
            }
            Hit::Explainer => Some(UiAction::OpenExplainer),
            Hit::Preset(i) => self.changed(|c| c.preset = ColorPreset::PICKABLE[i as usize]),
            Hit::Swatch(i) => Some(UiAction::PickColor(i)),
            Hit::Tier(i) => self.changed(|c| c.auto_tiers[i as usize] = !c.auto_tiers[i as usize]),
            Hit::GameMode => self.changed(|c| c.game_mode = !c.game_mode),
            Hit::Notify(i) => self.changed(|c| match i {
                0 => c.notify_leaks = !c.notify_leaks,
                1 => c.notify_warnings = !c.notify_warnings,
                _ => c.notify_results = !c.notify_results,
            }),
            Hit::Translucent => self.changed(|c| c.translucent = !c.translucent),
            Hit::TaskButton => Some(if self.snap.task_registered {
                UiAction::RemoveTask
            } else {
                UiAction::EnableCleaning
            }),
            Hit::RunKey => Some(UiAction::SetRunKey(!self.run_key)),
            Hit::DeepClean => Some(UiAction::DeepClean),
            Hit::EditExclusions => Some(UiAction::EditExclusions),
        }
    }

    // ---------------------------------------------------------------- paint

    pub fn paint(&mut self) {
        if !self.visible {
            return;
        }
        if self.gfx.is_none() {
            let s = self.scale();
            let w = (W * s).round() as u32;
            let h = (self.height() * s).round() as u32;
            self.gfx = Gfx::new(self.hwnd, w, h, self.dpi as f32).ok();
        }
        let Some(g) = self.gfx.take() else { return };
        self.hits.clear();
        let clear = if self.backdrop {
            self.pal.bg
        } else {
            self.pal.bg_solid
        };
        g.begin(clear);
        if !self.backdrop {
            g.stroke_round(
                Rect::new(0.0, 0.0, W, self.height()),
                8.0,
                self.pal.border,
                1.0,
            );
        }
        match self.view {
            View::Dashboard => self.draw_dashboard(&g),
            View::Settings => self.draw_settings(&g),
            View::Activity => self.draw_activity(&g),
        }
        if let Some((r, _)) = self.focus.and_then(|i| self.hits.get(i)) {
            g.stroke_round(r.inset(-2.0), 6.0, self.pal.text, 2.0);
        }
        if g.end() {
            self.gfx = Some(g);
        }
    }

    fn state_col(&self, st: PressureState) -> Rgba {
        let ap = Appearance {
            taskbar_light: self.ap.apps_light,
            ..self.ap
        };
        let preset = if self.cfg.preset == ColorPreset::Mono {
            ColorPreset::System
        } else {
            self.cfg.preset
        };
        state_color(preset, &self.cfg.custom_colors, st, &ap)
    }

    fn hovered(&self, h: Hit) -> bool {
        self.hover == Some(h)
    }

    fn icon_button(&mut self, g: &Gfx, r: Rect, glyph: &str, hit: Hit) {
        if self.hovered(hit) {
            g.fill_round(r, 6.0, self.pal.control_hover);
        }
        g.text(glyph, r, Font::Icon, self.pal.text, Align::Center);
        self.hits.push((r, hit));
    }

    fn primary_button(&mut self, g: &Gfx, r: Rect, label: &str, hit: Hit) {
        let mut c = self.pal.accent;
        if self.hovered(hit) {
            c = c.mix(
                Rgba::rgb(0xFFFFFF),
                if self.ap.apps_light { 0.1 } else { 0.12 },
            );
        }
        if self.pressed == Some(hit) {
            c = c.with_alpha(0.85);
        }
        g.fill_round(r, 6.0, c);
        g.text(
            label,
            r,
            Font::BodyStrong,
            self.pal.on_accent,
            Align::Center,
        );
        self.hits.push((r, hit));
    }

    fn secondary_button(&mut self, g: &Gfx, r: Rect, label: &str, hit: Hit) {
        let c = if self.hovered(hit) {
            self.pal.control_hover
        } else {
            self.pal.control
        };
        g.fill_round(r, 6.0, c);
        g.stroke_round(r, 6.0, self.pal.border, 1.0);
        g.text(label, r, Font::Body, self.pal.text, Align::Center);
        self.hits.push((r, hit));
    }

    fn segmented(
        &mut self,
        g: &Gfx,
        r: Rect,
        labels: &[&str],
        selected: usize,
        hit: impl Fn(u8) -> Hit,
    ) {
        g.fill_round(r, 6.0, self.pal.control);
        let n = labels.len() as f32;
        let w = r.w / n;
        for (i, label) in labels.iter().enumerate() {
            let seg = Rect::new(r.x + w * i as f32, r.y, w, r.h);
            let h = hit(i as u8);
            if i == selected {
                g.fill_round(seg.inset(2.0), 5.0, self.pal.accent);
                g.text(
                    label,
                    seg,
                    Font::CaptionStrong,
                    self.pal.on_accent,
                    Align::Center,
                );
            } else {
                if self.hovered(h) {
                    g.fill_round(seg.inset(2.0), 5.0, self.pal.control_hover);
                }
                g.text(label, seg, Font::Caption, self.pal.text, Align::Center);
            }
            self.hits.push((seg, h));
        }
    }

    fn toggle(&mut self, g: &Gfx, r: Rect, on: bool, hit: Hit) {
        let knob_r = 5.0;
        if on {
            let c = if self.hovered(hit) {
                self.pal.accent.mix(Rgba::rgb(0xFFFFFF), 0.1)
            } else {
                self.pal.accent
            };
            g.fill_round(r, r.h / 2.0, c);
            g.circle(
                r.right() - r.h / 2.0,
                r.y + r.h / 2.0,
                knob_r,
                self.pal.on_accent,
            );
        } else {
            if self.hovered(hit) {
                g.fill_round(r, r.h / 2.0, self.pal.control_hover);
            }
            g.stroke_round(r, r.h / 2.0, self.pal.text2, 1.0);
            g.circle(
                r.x + r.h / 2.0,
                r.y + r.h / 2.0,
                knob_r - 1.0,
                self.pal.text2,
            );
        }
        self.hits.push((r, hit));
    }

    fn draw_dashboard(&mut self, g: &Gfx) {
        let p = self.pal;
        let snap = self.snap.clone();
        let r = &snap.reading;
        let st_col = self.state_col(snap.state);
        let friendly = self.cfg.friendly;
        let sp = r.split();
        let extras = self.extras();
        // Friendly: the ring fills with app memory only (cache is ready memory).
        let used = if friendly {
            sp.apps_fraction()
        } else {
            r.used_fraction()
        } as f32;

        // Header: gauge, title, chips, buttons.
        let (cx, cy, rad) = (PAD + 28.0, PAD + 28.0, 24.0);
        g.arc(cx, cy, rad, 0.9999, p.track, 6.0);
        g.arc(cx, cy, rad, used, st_col, 6.0);
        g.text(
            &format!("{:.0}%", used * 100.0),
            Rect::new(cx - 26.0, cy - 12.0, 52.0, 24.0),
            Font::BodyStrong,
            p.text,
            Align::Center,
        );
        let tx = PAD + 68.0;
        g.text(
            "Memory",
            Rect::new(tx, 14.0, 90.0, 24.0),
            Font::Title,
            p.text,
            Align::Left,
        );
        let tw = g.text_width("Memory", Font::Title);
        let mut chip_x = tx + tw + 8.0;
        let chips: Vec<(String, Rgba)> = {
            let mut v = vec![(values::state_name(snap.state, friendly).to_string(), st_col)];
            if snap.paused {
                v.push(("Paused".into(), p.text2));
            } else if snap.game_mode {
                v.push(("Game mode".into(), p.text2));
            }
            v
        };
        for (label, c) in chips {
            let w = g.text_width(&label, Font::CaptionStrong) + 14.0;
            let cr = Rect::new(chip_x, 17.0, w, 18.0);
            g.fill_round(cr, 9.0, c.with_alpha(0.16));
            g.text(&label, cr, Font::CaptionStrong, c, Align::Center);
            chip_x += w + 6.0;
        }
        let sub = if r.total == 0 {
            "Reading memory…".into()
        } else if friendly {
            format!("{} ready for apps", fmt_bytes(sp.ready()))
        } else {
            format!("{} of {} in use", fmt_bytes(r.in_use), fmt_bytes(r.total))
        };
        g.text(
            &sub,
            Rect::new(tx, 38.0, W - tx - 84.0, 20.0),
            Font::Body,
            p.text2,
            Align::Left,
        );
        let line3 = if friendly {
            format!(
                "Apps {} · Speed-up cache {} · of {}",
                fmt_bytes(sp.apps),
                fmt_bytes(sp.cache),
                fmt_bytes(r.total)
            )
        } else {
            format!(
                "Pressure {:.0}% · Commit {:.0}% · Compressed {}",
                snap.s * 100.0,
                r.commit_fraction() * 100.0,
                fmt_bytes(snap.compressed)
            )
        };
        g.text(
            &line3,
            Rect::new(tx, 58.0, W - tx - PAD, 18.0),
            Font::Caption,
            p.text3,
            Align::Left,
        );
        let pause_glyph = if snap.paused { "\u{E768}" } else { "\u{E769}" };
        self.icon_button(
            g,
            Rect::new(W - PAD - 68.0, 10.0, 32.0, 32.0),
            pause_glyph,
            Hit::Pause,
        );
        self.icon_button(
            g,
            Rect::new(W - PAD - 32.0, 10.0, 32.0, 32.0),
            "\u{E713}",
            Hit::Settings,
        );

        let mut y = 92.0;
        g.line(PAD, y, W - PAD, y, p.divider, 1.0);
        y += 14.0;

        // Breakdown bar. Friendly: Apps / Speed-up cache / Free. Standard:
        // Task Manager's In use / Modified / Standby / Free.
        let total = r.total.max(1) as f32;
        let free = r.total.saturating_sub(r.in_use + r.modified + r.standby);
        let parts: Vec<(u64, Rgba, &str)> = if friendly {
            vec![
                (sp.apps, st_col, "Apps"),
                (sp.cache, p.seg_standby, "Speed-up cache"),
                (sp.free, p.track, "Free"),
            ]
        } else {
            vec![
                (r.in_use, st_col, "In use"),
                (r.modified, p.seg_modified, "Modified"),
                (r.standby, p.seg_standby, "Standby"),
                (free, p.track, "Free"),
            ]
        };
        let bar = Rect::new(PAD, y, W - 2.0 * PAD, 8.0);
        g.fill_round(bar, 4.0, p.track);
        let mut x = bar.x;
        for (bytes, c, _) in parts.iter().take(parts.len() - 1) {
            let w = bar.w * (*bytes as f32 / total);
            if w >= 1.0 {
                g.fill_round(Rect::new(x, y, (w - 2.0).max(1.0), 8.0), 4.0, *c);
            }
            x += w;
        }
        y += 18.0;
        let col_w = (W - 2.0 * PAD) / 2.0;
        for (i, (bytes, c, label)) in parts.iter().enumerate() {
            let lx = PAD + col_w * (i % 2) as f32;
            let ly = y + 20.0 * (i / 2) as f32;
            let dot = if *label == "Free" { p.text3 } else { *c };
            g.circle(lx + 4.0, ly + 9.0, 4.0, dot);
            g.text(
                &format!("{label}  {}", fmt_bytes(*bytes)),
                Rect::new(lx + 14.0, ly, col_w - 14.0, 18.0),
                Font::Caption,
                p.text2,
                Align::Left,
            );
        }
        y += 48.0;
        if extras.explainer {
            g.text_wrapped(
                "Speed-up cache keeps recent files and app data in memory. Windows hands it to apps the moment they need it.",
                Rect::new(PAD, y - 6.0, W - 2.0 * PAD, LINE2_H),
                Font::Caption,
                p.text3,
            );
            y += LINE2_H;
        }

        // Sparkline. Friendly: app memory with the cache stacked on top, so a
        // shrinking cache band shows it giving way to apps.
        g.text(
            if friendly {
                "Apps and speed-up cache · last 10 minutes"
            } else {
                "In use · last 10 minutes"
            },
            Rect::new(PAD, y, W - 2.0 * PAD, 16.0),
            Font::Caption,
            p.text3,
            Align::Left,
        );
        y += 20.0;
        let chart = Rect::new(PAD, y, W - 2.0 * PAD, 40.0);
        g.line(
            chart.x,
            chart.bottom(),
            chart.right(),
            chart.bottom(),
            p.divider,
            1.0,
        );
        if friendly && snap.history_split.len() >= 2 {
            let stacked: Vec<f32> = snap.history_split.iter().map(|(a, c)| a + c).collect();
            let apps: Vec<f32> = snap.history_split.iter().map(|(a, _)| *a).collect();
            g.sparkline(
                chart,
                &stacked,
                p.seg_standby,
                p.seg_standby.with_alpha(0.22),
            );
            g.sparkline(chart, &apps, st_col, st_col.with_alpha(0.30));
        } else if !friendly && snap.history.len() >= 2 {
            g.sparkline(chart, &snap.history, st_col, st_col.with_alpha(0.14));
        }
        // Each action as a dot on the time axis, so its effect on the curve is visible.
        let window = snap.history.len().saturating_sub(1) as u64 * 5_000;
        for a in snap.activity.iter().filter(|a| !a.alert) {
            if let Some(f) = values::marker_pos(snap.t_ms.saturating_sub(a.t_ms), window) {
                let x = chart.x + chart.w * f;
                g.line(
                    x,
                    chart.y,
                    x,
                    chart.bottom(),
                    p.accent.with_alpha(0.35),
                    1.0,
                );
                g.circle(x, chart.bottom(), 3.0, p.accent);
            }
        }
        y += 52.0;
        g.line(PAD, y, W - PAD, y, p.divider, 1.0);
        y += 10.0;

        // Top apps.
        g.text(
            "Apps using the most memory",
            Rect::new(PAD, y, 240.0, 18.0),
            Font::CaptionStrong,
            p.text2,
            Align::Left,
        );
        g.text(
            "Private",
            Rect::new(W - PAD - 80.0, y, 80.0, 18.0),
            Font::Caption,
            p.text3,
            Align::Right,
        );
        y += 22.0;
        for row in snap.top.iter().take(5) {
            let rr = Rect::new(PAD - 6.0, y, W - 2.0 * PAD + 12.0, 30.0);
            let hit = Hit::Proc(row.pid);
            if self.hovered(hit) {
                g.fill_round(rr, 6.0, p.control_hover);
            }
            g.text(
                &row.name,
                Rect::new(PAD + 2.0, y, 140.0, 30.0),
                Font::Body,
                p.text,
                Align::Left,
            );
            g.text(
                &fmt_bytes(row.private),
                Rect::new(W - PAD - 80.0, y, 78.0, 30.0),
                Font::BodyStrong,
                p.text,
                Align::Right,
            );
            let (note, c) = if row.leak {
                (
                    format!("\u{26A0} leaking {:.0} MB/h", row.slope_mib_h),
                    p.danger,
                )
            } else if row.runaway {
                ("growing fast".to_string(), p.warn)
            } else if row.slope_mib_h >= 30.0 {
                (format!("\u{2197} {:.0} MB/h", row.slope_mib_h), p.text3)
            } else if let Some(what) = row.acted {
                (what.to_string(), p.accent)
            } else if friendly && row.idle_min >= 60.0 && !row.critical {
                (format!("idle {}", values::fmt_idle(row.idle_min)), p.text3)
            } else if row.lowered {
                ("low priority".to_string(), p.text3)
            } else {
                (String::new(), p.text3)
            };
            if !note.is_empty() {
                g.text(
                    &note,
                    Rect::new(PAD + 144.0, y, W - 2.0 * PAD - 144.0 - 84.0, 30.0),
                    Font::Caption,
                    c,
                    Align::Right,
                );
            }
            self.hits.push((rr, hit));
            y += 32.0;
        }
        y += 4.0;
        g.line(PAD, y, W - PAD, y, p.divider, 1.0);
        y += 10.0;

        // Update available, the one-time explainer, an idle app worth closing.
        if let Some(text) = &extras.update {
            self.link_row(g, &mut y, text, "Install \u{2192}", Hit::InstallUpdate);
        }
        if extras.explainer {
            self.link_row(
                g,
                &mut y,
                "Cache now counts as ready memory.",
                "Why? \u{2192}",
                Hit::Explainer,
            );
        }

        // Notes: pool leak, privileges.
        if let Some(note) = &snap.pool_note {
            let first = note.lines().next().unwrap_or_default();
            g.text(
                &format!("\u{26A0} {first}"),
                Rect::new(PAD, y, W - 2.0 * PAD, 30.0),
                Font::Caption,
                p.danger,
                Align::Left,
            );
            y += 36.0;
        }
        if !snap.privileges.profile {
            let rr = Rect::new(PAD - 6.0, y, W - 2.0 * PAD + 12.0, 32.0);
            if self.hovered(Hit::EnableCleaning) {
                g.fill_round(rr, 6.0, p.control_hover);
            }
            g.text(
                "Monitoring only.",
                Rect::new(PAD, y, 120.0, 32.0),
                Font::Caption,
                p.text2,
                Align::Left,
            );
            g.text(
                "Enable cleaning \u{2192}",
                Rect::new(PAD + 110.0, y, W - 2.0 * PAD - 110.0, 32.0),
                Font::CaptionStrong,
                p.accent,
                Align::Right,
            );
            self.hits.push((rr, Hit::EnableCleaning));
            y += 40.0;
        }

        // Now / last action, with the way into the activity log.
        let now = values::now_line(
            snap.paused,
            !snap.privileges.profile,
            snap.game_mode,
            snap.acting,
            snap.state,
        );
        g.text(
            &format!("Now: {now}"),
            Rect::new(PAD, y, W - 2.0 * PAD, 18.0),
            Font::Caption,
            p.text2,
            Align::Left,
        );
        let (last, last_col) = match snap.activity.first() {
            Some(a) => {
                let col = if a.alert {
                    p.warn
                } else {
                    match values::result_text(a.freed, a.refault_ratio, a.pending).1 {
                        values::Tone::Good => p.good,
                        values::Tone::Warn => p.warn,
                        values::Tone::Neutral => p.text3,
                    }
                };
                (
                    format!(
                        "Last: {} · {}",
                        a.title,
                        fmt_ago(snap.t_ms.saturating_sub(a.t_ms))
                    ),
                    col,
                )
            }
            None if friendly && snap.state == PressureState::Normal => (
                format!("Nothing to clean: {} ready for apps", fmt_bytes(sp.ready())),
                p.text3,
            ),
            None => ("No actions yet".to_string(), p.text3),
        };
        let link = Rect::new(W - PAD - 80.0, y + 18.0, 80.0, 18.0);
        g.text(
            &last,
            Rect::new(PAD, y + 18.0, W - 2.0 * PAD - 84.0, 18.0),
            Font::Caption,
            last_col,
            Align::Left,
        );
        if self.hovered(Hit::Activity) {
            g.fill_round(link.inset(-3.0), 5.0, p.control_hover);
        }
        g.text(
            "Activity \u{2192}",
            link,
            Font::CaptionStrong,
            p.accent,
            Align::Right,
        );
        self.hits.push((link.inset(-3.0), Hit::Activity));
        y += 44.0;

        // Footer.
        self.primary_button(
            g,
            Rect::new(PAD, y, 132.0, 32.0),
            "Optimize now",
            Hit::Optimize,
        );
        let labels: Vec<&str> = Profile::ALL.iter().map(|p| p.label()).collect();
        let sel = Profile::ALL
            .iter()
            .position(|p| *p == self.cfg.profile)
            .unwrap_or(1);
        self.segmented(
            g,
            Rect::new(PAD + 140.0, y, W - 2.0 * PAD - 140.0, 32.0),
            &labels,
            sel,
            Hit::Profile,
        );
    }

    /// A full-width clickable row: text on the left, an accent link on the right.
    fn link_row(&mut self, g: &Gfx, y: &mut f32, text: &str, link: &str, hit: Hit) {
        let rr = Rect::new(PAD - 6.0, *y, W - 2.0 * PAD + 12.0, ROW_H);
        if self.hovered(hit) {
            g.fill_round(rr, 6.0, self.pal.control_hover);
        }
        g.text(
            text,
            Rect::new(PAD, *y, W - 2.0 * PAD - 90.0, ROW_H),
            Font::Caption,
            self.pal.text2,
            Align::Left,
        );
        g.text(
            link,
            Rect::new(W - PAD - 90.0, *y, 90.0, ROW_H),
            Font::CaptionStrong,
            self.pal.accent,
            Align::Right,
        );
        self.hits.push((rr, hit));
        *y += ROW_H + 4.0;
    }

    fn section(&self, g: &Gfx, y: &mut f32, title: &str) {
        *y += 14.0;
        g.text(
            title,
            Rect::new(PAD, *y, W - 2.0 * PAD, 18.0),
            Font::CaptionStrong,
            self.pal.text2,
            Align::Left,
        );
        *y += 24.0;
    }

    fn toggle_row(&mut self, g: &Gfx, y: &mut f32, label: &str, on: bool, hit: Hit) {
        let row = Rect::new(PAD - 6.0, *y, W - 2.0 * PAD + 12.0, 36.0);
        if self.hovered(hit) {
            g.fill_round(row, 6.0, self.pal.control);
        }
        g.text(
            label,
            Rect::new(PAD, *y, W - 2.0 * PAD - 56.0, 36.0),
            Font::Body,
            self.pal.text,
            Align::Left,
        );
        self.toggle(g, Rect::new(W - PAD - 40.0, *y + 8.0, 40.0, 20.0), on, hit);
        self.hits.pop();
        self.hits.push((row, hit));
        *y += 38.0;
    }

    /// Fixed header with a back button, then a clipped, scrolled content area.
    /// Returns (first hit index, content top).
    fn begin_scroll_view(&mut self, g: &Gfx, title: &str) -> (usize, f32) {
        let p = self.pal;
        self.icon_button(g, Rect::new(10.0, 12.0, 32.0, 32.0), "\u{E72B}", Hit::Back);
        g.text(
            title,
            Rect::new(50.0, 12.0, 200.0, 32.0),
            Font::Header,
            p.text,
            Align::Left,
        );
        g.line(PAD, HEADER_H - 1.0, W - PAD, HEADER_H - 1.0, p.divider, 1.0);
        g.clip(Rect::new(0.0, HEADER_H, W, SETTINGS_MAX_H - HEADER_H));
        (self.hits.len(), HEADER_H - self.scroll)
    }

    /// Ends a scroll view: records the content height, drops hits outside the
    /// viewport and draws the scroll indicator.
    fn end_scroll_view(&mut self, g: &Gfx, first_hit: usize, top: f32, y: f32) {
        let viewport = Rect::new(0.0, HEADER_H, W, SETTINGS_MAX_H - HEADER_H);
        self.content_h = y - top;
        g.unclip();
        let mut i = first_hit;
        while i < self.hits.len() {
            let r = self.hits[i].0;
            if r.bottom() <= viewport.y || r.y >= viewport.bottom() {
                self.hits.remove(i);
            } else {
                i += 1;
            }
        }
        if self.content_h > viewport.h {
            let frac = viewport.h / self.content_h;
            let th = (viewport.h * frac).max(24.0);
            let max = self.content_h - viewport.h;
            let ty = viewport.y + (viewport.h - th) * (self.scroll / max).clamp(0.0, 1.0);
            g.fill_round(
                Rect::new(W - 6.0, ty + 2.0, 3.0, th - 4.0),
                1.5,
                self.pal.text3.with_alpha(0.6),
            );
        }
    }

    /// What MemManager did, why, to which apps, and what it measurably changed.
    fn draw_activity(&mut self, g: &Gfx) {
        let p = self.pal;
        let snap = self.snap.clone();
        let (first_hit, top) = self.begin_scroll_view(g, "Activity");
        let mut y = top + 14.0;
        let w = W - 2.0 * PAD;

        // Summary: the actions in the log, plus what the cache did meanwhile.
        let acts = snap.activity.iter().filter(|a| a.measured);
        let (n, freed, slow) = acts.fold((0, 0u64, 0), |(n, f, k), a| {
            (
                n + 1,
                f + a.freed,
                k + usize::from(!a.pending && a.refault_ratio >= 0.25),
            )
        });
        g.text(
            &values::summary(n, freed, slow),
            Rect::new(PAD, y, w, 20.0),
            Font::BodyStrong,
            p.text,
            Align::Left,
        );
        y += 24.0;
        let window = snap.history_split.len().saturating_sub(1) as u64 * 5_000;
        let room = match (snap.history_split.first(), snap.history_split.last()) {
            (Some(a), Some(b)) => {
                values::room_line(values::room_made(*a, *b, snap.reading.total), window)
            }
            _ => None,
        };
        let hits = values::cache_line(snap.cache_hits, snap.disk_reads, snap.hits_window_ms);
        for line in [room, hits].into_iter().flatten() {
            y += g.text_wrapped(&line, Rect::new(PAD, y, w, 40.0), Font::Caption, p.text2) + 4.0;
        }
        y += 6.0;
        g.line(PAD, y, W - PAD, y, p.divider, 1.0);
        y += 12.0;

        if snap.activity.is_empty() {
            y += g.text_wrapped(
                "Nothing yet. MemManager only steps in when memory gets tight, and every step it takes shows up here with the reason and its measured effect.",
                Rect::new(PAD, y, w, 60.0),
                Font::Caption,
                p.text3,
            ) + 12.0;
        }
        for a in &snap.activity {
            let ago = fmt_ago(snap.t_ms.saturating_sub(a.t_ms));
            let ago_w = g.text_width(&ago, Font::Caption) + 4.0;
            let dot = if a.alert { p.warn } else { p.accent };
            g.circle(PAD + 3.0, y + 9.0, 3.0, dot);
            g.text(
                &a.title,
                Rect::new(PAD + 12.0, y, w - 12.0 - ago_w, 18.0),
                Font::BodyStrong,
                p.text,
                Align::Left,
            );
            g.text(
                &ago,
                Rect::new(W - PAD - ago_w, y, ago_w, 18.0),
                Font::Caption,
                p.text3,
                Align::Right,
            );
            y += 20.0;
            let who = if a.manual { "" } else { "Automatic · " };
            if !a.why.is_empty() {
                y += g.text_wrapped(
                    &format!("{who}{}", a.why),
                    Rect::new(PAD + 12.0, y, w - 12.0, 40.0),
                    Font::Caption,
                    p.text2,
                );
            }
            let (result, col) = if a.measured {
                let (t, tone) = values::result_text(a.freed, a.refault_ratio, a.pending);
                let c = match tone {
                    values::Tone::Good => p.good,
                    values::Tone::Warn => p.warn,
                    values::Tone::Neutral => p.text3,
                };
                (t, c)
            } else {
                (String::new(), p.text3)
            };
            let last = match (a.detail.is_empty(), result.is_empty()) {
                (true, true) => String::new(),
                (false, true) => a.detail.clone(),
                (true, false) => result,
                (false, false) => format!("{} · {result}", a.detail),
            };
            if !last.is_empty() {
                y += g.text_wrapped(
                    &last,
                    Rect::new(PAD + 12.0, y, w - 12.0, 40.0),
                    Font::Caption,
                    col,
                );
            }
            y += 12.0;
        }
        self.end_scroll_view(g, first_hit, top, y);
    }

    fn draw_settings(&mut self, g: &Gfx) {
        let p = self.pal;
        let cfg = self.cfg.clone();
        let snap = self.snap.clone();
        let (first_hit, top) = self.begin_scroll_view(g, "Settings");
        let mut y = top;

        self.section(g, &mut y, "VALUES");
        self.segmented(
            g,
            Rect::new(PAD, y, W - 2.0 * PAD, 32.0),
            &["Friendly", "Windows standard"],
            if cfg.friendly { 0 } else { 1 },
            Hit::Values,
        );
        y += 40.0;
        y += g.text_wrapped(
            if cfg.friendly {
                "Leads with memory that's ready for apps and shows cache as a speed-up. Every number is a real Windows value."
            } else {
                "Task Manager's terms: In use, Modified, Standby, Free and Available."
            },
            Rect::new(PAD, y, W - 2.0 * PAD, 40.0),
            Font::Caption,
            p.text3,
        ) + 8.0;

        self.section(g, &mut y, "TRAY ICON");
        g.text(
            "Style",
            Rect::new(PAD, y, 100.0, 30.0),
            Font::Body,
            p.text,
            Align::Left,
        );
        let styles: Vec<&str> = IconStyle::ALL.iter().map(|s| s.label()).collect();
        let si = IconStyle::ALL
            .iter()
            .position(|s| *s == cfg.icon_style)
            .unwrap_or(0);
        self.segmented(
            g,
            Rect::new(124.0, y, W - PAD - 124.0, 30.0),
            &styles,
            si,
            Hit::Style,
        );
        y += 38.0;
        g.text(
            "Shows",
            Rect::new(PAD, y, 100.0, 30.0),
            Font::Body,
            p.text,
            Align::Left,
        );
        let metrics: Vec<&str> = IconMetric::ALL
            .iter()
            .map(|m| m.label(cfg.friendly))
            .collect();
        let mi = IconMetric::ALL
            .iter()
            .position(|m| *m == cfg.icon_metric)
            .unwrap_or(0);
        self.segmented(
            g,
            Rect::new(124.0, y, W - PAD - 124.0, 30.0),
            &metrics,
            mi,
            Hit::Metric,
        );
        y += 38.0;
        g.text(
            "Colors",
            Rect::new(PAD, y, 100.0, 30.0),
            Font::Body,
            p.text,
            Align::Left,
        );
        let presets: Vec<&str> = ColorPreset::PICKABLE.iter().map(|c| c.label()).collect();
        let pi = ColorPreset::PICKABLE
            .iter()
            .position(|c| *c == cfg.preset)
            .unwrap_or(usize::MAX);
        self.segmented(
            g,
            Rect::new(124.0, y, W - PAD - 124.0, 30.0),
            &presets,
            pi,
            Hit::Preset,
        );
        y += 38.0;
        g.text(
            "State colors",
            Rect::new(PAD, y, 200.0, 30.0),
            Font::Body,
            p.text,
            Align::Left,
        );
        y += 30.0;
        let tb = Appearance {
            taskbar_light: self.ap.taskbar_light,
            ..self.ap
        };
        let sw = (W - 2.0 * PAD) / 4.0;
        for (i, st) in PressureState::ALL.iter().enumerate() {
            let sr = Rect::new(PAD + i as f32 * sw, y, sw, 30.0);
            let c = state_color(cfg.preset, &cfg.custom_colors, *st, &tb);
            let hit = Hit::Swatch(i as u8);
            if self.hovered(hit) {
                g.fill_round(sr.inset(-2.0), 8.0, p.control_hover);
            }
            let dot = Rect::new(sr.x + (sw - 20.0) / 2.0, sr.y + 2.0, 20.0, 20.0);
            g.fill_round(dot, 5.0, c);
            g.stroke_round(dot, 5.0, p.border, 1.0);
            self.hits.push((sr, hit));
        }
        y += 26.0;
        for (i, st) in PressureState::ALL.iter().enumerate() {
            g.text(
                values::state_name(*st, cfg.friendly),
                Rect::new(PAD + i as f32 * sw, y, sw, 14.0),
                Font::Caption,
                p.text3,
                Align::Center,
            );
        }
        y += 18.0;
        g.text(
            "Click a color to customize it.",
            Rect::new(PAD, y, W - 2.0 * PAD, 16.0),
            Font::Caption,
            p.text3,
            Align::Left,
        );
        y += 18.0;

        self.section(g, &mut y, "OPTIMIZATION");
        let labels: Vec<&str> = Profile::ALL.iter().map(|p| p.label()).collect();
        let sel = Profile::ALL
            .iter()
            .position(|p| *p == cfg.profile)
            .unwrap_or(1);
        self.segmented(
            g,
            Rect::new(PAD, y, W - 2.0 * PAD, 32.0),
            &labels,
            sel,
            Hit::Profile,
        );
        y += 40.0;
        let consequence = match cfg.profile {
            Profile::Conservative => {
                "Only clears cache Windows was about to discard. Never trims apps."
            }
            Profile::Balanced => {
                "Clears low-value cache early and quiets idle background apps when memory gets tight."
            }
            Profile::Aggressive => {
                "Keeps lots of RAM free. Apps you return to may take a moment to wake up."
            }
        };
        y += g.text_wrapped(
            consequence,
            Rect::new(PAD, y, W - 2.0 * PAD, 40.0),
            Font::Caption,
            p.text3,
        ) + 8.0;
        let tiers = [
            "Quiet idle background apps",
            "Clear low-priority cache",
            "Clear standby cache when needed",
            "Trim idle apps under pressure",
            "Idle maintenance (combine, cache cap)",
        ];
        for (i, label) in tiers.iter().enumerate() {
            self.toggle_row(g, &mut y, label, cfg.auto_tiers[i], Hit::Tier(i as u8));
        }
        self.toggle_row(
            g,
            &mut y,
            "Game mode (protect full-screen apps)",
            cfg.game_mode,
            Hit::GameMode,
        );

        self.section(g, &mut y, "NOTIFICATIONS");
        self.toggle_row(
            g,
            &mut y,
            "Memory leak alerts",
            cfg.notify_leaks,
            Hit::Notify(0),
        );
        self.toggle_row(
            g,
            &mut y,
            "Commit and kernel warnings",
            cfg.notify_warnings,
            Hit::Notify(1),
        );
        self.toggle_row(
            g,
            &mut y,
            "Results of manual actions",
            cfg.notify_results,
            Hit::Notify(2),
        );

        self.section(g, &mut y, "APPEARANCE");
        if supports_backdrop() {
            self.toggle_row(
                g,
                &mut y,
                "Translucent (acrylic) background",
                cfg.translucent,
                Hit::Translucent,
            );
        } else {
            g.text(
                "Translucent background needs Windows 11 22H2 or later.",
                Rect::new(PAD, y, W - 2.0 * PAD, 20.0),
                Font::Caption,
                p.text3,
                Align::Left,
            );
            y += 26.0;
        }

        self.section(g, &mut y, "SYSTEM");
        let (status, btn) = if snap.task_registered {
            ("Starts at login with cleaning privileges.", "Remove")
        } else if snap.privileges.profile {
            ("Running with privileges for this session.", "Run at login")
        } else {
            ("Monitoring only — cleaning needs admin rights.", "Enable")
        };
        g.text(
            status,
            Rect::new(PAD, y, W - 2.0 * PAD - 112.0, 32.0),
            Font::Caption,
            p.text2,
            Align::Left,
        );
        self.secondary_button(
            g,
            Rect::new(W - PAD - 104.0, y, 104.0, 30.0),
            btn,
            Hit::TaskButton,
        );
        y += 40.0;
        if !snap.task_registered {
            self.toggle_row(
                g,
                &mut y,
                "Start at login (monitor only)",
                self.run_key,
                Hit::RunKey,
            );
        }
        g.text(
            "Exclusions & lists",
            Rect::new(PAD, y, 200.0, 30.0),
            Font::Body,
            p.text,
            Align::Left,
        );
        self.secondary_button(
            g,
            Rect::new(W - PAD - 104.0, y, 104.0, 30.0),
            "Edit…",
            Hit::EditExclusions,
        );
        y += 38.0;
        g.text(
            "Deep clean",
            Rect::new(PAD, y, 200.0, 30.0),
            Font::Body,
            p.text,
            Align::Left,
        );
        self.secondary_button(
            g,
            Rect::new(W - PAD - 104.0, y, 104.0, 30.0),
            "Run…",
            Hit::DeepClean,
        );
        y += 34.0;
        y += g.text_wrapped(
            "Empties every working set and cache at once. Expect a short slowdown while apps page back in.",
            Rect::new(PAD, y, W - 2.0 * PAD, 40.0),
            Font::Caption,
            p.text3,
        ) + 8.0;

        self.section(g, &mut y, "UPDATES");
        self.toggle_row(
            g,
            &mut y,
            "Check for updates automatically",
            cfg.auto_update,
            Hit::AutoUpdate,
        );
        let (status, btn, hit) = match &self.update {
            UpdateStatus::Idle => (
                "Not checked yet.".to_string(),
                "Check now",
                Hit::CheckUpdate,
            ),
            UpdateStatus::Checking => ("Checking…".to_string(), "Check now", Hit::CheckUpdate),
            UpdateStatus::UpToDate { at_ms } => (
                format!(
                    "Up to date · checked {}",
                    fmt_ago(now_ms().saturating_sub(*at_ms))
                ),
                "Check now",
                Hit::CheckUpdate,
            ),
            UpdateStatus::Available(o) => (
                format!("Version {} is available.", o.version),
                "Install",
                Hit::InstallUpdate,
            ),
            UpdateStatus::Installing(v) => {
                (format!("Installing {v}…"), "Check now", Hit::CheckUpdate)
            }
            UpdateStatus::Failed(e) => (
                format!("Couldn't update: {e}"),
                "Try again",
                Hit::CheckUpdate,
            ),
        };
        y += g
            .text_wrapped(
                &status,
                Rect::new(PAD, y + 6.0, W - 2.0 * PAD - 112.0, 40.0),
                Font::Caption,
                p.text2,
            )
            .max(30.0)
            - 30.0;
        self.secondary_button(g, Rect::new(W - PAD - 104.0, y, 104.0, 30.0), btn, hit);
        y += 38.0;
        y += g.text_wrapped(
            "Updates come from this project's GitHub releases and are checked against their published SHA-256 before installing.",
            Rect::new(PAD, y, W - 2.0 * PAD, 40.0),
            Font::Caption,
            p.text3,
        ) + 8.0;

        self.section(g, &mut y, "ABOUT");
        let about = format!(
            "MemManager {} · using {} (private {})",
            env!("CARGO_PKG_VERSION"),
            fmt_bytes(snap.self_working_set),
            fmt_bytes(snap.self_private)
        );
        g.text(
            &about,
            Rect::new(PAD, y, W - 2.0 * PAD, 18.0),
            Font::Caption,
            p.text3,
            Align::Left,
        );
        y += 20.0;
        let caps = snap.caps;
        let yes = |b: bool| if b { "yes" } else { "no" };
        let detail = format!(
            "Memory lists {} · list commands {} · file cache {} · page combining {} · kernel pool {} paged / {} nonpaged · {} app(s) at low priority",
            yes(caps.lists),
            yes(caps.mem_commands),
            yes(caps.file_cache),
            yes(caps.combine),
            fmt_bytes(snap.reading.kernel_paged),
            fmt_bytes(snap.reading.kernel_nonpaged),
            snap.lowered_count
        );
        y += g.text_wrapped(
            &detail,
            Rect::new(PAD, y, W - 2.0 * PAD, 60.0),
            Font::Caption,
            p.text3,
        ) + 16.0;

        self.end_scroll_view(g, first_hit, top, y);
    }
}
