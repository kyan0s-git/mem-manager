# Sources

Every technical claim in `docs/research/` and `docs/architecture/` should trace to one of these.
Tags such as **[W3]** or **[M2]** are used inline in the other documents.

"Primary" means kernel or OS source code, official SDK headers, or vendor documentation.
"Secondary" means reverse-engineering notes, reference implementations, or articles. Undocumented
Windows behaviour is only ever backed by secondary sources plus the public NT headers that the
community maintains. Treat it as version-dependent.

## Windows

| Tag | Source | Kind | Used for |
|---|---|---|---|
| W1 | phnt (System Informer native headers), `ntexapi.h` — https://github.com/winsiderss/phnt/blob/master/ntexapi.h | Secondary (canonical community header) | `SystemMemoryListInformation` (class 80), `SYSTEM_MEMORY_LIST_INFORMATION`, `SYSTEM_MEMORY_LIST_COMMAND`, `SystemCombinePhysicalMemoryInformation` (130), `MEMORY_COMBINE_INFORMATION_EX`, `SystemFileCacheInformationEx`, `SYSTEM_PERFORMANCE_INFORMATION`, `SYSTEM_POOLTAG_INFORMATION`, privilege notes |
| W2 | Microsoft SDK docs (MicrosoftDocs/sdk-api), `SetProcessInformation` — https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/processthreadsapi/nf-processthreadsapi-setprocessinformation.md | Primary | `ProcessMemoryPriority`, `ProcessPowerThrottling` / EcoQoS semantics |
| W3 | Microsoft SDK docs, `SetProcessWorkingSetSizeEx` — https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/memoryapi/nf-memoryapi-setprocessworkingsetsizeex.md | Primary | Working-set min/max, `QUOTA_LIMITS_HARDWS_*`, `(SIZE_T)-1` trim |
| W4 | Microsoft SDK docs, `EmptyWorkingSet` — https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/psapi/nf-psapi-emptyworkingset.md | Primary | Per-process trim, access rights |
| W5 | Microsoft SDK docs, `CreateMemoryResourceNotification` — https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/memoryapi/nf-memoryapi-creatememoryresourcenotification.md | Primary | System-wide low and high memory events, plus the dead band between them |
| W6 | Microsoft SDK docs, `SetSystemFileCacheSize` — https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/memoryapi/nf-memoryapi-setsystemfilecachesize.md | Primary | File cache flush and hard limits |
| W7 | Microsoft SDK docs, `MEMORY_PRIORITY_INFORMATION` — https://github.com/MicrosoftDocs/sdk-api/blob/docs/sdk-api-src/content/processthreadsapi/ns-processthreadsapi-memory_priority_information.md | Primary | Priority constants 1–5 (VERY_LOW…NORMAL) |
| W8 | Mem Reduct source, `src/main.c` — https://github.com/henrypp/memreduct | Reference implementation | Exact cleaning sequence, cooldown after the auto-clean loop bug (issue #191), GDI tray icon renderer |
| W9 | M. Russinovich, "The Memory-Optimization Hoax", Windows IT Pro (2003) — https://www.itprotoday.com/cloud-computing/memory-optimization-hoax | Secondary (authoritative author) | Why blind working-set trimming and "free RAM" maximisation hurt performance |
| W10 | Russinovich, Solomon, Ionescu, Yosifovich, *Windows Internals*, 7th ed., Part 1, ch. 5 "Memory management" | Primary (book) | Page lists, standby priorities 0–7, modified page writer, working-set manager, store manager (compression) |
| W11 | ITPro Today, "Understand Windows 10 memory compression" — https://www.itprotoday.com/windows-10/understand-windows-10-memory-compression | Secondary | Compression store lives in the "Memory Compression" process working set |
| W12 | mingw-w64 `dwmapi.h` — https://github.com/mirror/mingw-w64/blob/master/mingw-w64-headers/include/dwmapi.h | Header | `DWMWA_WINDOW_CORNER_PREFERENCE = 33`, `DWMWA_SYSTEMBACKDROP_TYPE = 38`, `DWMSBT_*`, `DWMWCP_*` |
| W13 | mingw-w64 `shellapi.h` — https://github.com/mirror/mingw-w64/blob/master/mingw-w64-headers/include/shellapi.h | Header | `SHQueryUserNotificationState`, `QUNS_RUNNING_D3D_FULL_SCREEN = 3`, `QUNS_BUSY = 2`, `QUNS_PRESENTATION_MODE = 4` |
| W14 | Microsoft Learn, "PoolMon examples" — https://learn.microsoft.com/windows-hardware/drivers/devtest/poolmon-examples | Primary | Pool-tag leak triage (allocs much greater than frees, sort by diff) |
| W15 | Wagnardsoft, Intelligent Standby List Cleaner — https://www.wagnardsoft.com/ | Reference tool | Threshold-driven standby purging for games |
| W16 | DWM_SYSTEMBACKDROP_TYPE docs — https://learn.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type | Primary | Backdrop types need build 22621 or later |
| W17 | Microsoft "Geometry in Windows 11" — https://learn.microsoft.com/windows/apps/design/signature-experiences/geometry | Primary | 8 px radius for top-level containers and flyouts |

