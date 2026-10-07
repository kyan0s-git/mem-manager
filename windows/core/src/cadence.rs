//! Sampling cadence (policy-engine §8).

use crate::state::PressureState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    pub system_ms: u32,
    pub process_ms: u32,
}

/// `None` means "paused": only kernel events wake the sampler.
pub fn cadence(
    popup_visible: bool,
    state: PressureState,
    display_off: bool,
    on_battery: bool,
) -> Option<Cadence> {
    if display_off && !popup_visible {
        return None;
    }
    let mut c = if popup_visible {
        Cadence {
            system_ms: 1_000,
            process_ms: 2_000,
        }
    } else if state >= PressureState::Elevated {
        Cadence {
            system_ms: 5_000,
            process_ms: 15_000,
        }
    } else {
        Cadence {
            system_ms: 10_000,
            process_ms: 30_000,
        }
    };
    if on_battery && !popup_visible {
        c.system_ms *= 2;
        c.process_ms *= 2;
    }
    Some(c)
}

/// Coalescing tolerance: 10% of the period, at least 1 s (except sub-second periods).
pub fn tolerance_ms(period_ms: u32) -> u32 {
    (period_ms / 10).max(1_000.min(period_ms / 2))
}
