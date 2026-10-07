//! EWMA smoothing and the hysteresis state machine (policy-engine §4).

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum PressureState {
    #[default]
    Normal = 0,
    Elevated = 1,
    High = 2,
    Critical = 3,
}

impl PressureState {
    pub const ALL: [PressureState; 4] = [
        PressureState::Normal,
        PressureState::Elevated,
        PressureState::High,
        PressureState::Critical,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PressureState::Normal => "Normal",
            PressureState::Elevated => "Elevated",
            PressureState::High => "High",
            PressureState::Critical => "Critical",
        }
    }

    pub fn from_index(i: usize) -> PressureState {
        PressureState::ALL[i.min(3)]
    }

    /// Smoothed score at which this state is entered (0 for Normal).
    pub fn enter_threshold(self) -> f64 {
        ENTER[self as usize]
    }
}

const ENTER: [f64; 4] = [0.0, 0.40, 0.65, 0.85];
const EXIT: [f64; 4] = [0.0, 0.30, 0.55, 0.75];
const DWELL_S: [f64; 4] = [0.0, 30.0, 30.0, 60.0];
pub const TAU_S: f64 = 20.0;
pub const P_EVENT: f64 = 0.9;

#[derive(Clone, Debug, Default)]
pub struct StateMachine {
    s: Option<f64>,
    t_ms: u64,
    state: PressureState,
    entered_ms: u64,
}

impl StateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> PressureState {
        self.state
    }

    /// Smoothed pressure score `S` (0 before the first sample).
    pub fn smoothed(&self) -> f64 {
        self.s.unwrap_or(0.0)
    }

    /// Feeds one pressure score `p` taken at `t_ms`. `event` is a kernel
    /// emergency signal that forces `S ≥ P_EVENT` immediately.
    pub fn step(&mut self, t_ms: u64, p: f64, event: bool) -> PressureState {
        let mut s = match self.s {
            None => p,
            Some(prev) => {
                let dt = t_ms.saturating_sub(self.t_ms) as f64 / 1000.0;
                let alpha = 1.0 - (-dt / TAU_S).exp();
                prev + alpha * (p - prev)
            }
        };
        if event {
            s = s.max(P_EVENT);
        }
        self.s = Some(s);
        self.t_ms = t_ms;

        let cur = self.state as usize;
        let mut target = cur;
        for i in (cur + 1..=3).rev() {
            if s >= ENTER[i] {
                target = i;
                break;
            }
        }
        if target > cur {
            self.state = PressureState::from_index(target);
            self.entered_ms = t_ms;
        } else if cur > 0
            && s < EXIT[cur]
            && t_ms.saturating_sub(self.entered_ms) as f64 / 1000.0 >= DWELL_S[cur]
        {
            self.state = PressureState::from_index(cur - 1);
            self.entered_ms = t_ms;
        }
        self.state
    }
}
