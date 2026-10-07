# UI design: icon, popup, settings

> Goal: **clean, simple, modern**, and native on each OS. On Windows that means the Fluent
> flyout language (Windows 11 acrylic, 8 px radius, Segoe UI Variable). On macOS it means the
> menu bar extra language (popover material / Liquid Glass, SF Pro, SF Symbols). One information
> architecture, two native skins.

## 1. Status icon

### 1.1 Styles

| Style | Windows tray (square 16–32 px) | macOS menu bar (h = bar thickness, w ≤ 40 pt) | Shows |
|---|---|---|---|
| **Ring** (default) | Circular gauge, 2 px stroke at 16 px (scaled), filled arc = metric | Same, 16 pt diameter | Metric as a fraction; colour = state |
| **Bar** | Vertical bar filling bottom→top inside a rounded rect | Horizontal mini-bar, 22 × 8 pt | Metric; colour = state |
| **Number** | 2–3 digits, tabular figures, auto-fitted font size | "62%" in SF Mono-like tabular digits | Metric value |
| **Sparkline** | — (too small) | 32 × 14 pt, last 2 min | Pressure / usage trend |
| **Dot + number** | — | ● 62 | State dot plus value |

**State cue beyond colour** (accessibility):

- Ring: plain at **Normal**, a 1 px gap notch at **High**, and a filled centre dot at **Critical**.
- **Acting**: the ring animates once (a 300 ms sweep) when an action runs, then stops. There is no
  continuous animation.

### 1.2 Metrics (user picks one)

- **Windows:** memory used % *(default)*, commit %, or available GB.
- **macOS:** memory pressure *(default; the ring shows the policy score `S`, coloured by kernel
  level)*, memory used %, or compressed GB.

### 1.3 Colour tokens

These are the defaults for the "System" preset. Each token has a light-bar and a dark-bar value,
so it meets ≥ 3:1 contrast against the taskbar or menu bar.

| Token | Light bar | Dark bar | Notes |
|---|---|---|---|
| `normal` | Accent colour (Win: DWM AccentColor; mac: `controlAccentColor`) | same | Monochrome preset: foreground colour / template |
| `elevated` | `#B58400` | `#F2C744` | amber |
| `high` | `#C8551B` | `#FF8F4D` | orange |
| `critical` | `#C42B1C` | `#FF6B5E` | red |
| `acting` | Accent colour, brighter | same | Transient |
| `paused` | `#808080` | `#9A9A9A` | Ring drawn dashed |
| `limited` | `normal` colour with a small "!" badge | | No privileges / helper missing |

**Presets:** System, Traffic light, Monochrome, Colour-blind safe (`#0A6CD6` → `#E69F00` →
`#D55E00`, Okabe–Ito), and Custom (per-token colour picker).

## 2. Popup layout (shared information architecture)

```
┌──────────────────────────────────────────────┐
│  (◔ 62%)   Memory · Normal          ⚙  ⏸    │  header: gauge, state chip, settings, pause
│            9.9 of 16 GB used                  │
├──────────────────────────────────────────────┤
│  ▇▇▇▇▇▇▇▇▇▇▇▇▇▇▆▆▆▅▅▅▅▅▅▅░░░░░░░             │  breakdown bar
│  ■ In use 8.1  ■ Modified 0.4  ■ Standby 5.6 ░ Free 1.9      (Windows)
│  ■ App 6.2  ■ Wired 2.1  ■ Compressed 1.6  ░ Cached 4.4      (macOS)
│  Commit 14.2 / 24 GB · Compressed 1.1 GB     │  (Win)   Swap 0.8 GB (mac)
├──────────────────────────────────────────────┤
│  ╱╲__╱‾‾╲___╱‾‾‾  last 10 min                 │  sparkline (score or used %)
├──────────────────────────────────────────────┤
│  Top apps                                     │
│  ● Chrome        2.4 GB  ↗ 120 MB/h           │  growth arrow; ⚠ leak badge
│  ● Teams         1.1 GB  ⚠ leaking 380 MB/h   │  click → actions (Restart / Limit / Ignore)
│  ● Code          0.9 GB                       │
├──────────────────────────────────────────────┤
│  Recent: purged low-priority cache · 1.1 GB · 4 min ago · no refaults
├──────────────────────────────────────────────┤
│  [ Optimize now ]   Conservative|Balanced|Aggressive         │
└──────────────────────────────────────────────┘
```

- **Windows:** 360 epx wide. 16 epx padding. 8 px corner radius. Acrylic transient backdrop.
  Section separators are 1 px at 8% foreground.
- **macOS:** 340 pt wide. 14 pt padding. Popover material. The "Optimize now" button becomes
  **Nudge** (shown only when the helper is installed; otherwise "Install helper…" in Settings).
- **Typography:**
  - Windows: Segoe UI Variable Display (header 20/28) and Text (13/18 body, 11/16 caption).
  - macOS: SF Pro (`.title3`, `.body`, `.caption`). All numbers use tabular figures.
- **Motion:** open with a 140 ms ease-out fade plus 4 px slide from the anchor edge. Value changes
  tween over 200 ms. Respect "reduce motion" (Windows `SPI_GETCLIENTAREAANIMATION`; macOS
  `accessibilityDisplayShouldReduceMotion`).

## 3. Settings

| Section | Controls |
|---|---|
| **General** | Start at login · Language (system) · Check for updates |
| **Icon & colours** | Style (Ring / Bar / Number / Sparkline / Dot), Metric, Colour preset, per-state colour pickers with a live preview of the icon at 1×/2× on light and dark bars, "Show value in tooltip" |
| **Behaviour** | Profile slider (Conservative ↔ Aggressive) with plain-language consequences under it, auto-actions on/off per tier (Windows) / per action (macOS), Game mode (Win), Exclusions list (never touch), Heavy apps list (relaxed leak thresholds), Ignore list (no leak alerts), Auto-quit allowlist (mac) |
| **Notifications** | Leak alerts · Swap/commit warnings · Action results (off by default) |
| **Privileges** (Win) / **Helper** (mac) | Status, Enable cleaning / Install helper, Remove |
| **Advanced** | Sampling cadence multiplier, Deep clean / Purge disk cache (with explanations), Export ledger as CSV |
| **About** | Version, the app's own memory use (live), links to the docs |

**Plain-language consequence text** for the profile slider:

- **Conservative:** "Only clears cache Windows was about to discard. Never trims apps."
- **Balanced:** "Clears low-value cache early and quiets idle background apps when memory gets
  tight."
- **Aggressive:** "Keeps lots of RAM free. Apps you return to may take a moment to wake up."

## 4. Copy guidelines

- **Lead with the consequence, not the mechanism.** Write "Freed 1.1 GB of cache Windows was about
  to discard", not "MemoryPurgeLowPriorityStandbyList OK".
- **Be honest about trade-offs.** For example, "Limiting RAM slows Teams down but doesn't stop the
  leak. Restarting it does."
- **Never claim speed-ups we haven't measured.** The ledger's refault data is the only basis for
  "no slowdown detected".
