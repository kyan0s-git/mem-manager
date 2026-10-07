# macOS memory management: internals, what is observable, and what is safe to touch

> Scope: macOS 13 Ventura and later, Apple Silicon and Intel.
> Tags such as [M3] refer to [`sources.md`](sources.md).
> Almost everything here is verified against **XNU source and Apple's own system_cmds source**.
> That is why the macOS product can be precise about what it does and does not do.

---

## 1. The Mach VM in one page

- **Pages.** On Apple Silicon the page size is **16 KB**; on Intel it is 4 KB. Always use
  `vm_page_size` or `hw.pagesize` and never hard-code it. Every page count in `vm_statistics64` is
  in these units [M5].
- **Two kinds of memory:**
  - **File-backed ("external")** pages are clean copies of files: code, mapped resources and the
    unified buffer cache. They can be dropped instantly and re-read.
  - **Anonymous ("internal")** pages are heaps, stacks and private dirty data. They cannot be
    dropped; they must be **compressed** or **swapped**.
- **Page queues**, as seen by user space through `host_statistics64(HOST_VM_INFO64)` [M5]:

| Field | Meaning |
|---|---|
| `free_count` | Free pages. **Includes `speculative_count`** [M5]. |
| `speculative_count` | Read-ahead file pages that nobody has touched yet; reclaimable first |
| `active_count` / `inactive_count` | Recently used / aging pages, both file-backed and anonymous |
| `wire_count` | Wired (kernel, drivers, GPU-wired on Apple Silicon); cannot be reclaimed |
| `purgeable_count` | Pages in **volatile purgeable** objects, which the kernel may simply discard |
| `throttled_count` | Pages held back from the pageout daemon |
| `external_page_count` / `internal_page_count` | File-backed / anonymous split |
| `compressor_page_count` | **Physical** pages used to store compressed data |
| `total_uncompressed_pages_in_compressor` | Logical pages held by the compressor (so ratio = this / `compressor_page_count`) |
| `compressions` / `decompressions` | Lifetime counters; the decompression rate is our "soft refault" signal |
| `pageins` / `pageouts` | Lifetime file page-ins and page-outs; the page-in rate is our "hard refault" signal |
| `swapins` / `swapouts`, `swapped_count` | Compressor segments moved to and from swap, and pages currently in swap |
| `purges` | Lifetime purgeable pages discarded |
| `reactivations` | Inactive pages touched again before reclaim |

- **Order of reclaim.** The pageout daemon ages pages from active to inactive. Under pressure it
  reclaims, in rough order:
  1. Speculative and clean file pages.
  2. **Purgeable volatile** objects.
  3. Inactive **anonymous pages, which are compressed**.
  4. When the compressor itself is pressured, **compressor segments are swapped out** to
     `/System/Volumes/VM/swapfile*`.

## 2. Purgeable memory: the cooperative cache

Apps mark caches as **volatile purgeable** (`VM_PURGABLE_VOLATILE`; `NSPurgeableData`; IOSurface;
Metal heaps; WebKit and CoreGraphics caches). The kernel may **empty them without asking**. The app
checks when it next wants the data, and regenerates it if it was purged. This is exactly what
Windows lacks. On macOS, *the kernel already "cleans" app caches* when needed [M1].

## 3. Memory pressure, precisely

**Measurement** [M2]:

```
AVAILABLE_NON_COMPRESSED = active + inactive + free + speculative
AVAILABLE                = AVAILABLE_NON_COMPRESSED + compressed
```

Pressure rises when `AVAILABLE_NON_COMPRESSED` falls below thresholds expressed as fractions of
`AVAILABLE`. **These are the macOS values, with hysteresis** [M2]:

| Level | Enter when below | Leave when above | Kernel action |
|---|---|---|---|
| **Warning** | 0.5 × AVAILABLE (`COMPRESSOR_COMPACT_THRESHOLD`) | 0.6 × AVAILABLE | Minor compaction of compressed segments; apps notified "consider relaxing caching policy" |
| (Swap) | 0.4 × AVAILABLE (`COMPRESSOR_SWAP_THRESHOLD`) | — | Major compaction; **begin swapping** compressor segments |
| **Critical** | 1.2 × 0.29 × AVAILABLE ≈ 0.348 | 1.4 × 0.29 ≈ 0.406 | Swapper unthrottled; apps told "expect latencies, drop all caches" |

**Levels: the internal values differ from what user space sees.**

- Internally there are five levels: Normal 0, Warning 1, Urgent 2 (≡ Warning), Critical 3, and
  Jetsam 4 (kernel-only) [M2].
- **The sysctl `kern.memorystatus_vm_pressure_level` does *not* return those.** It returns the
  **dispatch bitmask value** produced by `convert_internal_pressure_level_to_dispatch_level()`:
  **1 = normal, 2 = warning (including urgent), 4 = critical** [M3][M6].
  - exelban/stats maps exactly 2→warning and 4→critical [M10].
  - Code that compares against 0/1/3 is wrong. This is a common bug.
