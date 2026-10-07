import AppKit
import MemCore

/// Draws the menu bar image (docs/architecture/ui-design.md §1).
struct IconSpec: Equatable {
    var style: IconStyle
    var fraction: Double
    var text: String
    var state: PressureState
    var color: NSColor
    var template: Bool
    var paused: Bool
    var history: [Double]
    var height: CGFloat
}

enum IconRenderer {
    static func width(for style: IconStyle) -> CGFloat {
        switch style {
        case .ring: return 18
        case .bar: return 26
        case .percent: return 38
        case .sparkline: return 36
        case .dot: return 34
        }
    }

    static func image(_ spec: IconSpec) -> NSImage {
        let size = NSSize(width: width(for: spec.style), height: spec.height)
        let img = NSImage(size: size, flipped: false) { rect in
            draw(spec, in: rect)
            return true
        }
        img.isTemplate = spec.template
        return img
    }

    private static func draw(_ s: IconSpec, in rect: NSRect) {
        let fg = s.template ? NSColor.black : s.color
        let track = (s.template ? NSColor.black : NSColor.labelColor).withAlphaComponent(0.25)
        let f = CGFloat(min(1, max(0, s.fraction)))
        switch s.style {
        case .ring:
            let d: CGFloat = 15
            let r = NSRect(x: (rect.width - d) / 2, y: (rect.height - d) / 2, width: d, height: d)
            let lw: CGFloat = 2.6
            let inner = r.insetBy(dx: lw / 2, dy: lw / 2)
            let trackPath = NSBezierPath(ovalIn: inner)
            trackPath.lineWidth = lw
            if s.paused {
                let dash: [CGFloat] = [2, 2]
                trackPath.setLineDash(dash, count: 2, phase: 0)
            }
            track.setStroke()
            trackPath.stroke()
            if !s.paused && f > 0.001 {
                let c = NSPoint(x: inner.midX, y: inner.midY)
                let arc = NSBezierPath()
                let gap: CGFloat = s.state >= .high ? 12 : 0
                arc.appendArc(withCenter: c, radius: inner.width / 2, startAngle: 90 - gap,
                              endAngle: 90 - gap - 360 * f, clockwise: true)
                arc.lineWidth = lw
                arc.lineCapStyle = .round
                fg.setStroke()
                arc.stroke()
            }
            if s.state == .critical {
                fg.setFill()
                NSBezierPath(ovalIn: r.insetBy(dx: 5, dy: 5)).fill()
            }
        case .bar:
            let w: CGFloat = 22, h: CGFloat = 9
            let r = NSRect(x: (rect.width - w) / 2, y: (rect.height - h) / 2, width: w, height: h)
            let outline = NSBezierPath(roundedRect: r.insetBy(dx: 0.5, dy: 0.5), xRadius: 2.5, yRadius: 2.5)
            outline.lineWidth = 1
            (s.state >= .elevated ? fg : track).setStroke()
            outline.stroke()
            if !s.paused {
                let inner = r.insetBy(dx: 2, dy: 2)
                fg.setFill()
                NSBezierPath(roundedRect: NSRect(x: inner.minX, y: inner.minY, width: max(1, inner.width * f), height: inner.height),
                             xRadius: 1.2, yRadius: 1.2).fill()
            }
        case .percent, .dot:
            let font = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .semibold)
            let para = NSMutableParagraphStyle()
            para.alignment = .right
            let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: s.paused ? track : fg, .paragraphStyle: para]
            let text = NSAttributedString(string: s.text, attributes: attrs)
            let th = text.size().height
            var textRect = NSRect(x: 0, y: (rect.height - th) / 2, width: rect.width - 1, height: th)
            if s.style == .dot {
                let d: CGFloat = 7
                fg.setFill()
                NSBezierPath(ovalIn: NSRect(x: 2, y: (rect.height - d) / 2, width: d, height: d)).fill()
                textRect.origin.x = 11
                textRect.size.width = rect.width - 12
            }
            text.draw(in: textRect)
            if s.state >= .high && !s.paused && s.style == .percent {
                fg.setFill()
                let uh: CGFloat = s.state == .critical ? 2 : 1
                NSBezierPath(rect: NSRect(x: 4, y: 1.5, width: rect.width - 6, height: uh)).fill()
            }
        case .sparkline:
            let r = rect.insetBy(dx: 1.5, dy: 3.5)
            let vals = Array(s.history.suffix(30))
            track.setStroke()
            let base = NSBezierPath()
            base.move(to: NSPoint(x: r.minX, y: r.minY))
            base.line(to: NSPoint(x: r.maxX, y: r.minY))
            base.lineWidth = 0.8
            base.stroke()
            guard vals.count >= 2 else { return }
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
            fg.withAlphaComponent(0.25).setFill()
            fill.fill()
            path.lineWidth = 1.4
            path.lineJoinStyle = .round
            fg.setStroke()
            path.stroke()
        }
    }
}
