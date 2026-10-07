# macOS architecture: complementary and cooperative

> Citation tags ([W…], [M…]) refer to [`../research/sources.md`](../research/sources.md).
> Implements [`policy-engine.md`](policy-engine.md) with the facts from
> [`../research/macos-memory-internals.md`](../research/macos-memory-internals.md).
> Language: **Swift**. AppKit is used for everything that is always on. SwiftUI is used **only**
> in the on-demand Settings window. Minimum macOS 13, which is required for `SMAppService`.
> Apple Silicon and Intel are built as a universal binary.

## 1. Process model

```
 MemManager.app  (LSUIElement agent, unprivileged, user session)
   ├─ main thread: AppKit — NSStatusItem, NSPopover, notifications, Settings window
   └─ "sampler" serial DispatchQueue (QoS .utility):
        • DispatchSource memory-pressure source  [.normal, .warning, .critical]  ← zero-cost wake source
        • DispatchSourceTimer (adaptive cadence, 10% leeway)
        • NSWorkspace activation observer (last-active timestamps)

 MemManagerHelper  (optional; root LaunchDaemon registered via SMAppService.daemon; off by default)
   └─ NSXPCListener(machServiceName: "dev.memmanager.helper")
        • nudge(level:durationMs:)      → kern.memorypressure_manual_trigger (latch-safe, §4)
        • purgeDiskCache()              → /usr/sbin/purge  (manual only)
        • version()                     → handshake
```

**Launch at login.** `SMAppService.mainApp.register()` is called when the user toggles "Open at
login".

## 2. Package layout

SwiftPM is used for builds and CI. A script assembles the signed `.app`.

```
macos/
  Package.swift
  Sources/
    MemCore/            platform-neutral policy engine + leak detector (mirrors windows/core; tested on vectors)
    MemManager/         the app: main.swift, StatusItemController, PopoverController, Sampler,
                        ProcessMonitor, Actions, HelperClient, SettingsWindow (SwiftUI), Notifications
    MemManagerHelper/   the daemon: main.swift (listener), Nudge.swift, Purge.swift, ClientValidation.swift
  Resources/
    Info.plist                          LSUIElement=YES, LSMinimumSystemVersion=13.0
    dev.memmanager.helper.plist         LaunchDaemon plist (BundleProgram, MachServices, AssociatedBundleIdentifiers)
  scripts/bundle.sh                     builds universal binaries, assembles MemManager.app:
                                        Contents/MacOS/{MemManager,MemManagerHelper}
                                        Contents/Library/LaunchDaemons/dev.memmanager.helper.plist
                                        then codesign (hardened runtime) + notarize (release only)
  Tests/MemCoreTests/                   loads ../../spec/test-vectors/*.json
```

## 3. Sampling (no privileges needed)

| Data | Source | Cadence |
|---|---|---|
| VM stats | `host_statistics64(HOST_VM_INFO64)` | Per system tick (see policy-engine §8) |
| Pressure level | Dispatch source events; `sysctl kern.memorystatus_vm_pressure_level` decoded as **1/2/4** | Event, plus each tick |
| Free % | `sysctl kern.memorystatus_level` | Each tick |
| Swap | `sysctl vm.swapusage` | Each tick |
| Process footprints | `proc_listallpids` → per PID `proc_pid_rusage(RUSAGE_INFO_V4)` | Process tick |

**Cheap process enumeration:**

- Keep a PID table recording `(pid, start time from ri_proc_start_abstime) → uid-ok?`.
- The first time we see a PID, `proc_pid_rusage` either succeeds or fails with `EPERM` (another
  user). Failures are remembered, so foreign PIDs cost one failed syscall in their lifetime, not one
  per tick.
- **Grouping by app.** Map each PID to an owning app bundle by its `proc_pidpath` prefix up to the
  first `.app/`. This groups `Google Chrome Helper (Renderer)` under Chrome and Electron helpers
  under their app. `NSRunningApplication` provides the display name and icon for GUI apps.

## 4. Actions

| Action | Min state | Trigger | Privilege | Behaviour |
|---|---|---|---|---|
| **Leak / runaway alert** | Any | Leak detector (policy-engine §7) on `phys_footprint` | None | `UNUserNotificationCenter` notification with category actions: **Quit**, **Relaunch**, **Ignore app**. Quit uses `NSRunningApplication.terminate()`, which is graceful and respects unsaved-document prompts. |
| **Idle heavy apps** | Elevated, sustained ≥ 2 min | Apps not activated for ≥ 2 h with footprint ≥ 500 MiB (Balanced) | None | Listed in the popover with one-click Quit. **Auto-quit only for apps on the user's allowlist**, and only with `terminate()`. |
| **Swap-thrash watchdog** | Warning, sustained ≥ 5 min | `swap_used` growing > 64 MiB/min and the `swapouts` rate is high | None | One notification naming the top 3 footprints, with a 1-hour cooldown |
| **Nudge** | Manual. Automatic only in the Aggressive profile at High (≤ 1 per 30 min). | User click / policy | Helper (root) | See §5 |
| **Purge disk cache** | Manual only | User click, with an explanation sheet | Helper (root) | Runs `/usr/sbin/purge`, which is `vfs_purge()`. Always labelled as useful for benchmarking, not for speed. |

**Never implemented:** periodic purge; allocating memory to force pressure (`memory_pressure -p`
style); killing daemons; `forceTerminate()` without explicit user action.

