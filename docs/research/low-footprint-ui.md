# Low-footprint, modern UI: tray (Windows) and menu bar (macOS)

> The product's credibility depends on its own footprint. A memory manager that uses 80 MB to
> save 200 MB is a joke. This document picks UI technology for each platform by measured or
> structural cost, and records the platform details needed for a polished status icon and popup.
> Tags refer to [`sources.md`](sources.md).

## 1. What "memory used by the UI" means

Two different costs are easy to confuse:

- **Shared, image-backed pages.** DLL and framework code loaded from disk. These pages are shared
  with every other process using the same library and are reclaimable, so they count toward our
  working set but **not** toward private bytes or footprint. Loading `d2d1.dll` or `AppKit` is
  cheap in the metric that matters.
- **Private, dirty memory.** Heaps, runtime metadata, view trees, GPU/staging buffers, and JIT
  code. *This* is what a runtime-heavy UI framework inflates: .NET, XAML, Chromium, the SwiftUI
  attribute graph.

So the budget metric is **Private Bytes / Commit** on Windows and **`phys_footprint`** on macOS.
The working set is reported too, as a secondary number. See `docs/architecture/resource-budget.md`.

## 2. Windows technology options

| Option | Private-memory character | Startup | Modern look | Verdict |
|---|---|---|---|---|
| **Win32 + Direct2D/DirectWrite** (Rust, `windows` crate) | Smallest: our own structs plus D2D device resources, created on demand and released on hide | Instant | Full control; DWM gives rounded corners and acrylic natively [W12][W16][W17] | **Chosen** |
| Win32 + GDI (Mem Reduct) | Smallest | Instant | No anti-aliased alpha, poor DPI scaling, dated look | Icon and popup quality too low |
| WPF (.NET) | Tens of MB (CLR, JIT, WPF visual tree) | Slow cold start | Fair | Too heavy for always-on |
| WinUI 3 / Windows App SDK | Tens of MB. Microsoft has publicly worked on WinUI allocation overhead, but it is still a XAML runtime. | Medium | Native Fluent | Too heavy for always-on; overkill for one flyout |
| WebView2 / Tauri | Separate Edge renderer and GPU processes, typically 50–150 MB total | Medium | Web-styled | Rejected |
| Electron | 100 MB+ | Slow | Web-styled | Rejected |

**Rust specifics:**

- Use the `windows` crate (or `windows-sys` for zero-cost raw bindings).
- No async runtime, no serde in the hot path, and `panic = "abort"`.
- The binary should be about 300–600 KB stripped.
- Use one message loop with `MsgWaitForMultipleObjectsEx` so that **window messages, kernel
  memory-condition events and waitable timers wake the same thread**. There is no extra thread for
  sampling.

## 3. Windows tray icon

- **API.** `Shell_NotifyIconW` with `NOTIFYICON_VERSION_4`, `NIF_GUID` and `NIF_SHOWTIP`:
  - The **GUID is bound to the executable path** unless the binary is Authenticode-signed. Moving
    an unsigned exe makes `NIM_ADD` fail. Fall back to `uID` identity in development builds.
  - Re-add the icon on the registered `TaskbarCreated` message, which fires when Explorer
    restarts.
  - With version 4, `WM_CONTEXTMENU`/`NIN_SELECT`/`NIN_KEYSELECT` arrive with the anchor point in
    `wParam`, which we use for popup placement.
- **Sizes.** Query `GetSystemMetricsForDpi(SM_CXSMICON, dpi)`: 16 px at 100%, 20 at 125%, 24 at
  150%, 32 at 200%. Render **at the exact pixel size**, never scaled.
- **Rendering pipeline (all CPU, no GPU device needed):**
  1. A `CreateDIBSection` 32-bpp top-down BGRA buffer (grow-only, reused).
  2. Draw with **Direct2D into a WIC bitmap render target** (software), using DirectWrite for
     digits. Or, for the ring, bar and dot styles, a ~200-line in-house anti-aliased rasterizer
     (coverage-based arcs and rects), which avoids D2D entirely for the icon.
  3. `CreateIconIndirect` with the colour DIB and an all-zero mask, so per-pixel alpha is used.
  4. `NIM_MODIFY`, then `DestroyIcon(previous)`.
  5. **Only redraw when the *displayed* value or state changes** (Mem Reduct does the same [W8]).
     That costs about one icon update per few seconds at most.