## macOS / XNU

| Tag | Source | Kind | Used for |
|---|---|---|---|
| M1 | XNU `doc/vm/memorystatus.md` — https://github.com/apple-oss-distributions/xnu/blob/main/doc/vm/memorystatus.md | Primary | Jetsam design, bands, `memorystatus_available_pages`, actions (kill, freeze, notify, swap) |
| M2 | XNU `doc/vm/memorystatus_notify.md` — https://github.com/apple-oss-distributions/xnu/blob/main/doc/vm/memorystatus_notify.md | Primary | Pressure levels, `AVAILABLE_NON_COMPRESSED_MEMORY`, rising and falling thresholds, macOS threshold values, low-swap notification |
| M3 | XNU `bsd/kern/kern_memorystatus_notify.c` — https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_memorystatus_notify.c | Primary | `kern.memorystatus_vm_pressure_level` returns the **dispatch bitmask** (1/2/4) and needs no privilege on macOS. `kern.memorypressure_manual_trigger` exists on release kernels, is write-only and root-only, and **latches** the level until reset to NORMAL. |
| M4 | XNU `bsd/kern/proc_info.c` — https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/proc_info.c | Primary | `proc_pid_rusage` is gated by `CHECK_SAME_USER`: same UID, or `PRIV_GLOBAL_PROC_INFO` (root) |
| M5 | XNU `osfmk/mach/vm_statistics.h` — https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/vm_statistics.h | Primary | `vm_statistics64` fields. Speculative pages are counted inside `free_count`. |
| M6 | XNU `bsd/sys/event_private.h` — https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/event_private.h | Primary | `NOTE_MEMORYSTATUS_PRESSURE_NORMAL=0x1`, `WARN=0x2`, `CRITICAL=0x4`, `LOW_SWAP=0x8`, `PROC_LIMIT_*` |
| M7 | XNU `bsd/kern/kern_memorystatus.c` — https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_memorystatus.c | Primary | `kern.memorystatus_level` (system free percentage, readable) |
| M8 | Apple `system_cmds/memory_pressure/memory_pressure.c` — https://github.com/apple-oss-distributions/system_cmds | Primary | `-S` uses `(TEST_LOW_MEMORY_PURGEABLE_TRIGGER_ALL<<16)|level` and then resets with NORMAL. "Free percentage" comes from `memorystatus_get_level`. |
| M9 | Apple `system_cmds/purge/purge.c` | Primary | `purge` is just `vfs_purge()`: it flushes the disk buffer cache |
| M10 | exelban/stats, `Modules/RAM/readers.swift` — https://github.com/exelban/stats | Reference implementation | Activity-Monitor-style formulas (App, Wired, Compressed, Cache), `vm.swapusage`, mapping of pressure 2 to warning and 4 to critical |
| M11 | Chromium `base/memory/memory_pressure_monitor_mac.cc` — https://chromium.googlesource.com/chromium/src/+/main/base/memory/ | Reference implementation | Dispatch memory-pressure source plus sysctl polling for the current level |
| M12 | WebKit `ProcessMemoryFootprint.h` — https://opensource.apple.com/source/WTF/ | Reference implementation | `proc_pid_rusage(RUSAGE_INFO_V4).ri_phys_footprint` as the canonical footprint |
| M13 | Apple, `SMAppService` docs — https://developer.apple.com/documentation/servicemanagement/smappservice | Primary | Registering a bundled LaunchDaemon (macOS 13+), user approval in System Settings |
| M14 | Apple, `NSXPCConnection.setCodeSigningRequirement(_:)` — https://developer.apple.com/documentation/foundation/nsxpcconnection/setcodesigningrequirement(_:) | Primary | Peer validation for the privileged helper (macOS 13+) |
| M15 | rio-window `platform_impl/macos/glass.rs` — https://docs.rs/crate/rio-window/latest/source/src/platform_impl/macos/glass.rs | Reference implementation | `NSGlassEffectView` (macOS 26) looked up at runtime, with fallback to classic blur |
| M16 | Apple, "Responding to low-memory warnings" / `DispatchSource.makeMemoryPressureSource` — https://developer.apple.com/documentation/dispatch/dispatchsource/makememorypressuresource(eventmask:queue:) | Primary | Event-driven pressure notification for user-space apps |

## General / statistics

| Tag | Source | Used for |
|---|---|---|
| G1 | P. K. Sen, "Estimates of the regression coefficient based on Kendall's tau", JASA 63 (1968) | Theil–Sen slope, a median of pairwise slopes that is robust to up to 29% outliers |
| G2 | H. B. Mann (1945), M. G. Kendall (1975), the Mann–Kendall trend test | Non-parametric monotonic-trend significance |
| G3 | US Patent 9,064,048 "Memory leak detection" — https://patents.google.com/patent/US9064048 | Regression-on-samples leak detection in production monitors |
| G4 | US Patent App. 2014/0372807 "Memory leak detection using transient workload detection and clustering" | Separating workload-driven growth from leaks |
