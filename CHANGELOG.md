# Changelog

## v0.1.0 — first pre-release

### Windows (tray app, Rust + Win32 + Direct2D)
- **Tiered, measured optimization**, cheapest step first:
  1. Lowers the memory priority of idle background apps.
  2. Clears the priority-0 standby cache.
  3. Clears the full standby cache, only when Windows is already repurposing it.
  4. Selectively trims idle, silent, background apps, and flushes the modified list.
  5. In the Aggressive profile: combines pages and caps the file cache.
- **Regret tracking:** every action records the disk page-ins it causes, and actions that hurt
  are backed off automatically.
- **Game mode:** keeps standby clear while a full-screen app runs and never trims during games.
- **Leak and runaway alerts:** robust trend statistics (Theil–Sen + Mann–Kendall) with a
  time-until-trouble estimate, a commit-limit warning, and kernel pool-tag attribution for
  driver leaks.
- **Tray icon:** ring, bar or number styles; system, traffic-light, mono, colour-safe or custom
  colours.
- **Fluent flyout:** dashboard and settings, rounded corners, optional acrylic background.
- **Installer:** one setup exe for x64 and ARM64. It optionally registers a logon task so
  MemManager starts with cleaning privileges without a UAC prompt each time, and uninstalls
  cleanly.
- **Footprint:** about 2.6–2.9 MB private bytes when idle; about 650 KB executable.

### macOS (menu bar app, Swift + AppKit)
- **Event-driven monitoring** of kernel memory pressure, with Activity-Monitor-consistent
  figures: App, Wired, Compressed, Cached, Swap.
- **Per-app leak alerts**, summing every helper process, with Quit / Relaunch / Ignore actions.
  Also a swap-thrash warning and idle-heavy-app suggestions with an opt-in auto-quit list.
- **Optional privileged helper:**
  - **Nudge** asks apps to release caches and empties purgeable memory. It is latch-safe: the
    real pressure level is always restored.
  - Manual disk-cache purge.
- **Menu bar icon:** ring, bar, percent, graph or dot, with colour presets.
- **Install:** drag-to-Applications DMG.
- **Footprint:** about 6 MB.

### Known limitations
- **Unsigned builds:**
  - Windows SmartScreen may warn.
  - On macOS, right-click → Open the first time.
  - The macOS helper (Nudge/Purge) can only be installed from a Developer-ID-signed build.
- **Not yet hand-tested:** the UIs are built and CI-verified headless only.
- **Accessibility:** Windows screen-reader (UI Automation) support for the flyout is pending.
