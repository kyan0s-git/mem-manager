//! What the numbers mean to a person (docs/architecture/ui-design.md §7).
//!
//! The default "friendly" presentation leads with memory that is *ready for
//! apps* (free memory plus the cache Windows hands out instantly) and shows
//! the cache as a speed-up, not as memory in use. Every figure is a real OS
//! value; only the grouping and the words differ. The "Windows standard"
//! option keeps Task Manager's terms. Pure functions, unit-tested on any OS.

use memmanager_core::state::PressureState;

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

/// "48,210".
pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Physical memory as a person thinks about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub total: u64,
    /// Memory apps and Windows are using, including changes still being saved (modified list).
    pub apps: u64,
    /// The standby list: recently used files and app data kept in memory, handed out instantly.
    pub cache: u64,
    /// Free and zeroed pages.
    pub free: u64,
}

impl Split {
    pub fn new(total: u64, standby: u64, free_zero: u64) -> Split {
        let cache = standby.min(total);
        let free = free_zero.min(total - cache);
        Split {
            total,
            apps: total - cache - free,
            cache,
            free,
        }
    }

    /// Ready for apps right now: Windows' own "Available" (free + standby).
    pub fn ready(&self) -> u64 {
        self.cache + self.free
    }

    pub fn apps_fraction(&self) -> f64 {
        self.apps as f64 / self.total.max(1) as f64
    }

    pub fn cache_fraction(&self) -> f64 {
        self.cache as f64 / self.total.max(1) as f64
    }
}

/// State names: calm words by default, the engine's terms in standard mode.
pub fn state_name(st: PressureState, friendly: bool) -> &'static str {
    if !friendly {
        return st.name();
    }
    match st {
        PressureState::Normal => "Comfortable",
        PressureState::Elevated => "Busy",
        PressureState::High => "Tight",
        PressureState::Critical => "Critical",
    }
}

/// How much new app memory the cache made room for between two (apps, cache)
/// fractions, in bytes. Zero unless apps grew while the cache shrank.
pub fn room_made(first: (f32, f32), last: (f32, f32), total: u64) -> u64 {
    let grew = last.0 - first.0;
    let gave = first.1 - last.1;
    if grew <= 0.0 || gave <= 0.0 {
        return 0;
    }
    (grew.min(gave) as f64 * total as f64) as u64
}

/// "Cache made room for 1.2 GB of app memory in the last 10 minutes."
pub fn room_line(bytes: u64, window_ms: u64) -> Option<String> {
    if bytes < 128 << 20 {
        return None;
    }
    Some(format!(
        "Cache made room for {} of app memory in the last {}.",
        fmt_bytes(bytes),
        fmt_window(window_ms)
    ))
}

/// "10 minutes", "1 hour".
pub fn fmt_window(ms: u64) -> String {
    let min = ((ms + 30_000) / 60_000).max(1);
    match min {
        1 => "minute".into(),
        m if m >= 59 => "hour".into(),
        m => format!("{m} minutes"),
    }
}

/// How often recently used data came back from memory instead of disk.
/// `hits`: page faults resolved from the standby/modified lists;
/// `disk`: pages read from disk, over `window_ms`.
pub fn cache_line(hits: u64, disk: u64, window_ms: u64) -> Option<String> {
    if window_ms < 60_000 || hits < 100 {
        return None;
    }
    Some(format!(
        "In the last {}, apps got data back from cache {} times instead of reading it from disk ({} disk reads).",
        fmt_window(window_ms),
        fmt_count(hits),
        fmt_count(disk)
    ))
}

/// Shown before "Optimize now" when memory is comfortable (friendly mode only):
/// what optimizing would do, and that it won't help right now.
pub fn optimize_preview(st: PressureState, s: &Split, friendly: bool) -> Option<String> {
    if !friendly || st != PressureState::Normal {
        return None;
    }
    Some(format!(
        "Memory is comfortable: {} is ready for apps, including {} of speed-up cache.\n\n\
         Optimizing now would trim apps you haven't used for a while (they take a moment to wake up) \
         and clear cache Windows has marked as low-value. It won't make anything faster right now, \
         and MemManager already steps in on its own when memory gets tight.\n\nOptimize anyway?",
        fmt_bytes(s.ready()),
        fmt_bytes(s.cache)
    ))
}

