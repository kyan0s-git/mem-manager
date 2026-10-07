import AppKit
import MemCore

/// The menu bar popover (docs/architecture/ui-design.md §2). Built on demand
/// and released when it closes.
final class DashboardViewController: NSViewController {
    let gauge = GaugeView()
    let titleLabel = label("Memory", size: 15, weight: .semibold)
    let chip = ChipView()
    let subtitle = label(size: 13, color: .secondaryLabelColor)
    let line3 = label(size: 11, color: .tertiaryLabelColor)
    let breakdown = BreakdownView()
    let legend = [label(size: 11, color: .secondaryLabelColor), label(size: 11, color: .secondaryLabelColor),
                  label(size: 11, color: .secondaryLabelColor), label(size: 11, color: .secondaryLabelColor)]
    let sparkCaption = label("Memory used · last 10 minutes", size: 11, color: .tertiaryLabelColor)
    let spark = SparklineView()
    let appsCaption = label("Apps using the most memory", size: 11, weight: .semibold, color: .secondaryLabelColor)
    let appsStack = NSStackView()
    let idleCaption = label("Idle apps holding a lot of memory", size: 11, weight: .semibold, color: .secondaryLabelColor)
    let idleStack = NSStackView()
    let action1 = label(size: 11, color: .secondaryLabelColor)
    let action2 = label(size: 11, color: .tertiaryLabelColor)
    let nudgeButton = NSButton(title: "Nudge", target: nil, action: nil)
    let profile = NSSegmentedControl(labels: Profile.allCases.map(\.label), trackingMode: .selectOne, target: nil, action: nil)
    let pauseButton = NSButton()
    let settingsButton = NSButton()

    weak var controller: AppController?