- On macOS the sysctl is readable **without privilege**: the privilege check is compiled only for
  `!XNU_TARGET_OS_OSX` [M3].
- `kern.memorystatus_level` is the system "free percentage" that `memory_pressure` prints. It is
  readable by anyone [M7][M8].

**Event-driven notification.** `DispatchSource.makeMemoryPressureSource(eventMask: [.normal, .warning, .critical])`
fires on transitions [M16]. Chromium registers exactly this, and reads the sysctl on demand for the
current level [M11]. It costs nothing while idle: no polling and no wakeups. That makes it the
backbone of the macOS app's sampling.

Further masks [M2][M6]:

- `PROC_LIMIT_WARN` / `PROC_LIMIT_CRITICAL` relate to *our own* per-process limit.
- `LOW_SWAP` (0x8) is **root-only**. It fires when the compressor or swap is exhausted. This is the
  condition behind the "Your system has run out of application memory" dialog.

## 4. memorystatus / jetsam on macOS

Jetsam is the kernel's killer. It works on 210 priority bands, and each process is assigned a band
by RunningBoard or launchd [M1]. On **iOS** it routinely kills apps. On **macOS** it is mostly
limited to:

- **Idle exit** of launchd daemons that opted into pressured exit. On macOS the `VM_pressure`
  thread also performs idle-exit kills [M1].
- **Limit kills** of daemons over their JetsamProperties memory limits.
- **Last-resort kills** when the compressor and swap space are exhausted.

Jetsam watches a *different* metric from pressure: `memorystatus_available_pages`, which is
fully-reclaimable memory (file-backed + free + purgeable, roughly) as a fraction of total RAM. Its
thresholds are typically around 5/10/15% [M1][M2]. Foreground apps on macOS are not jetsammed under
normal conditions. Instead, the system compresses and swaps, and in the end shows the force-quit
dialog.

**Implication.** The user-visible failure modes on macOS are **swap thrash** (beachballs, SSD
writes) and the **out-of-application-memory dialog**. Our watchdog targets both: swap growth plus
compressor saturation plus sustained pressure.

## 5. Making sense of Activity Monitor numbers

The formulas below are what Activity Monitor appears to use. They are confirmed by the widely used
Stats implementation [M10]:

| Activity Monitor | Formula from `vm_statistics64` (× page size) |
|---|---|
| Physical Memory | `hw.memsize` |
| **Memory Used** | `active + inactive + speculative + wired + compressor − purgeable − external` |
| App Memory | `Memory Used − wired − compressor` |
| Wired Memory | `wire_count` |
| Compressed | `compressor_page_count` (physical pages consumed by the compressor) |
| Cached Files | `purgeable + external` |
| Swap Used | `sysctl vm.swapusage` → `xsw_usage.xsu_used` |
| Memory Pressure graph | `kern.memorystatus_vm_pressure_level` colour, plus `kern.memorystatus_level` |
| Per-process "Memory" | `ri_phys_footprint` from `proc_pid_rusage` [M12] |

**`phys_footprint`** is the kernel's own charge for a process. It counts dirty private memory,
**including compressed and swapped pages**, plus IOKit and graphics allocations attributed to the
task. Unlike RSS, it **does not shrink when the process's pages are compressed**. That makes it
the right signal for leak trends [M12]. `ri_lifetime_max_phys_footprint` gives the peak.

## 6. What an unprivileged app can see

| Data | API | Requirement |
|---|---|---|
| System VM stats | `host_statistics64(mach_host_self(), HOST_VM_INFO64)` | None |
| Pressure level, free % | `sysctlbyname("kern.memorystatus_vm_pressure_level" / "kern.memorystatus_level")` | None on macOS [M3][M7] |
| Swap | `sysctlbyname("vm.swapusage")` | None |
| Pressure events | Dispatch memory-pressure source | None |
| List all PIDs | `proc_listallpids` | None (`NO_CHECK_SAME_USER`) [M4] |
| Per-process footprint | `proc_pid_rusage(pid, RUSAGE_INFO_V4)` | **Same UID**, or `PRIV_GLOBAL_PROC_INFO` (root) [M4] |
| Process name / path | `proc_name`, `proc_pidpath` | Same-UID rules apply for some flavours |
| App identity | `NSRunningApplication(processIdentifier:)` | None |
| Heap / VM-region inspection (`leaks`, `vmmap`, `footprint -p`) | `task_for_pid` / `task_read_for_pid` | Root plus debugging entitlement, blocked for hardened or SIP apps. **Not available to us.** |

