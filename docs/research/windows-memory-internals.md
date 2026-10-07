# Windows memory management: internals and every lever a manager can pull

> Scope: Windows 10 (1809 and later) and Windows 11, x64 and ARM64.
> Tags such as [W1] refer to [`sources.md`](sources.md).
> Anything marked **undocumented** relies on the NT native API as described by the phnt headers
> [W1]. It has been stable since Vista/Windows 8, but Microsoft does not guarantee it. The app
> must feature-detect it and degrade gracefully (§9).

---

## 1. The physical page life cycle

Windows tracks every physical page in the **PFN database**. Each page is in exactly one state
[W10]:

| State | Meaning | Counted as "Available"? |
|---|---|---|
| **Active / Valid** | Mapped in some working set (process, system or session) | No |
| **Transition** | I/O in progress | No |
| **Modified** | Removed from a working set, contents dirty and **not yet written** to pagefile or file | No; Task Manager shows it as part of "Modified" |
| **Modified-no-write** | Dirty, but must not be written yet (used by file systems for log ordering) | No |
| **Standby** | Removed from a working set, contents **clean** (match the backing store). Can be handed back to the owner on a *soft fault*, or repurposed. | **Yes** |
| **Free** | Unowned, contents garbage | Yes |
| **Zeroed** | Unowned and zero-filled (the zero-page thread runs at idle priority) | Yes |
| **Bad** | Hardware error | No |

The flow:

```
                 trim / age                    modified page writer
  Working set ──────────────► Modified list ─────────────────────────► Standby list (8 priorities)
      ▲    ▲                      │  (private pages since Win10: → compression store)      │
      │    │ soft fault           │                                                         │ repurpose
      │    └──────────────────────┘◄───────────────────── soft fault ───────────────────────┤ (lowest
      │                                                                                     │  priority first)
      │ demand-zero / hard fault                                                            ▼
      └──────────────────────────────────────── Zero list ◄── zero thread ── Free list ◄────┘
```

**Key consequence.** Task Manager's **Available = Standby + Free + Zeroed**. A cleaner that
trims working sets moves pages from *Active* into *Standby/Modified*. "Available" goes up, but
nothing has been freed in any useful sense, and the owners will soft-fault those pages straight
back [W9].

## 2. Standby priorities and repurposing

Since Vista, every physical page carries a **page priority from 0 to 7**. The standby list is
really **eight lists**. When the memory manager needs a page and the free and zero lists are empty,
it **repurposes from the lowest-priority standby list first** [W10]. The priorities are used like
this:

| Priority | Typical source |
|---|---|
| 0 | Not normally set by user processes. `MemoryPurgeLowPriorityStandbyList` purges exactly this list [W1][W8]. |
| 1–5 | Pages faulted in by a process take that process's **memory priority**: `MEMORY_PRIORITY_VERY_LOW=1`, `LOW=2`, `MEDIUM=3`, `BELOW_NORMAL=4`, **`NORMAL=5` (default)** [W7]. Background I/O (`PROCESS_MODE_BACKGROUND_BEGIN`) uses low priority. |
| low (≤2) | **SysMain (Superfetch) speculative prefetch** is inserted at low priorities, so it is the first to go [W10] |
| 6–7 | Reserved for SysMain, which raises pages it predicts are hot so they survive repurposing [W10] |

`NtQuerySystemInformation(SystemMemoryListInformation)` returns [W1]:

```c
typedef struct _SYSTEM_MEMORY_LIST_INFORMATION {
    SIZE_T ZeroPageCount;
    SIZE_T FreePageCount;
    SIZE_T ModifiedPageCount;
    SIZE_T ModifiedNoWritePageCount;
    SIZE_T BadPageCount;
    SIZE_T PageCountByPriority[8];        // standby pages per priority
    SIZE_T RepurposedPagesByPriority[8];  // cumulative repurposes per priority
    SIZE_T ModifiedPageCountPageFile;
} SYSTEM_MEMORY_LIST_INFORMATION;
```

**`RepurposedPagesByPriority` is the most important signal for this project.** It is a cumulative
counter of standby pages at each priority that were **taken away** to satisfy new demand. Its
deltas over time show how the cache is doing:

- **Repurposes ≈ 0** at all priorities: the cache is not under contention, and purging it only
  destroys value.
- **Repurposes climbing at priorities 0–2 only**: normal. Speculative and prefetched data is being
  recycled as designed.
