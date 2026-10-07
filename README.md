# MemManager

Two native memory managers, one per OS, built on the same measured policy engine:

| | **Windows** | **macOS** |
|---|---|---|
| Philosophy | **Rigorous and assertive.** NT exposes powerful levers and apps rarely cooperate, so MemManager shapes the page lists ahead of demand. | **Complementary.** XNU's VM already cooperates (pressure levels, purgeable memory, compressor), so MemManager adds visibility, leak detection and gentle nudges. |
| Stack | Rust, raw Win32, Direct2D/DirectWrite. A single elevated process started by a logon task. | Swift, AppKit, and SwiftUI for Settings only. An unprivileged app plus an optional root helper. |
| Own footprint | About 2.5 MB private bytes, measured in CI (budget ≤ 4 MB). The exe is about 380 KB. | Measured in CI (budget ≤ 15 MB footprint). |
| Status UI | Dynamic tray icon (ring, bar or number) and a Fluent flyout with optional acrylic | Dynamic menu bar item (ring, bar, percent, graph or dot) and a popover |

**How it differs from "RAM cleaners":**

- It never treats "free RAM" as the goal.
- Every action is tiered, cheapest first, and rate-limited.
- Every action is **measured for regret**: page-ins and refaults afterwards. Actions that cause
  them are backed off automatically.
- Leaks are detected with robust statistics (Theil–Sen + Mann–Kendall), with an estimate of how
  long until memory runs out.

## What it does

### Windows
- **Tier 1:** lowers the memory priority of idle background apps, so their pages are recycled
  first.
- **Tier 2:** clears the *priority-0 standby* cache, which Windows was about to discard anyway.
- **Tier 3:** clears the full standby cache, only when free memory is low **and** Windows is
  already cannibalising its cache.
- **Tier 4:** under physical pressure, trims idle apps that are not in the foreground and not
  playing audio, and writes out a large modified list.
- **Tier 5** (Aggressive profile, when the machine is idle and on AC): combines duplicate pages
  and caps a bloated file cache.
- **Game mode:** keeps standby clear while a full-screen game runs, and never trims apps then.
- **Leaks:** notifications for leaking or runaway apps, a commit-limit warning, and
  kernel-pool-leak attribution by pool tag.
- **Manual "Deep clean":** empties every working set and cache at once. It asks for confirmation
  because it causes a brief slowdown.

### macOS
- **Monitoring:** event-driven pressure monitoring via the kernel's dispatch source. Figures match
  Activity Monitor: App, Wired, Compressed, Cached Files, Swap.
- **Leak detection** per app, summing every helper process (e.g. all of Chrome), with
  Quit / Relaunch / Ignore notifications.
- **Swap-thrash watchdog** and **idle heavy apps** suggestions, with an opt-in auto-quit list.
- **Optional privileged helper:**
  - **Nudge:** briefly signals memory pressure so apps drop their own caches and purgeable memory
    is emptied. The real pressure level is always restored, even if the helper crashes.
  - **Manual disk-cache purge.**

## Install and run

Builds come from CI. Open the latest run on the branch under **Actions**, then download:

- `memmanager-x86_64-pc-windows-msvc` or `memmanager-aarch64-pc-windows-msvc`: `memmanager.exe`
- `MemManager-macos`: `MemManager.app.zip`

### Windows 10/11
1. Put `memmanager.exe` somewhere permanent, e.g. `%LOCALAPPDATA%\Programs\MemManager\`, and run
   it. It starts in **monitor-only** mode with no UAC prompt.
2. Click the tray icon, then **Enable cleaning →**, and accept the single UAC prompt. This
   registers a logon task with the highest privileges, so MemManager starts elevated at every
   login without prompting again. It relaunches immediately.
3. To remove it: **Settings → System → Remove**, then delete the exe.

Settings live in `%APPDATA%\MemManager\config.ini`. Exclusions, the heavy-app list and the
leak-ignore list can be edited there (**Settings → Exclusions & lists → Edit…**).

> Release builds should be Authenticode-signed. Unsigned binaries may trigger SmartScreen, and the
> tray icon uses a numeric ID instead of a GUID.

### macOS 13 or later
1. Unzip and move `MemManager.app` to `/Applications`, then open it. CI builds are ad-hoc
   signed, so right-click → **Open** the first time.
2. Click the menu bar icon for the dashboard. **Settings** (gear) controls the icon style,
   colours, profile, notifications and login item.
3. **Optional:** **Settings → Install helper…** enables Nudge and Purge. macOS asks for approval
   in System Settings → Login Items. The helper can only be registered by a
   Developer-ID-signed build.

## Documentation

### Research
- [Overview: what a memory manager can and can't do](docs/research/00-overview.md)
- [Windows memory internals and every cleaning lever](docs/research/windows-memory-internals.md)
- [macOS memory internals, pressure, and safe actions](docs/research/macos-memory-internals.md)
- [Leak and runaway detection](docs/research/leak-detection.md)
- [Survey of existing tools](docs/research/existing-tools.md)
- [Low-footprint UI: tray, menu bar, icons, theming](docs/research/low-footprint-ui.md)
- [Sources](docs/research/sources.md)

### Architecture
- [Policy engine (shared spec)](docs/architecture/policy-engine.md)
- [Windows architecture](docs/architecture/windows.md)
- [macOS architecture](docs/architecture/macos.md)
- [UI design](docs/architecture/ui-design.md)
- [Resource budget](docs/architecture/resource-budget.md)

## Repository layout

```
docs/                 research + architecture
spec/reference/       executable reference model of the policy engine (Python, stdlib only)
spec/test-vectors/    JSON vectors that both native implementations must reproduce
windows/core/         Rust policy engine (platform-neutral, tested on any OS)
windows/app/          Windows tray app (Win32, NT native API, Direct2D)
macos/Sources/        MemCore (Swift policy engine), MemSys (C shim), MemShared (XPC protocol),
                      MemManager (menu bar app), MemManagerHelper (privileged daemon)
.github/workflows/    CI: spec vectors, unit tests, live self-tests on Windows and macOS, artifacts
```

## Building

```sh
# Spec vectors (any OS)
python3 spec/reference/policy_ref.py --check spec/test-vectors

# Rust core (any OS)
cargo test --manifest-path windows/Cargo.toml -p memmanager-core

# Windows app (on Windows with the MSVC toolchain)
cargo build --manifest-path windows/Cargo.toml --release
windows\target\release\memmanager.exe --selftest=20      # headless live-system check, JSON report

# macOS app (macOS 13+, Xcode 15+)
cd macos
swift build -c release && swift test
.build/release/MemManager --selftest=10                   # headless live-system check
./scripts/bundle.sh release                               # → build/MemManager.app (ad-hoc signed)
```
