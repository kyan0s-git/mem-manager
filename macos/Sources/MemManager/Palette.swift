import AppKit
import MemCore

/// State colours (docs/architecture/ui-design.md §1.3), adapted to light/dark.
enum Palette {
    static func stateColor(_ state: PressureState, preset: ColorPreset, custom: [NSColor], dark: Bool) -> NSColor {
        func pick(_ light: UInt32, _ darkHex: UInt32) -> NSColor {
            let v = dark ? darkHex : light
            return NSColor(srgbRed: CGFloat((v >> 16) & 0xFF) / 255, green: CGFloat((v >> 8) & 0xFF) / 255,
                           blue: CGFloat(v & 0xFF) / 255, alpha: 1)
        }
        switch preset {
        case .mono:
            return dark ? .white : .black
        case .custom:
            return custom.indices.contains(state.rawValue) ? custom[state.rawValue] : .systemBlue
        case .colorblind:
            switch state {
            case .normal: return pick(0x0A6CD6, 0x56A8FF)
            case .elevated: return pick(0xB07800, 0xE69F00)
            case .high, .critical: return pick(0xC04E00, 0xFF7A33)
            }
        case .traffic:
            switch state {
            case .normal: return pick(0x107C10, 0x6CCB5F)
            case .elevated: return pick(0xB58400, 0xF2C744)
            case .high: return pick(0xC8551B, 0xFF8F4D)
            case .critical: return pick(0xC42B1C, 0xFF6B5E)
            }
        case .system:
            switch state {
            case .normal: return .controlAccentColor
            case .elevated: return pick(0xB58400, 0xF2C744)
            case .high: return pick(0xC8551B, 0xFF8F4D)
            case .critical: return pick(0xC42B1C, 0xFF6B5E)
            }
        }
    }
}