- **Repurposes climbing at priority 5 and above**: real working-set data is being evicted. The
  system is genuinely short of memory, and refaults will follow.

This is how the Windows app "balances low RAM use with caching performance". It purges only cache
that the kernel is *already* throwing away, and only when doing so ahead of time saves latency
(for example, avoiding repurpose work in the middle of a frame while gaming).

## 3. Working sets and the working-set manager

- **Process working sets** have soft minimum and maximum limits by default: 50 and 345 pages
  [W3]. The working-set manager (part of the *balance set manager*, which runs about once a
  second) **ages** pages using the accessed bits and **trims** working sets when available memory
  is low. It trims processes that are over their minimum, and idle ones first [W10].
- The **system working set** holds the file system cache, paged pool, and pageable kernel and
  driver code.
- **Hard limits.** `SetProcessWorkingSetSizeEx(..., QUOTA_LIMITS_HARDWS_MAX_ENABLE)` makes the
  maximum binding. The process can never hold more than that much RAM resident, and excess
  accesses become a soft-fault loop [W3]. This is how tools such as Process Lasso implement
  "memory limits". It **reduces RAM, not commit**, and a too-tight cap turns into thrashing.
- **Emptying.** `EmptyWorkingSet(h)`, or `SetProcessWorkingSetSize(h, -1, -1)`, removes as many
  pages as possible from one process [W3][W4]. It needs `PROCESS_SET_QUOTA` plus
  `PROCESS_QUERY_(LIMITED_)INFORMATION`.

## 4. Memory compression (Windows 10 and later)

The **store manager** keeps compressed **private** pages in the working set of the minimal
process **"Memory Compression"** (previously inside the System process) [W11]:

- When private pages are trimmed, they go to the modified list and are then **compressed into the
  store** instead of being written straight to the pagefile. The store itself can later be paged
  to disk.
- A fault on a compressed page is a **soft fault plus decompression**: microseconds of CPU, but
  far cheaper than disk.
- Typical compression ratios are reported at around 30–50% [W11].

**Implications for a cleaner:**

1. Emptying working sets of private memory mostly **moves data into the compression store**. It
   costs CPU to compress now and to decompress on the next touch, and the *Memory Compression*
   process's working set grows. The net RAM gain is the compression ratio, not the full size.
2. The store's size is a pressure indicator in its own right. We read it as the working set of the
   *Memory Compression* process from `SystemProcessInformation`.

## 5. Commit charge vs. physical memory

These are different resources, and confusing them is the most common cleaner mistake:

- **Commit charge** is the total private virtual memory that has been *promised*. It must be
  backed by RAM plus pagefiles, and the **commit limit** is RAM + pagefile size(s). When commit
  reaches the limit, allocations fail, apps crash, and Windows tries to grow the pagefile. Read
  it with `GetPerformanceInfo` (`CommitTotal`/`CommitLimit`) or with
  `SYSTEM_PERFORMANCE_INFORMATION.CommittedPages`/`CommitLimit` [W1].
- **Physical usage** is working sets plus modified pages plus the compression store.

| Problem | Does trimming help? | What helps |
|---|---|---|
| Low *available* RAM (high physical) | Briefly, at the cost of refaults | Memory priority, selective trims, standby shaping |
| High *commit* (approaching the limit) | **No.** Trimming does not uncommit anything. | Find the process with growing **private bytes** and restart or close it; grow the pagefile |

A user-mode memory leak is **commit growth**. A per-process `PrivatePageCount` that keeps rising
is the canonical leak signal (see [`leak-detection.md`](leak-detection.md)).

## 6. File cache, SysMain, and the "metafile" case

- **Cache Manager.** File data is cached in the system working set. When trimmed, it goes to
  standby. Large sequential reads (copies, backups, game installs) can fill standby with
  single-use data at priority 5, competing with useful pages.
  - **Mitigation:** run the copying tool at background or low memory priority, or temporarily cap
    the file cache with `SetSystemFileCacheSize(min, max, FILE_CACHE_MAX_HARD_ENABLE)` [W6].
  - **Flush:** `SetSystemFileCacheSize((SIZE_T)-1, (SIZE_T)-1, 0)` [W6], or
    `NtSetSystemInformation(SystemFileCacheInformationEx, {Min=Max=-1})` [W1][W8]. Either needs
    `SeIncreaseQuotaPrivilege`.
