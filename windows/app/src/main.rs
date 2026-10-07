//! MemManager for Windows: a rigorous, measured memory manager in the tray.
//! See docs/architecture/windows.md.
#![cfg_attr(windows, windows_subsystem = "windows")]
// TODO(ui): remove once the tray/flyout UI uses every engine API.
#![allow(dead_code)]

mod config;
mod icon;
mod palette;

#[cfg(windows)]
mod nt;
#[cfg(windows)]
mod pool;
#[cfg(windows)]
mod privilege;
#[cfg(windows)]
mod procs;
#[cfg(windows)]
mod selftest;
#[cfg(windows)]
mod shared;
#[cfg(windows)]
mod sysmem;
#[cfg(windows)]
mod task;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod util;
#[cfg(windows)]
mod winactions;
#[cfg(windows)]
mod worker;

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(a) = args.iter().find(|a| a.starts_with("--selftest")) {
        let secs = a
            .split_once('=')
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(10);
        let out = args
            .iter()
            .find_map(|a| a.strip_prefix("--out=").map(str::to_string));
        std::process::exit(selftest::run(secs, out));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("memmanager is a Windows application; build it for a Windows target.");
}
