; MemManager for Windows — installer (Inno Setup 6).
;
; Build:
;   iscc /DAppVersion=0.1.0 /DX64Exe=..\target\x86_64-pc-windows-msvc\release\memmanager.exe ^
;        /DArm64Exe=..\target\aarch64-pc-windows-msvc\release\memmanager.exe /DOutputDir=..\dist MemManager.iss
;
; One installer for x64 and ARM64. It installs per-machine, optionally registers
; the logon task that starts MemManager with memory-cleaning privileges (the
; installer is already elevated, so there is no second UAC prompt), and
; removes that task again on uninstall.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef X64Exe
  #define X64Exe "..\target\x86_64-pc-windows-msvc\release\memmanager.exe"
#endif
#ifndef Arm64Exe
  #define Arm64Exe ""
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{7D3C1E52-6A0B-4C1E-9B7E-5F2A1D9C4E61}
AppName=MemManager
AppVersion={#AppVersion}
AppVerName=MemManager {#AppVersion}
AppPublisher=kyan0s-git
AppPublisherURL=https://github.com/kyan0s-git/mem-manager
AppSupportURL=https://github.com/kyan0s-git/mem-manager/issues
AppUpdatesURL=https://github.com/kyan0s-git/mem-manager/releases
DefaultDirName={autopf}\MemManager
DefaultGroupName=MemManager
DisableProgramGroupPage=yes
PrivilegesRequired=admin
#if Arm64Exe != ""
ArchitecturesAllowed=x64compatible arm64
ArchitecturesInstallIn64BitMode=x64compatible arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename=MemManager-{#AppVersion}-windows-setup
Compression=lzma2/ultra
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=MemManager
UninstallDisplayIcon={app}\memmanager.exe
CloseApplications=no
VersionInfoVersion={#AppVersion}
VersionInfoProductName=MemManager
VersionInfoDescription=MemManager installer

[Tasks]
Name: "logontask"; Description: "Start at login with memory-cleaning privileges (recommended)"; GroupDescription: "Startup:"
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
#if Arm64Exe != ""
Source: "{#X64Exe}"; DestDir: "{app}"; DestName: "memmanager.exe"; Check: not IsArm64; Flags: ignoreversion
Source: "{#Arm64Exe}"; DestDir: "{app}"; DestName: "memmanager.exe"; Check: IsArm64; Flags: ignoreversion
#else
Source: "{#X64Exe}"; DestDir: "{app}"; DestName: "memmanager.exe"; Flags: ignoreversion
#endif

[Icons]
Name: "{group}\MemManager"; Filename: "{app}\memmanager.exe"
Name: "{group}\Uninstall MemManager"; Filename: "{uninstallexe}"
Name: "{autodesktop}\MemManager"; Filename: "{app}\memmanager.exe"; Tasks: desktopicon

[Run]
; Register the elevated logon task (the installer already runs elevated).
Filename: "{app}\memmanager.exe"; Parameters: "--register-task"; Tasks: logontask; Flags: runhidden waituntilterminated; StatusMsg: "Registering the startup task..."
; Launch now: through the task (elevated, as the logged-in user) or unelevated in monitor-only mode.
Filename: "{sys}\schtasks.exe"; Parameters: "/Run /TN MemManager"; Tasks: logontask; Flags: runhidden nowait postinstall; Description: "Start MemManager now"
Filename: "{app}\memmanager.exe"; Tasks: not logontask; Flags: nowait postinstall runasoriginaluser; Description: "Start MemManager now (monitor only)"

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM memmanager.exe"; Flags: runhidden; RunOnceId: "StopApp"
Filename: "{app}\memmanager.exe"; Parameters: "--unregister-task"; Flags: runhidden; RunOnceId: "RemoveTask"

[Registry]
; Remove the per-user "start at login (monitor only)" entry on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "MemManager"; Flags: uninsdeletevalue dontcreatekey

[Code]
procedure StopRunningApp();
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM memmanager.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(500);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  { Upgrades replace the exe in place: stop a running copy first. }
  StopRunningApp();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Dir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Dir := ExpandConstant('{userappdata}\MemManager');
    if DirExists(Dir) then
      if MsgBox('Remove your MemManager settings as well?', mbConfirmation, MB_YESNO) = IDYES then
        DelTree(Dir, True, True, True);
  end;
end;
