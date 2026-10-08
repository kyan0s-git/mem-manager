//! Notification-area icon: dynamic rendering, tooltip, balloons, theme tracking.

use crate::config::{Config, IconMetric, IconStyle};
use crate::icon::{self, Canvas, IconSpec};
use crate::palette::{Appearance, Rgba, state_color};
use crate::shared::Snapshot;
use crate::util::{WStr, copy_to_fixed};
use memmanager_core::state::PressureState;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CENTER,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FF_DONTCARE,
    GdiFlush, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIIF_RESPECT_QUIET_TIME,
    NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, HICON, ICONINFO, SM_CXSMICON, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW, WM_APP,
};

pub const WM_APP_TRAY: u32 = WM_APP + 1;
const UID: u32 = 1;

fn reg_dword(path: &str, value: &str) -> Option<u32> {
    let p = WStr::new(path);
    let v = WStr::new(value);
    let mut data = 0u32;
    let mut len = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            p.pcwstr(),
            v.pcwstr(),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut len),
        )
    };
    r.is_ok().then_some(data)
}

/// Reads the current theme from the registry and system parameters.
pub fn appearance() -> Appearance {
    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    let taskbar_light = reg_dword(PERSONALIZE, "SystemUsesLightTheme").is_some_and(|v| v != 0);
    let apps_light = reg_dword(PERSONALIZE, "AppsUseLightTheme").is_none_or(|v| v != 0);
    // AccentColor is 0xAABBGGRR.
    let accent = reg_dword(r"Software\Microsoft\Windows\DWM", "AccentColor")
        .map(|v| Rgba::rgb(((v & 0xFF) << 16) | (v & 0xFF00) | ((v >> 16) & 0xFF)))
        .unwrap_or(Rgba::rgb(0x0078D4));
    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let high_contrast = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some((&mut hc as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && (hc.dwFlags & HCF_HIGHCONTRASTON).0 != 0
    };
    Appearance {
        taskbar_light,
        apps_light,
        accent,
        high_contrast,
    }
}

/// Text coverage mask rendered with GDI (grayscale anti-aliasing).
fn text_mask(size: usize, text: &str) -> Option<Vec<f32>> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let chars = text.chars().count().max(1);
        let height = match chars {
            1 | 2 => size as f32 * 0.78,
            3 => size as f32 * 0.60,
            _ => size as f32 * 0.48,
        };
        let face = WStr::new("Segoe UI Variable Display");
        let mut font = CreateFontW(
            -(height.round() as i32),
            0,
            0,
            0,
            650,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            FF_DONTCARE.0 as u32,
            face.pcwstr(),
        );
        if font.is_invalid() {
            let f2 = WStr::new("Segoe UI");
            font = CreateFontW(
                -(height.round() as i32),
                0,
                0,
                0,
                600,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                FF_DONTCARE.0 as u32,
                f2.pcwstr(),
            );
        }
        let old_bmp = SelectObject(dc, HGDIOBJ(bmp.0));
        let old_font = SelectObject(dc, HGDIOBJ(font.0));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut rc = RECT {
            left: -2,
            top: 0,
            right: size as i32 + 2,
            bottom: size as i32,
        };
        let mut w: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut w,
            &mut rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        let _ = GdiFlush();
        let px = std::slice::from_raw_parts(bits.cast::<u8>(), size * size * 4);
        let mask = (0..size * size)
            .map(|i| {
                let (b, g, r) = (px[i * 4] as f32, px[i * 4 + 1] as f32, px[i * 4 + 2] as f32);
                (b.max(g).max(r)) / 255.0
            })
            .collect();
        SelectObject(dc, old_font);
        SelectObject(dc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(dc);
        Some(mask)
    }
}

