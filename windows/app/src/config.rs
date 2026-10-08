//! User settings, persisted as INI in `%APPDATA%\MemManager\config.ini`.

use memmanager_core::profile::Profile;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IconStyle {
    #[default]
    Ring,
    Bar,
    Number,
}

impl IconStyle {
    pub const ALL: [IconStyle; 3] = [IconStyle::Ring, IconStyle::Bar, IconStyle::Number];
    pub fn name(self) -> &'static str {
        match self {
            IconStyle::Ring => "ring",
            IconStyle::Bar => "bar",
            IconStyle::Number => "number",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            IconStyle::Ring => "Ring",
            IconStyle::Bar => "Bar",
            IconStyle::Number => "Number",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IconMetric {
    /// Memory ready for apps (free + cache), i.e. Windows "Available".
    #[default]
    Ready,
    Used,
    Commit,
}

impl IconMetric {
    pub const ALL: [IconMetric; 3] = [IconMetric::Ready, IconMetric::Used, IconMetric::Commit];
    pub fn name(self) -> &'static str {
        match self {
            IconMetric::Ready => "ready",
            IconMetric::Used => "used",
            IconMetric::Commit => "commit",
        }
    }
    pub fn parse(v: &str) -> Option<IconMetric> {
        match v.trim().to_ascii_lowercase().as_str() {
            "ready" | "available" => Some(IconMetric::Ready),
            "used" => Some(IconMetric::Used),
            "commit" => Some(IconMetric::Commit),
            _ => None,
        }
    }
    pub fn label(self, friendly: bool) -> &'static str {
        match (self, friendly) {
            (IconMetric::Ready, true) => "Ready GB",
            (IconMetric::Ready, false) => "Available",
            (IconMetric::Used, true) => "Apps %",
            (IconMetric::Used, false) => "In use",
            (IconMetric::Commit, _) => "Commit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorPreset {
    #[default]
    System,
    Traffic,
    Mono,
    ColorBlind,
    Custom,
}

impl ColorPreset {
    pub const PICKABLE: [ColorPreset; 4] = [
        ColorPreset::System,
        ColorPreset::Traffic,
        ColorPreset::Mono,
        ColorPreset::ColorBlind,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ColorPreset::System => "system",
            ColorPreset::Traffic => "traffic",
            ColorPreset::Mono => "mono",
            ColorPreset::ColorBlind => "colorblind",
            ColorPreset::Custom => "custom",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ColorPreset::System => "System",
            ColorPreset::Traffic => "Traffic",
            ColorPreset::Mono => "Mono",
            ColorPreset::ColorBlind => "Color-safe",
            ColorPreset::Custom => "Custom",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub profile: Profile,
    pub icon_style: IconStyle,
    pub icon_metric: IconMetric,
    pub preset: ColorPreset,
    /// Custom colours (0xRRGGBB) for normal, elevated, high, critical.
    pub custom_colors: [u32; 4],
    /// Automatic action toggles for tiers 1..=5.
    pub auto_tiers: [bool; 5],
    pub game_mode: bool,
    pub translucent: bool,
    /// Friendly values (default): "ready for apps", cache shown as a speed-up,
    /// calm state names. Off: Windows' standard terms (In use, Standby, …).
    pub friendly: bool,
    /// The one-time "why cache counts as ready" note has been seen.
    pub explained: bool,
    /// Check GitHub for a newer release once a day.
    pub auto_update: bool,
    pub notify_leaks: bool,
    pub notify_warnings: bool,
    pub notify_results: bool,
    /// Image names (lower case) never touched by any action.
    pub exclusions: Vec<String>,
    /// Image names with relaxed leak thresholds.
    pub heavy: Vec<String>,
    /// Image names that never raise leak alerts.
    pub ignore_leaks: Vec<String>,
}

pub const DEFAULT_HEAVY: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "code.exe",
    "devenv.exe",
    "idea64.exe",
    "rider64.exe",
    "pycharm64.exe",
    "vmmem",
    "vmmemwsl",
    "vmwp.exe",
    "vmware-vmx.exe",
    "virtualboxvm.exe",
    "photoshop.exe",
    "blender.exe",
    "obs64.exe",
    "python.exe",
    "ollama.exe",
];

impl Default for Config {
    fn default() -> Self {
        Self {
            profile: Profile::Balanced,
            icon_style: IconStyle::Ring,
            icon_metric: IconMetric::Ready,
            preset: ColorPreset::System,
            custom_colors: [0x0078D4, 0xB58400, 0xC8551B, 0xC42B1C],
            auto_tiers: [true; 5],
            game_mode: true,
            translucent: true,
            friendly: true,
            explained: false,
            auto_update: true,
            notify_leaks: true,
            notify_warnings: true,
            notify_results: false,
            exclusions: Vec::new(),
            heavy: DEFAULT_HEAVY.iter().map(|s| s.to_string()).collect(),
            ignore_leaks: Vec::new(),
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_color(v: &str) -> Option<u32> {
    let v = v.trim().trim_start_matches('#');
    (v.len() == 6)
        .then(|| u32::from_str_radix(v, 16).ok())
        .flatten()
}

fn parse_list(v: &str) -> Vec<String> {
    v.split([';', ','])
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("MemManager").join("config.ini"))
    }

    pub fn load() -> Config {
        Config::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| Config::parse(&s))
            .unwrap_or_default()
    }

    pub fn parse(text: &str) -> Config {
        let mut c = Config::default();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_ascii_lowercase();
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match (section.as_str(), k.as_str()) {
                ("general", "profile") => {
                    if let Some(p) = Profile::parse(v) {
                        c.profile = p;
                    }
                }
                ("icon", "style") => {
                    if let Some(s) = IconStyle::ALL
                        .into_iter()
                        .find(|s| s.name() == v.to_ascii_lowercase())
                    {
                        c.icon_style = s;
                    }
                }
                ("icon", "metric") => {
                    if let Some(m) = IconMetric::parse(v) {
                        c.icon_metric = m;
                    }
                }
                ("icon", "preset") => {
                    let all = [
                        ColorPreset::System,
                        ColorPreset::Traffic,
                        ColorPreset::Mono,
                        ColorPreset::ColorBlind,
                        ColorPreset::Custom,
                    ];
                    if let Some(p) = all.into_iter().find(|p| p.name() == v.to_ascii_lowercase()) {
                        c.preset = p;
                    }
                }
                ("icon", key) if key.starts_with("color_") => {
                    let idx = match key {
                        "color_normal" => Some(0),
                        "color_elevated" => Some(1),
                        "color_high" => Some(2),
                        "color_critical" => Some(3),
                        _ => None,
                    };
                    if let (Some(i), Some(col)) = (idx, parse_color(v)) {
                        c.custom_colors[i] = col;
                    }
                }
                ("behaviour", key) if key.starts_with("tier") => {
                    if let (Ok(n), Some(b)) = (key[4..].parse::<usize>(), parse_bool(v)) {
                        if (1..=5).contains(&n) {
                            c.auto_tiers[n - 1] = b;
                        }
                    }
                }
                ("behaviour", "game_mode") => c.game_mode = parse_bool(v).unwrap_or(c.game_mode),
                ("behaviour", "exclusions") => c.exclusions = parse_list(v),
                ("behaviour", "heavy") => c.heavy = parse_list(v),
                ("behaviour", "ignore_leaks") => c.ignore_leaks = parse_list(v),
                ("appearance", "translucent") => {
                    c.translucent = parse_bool(v).unwrap_or(c.translucent)
                }
                ("appearance", "values") => match v.to_ascii_lowercase().as_str() {
                    "friendly" => c.friendly = true,
                    "standard" => c.friendly = false,
                    _ => {}
                },
                ("appearance", "explained") => c.explained = parse_bool(v).unwrap_or(c.explained),
                ("updates", "auto_check") => c.auto_update = parse_bool(v).unwrap_or(c.auto_update),
                ("notify", "leaks") => c.notify_leaks = parse_bool(v).unwrap_or(c.notify_leaks),
                ("notify", "warnings") => {
                    c.notify_warnings = parse_bool(v).unwrap_or(c.notify_warnings)
                }
                ("notify", "results") => {
                    c.notify_results = parse_bool(v).unwrap_or(c.notify_results)
                }
                _ => {}
            }
        }
        c
    }

    pub fn serialize(&self) -> String {
        let b = |v: bool| if v { "1" } else { "0" };
        let mut s = String::from("; MemManager settings. Lists are separated by ';'.\n");
        s += &format!("[general]\nprofile={}\n\n", self.profile.name());
        s += &format!(
            "[icon]\nstyle={}\nmetric={}\npreset={}\n",
            self.icon_style.name(),
            self.icon_metric.name(),
            self.preset.name()
        );
        for (name, col) in ["normal", "elevated", "high", "critical"]
            .iter()
            .zip(self.custom_colors)
        {
            s += &format!("color_{name}=#{col:06X}\n");
        }
        s += "\n[behaviour]\n";
        for (i, t) in self.auto_tiers.iter().enumerate() {
            s += &format!("tier{}={}\n", i + 1, b(*t));
        }
        s += &format!("game_mode={}\n", b(self.game_mode));
        s += &format!("exclusions={}\n", self.exclusions.join(";"));
        s += &format!("heavy={}\n", self.heavy.join(";"));
        s += &format!("ignore_leaks={}\n", self.ignore_leaks.join(";"));
        s += &format!(
            "\n[appearance]\ntranslucent={}\nvalues={}\nexplained={}\n",
            b(self.translucent),
            if self.friendly {
                "friendly"
            } else {
                "standard"
            },
            b(self.explained)
        );
        s += &format!("\n[updates]\nauto_check={}\n", b(self.auto_update));
        s += &format!(
            "\n[notify]\nleaks={}\nwarnings={}\nresults={}\n",
            b(self.notify_leaks),
            b(self.notify_warnings),
            b(self.notify_results)
        );
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = Config::path() else {
            return Ok(());
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.with_extension("ini.tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(tmp, p)
    }

    /// Switches between friendly and Windows-standard values. The icon follows:
    /// "Ready" pairs with friendly values and "In use" with the standard ones.
    pub fn set_friendly(&mut self, on: bool) {
        self.friendly = on;
        match (on, self.icon_metric) {
            (false, IconMetric::Ready) => self.icon_metric = IconMetric::Used,
            (true, IconMetric::Used) => self.icon_metric = IconMetric::Ready,
            _ => {}
        }
    }

    pub fn is_excluded(&self, image_key: &str) -> bool {
        self.exclusions.iter().any(|e| e == image_key)
    }

    pub fn is_heavy(&self, image_key: &str) -> bool {
        self.heavy.iter().any(|e| e == image_key)
    }

    pub fn ignores_leaks(&self, image_key: &str) -> bool {
        self.ignore_leaks.iter().any(|e| e == image_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Config {
            profile: Profile::Aggressive,
            icon_style: IconStyle::Number,
            icon_metric: IconMetric::Commit,
            preset: ColorPreset::Custom,
            ..Config::default()
        };
        c.custom_colors[2] = 0x123456;
        c.auto_tiers[3] = false;
        c.exclusions = vec!["game.exe".into(), "obs64.exe".into()];
        c.notify_results = true;
        c.friendly = false;
        c.explained = true;
        c.auto_update = false;
        assert_eq!(Config::parse(&c.serialize()), c);
    }

    #[test]
    fn friendly_is_default_and_reversible() {
        let mut c = Config::default();
        assert!(c.friendly && c.auto_update && !c.explained);
        assert_eq!(c.icon_metric, IconMetric::Ready);
        c.set_friendly(false);
        assert_eq!(c.icon_metric, IconMetric::Used);
        c.set_friendly(true);
        assert_eq!(c.icon_metric, IconMetric::Ready);
        c.icon_metric = IconMetric::Commit;
        c.set_friendly(false);
        assert_eq!(c.icon_metric, IconMetric::Commit);
        // Old config files said "available" for the same value.
        assert_eq!(
            Config::parse("[icon]\nmetric=available\n").icon_metric,
            IconMetric::Ready
        );
    }

    #[test]
    fn tolerates_garbage() {
        let c = Config::parse("[general]\nprofile=nope\n[icon]\ncolor_high=#zzz\nrandom\n");
        assert_eq!(c, Config::default());
    }
}
