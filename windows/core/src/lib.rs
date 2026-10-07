//! Platform-neutral core of MemManager.
//!
//! Implements `docs/architecture/policy-engine.md`: pressure scoring, the
//! EWMA + hysteresis state machine, tiered actions with token buckets and
//! regret-driven back-off, and the Theil–Sen / Mann–Kendall leak detector.
//! Everything here is pure computation over plain data so it builds and is
//! tested on any host against `spec/test-vectors/`.
#![forbid(unsafe_code)]

pub mod actions;
pub mod cadence;
pub mod leak;
pub mod profile;
pub mod ring;
pub mod sample;
pub mod state;

/// Version of the policy-engine spec this crate implements.
pub const SPEC_VERSION: u32 = 1;

pub const MIB: f64 = 1024.0 * 1024.0;
