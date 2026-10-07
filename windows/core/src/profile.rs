//! Profile tables (policy-engine §6): the user's RAM-vs-cache slider.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Profile {
    Conservative,
    #[default]
    Balanced,
    Aggressive,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeakThresholds {
    /// Minimum Theil–Sen slope, MiB per hour.
    pub slope_mib_h: f64,
    /// Minimum absolute growth over the window, MiB.
    pub abs_mib: f64,
    /// Minimum growth relative to the baseline size.
    pub rel: f64,
}

impl Profile {
    pub const ALL: [Profile; 3] = [
        Profile::Conservative,
        Profile::Balanced,
        Profile::Aggressive,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Profile::Conservative => "conservative",
            Profile::Balanced => "balanced",
            Profile::Aggressive => "aggressive",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Profile::Conservative => "Conservative",
            Profile::Balanced => "Balanced",
            Profile::Aggressive => "Aggressive",
        }
    }

    pub fn parse(s: &str) -> Option<Profile> {
        Profile::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s.trim()))
    }

    pub fn leak_thresholds(self) -> LeakThresholds {
        match self {
            Profile::Conservative => LeakThresholds {
                slope_mib_h: 200.0,
                abs_mib: 512.0,
                rel: 0.40,
            },
            Profile::Balanced => LeakThresholds {
                slope_mib_h: 100.0,
                abs_mib: 256.0,
                rel: 0.25,
            },
            Profile::Aggressive => LeakThresholds {
                slope_mib_h: 50.0,
                abs_mib: 128.0,
                rel: 0.15,
            },
        }
    }

    /// Windows: fraction of RAM that should be immediately available.
    pub fn free_target_frac(self) -> f64 {
        match self {
            Profile::Conservative => 0.03,
            Profile::Balanced => 0.06,
            Profile::Aggressive => 0.10,
        }
    }

    /// Highest action tier that may run automatically.
    pub fn max_auto_tier(self) -> u8 {
        match self {
            Profile::Conservative => 2,
            Profile::Balanced => 4,
            Profile::Aggressive => 5,
        }
    }

    /// Full standby purge bucket: (capacity, refill per hour).
    pub fn standby_bucket(self) -> (f64, f64) {
        match self {
            Profile::Conservative => (1.0, 1.0),
            Profile::Balanced => (2.0, 4.0),
            Profile::Aggressive => (4.0, 12.0),
        }
    }

    /// Selective-trim candidates must have been idle at least this long.
    pub fn trim_idle_min(self) -> u32 {
        match self {
            Profile::Conservative => 30,
            Profile::Balanced => 10,
            Profile::Aggressive => 3,
        }
    }

    /// macOS: automatic Nudge at High (helper required).
    pub fn mac_auto_nudge(self) -> bool {
        matches!(self, Profile::Aggressive)
    }
}