/// "3 h", "45 min", "2 days".
pub fn fmt_idle(min: f64) -> String {
    if min < 60.0 {
        format!("{:.0} min", min.max(1.0))
    } else if min < 48.0 * 60.0 {
        format!("{:.0} h", min / 60.0)
    } else {
        format!("{:.0} days", min / 1440.0)
    }
}

/// Tray tooltip (two lines, ≤ 127 characters).
pub fn tooltip(
    friendly: bool,
    st: PressureState,
    s: &Split,
    in_use: u64,
    commit_pct: f64,
) -> String {
    if friendly {
        format!(
            "{} ready for apps · {}\nApps {} · Speed-up cache {}",
            fmt_bytes(s.ready()),
            state_name(st, true),
            fmt_bytes(s.apps),
            fmt_bytes(s.cache)
        )
    } else {
        format!(
            "Memory {:.0}% in use · {}\nCommit {:.0}% · {} available",
            in_use as f64 * 100.0 / s.total.max(1) as f64,
            st.name(),
            commit_pct,
            fmt_bytes(s.ready())
        )
    }
}

/// "Teams, Slack, Spotify +2".
pub fn apps_list(names: &[String]) -> String {
    let mut s = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 3 {
        s += &format!(" +{}", names.len() - 3);
    }
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Neutral,
    Warn,
}

/// What an action measurably changed: "1.1 GB back · no slowdown".
pub fn result_text(freed: u64, refault_ratio: f64, pending: bool) -> (String, Tone) {
    let back = if freed >= 1 << 20 {
        format!("{} back", fmt_bytes(freed))
    } else {
        "nothing freed".to_string()
    };
    if pending {
        (format!("{back} · measuring…"), Tone::Neutral)
    } else if refault_ratio < 0.05 {
        (format!("{back} · no slowdown"), Tone::Good)
    } else if refault_ratio < 0.25 {
        (format!("{back} · a little was read back"), Tone::Neutral)
    } else {
        (
            format!("{back} · caused a slowdown, backing off"),
            Tone::Warn,
        )
    }
}

/// One line over the activity list: "3 actions · 2.3 GB back · no slowdowns".
pub fn summary(actions: usize, freed: u64, slowdowns: usize) -> String {
    if actions == 0 {
        return "No actions yet.".into();
    }
    let n = if actions == 1 {
        "1 action".to_string()
    } else {
        format!("{actions} actions")
    };
    let slow = match slowdowns {
        0 => "no slowdowns".to_string(),
        1 => "1 slowdown".to_string(),
        k => format!("{k} slowdowns"),
    };
    format!("{n} · {} back · {slow}", fmt_bytes(freed))
}

/// What MemManager is doing right now, in a few words.
pub fn now_line(
    paused: bool,
    monitor_only: bool,
    game: bool,
    acting: bool,
    st: PressureState,
) -> &'static str {
    if acting {
        "Working on it…"
    } else if paused {
        "Paused · not acting on its own"
    } else if game {
        "Game mode · staying out of the way"
    } else if monitor_only {
        "Watching only · cleaning isn't enabled"
    } else {
        match st {
            PressureState::Normal => "Watching · nothing to do",
            PressureState::Elevated => "Memory is busy · ready to step in",
            PressureState::High => "Memory is tight · acting where it helps",
            PressureState::Critical => "Memory is critical · acting now",
        }
    }
}

/// Horizontal position (0 = left, 1 = now) of an event `age_ms` ago on a
/// graph spanning `window_ms`; `None` if it is off the graph.
pub fn marker_pos(age_ms: u64, window_ms: u64) -> Option<f32> {
    (window_ms > 0 && age_ms <= window_ms).then(|| 1.0 - age_ms as f32 / window_ms as f32)
}

/// Explainer page linked from the dashboard.
pub const EXPLAINER_URL: &str =
    "https://github.com/kyan0s-git/mem-manager/blob/HEAD/docs/understanding-memory.md";

#[cfg(test)]
mod tests {
    use super::*;

    const G: u64 = 1 << 30;