So the unprivileged app can monitor **every process owned by the logged-in user**: all GUI apps,
browsers and their helper processes, Electron apps, and user agents. These are where nearly all
user-visible leaks live. System daemons (WindowServer runs as `_windowserver`, plus root daemons)
are invisible without root. We note this as a possible future, *read-only*, helper extension, not
part of v1.

## 7. Every "cleaning" option on macOS, and its real effect

| Option | What it really is | Privilege | Effect | Our policy |
|---|---|---|---|---|
| `purge` | **`vfs_purge()`**, nothing else [M9]: flushes the unified buffer cache (clean file-backed pages) | root | "Cached Files" drops, and the next file accesses go to SSD | **Manual only**, through the helper, labelled "for benchmarking / cold-cache tests" |
| `memory_pressure -S -l warn` | `sysctl kern.memorypressure_manual_trigger = (6<<16) \| NOTE_..._WARN` [M8] | root (write-only sysctl, present on **release** kernels) [M3] | (a) Sets `memorystatus_vm_pressure_level` to Warning, **latched while `memorystatus_manual_testing_on`**. (b) If `memorystatus_purge_on_warning` is set, it loops `vm_purgeable_object_purge_one_unlocked`, **emptying volatile purgeable memory**. (c) Notifies **all** registered processes repeatedly until done, so they release caches. A later write of `(6<<16) \| NORMAL` restores the real level [M3]. | **"Nudge" (optional helper)**. This is a cooperative, kernel-sanctioned cache release. Reset is guaranteed (see the architecture doc) because **a crash between set and reset would leave the whole system believing it is under pressure**. |
| `memory_pressure -p N` | Allocates and touches memory until free% = N [M8] | none | Real pressure: compresses and swaps *everyone* | **Never.** It is the macOS equivalent of the "Hoax". |
| Quit app | `NSRunningApplication.terminate()` (graceful) / `forceTerminate()` | Same user | The only way to actually free anonymous memory | User-initiated, or opt-in auto-quit allowlist |
| Relaunch app | Terminate, then `NSWorkspace.openApplication` | Same user | Resets a leaky app | Offered when a leak is detected |
| Kill a daemon | `kill(2)` | Root for others | Usually respawned by launchd; risky | **Never** |

## 8. Apple Silicon specifics

- **Unified memory.** GPU buffers live in the same RAM. GPU-wired allocations appear in **wired**
  memory, and big ML or LLM workloads can wire tens of GB. `iogpu.wired_limit_mb` (sysctl, root)
  caps how much the GPU may wire. It is relevant to power users, and we show "Wired" prominently.
- **16 KB pages.** Every fixed-size buffer computed in pages must use the runtime page size.
- **SSD swap.** Swap is fast, but sustained heavy swap means SSD write wear. The watchdog reports
  cumulative `swapouts` per day as a health hint.
- **Efficiency cores.** Our sampling work runs at `.utility` or `.background` QoS, so it is
  scheduled on E-cores.

## 9. Being a good citizen

- **Timer leeway.** `DispatchSourceTimer.schedule(..., leeway: .seconds(n))` lets the OS coalesce
  our wakeups with others.
- **App Nap.** An `LSUIElement` agent with no visible windows is eligible. We don't fight it; the
  pressure source still wakes us.
- **Our own pressure handler** drops our history ring buffers beyond the minimum, along with
  rendered icon caches and any SwiftUI hosting controllers.
- **Purgeable for our caches.** Any cache we keep, such as rendered sparkline images, can be an
  `NSCache`, which empties itself under pressure.

## 10. Pathologies on macOS and the response to each

| Symptom | Likely cause | Our response |
|---|---|---|
| Yellow/red pressure, beachballs, swap climbing | Working set > RAM (many tabs, VMs, ML) | Show the top footprints, and suggest quitting idle heavy apps; optional Nudge |
| One app's memory climbs for hours | Leak in the app, a WebContent process or an Electron helper | Leak detector → notification with Quit / Relaunch / Ignore |
| "Out of application memory" dialog | Compressor and swap exhausted | Watchdog warned earlier (swap growth with pressure); name the culprit |
| High "Cached Files" | Normal: the file cache is working | **Explain, don't act.** This is the most common misconception. |
| High Wired | GPU/ML workloads, kernel extensions | Show it, explain it, and point to the GPU wired limit for power users |
| WindowServer huge | Multi-display or GPU leaks (system daemon) | Not visible without root; v1 cannot attribute it |

## 11. Summary for the design

1. **Events, not polling**: the dispatch pressure source plus slow adaptive sampling.
2. **Correct sysctl decoding** (1/2/4) and Activity-Monitor-consistent formulas.
3. **Leak detection on `phys_footprint`** for same-UID processes.
4. **Actions that respect the kernel**: notify, quit gracefully, relaunch; Nudge (optional, root,
   latch-safe).
5. **Never** periodic purge, and never allocate memory to fake pressure.
