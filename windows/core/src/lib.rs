//! Platform-neutral core of the Windows memory manager.
//!
//! Implements `docs/architecture/policy-engine.md`. Everything here is pure
//! computation over plain data so it builds and is tested on any host,
//! including the Linux CI job, against `spec/test-vectors/`.
//!
//! Scaffolding only: modules are declared with their responsibilities; the
//! implementation lands in the next milestone.
#![no_std]
#![forbid(unsafe_code)]

/// Version of the policy-engine spec this crate implements.
pub const SPEC_VERSION: u32 = 1;

/// Fixed-capacity ring buffers used for all history (no steady-state allocation).
pub mod ring {}

/// `Sample`/`ProcSample` model and the pressure score `P` (policy-engine §2–3).
pub mod sample {}

/// EWMA smoothing and the hysteresis state machine (policy-engine §4).
pub mod state {}

/// Action specs, token buckets, ledger and regret-driven penalties (policy-engine §5).
pub mod actions {}

/// Profile threshold sets: Conservative / Balanced / Aggressive (policy-engine §6).
pub mod profile {}

/// Theil–Sen + Mann–Kendall leak and runaway detector (policy-engine §7).
pub mod leak {}
