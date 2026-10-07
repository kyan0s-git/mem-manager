//! Token elevation and privilege management.

use crate::util::WStr;
use windows::Win32::Foundation::{CloseHandle, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
    TokenElevation,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open_token(access: windows::Win32::Security::TOKEN_ACCESS_MASK) -> Option<Token> {
    let mut h = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h).ok()? };
    Some(Token(h))
}

pub fn is_elevated() -> bool {
    let Some(t) = open_token(TOKEN_QUERY) else {
        return false;
    };
    let mut e = TOKEN_ELEVATION::default();
    let mut ret = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            t.0,
            TokenElevation,
            Some((&mut e as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        )
    };
    ok.is_ok() && e.TokenIsElevated != 0
}

/// Enables one privilege on the process token. Returns `true` only if it is
/// actually held (AdjustTokenPrivileges "succeeds" for privileges not held).
pub fn enable(name: &str) -> bool {
    let Some(t) = open_token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY) else {
        return false;
    };
    let mut luid = LUID::default();
    let n = WStr::new(name);
    if unsafe { LookupPrivilegeValueW(None, n.pcwstr(), &mut luid) }.is_err() {
        return false;
    }
    let tp = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    let r = unsafe { AdjustTokenPrivileges(t.0, false, Some(&tp), 0, None, None) };
    r.is_ok() && unsafe { GetLastError() } != ERROR_NOT_ALL_ASSIGNED
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Privileges {
    pub elevated: bool,
    /// SeProfileSingleProcessPrivilege: memory-list commands, page combining.
    pub profile: bool,
    /// SeIncreaseQuotaPrivilege: file-cache flush and limits.
    pub quota: bool,
}

pub fn enable_all() -> Privileges {
    Privileges {
        elevated: is_elevated(),
        profile: enable("SeProfileSingleProcessPrivilege"),
        quota: enable("SeIncreaseQuotaPrivilege"),
    }
}