- **Theme awareness:**
  - Taskbar light or dark: `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme`.
    The app content theme is `AppsUseLightTheme`.
  - Accent colour: `HKCU\Software\Microsoft\Windows\DWM\AccentColor`, or `DwmGetColorizationColor`.
  - React to `WM_SETTINGCHANGE` with `lParam == L"ImmersiveColorSet"` and to `WM_DPICHANGED`.
  - **High contrast:** `SystemParametersInfo(SPI_GETHIGHCONTRAST)` switches the icon to
    system colours.
- **Width limitation.** Tray icons are square, which fits two digits well and three at a squeeze
  ("100"). A "wide" mode uses **two adjacent tray icons** (a number and a mini-bar), a known
  technique. It is opt-in.

## 4. Windows popup (flyout)

- **Window.** `WS_POPUP` with `WS_EX_TOOLWINDOW` (no taskbar button) and `WS_EX_NOREDIRECTIONBITMAP`
  if we use a composition swap chain; otherwise a normal redirection surface.
- **Windows 11 look** [W12][W16][W17]:
  - `DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE (33), DWMWCP_ROUND (2))` gives
    the 8 px flyout radius.
  - `DwmSetWindowAttribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE (38), DWMSBT_TRANSIENTWINDOW (3))`
    gives acrylic on build 22621 or later, plus `DwmExtendFrameIntoClientArea({-1})` and a
    transparent clear colour.
  - `DWMWA_USE_IMMERSIVE_DARK_MODE (20)` matches dark mode.
  - Fallback for Windows 10 and older 11 builds: a solid `#202020`/`#F3F3F3` with a 1 px border.
- **Placement.** Use the `Shell_NotifyIconGetRect` anchor and `MonitorFromPoint` →
  `GetMonitorInfo().rcWork`. Open above, below or beside the anchor depending on the taskbar edge.
  Clamp to the work area and respect per-monitor DPI.
- **Dismissal.** `WM_ACTIVATE(WA_INACTIVE)` hides the popup. `Esc` closes it, and `Tab` focus
  navigation must work.
- **Rendering.**
  - An `ID2D1HwndRenderTarget`, or a DXGI swap chain with DirectComposition, **created on show and
    released on hide**. The popup then calls `SetProcessWorkingSetSizeEx(self, -1, -1)` on itself
    once, so the idle working set returns to baseline. Trimming *our own* idle process is the one
    working-set trim that is unambiguously good.
  - Text: DirectWrite with **Segoe UI Variable** (Win11), falling back to Segoe UI.
  - Glyphs come from the **Segoe Fluent Icons** font (Win11), falling back to Segoe MDL2 Assets.
    There are no image assets to ship or decode.
  - Animation: a single 120–160 ms fade or slide on open, using a high-resolution waitable timer
    that only runs during the animation.
- **Accessibility.** Custom-drawn UI needs a UI Automation provider. We implement a minimal
  `IRawElementProviderFragmentRoot` for the popup's buttons, toggles and the value readout, so
  Narrator and NVDA work.

## 5. macOS technology options

| Option | Private-memory character | Verdict |
|---|---|---|
| **AppKit** (`NSStatusItem`, `NSPopover`/`NSPanel`, CoreGraphics/CALayer drawing) | Small; the framework is shared and already loaded by every app | **Chosen for the always-on path and the popover** |
| SwiftUI `MenuBarExtra` (macOS 13+) | Pulls in the SwiftUI and AttributeGraph runtime and a persistent view graph. Convenient, but costs extra dirty memory for the app's whole lifetime. Capabilities are limited (custom drawing, right-click, colour beyond template images). | Not for the always-on path |
| SwiftUI in an `NSHostingController`, created on demand | The view graph is freed when the window closes | **Chosen for Settings only** |
| Electron / Tauri | Large | Rejected |

