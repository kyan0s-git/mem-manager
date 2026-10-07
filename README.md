# MemManager

Two native memory managers, one per OS, built on the same measured policy engine:

| | **Windows** | **macOS** |
|---|---|---|
| Philosophy | **Rigorous and assertive.** NT exposes powerful levers, and apps rarely cooperate, so we shape the page lists ahead of demand. | **Complementary.** XNU's VM is already cooperative (pressure levels, purgeable memory, compressor), so we add visibility, leak detection and gentle nudges. |
| Stack | Rust, raw Win32, Direct2D/DirectWrite. Single elevated process (logon task). | Swift and AppKit, with SwiftUI only for Settings. Unprivileged app plus an optional root helper. |
| Idle footprint budget | ≤ 4 MB private bytes | ≤ 15 MB footprint |
| Status UI | Dynamic tray icon (ring, bar or number), Fluent acrylic flyout | Dynamic menu bar item (ring, bar, %, sparkline), popover / Liquid Glass |

**What makes it different from "RAM cleaners":**

- It never treats "free RAM" as the goal.
- Every action is tiered (cheapest first), rate-limited, and **measured for regret** (page-ins and
  refaults afterwards).
- Actions that hurt are backed off automatically.
- Leaks are detected with robust statistics (Theil–Sen + Mann–Kendall), with an ETA until
  trouble.

## Status

**Research and architecture phase.** The docs are complete. The code is buildable scaffolding,
and CI builds it on Windows, macOS and Linux. The features are the next milestone.

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
windows/              Cargo workspace: core (platform-neutral), app (Win32 tray app)
macos/                SwiftPM package: MemCore, MemManager (app), MemManagerHelper (daemon)
.github/workflows/    CI
```

## Building

```sh
# Spec vectors
python3 spec/reference/policy_ref.py --check spec/test-vectors

# Windows (on Windows, MSVC toolchain)
cargo build --manifest-path windows/Cargo.toml --release

# Core only (any OS)
cargo test --manifest-path windows/Cargo.toml -p memmanager-core

# macOS (on macOS 13+, Xcode 15+)
cd macos && swift build -c release && swift test
./scripts/bundle.sh release      # lays out build/MemManager.app
```