    #[test]
    fn split_counts_cache_as_ready() {
        let s = Split::new(16 * G, 6 * G, 2 * G);
        assert_eq!(s.apps, 8 * G);
        assert_eq!(s.cache, 6 * G);
        assert_eq!(s.free, 2 * G);
        assert_eq!(s.ready(), 8 * G);
        assert!((s.apps_fraction() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn split_never_underflows() {
        let s = Split::new(4 * G, 5 * G, 3 * G);
        assert_eq!(s.cache, 4 * G);
        assert_eq!(s.free, 0);
        assert_eq!(s.apps, 0);
        assert_eq!(Split::new(0, 0, 0).apps_fraction(), 0.0);
    }

    #[test]
    fn names() {
        assert_eq!(state_name(PressureState::Normal, true), "Comfortable");
        assert_eq!(state_name(PressureState::High, true), "Tight");
        assert_eq!(state_name(PressureState::Elevated, false), "Elevated");
    }

    #[test]
    fn room_only_when_cache_gave_way() {
        let near = |a: u64, b: u64| (a as f64 - b as f64).abs() < b as f64 * 0.01;
        assert!(near(room_made((0.4, 0.3), (0.5, 0.2), 10 * G), G));
        assert!(near(room_made((0.4, 0.3), (0.5, 0.25), 10 * G), G / 2));
        assert_eq!(room_made((0.4, 0.3), (0.3, 0.2), 10 * G), 0);
        assert_eq!(room_made((0.4, 0.3), (0.5, 0.4), 10 * G), 0);
        assert!(room_line(64 << 20, 600_000).is_none());
        assert_eq!(
            room_line(G + G / 5, 600_000).unwrap(),
            "Cache made room for 1.2 GB of app memory in the last 10 minutes."
        );
    }

    #[test]
    fn counts_and_windows() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(48_210), "48,210");
        assert_eq!(fmt_count(1_234_567), "1,234,567");
        assert_eq!(fmt_window(3_600_000), "hour");
        assert_eq!(fmt_window(60_000), "minute");
        assert!(cache_line(50, 0, 3_600_000).is_none());
        assert!(cache_line(5000, 3, 30_000).is_none());
        assert!(
            cache_line(48_210, 312, 3_600_000)
                .unwrap()
                .contains("48,210 times")
        );
    }

    #[test]
    fn preview_only_when_comfortable_and_friendly() {
        let s = Split::new(16 * G, 6 * G, 2 * G);
        assert!(optimize_preview(PressureState::Normal, &s, true).is_some());
        assert!(optimize_preview(PressureState::Normal, &s, false).is_none());
        assert!(optimize_preview(PressureState::High, &s, true).is_none());
    }

    #[test]
    fn tooltips_fit() {
        let s = Split::new(16 * G, 6 * G, 2 * G);
        for f in [true, false] {
            let t = tooltip(f, PressureState::Critical, &s, 8 * G, 99.0);
            assert!(t.encode_utf16().count() < 128, "{t}");
        }
        assert!(tooltip(true, PressureState::Normal, &s, 8 * G, 50.0).starts_with("8.0 GB ready"));
        assert!(
            tooltip(false, PressureState::Normal, &s, 8 * G, 50.0).starts_with("Memory 50% in use")
        );
    }

    #[test]
    fn activity_text() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(apps_list(&n(&["A", "B"])), "A, B");
        assert_eq!(apps_list(&n(&["A", "B", "C", "D", "E"])), "A, B, C +2");
        assert_eq!(apps_list(&[]), "");
        assert_eq!(
            result_text(G, 0.0, false),
            ("1.0 GB back · no slowdown".into(), Tone::Good)
        );
        assert_eq!(result_text(0, 0.5, false).1, Tone::Warn);
        assert!(result_text(0, 0.0, true).0.ends_with("measuring…"));
        assert_eq!(summary(0, 0, 0), "No actions yet.");
        assert_eq!(summary(3, G, 0), "3 actions · 1.0 GB back · no slowdowns");
        assert_eq!(summary(1, 0, 1), "1 action · 0 KB back · 1 slowdown");
        assert_eq!(
            now_line(false, false, false, false, PressureState::Normal),
            "Watching · nothing to do"
        );
        assert_eq!(
            now_line(true, false, true, true, PressureState::High),
            "Working on it…"
        );
        assert_eq!(marker_pos(0, 600_000), Some(1.0));
        assert_eq!(marker_pos(300_000, 600_000), Some(0.5));
        assert_eq!(marker_pos(700_000, 600_000), None);
    }

    #[test]
    fn bytes() {
        assert_eq!(fmt_bytes(1536 << 20), "1.5 GB");
        assert_eq!(fmt_bytes(820 << 20), "820 MB");
    }
}
