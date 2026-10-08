import AppKit
import MemCore

func label(_ text: String = "", size: CGFloat = 13, weight: NSFont.Weight = .regular, color: NSColor = .labelColor) -> NSTextField {
    let l = NSTextField(labelWithString: text)
    l.font = .systemFont(ofSize: size, weight: weight)
    l.textColor = color
    l.lineBreakMode = .byTruncatingTail
    l.maximumNumberOfLines = 1
    l.translatesAutoresizingMaskIntoConstraints = false
    return l
}

/// Circular pressure gauge with the value in the middle.
final class GaugeView: NSView {
    var fraction: Double = 0 { didSet { needsDisplay = true } }
    var color: NSColor = .controlAccentColor { didSet { needsDisplay = true } }
    var text = "" { didSet { needsDisplay = true } }

    override var intrinsicContentSize: NSSize { NSSize(width: 54, height: 54) }

    override func draw(_ dirtyRect: NSRect) {
        let lw: CGFloat = 6
        let r = bounds.insetBy(dx: lw / 2 + 1, dy: lw / 2 + 1)
        let track = NSBezierPath(ovalIn: r)
        track.lineWidth = lw
        NSColor.tertiaryLabelColor.withAlphaComponent(0.35).setStroke()
        track.stroke()
        let f = CGFloat(min(1, max(0, fraction)))
        if f > 0.001 {
            let arc = NSBezierPath()
            arc.appendArc(withCenter: NSPoint(x: r.midX, y: r.midY), radius: r.width / 2, startAngle: 90,
                          endAngle: 90 - 360 * f, clockwise: true)
            arc.lineWidth = lw
            arc.lineCapStyle = .round
            color.setStroke()
            arc.stroke()
        }
        let attrs: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 13, weight: .semibold),
            .foregroundColor: NSColor.labelColor,
        ]
        let s = NSAttributedString(string: text, attributes: attrs)
        let sz = s.size()
        s.draw(at: NSPoint(x: bounds.midX - sz.width / 2, y: bounds.midY - sz.height / 2))
    }
}

/// Segmented bar: App / Wired / Compressed / Cached, matching Activity Monitor.
final class BreakdownView: NSView {
    struct Part { var value: UInt64; var color: NSColor; var name: String }
    var parts: [Part] = [] { didSet { needsDisplay = true } }
    var total: UInt64 = 1 { didSet { needsDisplay = true } }

    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: 8) }

    override func draw(_ dirtyRect: NSRect) {
        let r = bounds
        NSColor.tertiaryLabelColor.withAlphaComponent(0.25).setFill()
        NSBezierPath(roundedRect: r, xRadius: 4, yRadius: 4).fill()
        var x = r.minX
        for p in parts {
            let w = r.width * CGFloat(Double(p.value) / Double(max(total, 1)))
            if w >= 1 {
                p.color.setFill()
                NSBezierPath(roundedRect: NSRect(x: x, y: r.minY, width: max(1, w - 2), height: r.height), xRadius: 4, yRadius: 4).fill()
            }
            x += w
        }
    }
}

final class SparklineView: NSView {
    var values: [Double] = [] { didSet { needsDisplay = true } }
    var color: NSColor = .controlAccentColor { didSet { needsDisplay = true } }
    /// Optional series drawn first (e.g. apps + cache stacked behind apps).
    var under: [Double] = [] { didSet { needsDisplay = true } }
    var underColor: NSColor = .systemTeal { didSet { needsDisplay = true } }

    override var intrinsicContentSize: NSSize { NSSize(width: NSView.noIntrinsicMetric, height: 38) }

    override func draw(_ dirtyRect: NSRect) {
        let r = bounds.insetBy(dx: 0, dy: 2)
        NSColor.separatorColor.setStroke()
        let base = NSBezierPath()
        base.move(to: NSPoint(x: r.minX, y: r.minY))
        base.line(to: NSPoint(x: r.maxX, y: r.minY))
        base.lineWidth = 1
        base.stroke()
        if under.count >= 2 { series(under, color: underColor, alpha: 0.22, in: r) }
        if values.count >= 2 { series(values, color: color, alpha: under.isEmpty ? 0.15 : 0.3, in: r) }
    }

    private func series(_ vals: [Double], color: NSColor, alpha: CGFloat, in r: NSRect) {
        let path = NSBezierPath()
        for (i, v) in vals.enumerated() {
            let p = NSPoint(x: r.minX + r.width * CGFloat(i) / CGFloat(vals.count - 1),
                            y: r.minY + r.height * CGFloat(min(1, max(0, v))))
            if i == 0 { path.move(to: p) } else { path.line(to: p) }
        }
        let fill = path.copy() as! NSBezierPath
        fill.line(to: NSPoint(x: r.maxX, y: r.minY))
        fill.line(to: NSPoint(x: r.minX, y: r.minY))
        fill.close()
        color.withAlphaComponent(alpha).setFill()
        fill.fill()
        path.lineWidth = 1.5
        path.lineJoinStyle = .round
        color.setStroke()
        path.stroke()
    }
}

/// A wrapping caption (up to `lines` lines).
func caption(_ text: String = "", lines: Int = 2, color: NSColor = .tertiaryLabelColor) -> NSTextField {
    let l = NSTextField(wrappingLabelWithString: text)
    l.font = .systemFont(ofSize: 11)
    l.textColor = color
    l.maximumNumberOfLines = lines
    l.isSelectable = false
    l.translatesAutoresizingMaskIntoConstraints = false
    return l
}

/// Small rounded "chip" label.
final class ChipView: NSView {
    private let text = label(size: 11, weight: .semibold)
    var color: NSColor = .controlAccentColor { didSet { update() } }
    var title = "" { didSet { update() } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.cornerRadius = 9
        addSubview(text)
        NSLayoutConstraint.activate([
            text.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 7),
            text.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -7),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            heightAnchor.constraint(equalToConstant: 18),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    private func update() {
        text.stringValue = title
        text.textColor = color
        layer?.backgroundColor = color.withAlphaComponent(0.16).cgColor
    }
}

/// One app row: name, note, size; click shows a menu.
final class AppRowView: NSView {
    let name = label(size: 13)
    let note = label(size: 11, color: .secondaryLabelColor)
    let size = label(size: 13, weight: .semibold)
    var onClick: ((NSView) -> Void)?
    private var hover = false { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        note.alignment = .right
        size.alignment = .right
        for v in [name, note, size] { addSubview(v) }
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 28),
            name.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            name.centerYAnchor.constraint(equalTo: centerYAnchor),
            name.widthAnchor.constraint(lessThanOrEqualToConstant: 150),
            size.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            size.centerYAnchor.constraint(equalTo: centerYAnchor),
            size.widthAnchor.constraint(equalToConstant: 70),
            note.trailingAnchor.constraint(equalTo: size.leadingAnchor, constant: -6),
            note.leadingAnchor.constraint(greaterThanOrEqualTo: name.trailingAnchor, constant: 6),
            note.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self, userInfo: nil))
        setAccessibilityRole(.button)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func mouseEntered(with event: NSEvent) { hover = true }
    override func mouseExited(with event: NSEvent) { hover = false }
    override func mouseUp(with event: NSEvent) { onClick?(self) }
    override func accessibilityPerformPress() -> Bool { onClick?(self); return true }

    override func draw(_ dirtyRect: NSRect) {
        if hover {
            NSColor.labelColor.withAlphaComponent(0.07).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill()
        }
    }
}