- **NTFS metafile bloat.** On machines with millions of files, the cached MFT and metadata can
  grow without bound in the system working set. Microsoft's old "DynCache" service exists solely
  to cap it with `SetSystemFileCacheSize`. A file-cache hard cap is therefore a legitimate
  *aggressive-mode* feature.
- **SysMain (Superfetch)** fills standby with predicted pages at low priority and boosts hot ones.
  - **Purging all standby destroys SysMain's work**, and SysMain will spend I/O rebuilding it.
    This is another reason to prefer the *low-priority* purge and repurpose-driven decisions.
  - SysMain uses `MemoryCaptureAccessedBits` (commands 0/1) internally. We never call those.

## 7. Kernel pools

Kernel allocations come from **paged pool** and **nonpaged pool**. Each allocation is tagged with a
4-character **pool tag**. Leaks in drivers (network, VPN, antivirus, RGB utilities) show up as
pool growth while every process looks flat [W14].

- **Totals.** `SYSTEM_PERFORMANCE_INFORMATION.PagedPoolPages` and `NonPagedPoolPages`, plus the
  alloc/free counters [W1]. `GetPerformanceInfo` also exposes `KernelPaged` and `KernelNonpaged`.
- **Per tag.** `NtQuerySystemInformation(SystemPoolTagInformation)` returns
  `SYSTEM_POOLTAG { Tag; PagedAllocs; PagedFrees; PagedUsed; NonPagedAllocs; NonPagedFrees; NonPagedUsed }`
  for every tag [W1]. This is exactly what PoolMon shows.
- **Leak signature.** A tag whose `Used` grows monotonically while `Allocs − Frees` keeps
  increasing [W14].
- **Mapping a tag to a driver.** Use `pooltag.txt` from the Debugging Tools, or search
  `%SystemRoot%\System32\drivers\*.sys` for the 4-byte tag. We ship a small built-in table of
  common tags and fall back to scanning drivers on demand.
- **Nothing in user mode can free leaked pool.** The product can only *detect and explain* it, for
  example: "Nonpaged pool grew 1.8 GB in 6 h, top tag `NtFC` (NTFS) — likely a file-handle leak
  in a sync client; a reboot reclaims it."

## 8. Low-memory notifications

`CreateMemoryResourceNotification(LowMemoryResourceNotification | HighMemoryResourceNotification)`
returns a **system-wide** waitable handle [W5]. There is a dead band between "low" and "high" in
which neither is signalled. Internally these are the named events
`\KernelObjects\LowMemoryCondition` and `HighMemoryCondition`. The same directory also has
`Low/HighPagedPoolCondition`, `Low/HighNonPagedPoolCondition`, `Low/High/MaximumCommitCondition`
[W10].

**Design notes:**

- These events are **free to wait on**, because `WaitForMultipleObjects` costs nothing until
  signalled. That makes them ideal wake sources, so we rarely need to poll.
- The *Low* threshold is set by the kernel very low: tens of MB, scaled with RAM [W10]. That is
  far too late for proactive management. We use **LowMemoryCondition as an emergency trigger**,
  and our own thresholds, evaluated on a slow timer, for everything else.
- The **commit** condition events are opened by name through `NtOpenEvent` in
  `\KernelObjects` (undocumented path). They are the cheapest way to react to commit pressure
  immediately.

## 9. Every operation: mechanism, cost, privilege and our policy

All `NtSetSystemInformation` calls need an **elevated token with the named privilege enabled**
through `AdjustTokenPrivileges`. Administrators hold `SeProfileSingleProcessPrivilege` and
`SeIncreaseQuotaPrivilege`, but they are *disabled* by default [W1][W8].