fn hicon_from_canvas(c: &Canvas) -> Option<HICON> {
    let size = c.size;
    let pixels = c.to_bgra_straight();
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
        let zeros = vec![0u8; size.div_ceil(16) * 2 * size];
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, Some(zeros.as_ptr().cast()));
        let ii = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&ii).ok();
        let _ = DeleteObject(HGDIOBJ(mask.0));
        let _ = DeleteObject(HGDIOBJ(color.0));
        icon
    }
}

/// Renders the icon for one style. `text` is used by the number style.
pub fn render_canvas(size: usize, text: &str, spec: &IconSpec, style: IconStyle) -> Option<Canvas> {
    match style {
        IconStyle::Ring => Some(icon::draw_ring(spec)),
        IconStyle::Bar => Some(icon::draw_bar(spec)),
        IconStyle::Number => {
            let mut c = Canvas::new(size);
            let mask = text_mask(size, text)?;
            if spec.paused {
                c.fill_mask(spec.track.with_alpha(0.8), &mask);
            } else {
                c.fill_mask(spec.color, &mask);
            }
            // Shape cue beyond colour: underline at High, thicker at Critical.
            if spec.state >= PressureState::High && !spec.paused {
                let h = if spec.state == PressureState::Critical {
                    0.16
                } else {
                    0.08
                };
                let n = size as f32;
                c.fill(spec.color, |x, y| {
                    y >= n * (1.0 - h) && x >= n * 0.15 && x <= n * 0.85
                });
            }
            Some(c)
        }
    }
}

pub fn render_icon(size: usize, text: &str, spec: &IconSpec, metric: IconMetric) -> Option<HICON> {
    let _ = metric;
    let style = if text.is_empty() {
        IconStyle::Ring
    } else {
        IconStyle::Number
    };
    hicon_from_canvas(&render_canvas(size, text, spec, style)?)
}

/// What the icon shows for a snapshot: (fraction for ring/bar, text for number).
/// With friendly values the ring always fills with app memory only, so a full
/// cache never makes the icon look full; standard values keep Task Manager's.
pub fn icon_value(snap: &Snapshot, metric: IconMetric, friendly: bool) -> (f32, String) {
    let r = &snap.reading;
    let sp = r.split();
    let pct = |f: f32| format!("{}", (f * 100.0).round() as u32);
    match metric {
        IconMetric::Used => {
            let f = if friendly {
                sp.apps_fraction()
            } else {
                r.used_fraction()
            } as f32;
            (f, pct(f))
        }
        IconMetric::Commit => {
            let f = r.commit_fraction() as f32;
            (f, pct(f))
        }
        IconMetric::Ready => {
            let gb = sp.ready() as f64 / (1u64 << 30) as f64;
            let text = if gb >= 10.0 {
                format!("{}", gb.round() as u32)
            } else {
                format!("{gb:.1}")
            };
            let f = if friendly {
                sp.apps_fraction()
            } else {
                sp.ready() as f64 / r.total.max(1) as f64
            };
            (f as f32, text)
        }
    }
}

#[derive(Clone, PartialEq)]
struct IconKey {
    size: usize,
    style: IconStyle,
    text: String,
    frac_pct: u32,
    state: PressureState,
    color: u32,
    track: u32,
    paused: bool,
    limited: bool,
    acting: bool,
}

pub struct Tray {
    hwnd: HWND,
    added: bool,
    icon: Option<HICON>,
    key: Option<IconKey>,
    tip: String,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Tray {
        Tray {
            hwnd,
            added: false,
            icon: None,
            key: None,
            tip: "MemManager".into(),
        }
    }

