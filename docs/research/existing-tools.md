# Survey of existing tools, and what we take or reject

> Where a tool is open source, the claims below come from reading its source (tag in brackets).
> For closed-source tools the descriptions are limited to publicly visible behaviour. We avoid
> guessing at internals.

## Windows

### Mem Reduct (henrypp): open source, C, native Win32 [W8]
- **What it does.** Exposes every classic cleaning operation as a checkbox area:
  1. Working sets (`MemoryEmptyWorkingSets`)
  2. System file cache (`SystemFileCacheInformationEx` with −1/−1)
  3. Modified file cache (volume flush)
  4. Modified page list
  5. Standby list
  6. Priority-0 standby list
  7. Registry cache (8.1+)
  8. Combine memory lists (10+)

  It runs them in exactly that order. Auto-clean triggers on a usage-percentage threshold and/or
  a fixed interval.
- **Footprint.** Tiny: native C, GDI, no runtime.
- **Tray icon.** Draws the usage percentage as text into a small GDI bitmap, with a separate
  **monochrome mask bitmap** for transparency, then calls `CreateIconIndirect`. Warning and danger
  colours replace the background or the text colour. The icon is re-created only when the
  percentage changes.
- **Lessons:**
  - ✔ Minimal native implementation; the icon text reads at a glance; user-configurable colours.
  - ✔ **Cooldown after auto-clean.** Issue #191 was a loop: usage stayed above the threshold after
    cleaning, so it cleaned continuously. Every automatic action needs a cooldown and a budget.
  - ✘ **"Usage %" is the wrong trigger.** Usage includes nothing reclaimable, while "available"
    includes standby. A percentage threshold ignores *cache value*. We trigger on
    free+zero, repurpose rate and commit ratio instead.
  - ✘ Default areas include global working-set emptying, the "Hoax" operation [W9].
  - ✘ The GDI mask icon has no anti-aliased alpha. We render premultiplied 32-bit ARGB.

### Intelligent Standby List Cleaner (ISLC, Wagnardsoft) [W15]
- **What it does.** Purges the standby list when *the list exceeds X MB **and** free memory is
  below Y MB*, polling at a configurable interval. It became popular because of game stutter on
  Windows 10 builds where standby hoarding met repurpose latency.
- **Footprint.** A .NET Framework app that relies on Windows performance counters. Users report it
  failing to start when the counters are corrupted (fixed with `lodctr /R`) [W15].
- **Lessons:**
  - ✔ A **two-condition trigger** (large standby *and* low free) is the right shape. ISLC is
    effectively our "game mode".
  - ✘ Polling performance counters is heavy and fragile. We use `SystemMemoryListInformation`
    directly.
  - ✘ It purges *all* standby. We try **priority-0 first**, and only purge everything when the
    repurpose rate shows the cache is already being cannibalised.

### RAMMap (Sysinternals)
- **What it does.** A diagnostic viewer: page counts per list *and* per priority, per-process
  private and standby attribution, and per-file cache residency. Its *Empty* menu offers Working
  Sets, System Working Set, Modified Page List, Standby List and Priority 0 Standby List.
- **Lessons:**
  - ✔ The canonical vocabulary and breakdown. Our popup's memory bar follows its categories
    (Active / Modified / Standby / Free).
  - ✔ The fact that "Priority 0 standby" exists as a separate action is precedent for our tier-2
    purge.
  - ✘ No automation, by design.

### Process Lasso (Bitsum)
- **What it does.** A rules engine for processes: CPU priority/affinity balancing ("ProBalance"),
  **memory priority** rules, and working-set based "SmartTrim" with thresholds and exclusions. A
  service plus a GUI.
- **Lessons:**
  - ✔ **Exclusions and per-app rules are essential.** Never trim the foreground app, audio, games
    or system-critical processes.
  - ✔ Memory priority is a gentle, documented lever [W2]. It is our tier 1.
  - ✘ The service + .NET GUI is heavy for a single-purpose memory tool.