| # | Operation | API | Privilege / access | Min OS | What it does | Cost / risk | Our policy |
|---|---|---|---|---|---|---|---|
| 1 | **Set memory priority** | `SetProcessInformation(h, ProcessMemoryPriority, {1..5})` [W2] | `PROCESS_SET_INFORMATION` | Win8 | New pages from that process get lower priority, so they are trimmed and repurposed first | None while memory is plentiful; that process soft-faults more under pressure | **Tier 1, automatic.** Applied to idle background processes, never to the foreground app, audio, or anything on the exclusion list. Restored when the process becomes foreground. |
| 2 | **Purge low-priority standby** | `NtSetSystemInformation(80, MemoryPurgeLowPriorityStandbyList=5)` [W1] | SeProfileSingleProcess | Vista | Priority-0 standby → free | Negligible; that data was going to be repurposed first anyway | **Tier 2, automatic** when free+zero drops below threshold F₁ |
| 3 | **Purge standby** | `NtSetSystemInformation(80, MemoryPurgeStandbyList=4)` [W1] | SeProfileSingleProcess | Vista | All standby → free | Destroys the file and SysMain cache, so hard faults follow; SysMain rebuilds | **Tier 3, automatic only** when free is still below F₂ **and** the priority-≤2 repurpose rate is high, or **in game mode** (ISLC-style [W15]). Token-bucket limited and regret-tracked. |
| 4 | **Flush modified list** | `NtSetSystemInformation(80, MemoryFlushModifiedList=3)` [W1] | SeProfileSingleProcess | Vista | Writes dirty pages to backing store; they become standby | Write I/O burst | **Tier 4**, when the modified list exceeds M% of RAM or before a full standby purge in Deep clean |
| 5 | **Selective working-set trim** | `EmptyWorkingSet(h)` [W4] | `PROCESS_SET_QUOTA` + `QUERY_LIMITED` | XP | Trims one process | That process soft-faults on next use; private pages are compressed (CPU) | **Tier 4, automatic** only under *physical* pressure, and only for processes idle more than N min, not foreground, not audible, not excluded. Largest-and-idlest first. |
| 6 | **Empty all working sets** | `NtSetSystemInformation(80, MemoryEmptyWorkingSets=2)` [W1][W8] | SeProfileSingleProcess | Vista | Trims every process **and the system working set** | **Fault storm.** UI hitches for seconds, compression CPU spike. This is the "Hoax" operation [W9]. | **Manual "Deep clean" only**, with an explanatory confirmation |
| 7 | **Flush file cache** | `SetSystemFileCacheSize(-1,-1,0)` [W6] or `SystemFileCacheInformationEx` [W1] | SeIncreaseQuota | XP | Trims the system cache working set to standby | Cached file data becomes standby (still recoverable until repurposed) | Part of Deep clean; automatic only when the cache WS is more than C% of RAM and growing (metafile case) |
| 8 | **Cap file cache** | `SetSystemFileCacheSize(min, max, FILE_CACHE_MAX_HARD_ENABLE)` [W6] | SeIncreaseQuota | XP | Hard upper bound on the system cache WS | Slower large-file workloads if set too low | **Aggressive profile only**, opt-in; cleared on exit and on profile change |
| 9 | **Combine pages** | `NtSetSystemInformation(130, MEMORY_COMBINE_INFORMATION_EX{Flags})` [W1][W8] | SeProfileSingleProcess | Win10 (client) | Deduplicates identical physical pages (copy-on-write) | CPU scan proportional to RAM | **Idle-time maintenance**: at most once per hour, and only when the machine is idle and on AC power |
| 10 | **Hard WS cap on a process** | `SetProcessWorkingSetSizeEx(h, min, max, HARDWS_MAX_ENABLE)` [W3] | `PROCESS_SET_QUOTA` | 2003 | Binding RAM limit for one process | Soft-fault thrash if too tight; **does not limit commit** | Offered only as a *user action* on a flagged runaway ("Limit RAM to …"), with an explanation |
| 11 | **Registry hive flush** | `NtSetSystemInformation(SystemRegistryReconciliationInformation)` [W8] | admin | 8.1 | Flushes lazy registry writes | Negligible effect on memory | Not used (no meaningful benefit) |
| 12 | **Flush volume caches** | `FlushFileBuffers` on `\\.\X:` [W8] | admin | XP | Writes dirty file-cache data | I/O burst | Deep clean only |

**Feature detection.** Each NT call that returns `STATUS_INVALID_INFO_CLASS`,
`STATUS_NOT_IMPLEMENTED` or `STATUS_PRIVILEGE_NOT_HELD` disables that capability for the session.
The UI shows it as unavailable instead of retrying in a loop. This is the lesson of Mem Reduct
issue #191, where auto-clean looped while above threshold and was fixed with a cooldown [W8].

## 10. Measuring state cheaply