## 6. macOS menu bar item

- `NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)`.
- **Drawing.** `NSImage(size:flipped:drawingHandler:)`. The handler renders at the backing scale
  automatically (retina), so there are no bitmaps to cache per scale.
  - **Monochrome style**: `isTemplate = true`, so the system tints for light, dark, tinted
    menu bars and the highlighted state.
  - **Colour styles**: non-template image. We observe `button.effectiveAppearance` so colours
    keep contrast on light and dark menu bars.
- **Size.** Height is `NSStatusBar.system.thickness`, which is larger on notched displays. Keep the
  width compact, since items beyond the notch are hidden by the system. The default style is
  about 22–28 pt wide.
- **Styles.** Pressure ring; bar; percentage text; **mini sparkline** (the menu bar allows wide
  items, unlike the Windows tray); dot + number.
- **Update policy.** Same as Windows: redraw only when the displayed value or state changes.
- **Click handling.** Left click toggles the popover. Right click (or ⌥-click) shows a compact
  `NSMenu` (Nudge, Pause, Settings, Quit).

## 7. macOS popover

- `NSPopover` (`behavior = .transient`) hosting an AppKit view controller gives the system
  material, arrow, and Liquid Glass styling automatically on newer SDKs. Alternatively, use a
  borderless `NSPanel` with an `NSVisualEffectView` (`.popover`/`.menu` material,
  `.behindWindow`, `state = .active`) for full control of shape.
- **macOS 26 "Liquid Glass"**: `NSGlassEffectView` exists only on macOS 26. Look it up at runtime
  (`NSClassFromString("NSGlassEffectView")`) or use `if #available(macOS 26, *)` when compiling
  with the 26 SDK, placing our content view inside the glass view. Fall back to
  `NSVisualEffectView` on older systems [M15].
- **Content.** Draw charts with CoreGraphics in `draw(_:)` or `CAShapeLayer` paths, not Swift
  Charts. Use SF Symbols for glyphs.
- **Free on close.** On `popoverDidClose`, release the content view controller and its layers.

## 8. Settings windows (both platforms)

- **macOS.** A SwiftUI `Form` in an `NSHostingController` inside an `NSWindow` that is **created
  on open and released on close**. Use the `windowWillClose` hook to nil the reference.
- **Windows.** Settings are a second *view inside the flyout*, drawn with the same Direct2D widgets
  (segmented controls, toggles, colour swatches with the system colour picker, buttons) and
  scrollable. A single window means one set of device resources, released when the flyout
  hides. Exclusion and ignore lists are edited as plain text in `config.ini`, which the app
  reloads when the file changes.
- **Persistence.**
  - Windows: an INI-style file in `%APPDATA%\MemManager\config.ini`. A hand-written parser of
    about 150 lines avoids serde.
  - macOS: `UserDefaults`.

## 9. Colour and status customization model (shared)

- **State tokens:** `normal`, `elevated`, `high`, `critical`, `acting` (an action is running),
  `paused`, `limited` (no privileges or helper).
- **Presets:**
  - *System accent*: normal is the accent colour, and the others follow the traffic-light hues.
  - *Traffic light*.
  - *Monochrome* (template on macOS, foreground colour on Windows).
  - *Colour-blind safe*: blue → amber → orange-red, with **shape cues**. The ring gains a notch
    at High and a filled centre at Critical, so state never depends on hue alone.
- **Per-state custom colours** come from a colour picker and are stored as `#RRGGBB`.
- **Metric shown in the icon:**
  - Windows: memory in use %, commit %, or available GB.
  - macOS: pressure (as Activity Monitor does), memory used %, or compressed GB.
- **Tooltip / accessibility label:** a full sentence, e.g. "Memory 62% used, pressure normal,
  last action 4 min ago: purged low-priority cache (freed 1.1 GB)".
