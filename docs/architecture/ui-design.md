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

- **Windows:** ready for apps *(default; the number shows GB ready, the ring fills with app memory
  only)*, apps %, or commit %. With Windows-standard values these read Available, In use and
  Commit, as in Task Manager.
- **macOS:** memory pressure *(default; the ring shows the policy score `S`, coloured by kernel
  level)*, ready for apps, apps & macOS %, or compressed GB.

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

## 5. Friendly values (default) and OS-standard values

People read "memory used" as "memory I can't use", so a cache-filled bar looks like a problem even
though it's the opposite. The default presentation reframes the same numbers around what people
actually want: **room for more apps**. Nothing is hidden or invented. Every figure is a real OS
value, and one switch (**Settings → Values**) returns to the OS's own terms.

| | Friendly *(default)* | Windows standard | macOS: Activity Monitor terms |
|---|---|---|---|
| Headline | "9.4 GB ready for apps" (free + standby on Windows; free + cached files on macOS) | "6.6 GB of 16 GB in use" | "9.1 GB of 16 GB used" |
| Breakdown | Apps · Speed-up cache · Free | In use · Modified · Standby · Free | App · Wired · Compressed · Cached Files |
| Ring / icon | Fills with app memory only | Task Manager's In use | Pressure, as Activity Monitor |
| States | Comfortable · Busy · Tight · Critical | Normal · Elevated · High · Critical | Normal · Elevated · High · Critical |
| Graph | Apps with the cache stacked on top | In use | Memory used |

Friendly mode adds:

- **The cache earning its keep (Windows).** "In the last hour, apps got data back from cache 48,210
  times instead of reading it from disk." This comes from the kernel's transition-fault counter
  (`TransitionCount` in `SYSTEM_PERFORMANCE_INFORMATION`) against hard faults. macOS has no
  equivalent system-wide counter, so this line is Windows-only rather than estimated.
- **The cache giving way.** "Cache made room for 1.2 GB of app memory in the last 10 minutes."
  This is shown when apps grew while the cache shrank by at least 128 MB.
- **An honest preview before a manual clean while memory is comfortable** ("Optimize now" on
  Windows, "Nudge" on macOS). It says what will happen, what it costs, and that it won't make
  anything faster right now, then asks. Under real pressure the action runs directly.
- **Idle apps holding real memory**, such as "idle 3 h" in the app list, with a one-line hint. This
  points the wish for "low memory" at the one thing that actually frees memory without a cost
  elsewhere.
- **A one-time explainer link**, "Cache now counts as ready memory. Why? →", which opens
  [`docs/understanding-memory.md`](../understanding-memory.md).

**Guardrails.**

- Warnings under real pressure are unchanged.
- The cache is always visible, only labelled and coloured differently.
- Switching to standard values restores the OS terms, the "In use" icon metric and the standard
  breakdown.

## 6. Updates

- **Check:**
  - Settings → Updates → "Check now".
  - Automatically once a day (on by default), starting two minutes after launch.
  - The check asks the GitHub releases API for the newest release newer than the running version.
    Pre-releases are offered only to pre-release installs (0.x or suffixed versions).
- **Notify:** a notification appears once per newly found version, and a dashboard row reads
  "MemManager X is available · Install →".
- **Install:**
  - Download the platform's asset and its `.sha256`, and refuse anything that doesn't match.
  - **Windows, installed:** run the new setup silently. It upgrades in place with the previous
    choices and starts MemManager again.
  - **Windows, portable:** swap the exe in place (rename the running file, write the new one) and
    restart.
  - **macOS:** unpack the universal zip, check it is MemManager.app, then a small shell step waits
    for the app to quit, swaps the bundles (restoring the old one if the swap fails) and reopens it.
- **Footprint:** the check and install run on short-lived background work; nothing stays resident.

## 7. Seeing what MemManager does

Numbers say *that* something changed. These features show **what MemManager did and why**, so
you can judge it yourself. They are kept out of the way of the dashboard.

- **"Now:" line.** One sentence on what MemManager is doing at this moment, for example "Watching
  · nothing to do", "Memory is tight · acting where it helps", or "Game mode · staying out of the
  way".
- **"Last:" line**, coloured by the measured result, with an **Activity →** link next to it.
- **Graph markers.** Every action is a dot on the 10-minute graph's time axis, with a faint
  vertical line, so its effect on the curve is visible.
- **App badges (Windows).** Apps MemManager trimmed or quieted in the last 30 minutes say so in
  the app list.
- **Activity view.** It opens from the dashboard and lists the last 40 events, newest first. Each
  event has:
  - **What**, for example "Trimmed idle apps", "Warned: Teams looks like it's leaking", or a manual
    per-app action.
  - **Why:** the reading that triggered it ("Only 820 MB was quickly free and Windows was already
    recycling its cache"), or "You asked".
  - **Which apps** were affected, or the error if it failed.
  - **The measured result**, from the regret ledger: "1.1 GB back · no slowdown" (green), "a
    little was read back" or "measuring…" (grey), or "caused a slowdown, backing off" (amber).
  - At the top: a one-line summary ("3 actions · 2.3 GB back · no slowdowns"), the "cache made room
    for…" line, and the cache-hit count on Windows.

**Space.** The dashboard gained no permanent rows; the status rows were reworded instead. The
explanatory cache caption shows only alongside the one-time explainer. The detail lines from §5
moved into Activity.

**Cost.**

- The log is a fixed ring of 40 entries.
- App rows, graph history and the activity list are built only while the flyout or popover is
  open, so a closed app allocates nothing for them per tick.
- The macOS activity page is created when opened and released when closed, and it rebuilds its
  rows only when the log changes.

## 8. App icon

A blue tile with the white 270° gauge ring, about two-thirds full, around a small centre dot. It
is the same ring as the tray and menu bar icon, so the app and its status item read as one.
[`scripts/make_icons.py`](../../scripts/make_icons.py) (stdlib only) renders every size natively
with signed-distance anti-aliasing, using thicker strokes and no dot at the smallest sizes. It
writes:

- `windows/app/memmanager.ico`: embedded by `build.rs` as a `.res` with no resource compiler;
  also used as the installer icon.
- `macos/Resources/AppIcon.icns`: copied into the bundle by `bundle.sh`.
- `assets/icon.svg` (vector master) and `assets/icon-256.png`.

