//! In-app updates for Windows: ask GitHub for a newer release, download the
//! matching asset, verify it against the published SHA-256 and install it.
//!
//! - Installed copies (Inno Setup, `unins000.exe` next to the exe) run the new
//!   setup silently. It upgrades in place, keeps the chosen setup tasks and
//!   starts MemManager again.
//! - Portable copies swap the exe in place (a running exe can be renamed) and
//!   restart from the new file.
//!
//! Network calls run on short-lived background threads; nothing stays resident.

use crate::update::{self, Offer, Version};
use crate::util::WStr;
use std::path::PathBuf;
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::PCWSTR;

const USER_AGENT: &str = concat!(
    "MemManager/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/kyan0s-git/mem-manager)"
);
const ADD_HEADER: u32 = 0x2000_0000; // WINHTTP_ADDREQ_FLAG_ADD
const MAX_JSON: usize = 4 << 20;
const MAX_ASSET: usize = 64 << 20;

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version {
        major: 0,
        minor: 0,
        patch: 0,
        pre: None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Installed,
    Portable,
}

/// Installed by the setup (its uninstaller sits next to the exe) or portable.
pub fn kind() -> Kind {
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("unins000.exe")))
        .is_some_and(|p| p.exists());
    if installed {
        Kind::Installed
    } else {
        Kind::Portable
    }
}

fn arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    }
}

/// The release asset this copy updates from.
pub fn asset_name(kind: Kind, v: &Version) -> String {
    match kind {
        Kind::Installed => format!("MemManager-{v}-windows-setup.exe"),
        Kind::Portable => format!("MemManager-{v}-windows-{}.exe", arch()),
    }
}

struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn last_error(what: &str) -> String {
    format!(
        "{what} failed ({})",
        windows::core::Error::from_thread().code().0
    )
}

/// HTTPS GET into memory (redirects followed, as GitHub downloads need).
pub fn http_get(url: &str, accept: &str, limit: usize) -> Result<Vec<u8>, String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or("only https URLs are allowed")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    unsafe {
        let ua = WStr::new(USER_AGENT);
        let session = Handle(WinHttpOpen(
            ua.pcwstr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
        let h = WStr::new(host);
        let conn = Handle(WinHttpConnect(session.0, h.pcwstr(), 443, 0));
        if conn.0.is_null() {
            return Err(last_error("connect"));
        }
        let verb = WStr::new("GET");
        let p = WStr::new(path);
        let req = Handle(WinHttpOpenRequest(
            conn.0,
            verb.pcwstr(),
            p.pcwstr(),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err(last_error("request"));
        }
        let mut headers = format!("Accept: {accept}\r\nX-GitHub-Api-Version: 2022-11-28\r\n");
        // CI only: authenticated API calls avoid shared-runner rate limits. Never sent elsewhere.
        if host == "api.github.com" {
            if let Ok(t) = std::env::var("MEMMANAGER_GITHUB_TOKEN") {
                if !t.is_empty() {
                    headers += &format!("Authorization: Bearer {t}\r\n");
                }
            }
        }
        let headers: Vec<u16> = headers.encode_utf16().collect();
        let _ = WinHttpAddRequestHeaders(req.0, &headers, ADD_HEADER);
        WinHttpSendRequest(req.0, None, None, 0, 0, 0).map_err(|e| format!("send: {e}"))?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut())
            .map_err(|e| format!("response: {e}"))?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("status: {e}"))?;
        if status != 200 {
            return Err(match status {
                403 | 429 => "GitHub is rate-limiting requests; try again later".into(),
                404 => "not found".into(),
                s => format!("HTTP {s}"),
            });
        }
        let mut out = Vec::new();
        loop {
            let mut avail = 0u32;
            WinHttpQueryDataAvailable(req.0, &mut avail).map_err(|e| format!("read: {e}"))?;
            if avail == 0 {
                break;
            }
            if out.len() + avail as usize > limit {
                return Err("download is larger than expected".into());
            }
            let start = out.len();
            out.resize(start + avail as usize, 0);
            let mut read = 0u32;
            WinHttpReadData(req.0, out[start..].as_mut_ptr().cast(), avail, &mut read)
                .map_err(|e| format!("read: {e}"))?;
            out.truncate(start + read as usize);
        }
        Ok(out)
    }
}

/// Asks GitHub for a release newer than `current` that this copy can install.
pub fn check(current: &Version) -> Result<Option<Offer>, String> {
    check_as(current, kind())
}

fn check_as(current: &Version, k: Kind) -> Result<Option<Offer>, String> {
    let body = http_get(
        update::RELEASES_URL,
        "application/vnd.github+json",
        MAX_JSON,
    )?;
    let text = String::from_utf8(body).map_err(|_| "unexpected response from GitHub")?;
    let releases = update::parse_releases(&text).ok_or("unexpected response from GitHub")?;
    Ok(update::choose(&releases, current, |v| asset_name(k, v)))
}

