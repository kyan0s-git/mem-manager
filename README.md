<img src="assets/icon-256.png" width="96" alt="MemManager icon" align="right">

# MemManager

Two native memory managers, one per OS, built on the same measured policy engine:

| | **Windows** | **macOS** |
|---|---|---|
| Philosophy | **Rigorous and assertive.** NT exposes powerful levers and apps rarely cooperate, so MemManager shapes the page lists ahead of demand. | **Complementary.** XNU's VM already cooperates (pressure levels, purgeable memory, compressor), so MemManager adds visibility, leak detection and gentle nudges. |
| Stack | Rust, raw Win32, Direct2D/DirectWrite. A single elevated process started by a logon task. | Swift, AppKit, and SwiftUI for Settings only. An unprivileged app plus an optional root helper. |
| Own footprint | About 2.6–2.9 MB private bytes, measured in CI (budget ≤ 4 MB). The exe is about 650 KB, with the benchmark built in. | About 6 MB phys_footprint in the headless engine, measured in CI (budget ≤ 15 MB). |
| Status UI | Dynamic tray icon (ring, bar or number) and a Fluent flyout with optional acrylic | Dynamic menu bar item (ring, bar, percent, graph or dot) and a popover |

**How it differs from "RAM cleaners":**

- It never treats "free RAM" as the goal.
- Every action is tiered, cheapest first, and rate-limited.
- Every action is **measured for regret**: page-ins and refaults afterwards. Actions that cause
  them are backed off automatically.
- Leaks are detected with robust statistics (Theil–Sen + Mann–Kendall), with an estimate of how
  long until memory runs out.

## What it does

### On both
- **Friendly values (default):** the dashboard leads with memory that's **ready for apps**. The
  cache is shown as a speed-up that's handed to apps instantly, not as memory in use. Calm state
  names: Comfortable, Busy, Tight, Critical. Every number is a real OS value, and **Settings →
  Values** switches back to Task Manager or Activity Monitor terms
  ([why](docs/understanding-memory.md)).
- **See what it does:**
  - **Activity** lists every action and warning in plain words: what was done, why (the reading
    that triggered it), which apps were affected, and the measured result.
  - Actions also appear as dots on the dashboard graph, so you can watch their effect on the
    curve.
  - A "Now:" line says what MemManager is doing at this moment.
- **Honest cleaning:** while memory is comfortable, a manual clean first says what it would cost
  and that it won't make anything faster. Idle apps holding real memory are pointed out instead.
- **In-app updates:**
  - Checks GitHub releases daily (or on demand).
  - Verifies the download's SHA-256.
  - Installs in place and restarts with your settings; no re-downloading.

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

Download from the **[Releases](https://github.com/kyan0s-git/mem-manager/releases)** page. Each
asset has a `.sha256` file next to it.

| OS | Recommended | Alternative |
|---|---|---|
| Windows 10/11 (x64 and ARM64) | `MemManager-<ver>-windows-setup.exe`: an installer for both architectures | `MemManager-<ver>-windows-<arch>-portable.zip` |
| macOS 13 or later (universal) | `MemManager-<ver>-macos-universal.dmg` | `MemManager-<ver>-macos-universal.zip` |

Measured results are in [docs/benchmarks.md](docs/benchmarks.md).

### Windows
1. Run `MemManager-<ver>-windows-setup.exe` and accept the UAC prompt. It installs to
   `C:\Program Files\MemManager`, adds a Start-menu entry and an **Apps & features** uninstall
   entry.
2. Leave **"Start at login with memory-cleaning privileges"** checked (the default). Setup
   registers a logon task with the highest privileges, so MemManager starts elevated at every
   login with no further prompts. It starts as soon as setup finishes.
   - If you uncheck it, MemManager runs in **monitor-only** mode. You can enable cleaning later
     from the tray popup (**Enable cleaning →**, one UAC prompt).
3. **Upgrade:** run a newer setup; it closes the running copy and replaces it in place.
4. **Uninstall:** **Settings → Apps → MemManager → Uninstall**. This removes the logon task, the
   files, and, if you agree, your settings in `%APPDATA%\MemManager`.

*Portable zip:* put `memmanager.exe` somewhere permanent and run it. It starts in monitor-only
mode. **Enable cleaning →** registers the logon task. To remove it: **Settings → System →
Remove**, then delete the exe.

Settings live in `%APPDATA%\MemManager\config.ini`. Exclusions, the heavy-app list and the
leak-ignore list can be edited there (**Settings → Exclusions & lists → Edit…**).

> Builds are not Authenticode-signed yet, so SmartScreen may warn you (**More info → Run
> anyway**). Unsigned builds use a numeric tray-icon ID instead of a GUID.

### macOS
1. Open the DMG and drag **MemManager** onto **Applications**.
2. Open it from Applications. The builds are ad-hoc signed, so macOS blocks the first launch:
   right-click → **Open**, or allow it in **System Settings → Privacy & Security**.
3. Click the menu bar icon for the dashboard. **Settings** (gear) controls the icon style,
   colours, profile, notifications and login item.
4. **Optional:** **Settings → Install helper…** enables Nudge and Purge. macOS asks for approval
   in System Settings → Login Items. The helper can only be registered by a
   Developer-ID-signed build.
5. **Uninstall:** Quit from the menu, remove the helper in Settings if you installed it, then
   drag MemManager.app to the Trash.

### Development builds
Every CI run on the branch uploads the same packages (version `0.0.0`) under **Actions**.

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
- [Why "ready for apps" counts the cache](docs/understanding-memory.md)

## Repository layout

```
docs/                 research + architecture
spec/reference/       executable reference model of the policy engine (Python, stdlib only)
spec/test-vectors/    JSON vectors that both native implementations must reproduce
windows/core/         Rust policy engine (platform-neutral, tested on any OS)
windows/app/          Windows tray app (Win32, NT native API, Direct2D)
macos/Sources/        MemCore (Swift policy engine), MemSys (C shim), MemShared (XPC protocol),
                      MemManager (menu bar app), MemManagerHelper (privileged daemon)
.github/workflows/    CI (spec vectors, unit tests, live self-tests, benchmarks, packages) and release
windows/installer/    Inno Setup script for the Windows installer
scripts/              benchmark report generator
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
./scripts/make_dmg.sh build/MemManager.dmg                 # drag-to-Applications DMG
sudo .build/release/MemManager --bench --out=bench.json   # measured-reduction benchmark
```

```bat
:: Windows benchmark and installer (elevated prompt; Inno Setup 6)
windows\target\release\memmanager.exe --bench --out=bench.json
iscc /DAppVersion=0.1.0 /DX64Exe=..\target\release\memmanager.exe windows\installer\MemManager.iss
```
