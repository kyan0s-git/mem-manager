//! MemManager for Windows — entry point.
//!
//! Scaffolding only. The module plan is in `docs/architecture/windows.md` §2:
//! `nt`, `sample`, `actions`, `procs`, `tray`, `popup`, `settings`, `widgets`,
//! `task`, `config`, `notify`. This binary currently builds the toolchain,
//! manifest and dependency setup end to end and exits immediately.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    // Touch the core crate so the dependency edge is exercised by the build.
    let _ = memmanager_core::SPEC_VERSION;
}
