# Research overview: what a memory manager can do on Windows and macOS

> Read this first. It frames everything else in `docs/research/`: what "cleaning memory"
> actually means, why the two OSes need different products, and how we decide whether the
> product helps.
> Citation tags like [W1] and [M2] refer to [`sources.md`](sources.md).

## 1. The core tension: free RAM vs. cache

Both kernels treat RAM that holds no data as wasted. They fill it with **cache**: file pages that
were read once, pages trimmed from processes that may need them again, and prefetched pages. Cache
can be reclaimed almost for free when someone needs the memory. So the "memory used" number in a
task manager is a poor health metric:

- **High usage with low pressure is healthy.** The cache is doing its job.
- **Low usage that was produced by flushing cache is unhealthy.** The next access to those pages
  turns into disk I/O (a hard fault), or into decompression on macOS.

Mark Russinovich made this argument in 2003 in "The Memory-Optimization Hoax" [W9]. Tools that
"free RAM" by trimming every working set mostly move pages from *in use* to *standby*. Task Manager
counts standby as "available", so the number improves while nothing real changes, and the
processes then soft-fault (or, worse, hard-fault) their pages back in.

**So the real goals of this project are:**

1. **Reduce memory pressure**, meaning the moments when the kernel must evict *useful* data or page
   out *anonymous* memory. Raising the "free" number is not the goal.
2. **Find and contain leaks and runaways.** These are processes whose committed or anonymous memory
   grows without bound. This is the one area where the kernel structurally cannot help, because it
   cannot tell a leak from legitimate use.
3. **Shape the cache, don't destroy it.** Drop *low-value* cache (speculative and prefetched pages,
   pages that are being repurposed anyway) before it competes with *high-value* data. Leave the rest
   alone.
4. **Measure every action.** If an action causes more page-ins or refaults than it saves, back off.
   This feedback loop is what separates this project from the classic "RAM cleaner".

## 2. Why Windows and macOS need different products

| Dimension | Windows (NT memory manager) | macOS (XNU / Mach VM) |
|---|---|---|
| Main reclaim mechanism | Working-set trimming into **standby/modified page lists** [W10] | Page-queue aging (active → inactive → free), **compressor**, then swap [M2] |
| Compression | Since Windows 10, the "store manager" compresses into the *Memory Compression* process [W11] | A core part of the design since 10.9. The compressor holds anonymous pages, and swap stores compressor segments [M2][M5] |
| Pressure signal | `CreateMemoryResourceNotification` (one coarse low/high event pair) [W5] | Graded pressure levels (normal/warn/critical) with documented thresholds and hysteresis [M2][M3] |
| Cooperative cache release | Mostly absent. Apps rarely react to low-memory events. | Pervasive. Apps receive `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`, and **purgeable memory** lets the kernel drop app caches directly [M1][M16] |
| Killing under pressure | None for desktop processes. Commit exhaustion produces failed allocations instead. | Jetsam/memorystatus: idle-exit and limit kills; on macOS mostly daemons [M1] |
| User-space control surface | Large, mostly undocumented: purge standby (all, or low-priority only), flush modified, empty all working sets, combine pages, file-cache limits, per-process memory priority, hard working-set caps [W1][W2][W3][W6] | Small: read statistics, receive pressure events, terminate apps. With root, `vfs_purge` and a pressure-simulation sysctl [M3][M8][M9] |
| Known pathologies | Standby-list hoarding causing stutter in some games (the ISLC use case [W15]), file-cache/metafile bloat, driver pool leaks [W14], commit exhaustion | Runaway app or WebContent footprints, swap thrash on low-RAM machines, WindowServer and other daemon leaks |

**What follows for the product:**

- **Windows: rigorous and assertive.** The kernel exposes powerful levers, its defaults are tuned
  for throughput rather than interactivity, and apps don't cooperate. A well-built manager can
  *add* value by shaping lists ahead of demand, deprioritising background pages and capping
  runaway processes, *provided* every action is measured.
- **macOS: complementary.** The VM already does what Windows cleaners try to do, and does it
  cooperatively. Forcing it (`purge`, allocate-to-pressure tricks) makes things worse. Our value
  is **visibility** (pressure, compressor, swap, per-app footprint growth), **leak and runaway
  detection**, and **gentle, cooperative nudges**: ask apps to drop caches through the kernel's own
  notification path, and quit idle memory hogs gracefully.

## 3. What each product does and doesn't do