| Need | Cheapest source | Notes |
|---|---|---|
| Lists, standby by priority, repurposes | `NtQuerySystemInformation(SystemMemoryListInformation)` | One call, fixed 96-byte struct [W1] |
| Commit, pools, fault counters, page reads | `NtQuerySystemInformation(SystemPerformanceInformation)` | `PageReadCount` (hard), `TransitionCount` / `CacheTransitionCount` (soft), `DemandZeroCount`, pool pages [W1] |
| Totals, page size | `GlobalMemoryStatusEx`, `GetPerformanceInfo` | Documented fallbacks |
| Per-process memory | `NtQuerySystemInformation(SystemProcessInformation)` | **One call for all processes**: `WorkingSetSize`, `PrivatePageCount`, `PagefileUsage`, `PeakWorkingSetSize`, `HandleCount`, image name, PID. The buffer is reused (grow-only). It costs about 100–300 µs on 300 processes, versus milliseconds for `OpenProcess` plus `GetProcessMemoryInfo` per PID. |
| Pool tags | `NtQuerySystemInformation(SystemPoolTagInformation)` | Only sampled every few minutes, or on demand |
| Foreground and game state | `GetForegroundWindow`, `SHQueryUserNotificationState` (`QUNS_RUNNING_D3D_FULL_SCREEN=3`, `QUNS_BUSY=2`, `QUNS_PRESENTATION_MODE=4`) [W13] | Event-driven via `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` |
| Audio playing | `IAudioSessionManager2` session enumeration | Only checked before trimming a candidate |
| Display on/off, AC/battery | `RegisterPowerSettingNotification(GUID_CONSOLE_DISPLAY_STATE, GUID_ACDC_POWER_SOURCE)` | Pause sampling while the display is off |

**PDH and WMI are deliberately avoided.** Both pull in large DLLs and add megabytes of working set.
ISLC's dependency on performance counters is also why it breaks when the counters are corrupted
[W15].

## 11. Pathologies and the lever for each

| Symptom | Root cause | Lever | Tier |
|---|---|---|---|
| Frame-time spikes in games with "lots of free RAM" | Large standby, tiny free+zero, repurposing in the hot path | Purge standby **before** it is needed while fullscreen (free < F_game) | Game mode |
| Sluggish after a big file copy | Single-use file data at priority 5 pushing out app pages | Lower the copy process's memory priority; purge low-priority standby; optional cache cap | 1–2, 8 |
| Background apps hogging RAM (chat, launchers, updaters) | Idle working sets at normal priority | Memory priority VERY_LOW and selective trim when idle | 1, 4 |
| "Memory full" with the browser open for days | Private-bytes growth in one or a few processes | Leak detector, then a notification: restart tab/process or cap WS | Leak |
| Commit at 90% and app crashes | A commit leak, or a pagefile that is too small | Name the top committers; suggest a restart or a pagefile setting | Leak |
| RAM vanishes but no process shows it | Driver pool leak, or metafile/cache bloat | Pool-tag report; file-cache cap | Report, 8 |
| System slow after "RAM cleaner" ran | Global WS emptying → fault storm | Don't do it automatically (we don't) | — |

## 12. Version and compatibility matrix

| Feature | 10 1809–21H2 | 10 22H2 | 11 21H2 | 11 22H2+ (22621) | 11 24H2/25H2 |
|---|---|---|---|---|---|
| Memory list commands 2–5 | ✓ | ✓ | ✓ | ✓ | ✓ |
| Page combining (class 130) | ✓ | ✓ | ✓ | ✓ | ✓ |
| Memory compression | ✓ | ✓ | ✓ | ✓ | ✓ |
| EcoQoS (`PROCESS_POWER_THROTTLING_EXECUTION_SPEED`) | LowQoS | LowQoS | ✓ | ✓ | ✓ |
| Rounded corners (`DWMWA_WINDOW_CORNER_PREFERENCE=33`) | — | — | ✓ | ✓ | ✓ |
| System backdrop (`DWMWA_SYSTEMBACKDROP_TYPE=38`) | — | — | — | ✓ [W16] | ✓ |
| ARM64 native | ✓ | ✓ | ✓ | ✓ | ✓ |

Treat everything undocumented as **probe at startup, then cache the result**.

## 13. Security and operational notes

- The process runs elevated, so it must be minimal and memory-safe. That is why it is written in
  Rust. It must never take commands from other processes and must not load plugins.
- Opening other processes uses the narrowest access mask. Protected (PPL) processes cannot be
  opened and are skipped silently.
- `SeDebugPrivilege` is **not** enabled. Without it, other-session and service processes are
  left alone. That is fine, and it is the right privacy and safety default.
- Antivirus heuristics sometimes flag unsigned elevated binaries that call `NtSetSystemInformation`.
  Release builds must be code-signed (Authenticode). Ideally an EV certificate is used, or Azure
  Trusted Signing.
