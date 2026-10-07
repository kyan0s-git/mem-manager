//! Small helpers shared by every module.

use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::core::PCWSTR;

/// NUL-terminated UTF-16 copy of `s`.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Keeps a UTF-16 buffer alive while exposing it as `PCWSTR`.
pub struct WStr(Vec<u16>);

impl WStr {
    pub fn new(s: &str) -> Self {
        WStr(wide(s))
    }
    pub fn pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

/// Copies `s` into a fixed UTF-16 buffer (truncating, always NUL-terminated).
pub fn copy_to_fixed(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16()) {
        *d = c;
        n += 1;
    }
    let end = n.min(dst.len().saturating_sub(1));
    dst[end] = 0;
}

/// Monotonic milliseconds since boot.
pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// "1.4 GB", "820 MB", "12 KB" (binary units, Windows-style labels).
pub fn fmt_bytes(b: u64) -> String {
    const K: f64 = 1024.0;
    let b = b as f64;
    if b >= K * K * K {
        let g = b / (K * K * K);
        if g >= 100.0 {
            format!("{g:.0} GB")
        } else {
            format!("{g:.1} GB")
        }
    } else if b >= K * K {
        format!("{:.0} MB", b / (K * K))
    } else {
        format!("{:.0} KB", b / K)
    }
}

/// "4 min ago", "2 h ago", "just now".
pub fn fmt_ago(ms: u64) -> String {
    let s = ms / 1000;
    if s < 45 {
        "just now".into()
    } else if s < 3600 {
        format!("{} min ago", (s + 30) / 60)
    } else if s < 86_400 {
        format!("{} h ago", (s + 1800) / 3600)
    } else {
        format!("{} d ago", s / 86_400)
    }
}

/// "3 h", "45 min".
pub fn fmt_hours(h: f64) -> String {
    if h < 1.0 {
        format!("{:.0} min", (h * 60.0).max(1.0))
    } else if h < 48.0 {
        format!("{h:.0} h")
    } else {
        format!("{:.0} days", h / 24.0)
    }
}

/// Lower-cased file name of a path or image name.
pub fn image_key(name: &str) -> String {
    name.rsplit(['\\', '/'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// Display name: image name without ".exe".
pub fn display_name(image: &str) -> String {
    let base = image.rsplit(['\\', '/']).next().unwrap_or(image);
    base.strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base)
        .to_string()
}

/// Minimal JSON string escaping for the self-test report.
pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
