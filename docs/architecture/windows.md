# Windows architecture: rigorous and assertive

> Citation tags ([W…], [M…]) refer to [`../research/sources.md`](../research/sources.md).
> Implements [`policy-engine.md`](policy-engine.md) with the levers researched in
> [`../research/windows-memory-internals.md`](../research/windows-memory-internals.md).
> Language: **Rust**, using the `windows` / `windows-sys` crates. **No async runtime, no GUI framework.**
> Supported OS: Windows 10 1809+ and Windows 11, on x64 and ARM64.

## 1. Process model

```
            (logon)                          ┌───────────────────────────────────────────┐
 Task Scheduler ──── "MemManager" task ────► │ memmanager.exe  (elevated, HIGHEST)        │
  ONLOGON, RunLevel=HIGHEST, no UAC prompt   │  one UI thread + one worker thread         │
                                             └───────────────────────────────────────────┘
 Manual launch (not elevated) ──► same exe in **monitor-only** mode ──► "Enable cleaning" button
                                         └─► ShellExecute "runas" self --register-task (one UAC prompt)
```

- **Manifest:**
  - `requestedExecutionLevel = asInvoker`, so a manual launch never prompts.
  - `dpiAwareness = PerMonitorV2`, `longPathAware`, Common Controls v6.
  - `supportedOS` GUIDs for Windows 10/11.
- **Elevation.** Task registration goes through `ITaskService` (COM, Task Scheduler 2.0):
  - Trigger: `TASK_TRIGGER_LOGON` for the current user. Principal: `TASK_RUNLEVEL_HIGHEST`,
    `TASK_LOGON_INTERACTIVE_TOKEN`.
  - Settings: `DisallowStartIfOnBatteries = false`, `ExecutionTimeLimit = PT0S`,
    `MultipleInstances = IgnoreNew`.
  - Removal: `--unregister-task`, also run by the uninstaller.
- **Single instance.** A named mutex `Local\MemManager.{GUID}`. A second launch posts a
  "show popup" message to the first.
- **Startup privileges.** `AdjustTokenPrivileges` enables `SeProfileSingleProcessPrivilege` and
  `SeIncreaseQuotaPrivilege` if they are present. Missing privileges switch the corresponding
  capabilities off (§4).

### Threads

| Thread | Responsibilities | Wakes on |
|---|---|---|
| **UI** | Message loop, tray icon, popup and settings rendering, notifications | Window messages only |
| **Worker** | Sampling, policy engine, actions, leak detector | `MsgWaitForMultipleObjectsEx` / `WaitForMultipleObjects` on: waitable timer, `LowMemoryCondition`, `HighCommitCondition`, a UI→worker command event |

The worker publishes a **snapshot** to the UI thread: a small `Copy` struct behind a seqlock,
followed by `PostMessage(WM_APP_SNAPSHOT)`. The UI thread never blocks on the worker. Two threads
keep long NT calls (process enumeration, a purge that takes 100+ ms) from stalling the tray.

## 2. Crate layout

```
windows/
  Cargo.toml                 workspace; release profile tuned for size (see §7)
  core/                      platform-neutral: policy engine, leak detector, ring buffers, config model
    src/lib.rs               (compiles and is unit-tested on Linux CI against spec/test-vectors)
  app/
    build.rs                 embeds manifest + version resource (+ icon)
    src/main.rs              entry, single instance, privilege setup, thread spawn
    src/nt.rs                NtQuerySystemInformation / NtSetSystemInformation FFI (classes 2, 5, 22, 80, 81, 130; minimal structs)
    src/sample.rs            Sample/ProcSample collection with reused buffers
    src/actions.rs           tiered actions (§3) + feature probing
    src/procs.rs             candidate selection: idle, foreground, audio, exclusions
    src/tray.rs              Shell_NotifyIcon v4, icon rasterizer, theme tracking
    src/popup.rs             flyout window, Direct2D/DirectWrite rendering, UIA provider
    src/settings.rs          settings window (same widget set)
    src/widgets.rs           tiny retained widget set: label, gauge, bar, button, toggle, slider, swatch, list
    src/task.rs              ITaskService registration
    src/config.rs            INI read/write (%APPDATA%\MemManager\config.ini)
    src/notify.rs            balloon/toast via NIF_INFO (no WinRT dependency)
```

## 3. Action tiers (Windows predicates)

The states and budgets come from the policy engine. The table gives the **platform predicate**
each action adds.

| Tier | Action | Min state | Predicate | Cooldown (×penalty) | Bucket (Balanced) |
|---|---|---|---|---|---|
| 1 | **Deprioritise idle background processes** (`ProcessMemoryPriority = LOW (2)`, or `VERY_LOW (1)` at High) | Elevated | Process idle for ≥ N min (no CPU-time delta, not foreground, no audio session, not excluded, not a system-critical image) | 60 s | ∞ (cheap and reversible) |
| 2 | **Purge priority-0 standby** | Elevated | `standby[0] ≥ 256 MiB` | 30 s | 10 / 60 per h |
| 3 | **Purge all standby** | High | `fast_avail < F_target` after tier 2 **and** (Σ repurpose rate over priorities 0–2 ≥ 0.05% of RAM/s **or** game mode) | 120 s | 2 / 4 per h |
| 4a | **Selective trim** (`EmptyWorkingSet` on up to 5 candidates) | High | Physical (not commit) pressure; candidates from §5 | 120 s | 6 / 12 per h |
| 4b | **Flush modified list** | High | `modified ≥ 5%` of RAM | 300 s | 2 / 4 per h |
| 5 | **Combine pages** | Normal + idle | User idle ≥ 5 min, on AC, last run ≥ 1 h | 3600 s | 1 / 1 per h |
| 5 | **File-cache cap** (Aggressive only) | Elevated | System cache WS ≥ 25% of RAM and growing | — (state toggles on and off) | — |
| — | **Deep clean** (manual) | — | User confirms. Sequence: WS empty → file cache → modified → standby → combine | — | — |