    fn base(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: UID,
            ..Default::default()
        }
    }

    pub fn add(&mut self) {
        let mut nid = self.base();
        nid.uFlags = NIF_MESSAGE
            | NIF_TIP
            | NIF_SHOWTIP
            | if self.icon.is_some() {
                NIF_ICON
            } else {
                Default::default()
            };
        nid.uCallbackMessage = WM_APP_TRAY;
        if let Some(i) = self.icon {
            nid.hIcon = i;
        }
        copy_to_fixed(&mut nid.szTip, &self.tip);
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                self.added = true;
                nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
            }
        }
    }

    /// Explorer restarted: the icon must be added again.
    pub fn readd(&mut self) {
        self.added = false;
        self.add();
    }

    pub fn remove(&mut self) {
        if self.added {
            let nid = self.base();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }
            self.added = false;
        }
        if let Some(i) = self.icon.take() {
            unsafe {
                let _ = DestroyIcon(i);
            }
        }
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }

    pub fn update(&mut self, snap: &Snapshot, cfg: &Config, ap: &Appearance) {
        let dpi = unsafe { GetDpiForWindow(self.hwnd) }.max(96);
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.clamp(16, 64) as usize;
        let (frac, text) = icon_value(snap, cfg.icon_metric, cfg.friendly);
        let state = snap.state;
        let color = if snap.acting {
            ap.accent.mix(
                Rgba::rgb(0xFFFFFF),
                if ap.taskbar_light { 0.0 } else { 0.25 },
            )
        } else {
            state_color(cfg.preset, &cfg.custom_colors, state, ap)
        };
        let track = if ap.taskbar_light {
            Rgba::with_a(0x000000, 0.22)
        } else {
            Rgba::with_a(0xFFFFFF, 0.28)
        };
        let limited = !snap.privileges.profile;
        let key = IconKey {
            size,
            style: cfg.icon_style,
            text: if cfg.icon_style == IconStyle::Number {
                text.clone()
            } else {
                String::new()
            },
            frac_pct: (frac * 100.0).round() as u32,
            state,
            color: color.to_hex(),
            track: track.to_hex(),
            paused: snap.paused,
            limited,
            acting: snap.acting,
        };
        let tip = tooltip(snap, cfg.friendly);
        let icon_changed = self.key.as_ref() != Some(&key);
        if !icon_changed && tip == self.tip && self.added {
            return;
        }
        let mut nid = self.base();
        nid.uFlags = NIF_TIP | NIF_SHOWTIP;
        if icon_changed {
            let spec = IconSpec {
                size,
                fraction: frac,
                state,
                color,
                track,
                paused: snap.paused,
                limited,
            };
            if let Some(canvas) = render_canvas(size, &text, &spec, cfg.icon_style) {
                if let Some(h) = hicon_from_canvas(&canvas) {
                    if let Some(old) = self.icon.replace(h) {
                        unsafe {
                            let _ = DestroyIcon(old);
                        }
                    }
                    nid.uFlags |= NIF_ICON;
                    nid.hIcon = h;
                    self.key = Some(key);
                }
            }
        }
        self.tip = tip;
        copy_to_fixed(&mut nid.szTip, &self.tip);
        if !self.added {
            self.add();
            return;
        }
        unsafe {
            if !Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool() {
                self.added = false;
                self.add();
            }
        }
    }

    pub fn balloon(&self, title: &str, text: &str, warning: bool) {
        if !self.added {
            return;
        }
        let mut nid = self.base();
        nid.uFlags = NIF_INFO;
        copy_to_fixed(&mut nid.szInfoTitle, title);
        copy_to_fixed(&mut nid.szInfo, text);
        nid.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO } | NIIF_RESPECT_QUIET_TIME;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// Screen rectangle of the icon (for anchoring the flyout).
    pub fn anchor(&self) -> Option<RECT> {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: UID,
            ..Default::default()
        };
        unsafe { Shell_NotifyIconGetRect(&id).ok() }
    }
}

pub fn tooltip(snap: &Snapshot, friendly: bool) -> String {
    let r = &snap.reading;
    if r.total == 0 {
        return "MemManager".into();
    }
    let mut s = crate::values::tooltip(
        friendly,
        snap.state,
        &r.split(),
        r.in_use,
        r.commit_fraction() * 100.0,
    );
    if snap.paused {
        s.push_str("\nPaused");
    } else if !snap.privileges.profile {
        s.push_str("\nMonitor only");
    }
    s
}
