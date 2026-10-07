//! MemManager for Windows: a rigorous, measured memory manager in the tray.
//! See docs/architecture/windows.md.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod config;
mod icon;
mod palette;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod bench;
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
mod ui;
#[cfg(windows)]
mod util;
#[cfg(windows)]
mod winactions;
#[cfg(windows)]
mod worker;

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
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
    if let Some(mib) = args
        .iter()
        .find_map(|a| a.strip_prefix("--bench-child="))
        .and_then(|v| v.parse().ok())
    {
        let id = args
            .iter()
            .find_map(|a| a.strip_prefix("--bench-id="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        std::process::exit(bench::child(mib, id));
    }
    if args.iter().any(|a| a == "--bench") {
        let out = args
            .iter()
            .find_map(|a| a.strip_prefix("--out=").map(str::to_string));
        std::process::exit(bench::run(out));
    }
    if has("--register-task") {
        std::process::exit(app::register_task(has("--launch")));
    }
    if has("--unregister-task") {
        std::process::exit(task::unregister());
    }
    let privileges = privilege::enable_all();
    std::process::exit(app::run(privileges));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("memmanager is a Windows application; build it for a Windows target.");
}