### Windows (summary; full design in `docs/architecture/windows.md`)

**Automatic, cheapest action first:**

1. Lower the memory priority of idle background processes, so their pages are repurposed first.
2. Purge the low-priority standby list when free memory is low.
3. Purge the full standby list only when the cache is *already being cannibalised*, as shown by
   `RepurposedPagesByPriority` [W1].
4. Under commit pressure, selectively trim idle, non-foreground processes and flush the modified
   list.
5. When the system is idle, combine identical pages and cap a bloated file cache.

**Not automatic:**

- A global "empty all working sets". It is a manual "Deep clean" only, with a warning.
- Restarting or killing leaky processes. The tool notifies; the user decides.

### macOS (summary; full design in `docs/architecture/macos.md`)

**Automatic:**

- Monitoring, event-driven through the pressure dispatch source.
- Leak and runaway detection on the user's own processes.
- Idle-app suggestions under sustained pressure.
- A swap-thrash watchdog.

**Optional (privileged helper, off by default):**

- **Nudge**: briefly raise the kernel pressure level to *warn* through
  `kern.memorypressure_manual_trigger`. Apps release caches and purgeable memory is emptied, then
  the level is reset to normal [M3][M8]. The trigger **latches**, so the reset must be guaranteed;
  see the macOS architecture doc.
- **Purge disk cache**: manual only, with an explanation of why it is usually pointless [M9].

**Never:** periodic purge, or allocating memory to force pressure.

## 4. How we decide whether the product helps

A memory manager that reports "freed 1.2 GB!" proves nothing. Both apps record an **action
ledger** and compute **regret**:

| Metric | Windows source | macOS source |
|---|---|---|
| Hard faults (disk reads to resolve faults) | `SYSTEM_PERFORMANCE_INFORMATION.PageReadCount` delta [W1] | `vm_statistics64.pageins` delta [M5] |
| Soft re-faults | `TransitionCount` / `CacheTransitionCount` delta [W1] | `decompressions` and `reactivations` delta [M5] |
| Pressure | free+zero pages, commit ratio, repurpose rate | pressure level, `kern.memorystatus_level`, swap growth |
| Effect size | available memory before/after, and 60 s later | same |

An action that raises the hard- or soft-fault rate in the following 60 s above baseline by more
than its budget gets **backed off**: longer cooldown and stricter threshold. This turns the
"aggressive vs. cache-friendly" slider into a closed loop instead of a guess. The details are in
`docs/architecture/policy-engine.md`.

## 5. The tool's own footprint

Classic cleaners are ironic: some ship .NET or Electron UIs that use 50–200 MB to "save" memory.
Our budgets (`docs/architecture/resource-budget.md`) are:

- **Windows:** ≤ 4 MB private bytes when idle.
- **macOS:** ≤ 15 MB footprint when idle.
- **Both:** about one wakeup every 10 s when idle, and none while the display is off.

How we get there:

- **Event-driven sampling**: kernel notification objects and dispatch sources instead of polling.
- **One syscall per sample** for all processes on Windows (`SystemProcessInformation`) instead of
  `OpenProcess` per PID.
- **Fixed-size ring buffers.** No growth and no allocator churn on the hot path.
- **Native rendering with no runtime**: Win32 + Direct2D on Windows, AppKit on macOS. Heavy UI
  (Direct2D device resources, the SwiftUI Settings window) is created on demand and released
  afterwards.
- **Good-citizen self-settings**: EcoQoS and low memory priority on Windows [W2]; timer leeway
  and App Nap on macOS.

## 6. Document map

| Document | Contents |
|---|---|
| [`windows-memory-internals.md`](windows-memory-internals.md) | NT memory manager internals and **every** cleaning operation: mechanics, cost, privilege, version, when it helps |
| [`macos-memory-internals.md`](macos-memory-internals.md) | XNU VM, compressor, memorystatus, metrics mapping, permissions, what `purge` and `memory_pressure` really do |
| [`leak-detection.md`](leak-detection.md) | Leak taxonomy (user-mode, kernel pool), statistics, false positives |
| [`existing-tools.md`](existing-tools.md) | Survey of current cleaners and monitors, and what we take from each or avoid |
| [`low-footprint-ui.md`](low-footprint-ui.md) | Tray and menu-bar technology and its RAM costs, dynamic icon rendering, theming, DPI |
| [`sources.md`](sources.md) | Bibliography |
