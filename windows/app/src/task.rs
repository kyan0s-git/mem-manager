//! Elevation via a Task Scheduler logon task (docs/architecture/windows.md §1).
//! `--register-task` must run elevated; it writes the task XML and calls schtasks.

use std::os::windows::process::CommandExt;
use std::process::Command;

pub const TASK_NAME: &str = "MemManager";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn current_user() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => user,
    }
}

pub fn task_xml(exe: &str, user: &str) -> String {
    let exe = xml_escape(exe);
    let user = xml_escape(user);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Starts MemManager at logon with the privileges it needs to manage memory.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>"{exe}"</Command>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn is_registered() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME])
}

/// Registers the logon task (caller must be elevated). Returns a process exit code.
pub fn register() -> i32 {
    let Ok(exe) = std::env::current_exe() else {
        return 2;
    };
    let xml = task_xml(&exe.to_string_lossy(), &current_user());
    let path = std::env::temp_dir().join("memmanager-task.xml");
    // schtasks expects UTF-16 LE with a BOM for this declaration.
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    if std::fs::write(&path, bytes).is_err() {
        return 3;
    }
    let ok = schtasks(&[
        "/Create",
        "/TN",
        TASK_NAME,
        "/XML",
        &path.to_string_lossy(),
        "/F",
    ]);
    let _ = std::fs::remove_file(&path);
    if ok { 0 } else { 4 }
}

pub fn unregister() -> i32 {
    if schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]) {
        0
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_is_escaped_and_complete() {
        let x = task_xml(r"C:\Apps & Tools\memmanager.exe", r"PC\me");
        assert!(x.contains(r#"<Command>"C:\Apps &amp; Tools\memmanager.exe"</Command>"#));
        assert!(x.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(x.contains("<UserId>PC\\me</UserId>"));
    }
}
