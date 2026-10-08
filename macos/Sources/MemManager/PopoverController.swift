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
    let cacheNote = caption("Speed-up cache keeps recent files in memory. macOS hands it to apps the moment they need it.")
    let activityLink = NSButton(title: "", target: nil, action: nil)
    private var mainStack: NSStackView!
    private var activityPage: ActivityPageView?
    private var lastSnap = Snapshot()
    private var lastHelper = false
    let updateButton = NSButton(title: "", target: nil, action: nil)
    let explainerButton = NSButton(title: "", target: nil, action: nil)
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
        for (b, sel) in [(updateButton, #selector(installUpdate)), (explainerButton, #selector(openExplainer)),
                         (activityLink, #selector(openActivity))] {
            b.isBordered = false
            b.bezelStyle = .inline
            b.alignment = .left
            b.target = self
            b.action = sel
        }
        explainerButton.attributedTitle = linkTitle("Cache now counts as ready memory. Why? →")
        cacheNote.preferredMaxLayoutWidth = 312
        activityLink.attributedTitle = linkTitle("Activity →")
        activityLink.setContentHuggingPriority(.required, for: .horizontal)
        action2.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

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

        let statusRow = NSStackView(views: [action2, NSView(), activityLink])
        statusRow.alignment = .centerY
        let stack = NSStackView(views: [
            header, separator(), breakdown, legendGrid, cacheNote, sparkCaption, spark, separator(),
            appsCaption, appsStack, idleCaption, idleStack, separator(), updateButton, explainerButton,
            action1, statusRow, footer,
        ])
        mainStack = stack
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.edgeInsets = NSEdgeInsets(top: 14, left: 14, bottom: 14, right: 14)
        stack.translatesAutoresizingMaskIntoConstraints = false
        stack.setCustomSpacing(4, after: sparkCaption)
        stack.setCustomSpacing(2, after: action1)
        root.widthAnchor.constraint(equalToConstant: 340).isActive = true
        pin(stack)
        for v in [header, breakdown, spark, appsStack, idleStack, footer, legendGrid, cacheNote, statusRow] {
            v.translatesAutoresizingMaskIntoConstraints = false
            v.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -28).isActive = true
        }
    }

    /// Makes `v` the popover's content, filling it.
    private func pin(_ v: NSView) {
        v.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(v)
        NSLayoutConstraint.activate([
            v.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            v.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            v.topAnchor.constraint(equalTo: view.topAnchor),
            v.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
    }

    /// Swaps between the dashboard and the activity page (built on first use).
    private func setActivity(_ on: Bool) {
        if on {
            let page = activityPage ?? ActivityPageView { [weak self] in self?.setActivity(false) }
            activityPage = page
            mainStack.removeFromSuperview()
            pin(page)
            page.render(lastSnap, helperInstalled: lastHelper)
        } else {
            activityPage?.removeFromSuperview()
            activityPage = nil
            pin(mainStack)
        }
        view.layoutSubtreeIfNeeded()
        preferredContentSize = view.fittingSize
    }

    @objc private func openActivity() { setActivity(true) }

    private func separator() -> NSView {
        let b = NSBox()
        b.boxType = .separator
        return b
    }

    private func linkTitle(_ s: String) -> NSAttributedString {
        NSAttributedString(string: s, attributes: [.foregroundColor: NSColor.linkColor, .font: NSFont.systemFont(ofSize: 11, weight: .semibold)])
    }

    @objc private func installUpdate() { controller?.installUpdate() }
    @objc private func openExplainer() { controller?.openExplainer() }
    @objc private func openSettings() { controller?.showSettings() }
    @objc private func togglePause() { controller?.togglePause() }
    @objc private func nudge() { controller?.nudge() }
    @objc private func profileChanged() {
        let i = profile.selectedSegment
        if Profile.allCases.indices.contains(i) { AppSettings.shared.profile = Profile.allCases[i] }
    }

    func render(_ snap: Snapshot, settings: AppSettings, helperInstalled: Bool) {
        _ = view
        lastSnap = snap
        lastHelper = helperInstalled
        if let page = activityPage {
            page.render(snap, helperInstalled: helperInstalled)
            return
        }
        let dark = view.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        let preset = settings.preset == .mono ? ColorPreset.system : settings.preset
        let col = Palette.stateColor(snap.state, preset: preset, custom: settings.customColors, dark: dark)
        let r = snap.reading
        gauge.fraction = snap.s
        gauge.color = col
        gauge.text = "\(Int((snap.s * 100).rounded()))%"
        gauge.setAccessibilityLabel("Memory pressure \(snap.state.name)")
        let friendly = settings.friendlyValues
        let sp = r.split
        chip.title = snap.paused ? "Paused" : Presentation.stateName(snap.state, friendly: friendly)
        chip.color = snap.paused ? .secondaryLabelColor : col
        breakdown.total = max(r.total, 1)
        if friendly {
            // Lead with memory that's ready for apps; cached files are part of it.
            subtitle.stringValue = r.total > 0 ? "\(fmtBytes(sp.ready)) ready for apps" : "Reading memory…"
            line3.stringValue = "Apps & macOS \(fmtBytes(sp.apps)) · of \(fmtBytes(r.total)) · Swap \(fmtBytes(r.swapUsed))"
            breakdown.parts = [
                .init(value: sp.apps, color: col, name: "Apps & macOS"),
                .init(value: sp.cache, color: .systemTeal, name: "Speed-up cache"),
            ]
            let names = [("Apps & macOS", sp.apps), ("Speed-up cache", sp.cache), ("Free", sp.free)]
            for (i, (n, v)) in names.enumerated() { legend[i].stringValue = "● \(n) \(fmtBytes(v))" }
            legend[0].textColor = col
            legend[1].textColor = .systemTeal
            legend[2].textColor = .tertiaryLabelColor
            legend[3].stringValue = ""
            sparkCaption.stringValue = "Apps and speed-up cache · last 10 minutes"
            spark.values = snap.historyApps
            spark.under = zip(snap.historyApps, snap.historyCache).map { $0 + $1 }
            spark.underColor = .systemTeal
        } else {
            subtitle.stringValue = r.total > 0 ? "\(fmtBytes(r.used)) of \(fmtBytes(r.total)) used" : "Reading memory…"
            line3.stringValue = "Kernel: \(r.kernelLevelName) · Swap \(fmtBytes(r.swapUsed)) · \(String(format: "%.1f", r.compressionRatio))× compression"
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
            sparkCaption.stringValue = "Memory used · last 10 minutes"
            spark.values = snap.history
            spark.under = []
        }
        // Each action as a dot on the time axis, so its effect on the curve is visible.
        let window = UInt64(max(0, snap.history.count - 1)) * 5_000
        spark.markers = snap.activity.filter { !$0.alert }.compactMap {
            Presentation.markerPos(ageMs: snap.tMs &- $0.tMs, windowMs: window)
        }
        cacheNote.isHidden = !(friendly && !settings.explained)
        spark.color = col
        idleCaption.stringValue = friendly
            ? "Idle apps holding real memory — quit to free it"
            : "Idle apps holding a lot of memory"
        explainerButton.isHidden = !(friendly && !settings.explained)
        if case let .available(o) = Updater.shared.status {
            updateButton.attributedTitle = linkTitle("MemManager \(o.version) is available · Install →")
            updateButton.isHidden = false
        } else {
            updateButton.isHidden = true
        }
        pauseButton.image = NSImage(systemSymbolName: snap.paused ? "play.fill" : "pause.fill",
                                    accessibilityDescription: snap.paused ? "Resume" : "Pause for 1 hour")
        pauseButton.toolTip = snap.paused ? "Resume automatic actions" : "Pause automatic actions for 1 hour"

        rebuild(appsStack, rows: snap.apps.prefix(5).map { $0 }, idle: false)
        idleCaption.isHidden = snap.idleHeavy.isEmpty
        idleStack.isHidden = snap.idleHeavy.isEmpty
        rebuild(idleStack, rows: snap.idleHeavy.prefix(3).map { $0 }, idle: true)

        action1.stringValue = "Now: " + Presentation.nowLine(paused: snap.paused, acting: snap.acting, state: snap.state)
        if let a = snap.activity.first {
            action2.stringValue = "Last: \(a.title) · \(fmtAgo(snap.tMs &- a.tMs))"
            action2.textColor = a.alert ? .systemOrange : toneColor(
                Presentation.resultText(freed: a.freed, refaultRatio: a.refaultRatio, pending: a.pending).1)
        } else {
            action2.stringValue = friendly && snap.state == .normal
                ? "Nothing to clean: \(fmtBytes(sp.ready)) ready for apps" : "No actions yet"
            action2.textColor = .tertiaryLabelColor
        }
        nudgeButton.isEnabled = helperInstalled
        nudgeButton.toolTip = helperInstalled
            ? "Ask apps to release their caches and empty purgeable memory"
            : "Install the helper in Settings to enable Nudge"
        if let i = Profile.allCases.firstIndex(of: settings.profile) { profile.selectedSegment = i }
    }

    private func toneColor(_ t: Tone) -> NSColor {
        switch t {
        case .good: return .systemGreen
        case .warn: return .systemOrange
        case .neutral: return .tertiaryLabelColor
        }
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
