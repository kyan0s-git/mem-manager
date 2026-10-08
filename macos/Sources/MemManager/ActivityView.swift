import AppKit
import MemCore

/// The popover's activity page: what MemManager did (or warned about), why,
/// to which apps, and what each action measurably changed. Built when opened,
/// released when closed; entry views are rebuilt only when the log changes.
final class ActivityPageView: NSView {
    private let onBack: () -> Void
    private let back = NSButton(title: "Activity", target: nil, action: nil)
    private let summary = label(size: 13, weight: .semibold)
    private let room = caption(color: .secondaryLabelColor)
    /// What MemManager can actually do on this Mac (macOS manages memory itself).
    private let can = caption(lines: 4, color: .tertiaryLabelColor)
    private let empty = caption("Nothing yet. MemManager only steps in when memory gets tight, and every step it takes shows up here with the reason and its measured effect.",
                                lines: 4)
    private let list = NSStackView()
    private var shown: [ActivityRow] = []
    private var agoLabels: [NSTextField] = []

    init(onBack: @escaping () -> Void) {
        self.onBack = onBack
        super.init(frame: NSRect(x: 0, y: 0, width: 340, height: 480))
        back.image = NSImage(systemSymbolName: "chevron.left", accessibilityDescription: "Back")
        back.imagePosition = .imageLeading
        back.isBordered = false
        back.font = .systemFont(ofSize: 15, weight: .semibold)
        back.target = self
        back.action = #selector(goBack)
        back.keyEquivalent = "\u{1b}"

        list.orientation = .vertical
        list.alignment = .leading
        list.spacing = 12
        list.translatesAutoresizingMaskIntoConstraints = false
        let doc = FlippedView()
        doc.translatesAutoresizingMaskIntoConstraints = false
        doc.addSubview(list)
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = doc
        scroll.translatesAutoresizingMaskIntoConstraints = false
        for c in [room, empty, can] { c.preferredMaxLayoutWidth = 312 }

        let sep = NSBox()
        sep.boxType = .separator
        let top = NSStackView(views: [back, summary, room, can, sep])
        top.orientation = .vertical
        top.alignment = .leading
        top.spacing = 6
        top.setCustomSpacing(10, after: back)
        top.translatesAutoresizingMaskIntoConstraints = false
        addSubview(top)
        addSubview(scroll)
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: 340),
            heightAnchor.constraint(equalToConstant: 480),
            top.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            top.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            top.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -14),
            sep.widthAnchor.constraint(equalTo: top.widthAnchor),
            scroll.topAnchor.constraint(equalTo: top.bottomAnchor, constant: 8),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
            doc.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            list.topAnchor.constraint(equalTo: doc.topAnchor),
            list.leadingAnchor.constraint(equalTo: doc.leadingAnchor),
            list.trailingAnchor.constraint(equalTo: doc.trailingAnchor, constant: -6),
            list.bottomAnchor.constraint(equalTo: doc.bottomAnchor),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    @objc private func goBack() { onBack() }

    func render(_ snap: Snapshot, helperInstalled: Bool) {
        can.stringValue = helperInstalled
            ? "macOS manages memory itself. MemManager adds: Nudge (asks apps to drop caches they can rebuild), leak and swap alerts, and a one-click Quit for idle apps when memory stays tight."
            : "macOS manages memory itself, so MemManager doesn't override it. It watches, alerts about leaks and swap, and offers a one-click Quit for idle apps when memory stays tight. Nudge needs the helper (Settings), which needs a Developer-ID-signed build."
        let acts = snap.activity.filter(\.measured)
        let slow = acts.filter { !$0.pending && $0.refaultRatio >= 0.25 }.count
        summary.stringValue = Presentation.summary(actions: acts.count, freed: acts.reduce(UInt64(0)) { $0 + $1.freed },
                                                   slowdowns: slow)
        let window = UInt64(max(0, snap.historyApps.count - 1)) * 5_000
        if let fa = snap.historyApps.first, let la = snap.historyApps.last,
           let fc = snap.historyCache.first, let lc = snap.historyCache.last,
           let line = Presentation.roomLine(
               bytes: Presentation.roomMade(first: (fa, fc), last: (la, lc), total: snap.reading.total), windowMs: window) {
            room.stringValue = line
            room.isHidden = false
        } else {
            room.isHidden = true
        }
        if snap.activity != shown {
            shown = snap.activity
            rebuild()
        }
        for (l, a) in zip(agoLabels, shown) { l.stringValue = fmtAgo(snap.tMs &- a.tMs) }
    }

    private func rebuild() {
        for v in list.arrangedSubviews {
            list.removeArrangedSubview(v)
            v.removeFromSuperview()
        }
        agoLabels.removeAll(keepingCapacity: true)
        if shown.isEmpty {
            list.addArrangedSubview(empty)
            return
        }
        for a in shown {
            let dot = label("●", size: 9, color: a.alert ? .systemOrange : .controlAccentColor)
            let title = label(a.title, size: 13, weight: .semibold)
            title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            let ago = label(size: 11, color: .tertiaryLabelColor)
            ago.setContentHuggingPriority(.required, for: .horizontal)
            agoLabels.append(ago)
            let row = NSStackView(views: [dot, title, NSView(), ago])
            row.alignment = .firstBaseline
            row.spacing = 6
            var lines: [NSView] = [row]
            if !a.why.isEmpty {
                let why = caption((a.manual ? "" : "Automatic · ") + a.why, color: .secondaryLabelColor)
                why.preferredMaxLayoutWidth = 290
                lines.append(why)
            }
            var result = ""
            var color = NSColor.tertiaryLabelColor
            if a.measured {
                let (t, tone) = Presentation.resultText(freed: a.freed, refaultRatio: a.refaultRatio, pending: a.pending)
                result = t
                color = tone == .good ? .systemGreen : (tone == .warn ? .systemOrange : .tertiaryLabelColor)
            }
            let last = [a.detail, result].filter { !$0.isEmpty }.joined(separator: " · ")
            if !last.isEmpty {
                let l = caption(last, color: color)
                l.preferredMaxLayoutWidth = 290
                lines.append(l)
            }
            let entry = NSStackView(views: lines)
            entry.orientation = .vertical
            entry.alignment = .leading
            entry.spacing = 2
            list.addArrangedSubview(entry)
            entry.widthAnchor.constraint(equalTo: list.widthAnchor).isActive = true
            row.widthAnchor.constraint(equalTo: entry.widthAnchor).isActive = true
            entry.setAccessibilityElement(true)
            entry.setAccessibilityLabel([a.title, a.why, last].filter { !$0.isEmpty }.joined(separator: ". "))
        }
    }
}

/// Top-to-bottom document view for scroll views.
final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}