/// Downloads the offer's asset and its checksum; returns the verified file.
pub fn download(o: &Offer) -> Result<PathBuf, String> {
    if o.asset.size > MAX_ASSET as u64 {
        return Err("download is larger than expected".into());
    }
    let sum = http_get(&o.checksum.url, "application/octet-stream", 4096)?;
    let data = http_get(&o.asset.url, "application/octet-stream", MAX_ASSET)?;
    if !update::verify(&data, &String::from_utf8_lossy(&sum)) {
        return Err(
            "the download did not match its published checksum, so it was discarded".into(),
        );
    }
    let dir = std::env::temp_dir().join("MemManager-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp folder: {e}"))?;
    let path = dir.join(&o.asset.name);
    std::fs::write(&path, &data).map_err(|e| format!("saving the update: {e}"))?;
    Ok(path)
}

fn shell_open(file: &std::path::Path, params: &str) -> Result<(), String> {
    let f = WStr::new(&file.to_string_lossy());
    let verb = WStr::new("open");
    let p = WStr::new(params);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: verb.pcwstr(),
        lpFile: f.pcwstr(),
        lpParameters: p.pcwstr(),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut sei).map_err(|_| "the installation was cancelled".to_string())?;
        if !sei.hProcess.is_invalid() {
            let _ = windows::Win32::Foundation::CloseHandle(sei.hProcess);
        }
    }
    Ok(())
}

/// Downloads, verifies and starts installing `o`. On `Ok` the caller must exit
/// so the new version can replace this one (it starts itself afterwards).
pub fn install(o: &Offer) -> Result<(), String> {
    let path = download(o)?;
    match kind() {
        // The setup asks for admin rights if needed, closes this app, upgrades
        // in place with the previous choices and starts MemManager again.
        Kind::Installed => shell_open(&path, "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART"),
        Kind::Portable => {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let old = exe.with_extension("exe.old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(&exe, &old).map_err(|e| {
                format!("can't replace the app in this folder ({e}); download the new version from the releases page")
            })?;
            if let Err(e) = std::fs::copy(&path, &exe) {
                let _ = std::fs::rename(&old, &exe);
                return Err(format!("couldn't write the new version: {e}"));
            }
            let _ = std::fs::remove_file(&path);
            std::process::Command::new(&exe)
                .arg("--after-update")
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("couldn't start the new version: {e}"))
        }
    }
}

/// Start of a process launched by a portable update: wait for the previous
/// instance to exit, then remove its renamed exe.
pub fn after_update() {
    for _ in 0..150 {
        let exists = unsafe {
            windows::Win32::System::Threading::OpenMutexW(
                windows::Win32::System::Threading::SYNCHRONIZATION_SYNCHRONIZE,
                false,
                windows::core::w!("Local\\MemManager.SingleInstance"),
            )
        };
        match exists {
            Ok(h) => unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            },
            Err(_) => break,
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    cleanup();
}

/// Removes leftovers of a previous update (renamed exe, downloaded setup).
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("MemManager-update"));
}

/// `--check-update [--pretend-version=X] [--as-installed] [--download]`:
/// prints a JSON report (CI uses it to exercise the real GitHub API, the
/// download and the checksum check, without installing anything).
pub fn cli(
    pretend: Option<&str>,
    as_installed: bool,
    download_too: bool,
    out_path: Option<String>,
) -> i32 {
    let current = pretend
        .and_then(Version::parse)
        .unwrap_or_else(current_version);
    let k = if as_installed {
        Kind::Installed
    } else {
        kind()
    };
    let mut out = format!(
        "{{\"current\":\"{current}\",\"kind\":\"{}\",\"asset_pattern\":\"{}\"",
        if k == Kind::Installed {
            "installed"
        } else {
            "portable"
        },
        asset_name(k, &current)
    );
    let code = match check_as(&current, k) {
        Ok(None) => {
            out += ",\"offer\":null";
            0
        }
        Ok(Some(o)) => {
            out += &format!(
                ",\"offer\":{{\"version\":\"{}\",\"asset\":\"{}\",\"size\":{}}}",
                o.version, o.asset.name, o.asset.size
            );
            if download_too {
                match download(&o) {
                    Ok(p) => {
                        out += ",\"verified\":true";
                        let _ = std::fs::remove_file(p);
                        0
                    }
                    Err(e) => {
                        out += &format!(
                            ",\"verified\":false,\"error\":{}",
                            crate::util::json_str(&e)
                        );
                        1
                    }
                }
            } else {
                0
            }
        }
        Err(e) => {
            out += &format!(",\"error\":{}", crate::util::json_str(&e));
            1
        }
    };
    out.push('}');
    match out_path {
        Some(p) => {
            let _ = std::fs::write(p, &out);
        }
        None => println!("{out}"),
    }
    code
}