### CleanMem, Wise Memory Optimizer, and similar
- **What they do.** Periodically trim process working sets and/or purge standby, usually from a
  scheduled task. They present the "freed MB" as success.
- **Lessons:**
  - ✘ They measure success by "available" going up, which is exactly the metric the trim
    inflates [W9].
  - ✔ The scheduled-task launch model (elevated at logon, no UAC prompt) is the right
    distribution model for an elevated tray tool. We adopt it.

### Windows built-ins
- **Task Manager "Efficiency mode"** (Windows 11) sets EcoQoS and a lower base priority for a
  process. This is precedent for our self-throttling (EcoQoS on our own process [W2]), and for
  offering "make this background app efficient" as a user action.
- **Resource Monitor** has the same list categories as RAMMap ("Hardware Reserved / In Use /
  Modified / Standby / Free").

## macOS

### Stats (exelban): open source, Swift/AppKit [M10]
- **What it does.** A modular menu bar monitor: CPU, GPU, RAM, disk, network, sensors and battery,
  each as a widget with a popup. It is read-only and does no cleaning.
- **RAM reader:**
  - Reads `host_statistics64(HOST_VM_INFO64)`.
  - Computes `used = active + inactive + speculative + wired + compressed − purgeable − external`,
    `app = used − wired − compressed`, and `cache = purgeable + external`.
  - Reads `vm.swapusage` and `kern.memorystatus_vm_pressure_level`, mapping 2→warning and
    4→critical.
- **Lessons:**
  - ✔ Formula parity with Activity Monitor (we adopt the same formulas).
  - ✔ Rich widget styles in the menu bar: mini chart, bar, pie, text.
  - ✘ It polls on a timer (default about 1 s) and runs many modules at once. Our single-purpose
    app is event-driven.

### iStat Menus (Bjango, commercial)
- **What it does.** A polished menu bar suite with memory pressure, history graphs and top
  processes. Read-only.
- **Lessons.** A high bar for **visual polish**: graphs in the menu bar, dense but legible popups,
  and full theming. This is our design bar.

### CleanMyMac (MacPaw, commercial) and Mac App Store "memory cleaner" apps
- **What they do.** Offer a "Free up RAM" button. The implementation details are not public.
- **Structural constraint.** App Store apps are **sandboxed**. They cannot call `purge`, cannot
  write `kern.memorypressure_manual_trigger` (root only [M3]), and cannot read other users'
  processes. The only lever left to a sandboxed app that wants to "free RAM" is to **allocate and
  touch memory itself**, forcing the kernel to compress and evict *everyone else*. That is the
  `memory_pressure -p` pattern [M8], and we reject it.
- **Lesson.** A trustworthy macOS memory tool has to be distributed outside the App Store (Developer
  ID + notarization) if it wants any real lever. Even then, the honest levers are few: **Nudge,
  quit, relaunch**.

### Apple's own tools
- **Activity Monitor**: the reference for numbers and pressure colours; users already understand it.
- **`memory_pressure`**, **`vm_stat`**, **`footprint`**, **`leaks`**, **`vmmap`**, **`heap`**: the
  CLI tools are the developer's view. `footprint` is the reference for `phys_footprint` semantics.

## Synthesis: our differentiators

| Gap in existing tools | Our answer |
|---|---|
| Success measured as "freed MB" | **Regret-aware** action ledger: page-ins and refaults after every action; automatic back-off |
| Usage-% triggers | Triggers on **free+zero, repurpose rate by priority, commit ratio** (Windows) and **pressure level + swap trend** (macOS) |
| All-or-nothing cleaning | **Tiered**, cheapest first, with per-tier budgets and cooldowns |
| No leak story | **Robust trend detection** (Theil–Sen + Mann–Kendall) with ETA-to-trouble, and pool-tag attribution on Windows |
| Heavy runtimes | Native Rust/Win32 and Swift/AppKit, event-driven, with ≤ 4 MB / ≤ 15 MB budgets |
| One product for both OSes | **Two products, two philosophies**: assertive on Windows, cooperative on macOS |