## 5. The privileged helper and latch safety

`kern.memorypressure_manual_trigger` **latches** the system pressure level while
`memorystatus_manual_testing_on` is set. It is only cleared by writing a NORMAL request [M3]. If
the helper crashes between "set WARN" and "reset NORMAL", **every app on the system would keep
believing memory is under pressure**. The helper therefore guarantees the reset on every path:

1. **Bounded duration.** `durationMs` is clamped to [500, 5000]. The reset is scheduled on a
   dedicated serial queue *before* the WARN write is issued.
2. **Marker file.**
   - `/var/db/dev.memmanager.helper/nudge-active` (root-owned, 0600) is written and `fsync`ed
     before the WARN write, and deleted after the NORMAL write.
   - On **every helper start**, if the marker exists, write NORMAL and delete it first.
3. **launchd keep-alive.** The daemon plist sets `KeepAlive = { Crashed = true }`, so a crash
   relaunches the helper, and step 2 resets the level.
4. **Signals.** `SIGTERM`/`SIGINT` handlers (dispatch signal sources) perform the reset before
   exit.
5. **Single flight.** Only one nudge at a time. A rate limit of at most 1 per 5 min is enforced *in
   the helper*, independent of the app.
6. **Exact values.**
   - Set: `(6 << 16) | 0x2` (TEST_LOW_MEMORY_PURGEABLE_TRIGGER_ALL, NOTE_MEMORYSTATUS_PRESSURE_WARN).
   - Reset: `(6 << 16) | 0x1` (NORMAL). These match `memory_pressure -S` [M8].
   - **Never CRITICAL** from automatic policy. The manual "Strong nudge" uses `0x4` and requires an
     extra confirmation.

**XPC security:**

- `NSXPCListener.delegate` accepts a connection only after
  `connection.setCodeSigningRequirement(...)` [M14] with
  `anchor apple generic and identifier "dev.memmanager.MemManager" and certificate leaf[subject.OU] = "<TEAMID>"`.
- The exported interface takes only integers, with no paths and no strings to execute. The purge
  path is a constant.
- The helper has no network access and no file reads beyond its marker file. It exits after 60 s
  idle; launchd relaunches it on demand through `MachServices`.

**Registration UX.**

- `SMAppService.daemon(plistName: "dev.memmanager.helper.plist").register()`.
- If `status == .requiresApproval`, the UI shows a one-line explanation and a button that calls
  `SMAppService.openSystemSettingsLoginItems()`.
- Unregister removes the helper completely.

**Measuring the effect.** Each Nudge creates a ledger entry (policy-engine §5.1). "Freed" is the
change in fast-available memory; "regret" is the page-in rate afterwards. If apps ignore pressure
notifications, the ledger shows it, and the Aggressive auto-nudge backs off.

## 6. UI

The visual spec is in [`ui-design.md`](ui-design.md).

- **Status item:**
  - `NSStatusItem(variableLength)` with an image from `NSImage(size:flipped:drawingHandler:)`.
  - Styles: pressure ring, bar, %, sparkline, or dot + number.
  - Template image in monochrome mode; colour otherwise.
  - Redrawn only when the displayed value or state changes.
  - Left click toggles the popover. Right click / ⌥-click opens a compact `NSMenu`.
- **Popover** (`NSPopover.behavior = .transient`, AppKit content, about 340 × 440 pt):
  - Header: pressure ring, state chip, used / total.
  - Breakdown bar: App / Wired / Compressed / Cached Files, matching Activity Monitor.
  - Swap line.
  - 10-minute pressure sparkline.
  - Top apps (grouped) with growth arrows and leak badges.
  - "Idle heavy apps" section, shown only when relevant.
  - Ledger tail.
  - Footer: Nudge (enabled only if the helper is installed), Settings, Pause.
- **Liquid Glass.** On macOS 26 the popover adopts the system glass material automatically. For
  the custom panel variant, look up `NSGlassEffectView` at runtime and fall back to
  `NSVisualEffectView`.
- **Settings window.** SwiftUI `Form` in an `NSHostingController`. It is created on open and its
  reference is dropped in `windowWillClose`. Tabs: General, Icon & Colors, Behaviour (profile,
  allowlists, ignore list), Helper, About.
- **Notifications.** `UNUserNotificationCenter` categories `LEAK` (Quit / Relaunch / Ignore) and
  `SWAP`. Permission is requested on first need, not at launch.

## 7. Self-footprint tactics

- Fixed-capacity ring buffers (same sizes as Windows; see windows.md §6). Process history is held
  only for the top 24 candidates.
- On our own `.warning`/`.critical` pressure event: drop history beyond 10 minutes and clear
  rendered caches. On `.critical`, also close Settings if it is not frontmost.
- No Combine publishers on the hot path. Plain closures on one serial queue.
- `ProcessInfo.processInfo.automaticTerminationSupportEnabled = false`; App Nap is allowed. The
  timer `leeway` is 10%.
- Budget: ≤ 15 MB footprint idle, ≤ 30 MB with the popover open (see
  [`resource-budget.md`](resource-budget.md)).

## 8. Distribution

- Developer ID Application signing with hardened runtime; notarized; stapled DMG.
- The **Mac App Store is not possible**: a sandboxed app cannot register a root daemon or read
  other processes' footprints.
- Sparkle 2 for updates (optional; adds about 1 MB on disk and is not resident while idle).
