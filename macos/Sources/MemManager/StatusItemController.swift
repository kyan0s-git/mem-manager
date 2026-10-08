import AppKit
import MemCore

/// The menu bar item: redraws only when what it shows changes.
final class StatusItemController {
    let item: NSStatusItem
    private var lastSpec: IconSpec?
    var onLeftClick: (() -> Void)?
    var onRightClick: (() -> Void)?

    init() {
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let b = item.button {
            b.target = self
            b.action = #selector(clicked(_:))
            b.sendAction(on: [.leftMouseUp, .rightMouseUp])
            b.setAccessibilityLabel("MemManager")
        }
    }

    @objc private func clicked(_ sender: NSStatusBarButton) {
        let ev = NSApp.currentEvent
        if ev?.type == .rightMouseUp || ev?.modifierFlags.contains(.option) == true || ev?.modifierFlags.contains(.control) == true {
            onRightClick?()
        } else {
            onLeftClick?()
        }
    }

    static func value(_ snap: Snapshot, metric: IconMetric, friendly: Bool) -> (Double, String) {
        let r = snap.reading
        switch metric {
        case .ready:
            // Friendly: the ring fills with app memory only, so a full cache never looks full.
            let sp = r.split
            let gb = Double(sp.ready) / 1_073_741_824
            let f = friendly ? sp.appsFraction : (r.total > 0 ? Double(sp.ready) / Double(r.total) : 0)
            return (f, gb >= 10 ? String(format: "%.0fG", gb) : String(format: "%.1fG", gb))
        case .pressure:
            return (snap.s, "\(Int((snap.s * 100).rounded()))%")
        case .used:
            return (r.usedFraction, "\(Int((r.usedFraction * 100).rounded()))%")
        case .compressed:
            let f = r.total > 0 ? Double(r.compressed) / Double(r.total) : 0
            let gb = Double(r.compressed) / 1_073_741_824
            return (f, gb >= 10 ? String(format: "%.0fG", gb) : String(format: "%.1fG", gb))
        }
    }

    func update(_ snap: Snapshot, settings: AppSettings) {
        guard let button = item.button else { return }
        let dark = button.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let (frac, text) = StatusItemController.value(snap, metric: settings.iconMetric, friendly: settings.friendlyValues)
        var color = Palette.stateColor(snap.state, preset: settings.preset, custom: settings.customColors, dark: dark)
        if snap.acting { color = .controlAccentColor }
        let spec = IconSpec(
            style: settings.iconStyle,
            fraction: (frac * 50).rounded() / 50,
            text: text,
            state: snap.state,
            color: color,
            template: settings.preset == .mono,
            paused: snap.paused,
            history: settings.iconStyle == .sparkline ? snap.history.map { ($0 * 40).rounded() / 40 } : [],
            height: NSStatusBar.system.thickness
        )
        if spec != lastSpec {
            lastSpec = spec
            button.image = IconRenderer.image(spec)
        }
        let r = snap.reading
        let sp = r.split
        let tip = settings.friendlyValues
            ? "\(fmtBytes(sp.ready)) ready for apps · \(Presentation.stateName(snap.state, friendly: true)) · Apps & macOS \(fmtBytes(sp.apps)) · Speed-up cache \(fmtBytes(sp.cache))"
            : String(format: "Memory pressure %@ · %.0f%% used · %@ compressed · swap %@",
                     snap.state.name, r.usedFraction * 100, fmtBytes(r.compressed), fmtBytes(r.swapUsed))
        if button.toolTip != tip {
            button.toolTip = tip
            button.setAccessibilityValue(tip)
        }
    }

    func invalidate() { lastSpec = nil }
}
