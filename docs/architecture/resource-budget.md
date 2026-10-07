# Resource budget

> The tool must be negligible next to what it manages. These budgets are **release criteria**.
> A build that exceeds them is a bug.

## 1. Budgets

| Metric | Windows | macOS | Measured as |
|---|---|---|---|
| Idle memory (popup closed, 10 min after start) | **≤ 4 MB** private bytes, ≤ 6 MB working set | **≤ 15 MB** `phys_footprint` | Windows: `GetProcessMemoryInfo` → `PrivateUsage`, `WorkingSetSize`. macOS: `footprint <pid>` / `proc_pid_rusage` |
| Popup open | ≤ 15 MB private | ≤ 30 MB footprint | Same |
| After popup closes (30 s) | Back within 0.5 MB of idle | Back within 2 MB of idle | Same |
| Average CPU, idle | < 0.05% of one core | < 0.1% of one core | 1 h average: Windows `GetProcessTimes`; macOS `ri_user_time + ri_system_time` |
| Wakeups, idle | ≤ 1 per 10 s | ≤ 1 per 10 s | Windows: ETW/WPA thread context switches. macOS: `ri_pkg_idle_wkups` + `ri_interrupt_wkups`; Activity Monitor "Idle Wake Ups" |
| Wakeups, display off | ≈ 0 (kernel events only) | ≈ 0 | Same |
| Per-sample cost | ≤ 1 ms system tick, ≤ 5 ms process tick | Same | Internal timing counters, exposed in Settings › About |
| Binary size | ≤ 1 MB exe | ≤ 5 MB app bundle (universal) | Release artefacts |
| Cold start to icon visible | ≤ 150 ms | ≤ 300 ms | Process start → icon added |

## 2. How we stay inside it

- Event-driven waits. Windows waits on `MsgWaitForMultipleObjectsEx` over kernel memory events
  plus coalescible waitable timers. macOS uses the dispatch pressure source plus timers with
  leeway.
- One syscall per sample for all processes on Windows. On macOS, foreign-UID PIDs are cached as
  "skip".
- Fixed rings with no steady-state allocation. Grow-only scratch buffers.
- Heavy UI is created on demand and released afterwards: Direct2D devices, SwiftUI Settings.
- On Windows, the process trims its own working set after the popup closes, and runs at EcoQoS
  with low memory priority.
- No PDH or WMI on Windows. No Combine or SwiftUI on the macOS always-on path.

## 3. Verification

- **CI (later milestone).** A smoke test launches the app with `--selftest-budget 120`. It runs
  headless for 120 s with the sampling cadence ×10 faster, then prints JSON with peak and steady
  private bytes / footprint, wakeups and CPU. The job fails if any budget is exceeded.
  - Windows: `windows-latest`.
  - macOS: `macos-latest`. The status item runs fine on CI runners without a visible display
    session; if not, the self-test mode skips UI creation and measures the engine alone.
- **Manual protocol before release:**
  1. Fresh boot, let the app idle for 30 min, record the metrics.
  2. Open and close the popup 50 times, then check return-to-idle and that nothing leaks (the
     tool must not leak; the leak detector watches itself too).
  3. Leave it running for 24 h under a browser workload, then check that memory is flat
     (Theil–Sen slope on our own samples < 0.1 MB/h).