    override func loadView() {
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 340, height: 480))
        view = root

        for b in [pauseButton, settingsButton] {
            b.bezelStyle = .inline
            b.isBordered = false
            b.imagePosition = .imageOnly
            b.translatesAutoresizingMaskIntoConstraints = false
            b.target = self
        }
        settingsButton.image = NSImage(systemSymbolName: "gearshape", accessibilityDescription: "Settings")
        settingsButton.action = #selector(openSettings)
        pauseButton.action = #selector(togglePause)
        nudgeButton.bezelStyle = .rounded
        nudgeButton.controlSize = .large
        nudgeButton.keyEquivalent = "\r"
        nudgeButton.target = self
        nudgeButton.action = #selector(nudge)
        nudgeButton.toolTip = "Ask apps to release their caches (uses the optional helper)"
        profile.target = self
        profile.action = #selector(profileChanged)
        profile.controlSize = .small

        let titleRow = NSStackView(views: [titleLabel, chip])
        titleRow.spacing = 8
        let headText = NSStackView(views: [titleRow, subtitle, line3])
        headText.orientation = .vertical
        headText.alignment = .leading
        headText.spacing = 2
        let buttons = NSStackView(views: [pauseButton, settingsButton])
        buttons.spacing = 4
        let header = NSStackView(views: [gauge, headText, NSView(), buttons])
        header.alignment = .centerY
        header.spacing = 12

        let legendGrid = NSGridView(views: [[legend[0], legend[1]], [legend[2], legend[3]]])
        legendGrid.rowSpacing = 4
        legendGrid.columnSpacing = 16

        appsStack.orientation = .vertical
        appsStack.spacing = 2
        appsStack.alignment = .leading
        idleStack.orientation = .vertical
        idleStack.spacing = 2
        idleStack.alignment = .leading

        let footer = NSStackView(views: [nudgeButton, NSView(), profile])
        footer.alignment = .centerY

        let stack = NSStackView(views: [
            header, separator(), breakdown, legendGrid, sparkCaption, spark, separator(),
            appsCaption, appsStack, idleCaption, idleStack, separator(), action1, action2, footer,
        ])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.edgeInsets = NSEdgeInsets(top: 14, left: 14, bottom: 14, right: 14)
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.setCustomSpacing(4, after: sparkCaption)
        stack.setCustomSpacing(2, after: action1)
        root.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            stack.topAnchor.constraint(equalTo: root.topAnchor),
            stack.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            root.widthAnchor.constraint(equalToConstant: 340),
        ])
        for v in [header, breakdown, spark, appsStack, idleStack, footer, legendGrid] {
            v.translatesAutoresizingMaskIntoConstraints = false
            v.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -28).isActive = true
        }
    }

    private func separator() -> NSView {
        let b = NSBox()
        b.boxType = .separator
        return b
    }

    @objc private func openSettings() { controller?.showSettings() }
    @objc private func togglePause() { controller?.togglePause() }
    @objc private func nudge() { controller?.nudge() }
    @objc private func profileChanged() {
        let i = profile.selectedSegment
        if Profile.allCases.indices.contains(i) { AppSettings.shared.profile = Profile.allCases[i] }
    }

    func render(_ snap: Snapshot, settings: AppSettings, helperInstalled: Bool) {
        _ = view
        let dark = view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let preset = settings.preset == .mono ? ColorPreset.system : settings.preset
        let col = Palette.stateColor(snap.state, preset: preset, custom: settings.customColors, dark: dark)
        let r = snap.reading
        gauge.fraction = snap.s
        gauge.color = col
        gauge.text = "\(Int((snap.s * 100).rounded()))%"
        gauge.setAccessibilityLabel("Memory pressure \(snap.state.name)")
        chip.title = snap.paused ? "Paused" : snap.state.name
        chip.color = snap.paused ? .secondaryLabelColor : col
        subtitle.stringValue = r.total > 0 ? "\(fmtBytes(r.used)) of \(fmtBytes(r.total)) used" : "Reading memory…"
        line3.stringValue = "Kernel: \(r.kernelLevelName) · Swap \(fmtBytes(r.swapUsed)) · \(String(format: "%.1f", r.compressionRatio))× compression"
        breakdown.total = max(r.total, 1)
        breakdown.parts = [
            .init(value: r.app, color: col, name: "App"),
            .init(value: r.wired, color: .systemPurple, name: "Wired"),
            .init(value: r.compressed, color: .systemOrange, name: "Compressed"),
            .init(value: r.cached, color: .systemTeal, name: "Cached"),
        ]
        let names = [("App", r.app), ("Wired", r.wired), ("Compressed", r.compressed), ("Cached files", r.cached)]
        for (i, (n, v)) in names.enumerated() { legend[i].stringValue = "● \(n) \(fmtBytes(v))" }
        legend[0].textColor = col
        legend[1].textColor = .systemPurple
        legend[2].textColor = .systemOrange
        legend[3].textColor = .systemTeal
        spark.values = snap.history
        spark.color = col
        pauseButton.image = NSImage(systemSymbolName: snap.paused ? "play.fill" : "pause.fill",
                                    accessibilityDescription: snap.paused ? "Resume" : "Pause for 1 hour")
        pauseButton.toolTip = snap.paused ? "Resume automatic actions" : "Pause automatic actions for 1 hour"

        rebuild(appsStack, rows: snap.apps.prefix(5).map { $0 }, idle: false)
        idleCaption.isHidden = snap.idleHeavy.isEmpty
        idleStack.isHidden = snap.idleHeavy.isEmpty
        rebuild(idleStack, rows: snap.idleHeavy.prefix(3).map { $0 }, idle: true)

        if let e = snap.ledger.first {
            let freed = e.freed > 0 ? " · \(fmtBytes(e.freed)) freed" : (e.detail.isEmpty ? "" : " · \(e.detail)")
            action1.stringValue = e.label + freed
            let impact: String
            if e.pending { impact = "measuring impact…" }
            else if e.refaultRatio < 0.05 { impact = "no slowdown measured" }
            else if e.refaultRatio < 0.25 { impact = "a few pages were read back afterwards" }
            else { impact = "caused page-ins — backing off" }
            action2.stringValue = "\(e.manual ? "" : "automatic · ")\(fmtAgo(snap.tMs &- e.tMs)) · \(impact)"
        } else {
            action1.stringValue = snap.state == .normal ? "Memory is healthy — macOS is managing it well." : "Watching memory pressure…"
            action2.stringValue = "\(snap.visibleProcesses) of your processes monitored"
        }
        nudgeButton.isEnabled = helperInstalled
        nudgeButton.toolTip = helperInstalled
            ? "Ask apps to release their caches and empty purgeable memory"
            : "Install the helper in Settings to enable Nudge"
        if let i = Profile.allCases.firstIndex(of: settings.profile) { profile.selectedSegment = i }
    }

    private func rebuild(_ stack: NSStackView, rows: [AppRow], idle: Bool) {
        while stack.arrangedSubviews.count > rows.count {
            let v = stack.arrangedSubviews.last!
            stack.removeArrangedSubview(v)
            v.removeFromSuperview()
        }
        while stack.arrangedSubviews.count < rows.count {
            let v = AppRowView()
            stack.addArrangedSubview(v)
            v.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        for (v, row) in zip(stack.arrangedSubviews.compactMap { $0 as? AppRowView }, rows) {
            v.name.stringValue = row.name
            v.size.stringValue = fmtBytes(row.footprint)
            if idle, let h = row.idleHours {
                v.note.stringValue = "idle \(fmtHours(h))"
                v.note.textColor = .secondaryLabelColor
            } else if row.leak {
                v.note.stringValue = "⚠︎ leaking \(Int(row.slope)) MB/h"
                v.note.textColor = .systemRed
            } else if row.runaway {
                v.note.stringValue = "growing fast"
                v.note.textColor = .systemOrange
            } else if row.slope >= 30 {
                v.note.stringValue = "↗ \(Int(row.slope)) MB/h"
                v.note.textColor = .tertiaryLabelColor
            } else {
                v.note.stringValue = ""
            }
            v.setAccessibilityLabel("\(row.name), \(fmtBytes(row.footprint)) \(v.note.stringValue)")
            v.onClick = { [weak self] view in self?.controller?.showAppMenu(row, from: view) }
        }
    }
}