- **Restore.** Memory priorities that we lowered are restored to NORMAL when a process becomes
  foreground (`EVENT_SYSTEM_FOREGROUND` hook), and on app exit. A small table tracks them, at most
  256 entries.
- **Game mode.** `SHQueryUserNotificationState() == QUNS_RUNNING_D3D_FULL_SCREEN` or `QUNS_BUSY`
  with a fullscreen foreground window. While it is on:
  - Tiers 1 and 4 are suspended. Never trim anything while the user is gaming.
  - Tier 3 uses an ISLC-style predicate: `free+zero < max(1 GiB, 6% RAM)` and
    `standby ≥ 1 GiB`, with a 20 s cooldown.

## 4. Capability probing

At startup, and after resume from sleep, the app probes once and caches the result:

| Capability | Probe | When missing |
|---|---|---|
| `mem_list_query` | Query class 80 | Fall back to `GetPerformanceInfo` (no per-priority data, so tier 3 uses only free-memory conditions) |
| `mem_list_cmd` | `SeProfileSingleProcessPrivilege` enabled | Tiers 2, 3, 4b and 5-combine are disabled; UI shows "Limited" |
| `file_cache` | `SeIncreaseQuotaPrivilege` enabled | The file-cache cap and Deep clean step are disabled |
| `combine` | First call returns `STATUS_INVALID_INFO_CLASS` | Disabled |
| `pool_tags` | Query `SystemPoolTagInformation` | Pool attribution disabled (totals still shown) |
| `backdrop` | Build ≥ 22621 | Solid popup background |

## 5. Selective-trim candidate rules

A process is a candidate only if **all** of the following hold:

- It is not the foreground process, and not in the foreground process's tree (same root GUI app).
- It has no active audio session (`IAudioSessionManager2`; checked lazily, only for top
  candidates).
- Its CPU time delta is 0 over the idle window, and its working set ≥ 100 MiB.
- It is not on the built-in critical list: `csrss`, `wininit`, `winlogon`, `lsass`, `services`,
  `smss`, `dwm`, `audiodg`, `fontdrvhost`, `MsMpEng`, `Memory Compression`, `System`, `Registry`,
  `explorer`, the shell host processes, and ourselves.
- It is not on the user's exclusion list (by image name or path).
- It can be opened with `PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION`.

**Ranking.** `score = working_set × idle_minutes^0.5`; we trim the top 5.

## 6. Data and memory layout (self-footprint)

| Structure | Size | Notes |
|---|---|---|
| System history ring | 720 × 48 B ≈ 34 KB | 1 h at 5 s |
| Process snapshot buffer | Grow-only, typically 256–512 KB | Reused for every `SystemProcessInformation` call |
| Per-process summary table | ≤ 1024 × 32 B | Open addressing, keyed by (pid, create time) |
| Leak candidate rings | 24 × 1.4 KB + 24 × 1.7 KB | See leak-detection.md §2 |
| Ledger | 64 × 64 B | |
| Icon DIB | 32 × 32 × 4 B | Reused |
| D2D/DWrite device resources | Created on popup show, **released on hide** | |

- **No allocations on the steady-state sampling path.** All buffers are pre-sized or grow-only.
- After the popup hides: release D2D resources, call `HeapCompact(GetProcessHeap())`, then
  `SetProcessWorkingSetSizeEx(self, -1, -1)`.
- **Self-throttling** (via `SetProcessInformation`):
  - Our own `ProcessPowerThrottling` uses `EXECUTION_SPEED` (EcoQoS), plus `IGNORE_TIMER_RESOLUTION`.
  - Our own `ProcessMemoryPriority` is LOW.
  - The worker runs at `THREAD_PRIORITY_BELOW_NORMAL`, except when it is executing a user-requested
    action.

## 7. Build configuration

```toml
[profile.release]
opt-level = "s"         # size; hot loops are tiny
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
debug = false
```

- Statically link the CRT (`-C target-feature=+crt-static`), so there is no VC++ redistributable.
- Targets: `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`.
- Sign with Authenticode in the release workflow, so the tray GUID binding (see
  low-footprint-ui.md) and antivirus reputation both work.

## 8. UI summary

The visual spec is in [`ui-design.md`](ui-design.md).

- **Tray:** dynamic icon (ring, bar or number), state colours, tooltip sentence.
  - Left click: popup.
  - Right click: menu with Clean now (tiered), Deep clean…, Pause 1 h, Profile ▸, Settings,
    Exit.
- **Popup** (360 × ~420 epx):
  - Header: big gauge with used %, plus state chip.
  - Breakdown bar: In use / Modified / Standby / Free.
  - Commit line, and the compressed store figure.
  - 10-minute sparkline.
  - Top 5 processes by private bytes, with growth arrows and a leak badge.
  - Last 3 ledger entries with "freed / refaults".
  - Footer: Clean button, profile segmented control, settings gear.
- **Notifications:** leak alerts, Deep clean results, and privilege-missing hints. They use
  `NIF_INFO` balloons, which Windows 10/11 render as toasts, so there is no WinRT dependency.
