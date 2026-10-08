# WinMemoryCleaner: how it works, and what MemManager takes from it

This is an analysis of [IgorMundstein/WinMemoryCleaner](https://github.com/IgorMundstein/WinMemoryCleaner)
at commit `a19f257` (December 2025). The project is GPL-3.0. This document describes its
methodology only; no code was copied. File references are to its `src/` tree.

## 1. What it measures

`Service/ComputerService.cs` and `Model/Memory/*.cs` use **one** API: `GlobalMemoryStatusEx`.

| Shown as | Source | What it really is |
|---|---|---|
| Physical: used / free % | `TotalPhys`, `AvailPhys`, `MemoryLoad` | **Available** memory, which already includes the standby cache |
| Virtual: used / free | `TotalPageFile`, `AvailPageFile` | **Commit limit** and commit headroom (RAM + page file), not the page file alone |

There is no breakdown into working sets, modified, standby or free lists, and no
measurement of how hard Windows is working (faults, repurposing).

A consequence worth knowing: `AvailPhys` counts standby pages as available, so **purging
the standby list barely changes the "free %" it displays**. Standby pages simply move from
the standby list to the free list, and both are "available". The visible jump after a
clean comes almost entirely from emptying working sets, which pushes apps' pages to the
standby and modified lists.

## 2. When it cleans

All triggers are in `ViewModel/MainViewModel.cs` and `WindowsService/WinService.cs`.

| Trigger | Rule | Default |
|---|---|---|
| Manual | The Optimize button or a hotkey | Available |
| Schedule | Every *N* hours | Off (`AutoOptimizationInterval = 0`) |
| Low memory | Physical free % below *X*, checked every second in the app (every 60 s as a service), at most once per 5 minutes | Off (`AutoOptimizationMemoryUsage = 0`) |

The rules have no hysteresis and no notion of cause. A run is logged with its duration, but
**its effect is not measured**: nothing records how much was read back from disk afterwards,
or whether the next run should wait longer.

## 3. What a clean does

All of the steps below are in `ComputerService.Optimize`. They run in this order, each
area only if selected. **By default every area is selected except low-priority standby.**

| # | Area | Call | Privilege |
|---|---|---|---|
| 1 | Working sets | `NtSetSystemInformation(SystemMemoryListInformation=80, MemoryEmptyWorkingSets=2)`; with an exclusion list, `EmptyWorkingSet` per process instead | SeProfileSingleProcess, or SeDebug per process |
| 2 | System file cache | `NtSetSystemInformation(SystemFileCacheInformation=21, {-1, -1})`, then `SetSystemFileCacheSize(-1, -1, 0)` (flush) | SeIncreaseQuota |
| 3 | Modified page list | class 80, `MemoryFlushModifiedList=3` | SeProfileSingleProcess |
| 4 | Standby list | class 80, `MemoryPurgeStandbyList=4`, or `…LowPriority…=5` when selected | SeProfileSingleProcess |
| 5 | Combined page list | `SystemCombinePhysicalMemoryInformation=130` | SeProfileSingleProcess |
| 6 | Registry cache | `SystemRegistryReconciliationInformation=155`, which writes dirty registry-hive data to disk | — |
| 7 | Modified file cache | For each fixed volume: `FSCTL_RESET_WRITE_ORDER`, `FSCTL_DISCARD_VOLUME_CACHE`, `FlushFileBuffers` | Admin (volume handle) |
| 8 | Its own .NET heap | Garbage collection | — |

**Why that order costs the most.** Step 1 evicts every app's pages: clean ones go to standby,
dirty ones to modified. Step 3 writes the modified pages out to the page file. Step 4 then
discards the standby list, which now holds everything apps were using a moment ago. Every
app has to read its data back from disk or the page file.

We measured exactly this sequence as MemManager's manual Deep clean (see
[benchmarks](../benchmarks.md)). It produced about 5 GB "freed" on a 16 GB machine, then
**560,000–980,000 hard faults in the next 10 seconds** against a baseline of about 300–12,000, and
10–40 seconds for apps to page back in.

## 4. Side by side

| | WinMemoryCleaner | MemManager (Windows) |
|---|---|---|
| Measurement | `GlobalMemoryStatusEx` only | Memory lists (standby by priority, modified, free/zero), commit and page file, hard faults, cache repurposing, per-process private bytes |
| Trigger | Free % below a threshold, or a timer | A pressure score (fast-available RAM, commit, cache churn while RAM is short), smoothed, with hysteresis |
| What runs | Everything selected, every time | The cheapest step that helps, by tier: deprioritize → low-priority cache → standby (only when Windows is already recycling cache) → trim idle apps → flush modified |
| Feedback | None | Every action's refaults are measured; actions that hurt are backed off |
| Virtual memory | Shown (commit used/limit) | Shown (commit used/limit **and** page-file use), plus page-file growth per action |
| Leaks | — | Per-app trend detection and kernel pool-tag attribution |
| Footprint | WPF/.NET app | ~3 MB private, Win32/Direct2D |

## 5. What MemManager adopts, and what it doesn't

**Adopted**

- **Virtual memory on the dashboard.** WinMemoryCleaner is right to show commit next to RAM:
  apps fail to allocate when commit reaches its limit, however much RAM is free. MemManager now
  shows "Virtual memory X of Y · page file A of B".
- **The cause of the state.** When commit, not RAM, is the problem, MemManager now says so
  ("Virtual memory is nearly full · RAM cleaning won't help"), and RAM-only actions don't run
  for it. Trimming and purging never lower commit.
- **Page-file accounting for every clean.** Each action in Activity shows how much of the
  "freed" RAM was written to the page file. Of all the numbers here, this is the one RAM
  cleaners leave out.

**Not adopted, and why**

- **Free-% thresholds and timers.** Free % includes the cache, so it says little about whether
  memory is actually short. A timer cleans when nothing is wrong. MemManager acts on pressure
  and measures the cost.
- **Registry reconciliation** (step 6). It writes registry changes to disk. It doesn't
  meaningfully free RAM, and Windows does it on its own schedule.
- **Discarding volume caches** (step 7) as a routine. Like a standby purge, it trades speed for
  a bigger "free" number. MemManager's manual Deep clean already flushes volumes for anyone
  who explicitly wants everything emptied.
- **Running every area at once.** This is the costliest possible sequence (§3). MemManager
  keeps it as an explicit, confirmed Deep clean, with the cost stated up front.

## 6. Takeaway

WinMemoryCleaner is a clean, well-scoped *cleaner*. It uses the same documented and
undocumented levers MemManager uses, and its working-set and standby calls match ours.
The difference is methodology. It decides from one coarse number and doesn't look at the
consequences. MemManager decides from the memory lists, commit, faults and cache churn, uses
the smallest step that helps, and measures what each step cost.
