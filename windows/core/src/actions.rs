//! Tiered actions, token buckets, the ledger and regret-driven back-off
//! (policy-engine §5).

use crate::ring::Ring;
use crate::state::PressureState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionSpec {
    /// Platform-defined identifier (stable; used in config and the ledger).
    pub id: u8,
    pub tier: u8,
    pub min_state: PressureState,
    pub cooldown_s: f64,
    pub bucket_capacity: f64,
    pub refill_per_h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TokenBucket {
    pub tokens: f64,
    pub capacity: f64,
    pub refill_per_h: f64,
    last_ms: u64,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_per_h: f64, now_ms: u64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_per_h,
            last_ms: now_ms,
        }
    }

    pub fn refill(&mut self, now_ms: u64) {
        let dt_h = now_ms.saturating_sub(self.last_ms) as f64 / 3_600_000.0;
        self.tokens = (self.tokens + dt_h * self.refill_per_h).min(self.capacity);
        self.last_ms = now_ms;
    }

    pub fn available(&mut self, now_ms: u64) -> bool {
        self.refill(now_ms);
        self.tokens >= 1.0
    }

    pub fn take(&mut self, now_ms: u64) -> bool {
        if self.available(now_ms) {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

pub const MAX_PENALTY: f64 = 16.0;
pub const BAD_REFAULT_RATIO: f64 = 0.25;
pub const GOOD_REFAULT_RATIO: f64 = 0.05;
pub const GOOD_STREAK_TO_RELAX: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActionRuntime {
    pub spec: ActionSpec,
    pub bucket: TokenBucket,
    pub last_run_ms: Option<u64>,
    pub penalty: f64,
    good_streak: u8,
    /// User toggle / capability: disabled actions are never picked.
    pub enabled: bool,
}

impl ActionRuntime {
    pub fn new(spec: ActionSpec, now_ms: u64) -> Self {
        Self {
            spec,
            bucket: TokenBucket::new(spec.bucket_capacity, spec.refill_per_h, now_ms),
            last_run_ms: None,
            penalty: 1.0,
            good_streak: 0,
            enabled: true,
        }
    }

    /// Entry threshold on `S`, raised by `0.05·log2(penalty)`.
    pub fn threshold(&self) -> f64 {
        self.spec.min_state.enter_threshold() + 0.05 * self.penalty.log2()
    }

    pub fn cooldown_ok(&self, now_ms: u64) -> bool {
        match self.last_run_ms {
            None => true,
            Some(t) => {
                now_ms.saturating_sub(t) as f64 >= self.spec.cooldown_s * self.penalty * 1000.0
            }
        }
    }

    pub fn eligible(&mut self, state: PressureState, s: f64, now_ms: u64) -> bool {
        self.enabled
            && state >= self.spec.min_state
            && s >= self.threshold()
            && self.cooldown_ok(now_ms)
            && self.bucket.available(now_ms)
    }

    pub fn mark_run(&mut self, now_ms: u64) {
        self.bucket.take(now_ms);
        self.last_run_ms = Some(now_ms);
    }

    /// Applies a finished regret measurement; returns the new penalty.
    pub fn apply_regret(&mut self, refault_ratio: f64) -> f64 {
        if refault_ratio > BAD_REFAULT_RATIO {
            self.penalty = (self.penalty * 2.0).min(MAX_PENALTY);
            self.good_streak = 0;
        } else if refault_ratio < GOOD_REFAULT_RATIO {
            self.good_streak += 1;
            if self.good_streak >= GOOD_STREAK_TO_RELAX {
                self.penalty = (self.penalty / 2.0).max(1.0);
                self.good_streak = 0;
            }
        } else {
            self.good_streak = 0;
        }
        self.penalty
    }
}

/// Holds every action of one platform and picks the next one to run.
#[derive(Clone, Debug, Default)]
pub struct Scheduler {
    pub actions: Vec<ActionRuntime>,
}

impl Scheduler {
    pub fn new(specs: &[ActionSpec], now_ms: u64) -> Self {
        let mut actions: Vec<ActionRuntime> = specs
            .iter()
            .map(|s| ActionRuntime::new(*s, now_ms))
            .collect();
        actions.sort_by_key(|a| a.spec.tier);
        Self { actions }
    }

    pub fn get_mut(&mut self, id: u8) -> Option<&mut ActionRuntime> {
        self.actions.iter_mut().find(|a| a.spec.id == id)
    }

    pub fn get(&self, id: u8) -> Option<&ActionRuntime> {
        self.actions.iter().find(|a| a.spec.id == id)
    }

    /// Lowest-tier eligible action whose platform predicate holds.
    /// `max_tier` comes from the profile; `predicate` from the platform.
    pub fn pick(
        &mut self,
        state: PressureState,
        s: f64,
        now_ms: u64,
        max_tier: u8,
        mut predicate: impl FnMut(u8) -> bool,
    ) -> Option<u8> {
        for a in self.actions.iter_mut() {
            if a.spec.tier > max_tier || a.spec.tier == 0 {
                continue;
            }
            if a.eligible(state, s, now_ms) && predicate(a.spec.id) {
                return Some(a.spec.id);
            }
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LedgerEntry {
    pub t_ms: u64,
    pub action_id: u8,
    pub manual: bool,
    pub state: PressureState,
    pub fast_avail_before: u64,
    pub fast_avail_after: u64,
    pub baseline_hard_rate: f64,
    pub post_hard_pages: f64,
    pub freed_pages: f64,
    pub refault_ratio: f64,
    pub penalty_after: f64,
    /// Measurement still running (post window not finished).
    pub pending: bool,
}

impl LedgerEntry {
    pub fn freed_bytes(&self) -> u64 {
        self.fast_avail_after.saturating_sub(self.fast_avail_before)
    }
}

pub type Ledger = Ring<LedgerEntry, 64>;

pub const POST_WINDOW_MS: u64 = 60_000;
pub const AFTER_DELAY_MS: u64 = 2_000;
pub const BASELINE_TAU_S: f64 = 120.0;

#[derive(Clone, Copy, Debug)]
struct Pending {
    start_ms: u64,
    hard_at_start: u64,
    after_set: bool,
}

/// Measures the effect of each action (policy-engine §5.1).
#[derive(Clone, Debug, Default)]
pub struct RegretTracker {
    baseline_rate: Option<f64>,
    last: Option<(u64, u64)>, // (t_ms, hard_faults_cum)
    pending: Option<Pending>,
    page_size: u64,
}

impl RegretTracker {
    pub fn new(page_size: u64) -> Self {
        Self {
            page_size: page_size.max(1),
            ..Default::default()
        }
    }

    pub fn baseline_rate(&self) -> f64 {
        self.baseline_rate.unwrap_or(0.0)
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Feed every system sample. Returns `true` when the pending measurement
    /// finished; the caller then reads `ledger.last()` and applies the penalty.
    pub fn observe(
        &mut self,
        t_ms: u64,
        hard_faults_cum: u64,
        fast_avail: u64,
        ledger: &mut Ledger,
    ) -> bool {
        // Baseline EWMA only while no action is being measured.
        if let (Some((pt, ph)), None) = (self.last, self.pending) {
            let dt = t_ms.saturating_sub(pt) as f64 / 1000.0;
            if dt > 0.0 {
                let rate = hard_faults_cum.saturating_sub(ph) as f64 / dt;
                self.baseline_rate = Some(match self.baseline_rate {
                    None => rate,
                    Some(b) => b + (1.0 - (-dt / BASELINE_TAU_S).exp()) * (rate - b),
                });
            }
        }
        self.last = Some((t_ms, hard_faults_cum));

        let Some(mut p) = self.pending else {
            return false;
        };
        let Some(entry) = ledger.last_mut() else {
            self.pending = None;
            return false;
        };
        let elapsed = t_ms.saturating_sub(p.start_ms);
        if !p.after_set && elapsed >= AFTER_DELAY_MS {
            entry.fast_avail_after = fast_avail;
            p.after_set = true;
        }
        if elapsed >= POST_WINDOW_MS {
            Self::finish(entry, &p, t_ms, hard_faults_cum, fast_avail, self.page_size);
            self.pending = None;
            return true;
        }
        self.pending = Some(p);
        false
    }

    fn finish(
        entry: &mut LedgerEntry,
        p: &Pending,
        t_ms: u64,
        hard_cum: u64,
        fast_avail: u64,
        page_size: u64,
    ) {
        if !p.after_set {
            entry.fast_avail_after = fast_avail;
        }
        let elapsed_s = t_ms.saturating_sub(p.start_ms) as f64 / 1000.0;
        let raw = hard_cum.saturating_sub(p.hard_at_start) as f64;
        entry.post_hard_pages = (raw - entry.baseline_hard_rate * elapsed_s).max(0.0);
        entry.freed_pages = entry.freed_bytes() as f64 / page_size as f64;
        entry.refault_ratio = entry.post_hard_pages / entry.freed_pages.max(1.0);
        entry.pending = false;
    }

    /// Records an action that is about to run. If a measurement is still
    /// pending it is closed early (returns `true`, see `observe`).
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        t_ms: u64,
        action_id: u8,
        manual: bool,
        state: PressureState,
        hard_faults_cum: u64,
        fast_avail: u64,
        ledger: &mut Ledger,
    ) -> bool {
        let mut closed = false;
        if let (Some(p), Some(entry)) = (self.pending.take(), ledger.last_mut()) {
            Self::finish(entry, &p, t_ms, hard_faults_cum, fast_avail, self.page_size);
            closed = true;
        }
        ledger.push(LedgerEntry {
            t_ms,
            action_id,
            manual,
            state,
            fast_avail_before: fast_avail,
            fast_avail_after: fast_avail,
            baseline_hard_rate: self.baseline_rate(),
            penalty_after: 1.0,
            pending: true,
            ..Default::default()
        });
        self.pending = Some(Pending {
            start_ms: t_ms,
            hard_at_start: hard_faults_cum,
            after_set: false,
        });
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: u8, tier: u8, min: PressureState) -> ActionSpec {
        ActionSpec {
            id,
            tier,
            min_state: min,
            cooldown_s: 30.0,
            bucket_capacity: 2.0,
            refill_per_h: 4.0,
        }
    }

    #[test]
    fn bucket_refills() {
        let mut b = TokenBucket::new(2.0, 4.0, 0);
        assert!(b.take(0));
        assert!(b.take(0));
        assert!(!b.take(0));
        assert!(b.available(15 * 60_000)); // +1 token after 15 min
    }

    #[test]
    fn scheduler_picks_lowest_eligible_tier() {
        let mut s = Scheduler::new(
            &[
                spec(3, 3, PressureState::High),
                spec(2, 2, PressureState::Elevated),
            ],
            0,
        );
        assert_eq!(s.pick(PressureState::High, 0.7, 0, 5, |_| true), Some(2));
        assert_eq!(
            s.pick(PressureState::High, 0.7, 0, 5, |id| id != 2),
            Some(3)
        );
        assert_eq!(
            s.pick(PressureState::Elevated, 0.45, 0, 5, |id| id != 2),
            None
        );
        assert_eq!(s.pick(PressureState::High, 0.7, 0, 2, |id| id != 2), None);
        s.get_mut(2).unwrap().mark_run(0);
        assert_eq!(
            s.pick(PressureState::High, 0.7, 10_000, 5, |_| true),
            Some(3)
        );
        assert_eq!(
            s.pick(PressureState::High, 0.7, 31_000, 5, |_| true),
            Some(2)
        );
    }

    #[test]
    fn penalty_doubles_and_relaxes() {
        let mut a = ActionRuntime::new(spec(2, 2, PressureState::Elevated), 0);
        assert_eq!(a.apply_regret(0.5), 2.0);
        assert_eq!(a.apply_regret(0.5), 4.0);
        assert!((a.threshold() - (0.40 + 0.10)).abs() < 1e-12);
        a.apply_regret(0.0);
        a.apply_regret(0.0);
        assert_eq!(a.apply_regret(0.0), 2.0);
        for _ in 0..10 {
            a.apply_regret(1.0);
        }
        assert_eq!(a.penalty, MAX_PENALTY);
    }

    #[test]
    fn regret_measures_refaults() {
        let mut ledger = Ledger::new();
        let mut r = RegretTracker::new(4096);
        // Baseline: 10 hard faults/s.
        for i in 0..=12u64 {
            r.observe(i * 10_000, i * 100, 1 << 30, &mut ledger);
        }
        assert!((r.baseline_rate() - 10.0).abs() < 1e-6);
        let t0 = 120_000;
        r.begin(
            t0,
            2,
            false,
            PressureState::High,
            1200,
            1 << 30,
            &mut ledger,
        );
        // 2 s later 256 MiB more is available.
        let after = (1u64 << 30) + (256 << 20);
        assert!(!r.observe(t0 + 2_000, 1220, after, &mut ledger));
        // 60 s later: baseline would be 600 pages; we saw 600 + 16384.
        assert!(r.observe(t0 + 60_000, 1200 + 600 + 16_384, after, &mut ledger));
        let e = ledger.last().unwrap();
        assert!(!e.pending);
        assert_eq!(e.freed_pages, 65_536.0);
        assert!((e.post_hard_pages - 16_384.0).abs() < 1e-6);
        assert!((e.refault_ratio - 0.25).abs() < 1e-9);
    }
}
