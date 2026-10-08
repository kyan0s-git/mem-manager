import AppKit
import Combine
import Foundation
import MemCore

enum IconStyle: String, CaseIterable, Identifiable {
    case ring, bar, percent, sparkline, dot
    var id: String { rawValue }
    var label: String {
        switch self {
        case .ring: return "Ring"
        case .bar: return "Bar"
        case .percent: return "Percent"
        case .sparkline: return "Graph"
        case .dot: return "Dot"
        }
    }
}

enum IconMetric: String, CaseIterable, Identifiable {
    case pressure, ready, used, compressed
    var id: String { rawValue }
    func label(friendly: Bool) -> String {
        switch self {
        case .pressure: return "Pressure"
        case .ready: return friendly ? "Ready for apps" : "Free + cached"
        case .used: return friendly ? "Apps & macOS" : "Memory used"
        case .compressed: return "Compressed"
        }
    }
}

enum ColorPreset: String, CaseIterable, Identifiable {
    case system, traffic, mono, colorblind, custom
    var id: String { rawValue }
    var label: String {
        switch self {
        case .system: return "System"
        case .traffic: return "Traffic light"
        case .mono: return "Monochrome"
        case .colorblind: return "Color-blind safe"
        case .custom: return "Custom"
        }
    }
}

/// Immutable copy of the settings the engine needs.
struct EngineSettings {
    var profile: Profile = .balanced
    var autoNudge = false
    var notifyLeaks = true
    var notifySwap = true
    var heavy: Set<String> = []
    var ignoreLeaks: Set<String> = []
    var autoQuit: Set<String> = []

    func isHeavy(_ name: String) -> Bool { heavy.contains(name.lowercased()) }
    func ignores(_ key: String) -> Bool { ignoreLeaks.contains(key.lowercased()) }
}

/// User settings persisted in UserDefaults; observable by the SwiftUI settings window.
final class AppSettings: ObservableObject {
    static let shared = AppSettings()
    private let d = UserDefaults.standard
    private var loading = true
    var onChange: (() -> Void)?

    @Published var profile: Profile = .balanced { didSet { save() } }
    @Published var iconStyle: IconStyle = .ring { didSet { save() } }
    @Published var iconMetric: IconMetric = .pressure { didSet { save() } }
    @Published var preset: ColorPreset = .system { didSet { save() } }
    @Published var customColors: [NSColor] = [.systemBlue, .systemYellow, .systemOrange, .systemRed] { didSet { save() } }
    @Published var autoNudge = false { didSet { save() } }
    @Published var notifyLeaks = true { didSet { save() } }
    @Published var notifySwap = true { didSet { save() } }
    @Published var ignoreLeaks: [String] = [] { didSet { save() } }
    @Published var autoQuit: [String] = [] { didSet { save() } }
    @Published var heavy: [String] = AppSettings.defaultHeavy { didSet { save() } }
    /// Friendly values (default): "ready for apps", cache shown as a speed-up, calm
    /// state names. Off: Activity Monitor's terms (Memory Used, Cached Files, …).
    @Published var friendlyValues = true { didSet { save() } }
    /// The one-time "why cache counts as ready" note has been seen.
    @Published var explained = false { didSet { save() } }
    /// Check GitHub for a newer release once a day.
    @Published var autoUpdate = true { didSet { save() } }

    static let defaultHeavy = [
        "safari", "google chrome", "firefox", "arc", "microsoft edge", "brave browser", "opera", "vivaldi",
        "xcode", "code", "visual studio code", "cursor", "intellij idea", "pycharm", "android studio",
        "docker", "docker desktop", "utm", "parallels desktop", "vmware fusion", "final cut pro", "logic pro",
        "adobe photoshop 2025", "blender", "ollama", "lm studio", "python", "simulator",
    ]

    private init() {
        if let v = d.string(forKey: "profile"), let p = Profile(rawValue: v) { profile = p }
        if let v = d.string(forKey: "iconStyle"), let st = IconStyle(rawValue: v) { iconStyle = st }
        if let v = d.string(forKey: "iconMetric"), let m = IconMetric(rawValue: v) { iconMetric = m }
        if let v = d.string(forKey: "preset"), let p = ColorPreset(rawValue: v) { preset = p }
        if let hexes = d.array(forKey: "customColors") as? [String], hexes.count == 4 {
            customColors = hexes.map { NSColor(hex: $0) ?? .systemBlue }
        }
        if d.object(forKey: "autoNudge") != nil { autoNudge = d.bool(forKey: "autoNudge") }
        if d.object(forKey: "notifyLeaks") != nil { notifyLeaks = d.bool(forKey: "notifyLeaks") }
        if d.object(forKey: "notifySwap") != nil { notifySwap = d.bool(forKey: "notifySwap") }
        ignoreLeaks = d.stringArray(forKey: "ignoreLeaks") ?? []
        autoQuit = d.stringArray(forKey: "autoQuit") ?? []
        heavy = d.stringArray(forKey: "heavy") ?? AppSettings.defaultHeavy
        if d.object(forKey: "friendlyValues") != nil { friendlyValues = d.bool(forKey: "friendlyValues") }
        explained = d.bool(forKey: "explained")
        if d.object(forKey: "autoUpdate") != nil { autoUpdate = d.bool(forKey: "autoUpdate") }
        loading = false
    }

    private func save() {
        guard !loading else { return }
        d.set(profile.rawValue, forKey: "profile")
        d.set(iconStyle.rawValue, forKey: "iconStyle")
        d.set(iconMetric.rawValue, forKey: "iconMetric")
        d.set(preset.rawValue, forKey: "preset")
        d.set(customColors.map(\.hexString), forKey: "customColors")
        d.set(autoNudge, forKey: "autoNudge")
        d.set(notifyLeaks, forKey: "notifyLeaks")
        d.set(notifySwap, forKey: "notifySwap")
        d.set(ignoreLeaks, forKey: "ignoreLeaks")
        d.set(autoQuit, forKey: "autoQuit")
        d.set(heavy, forKey: "heavy")
        d.set(friendlyValues, forKey: "friendlyValues")
        d.set(explained, forKey: "explained")
        d.set(autoUpdate, forKey: "autoUpdate")
        onChange?()
    }

    var engine: EngineSettings {
        EngineSettings(
            profile: profile,
            autoNudge: autoNudge,
            notifyLeaks: notifyLeaks,
            notifySwap: notifySwap,
            heavy: Set(heavy.map { $0.lowercased() }),
            ignoreLeaks: Set(ignoreLeaks.map { $0.lowercased() }),
            autoQuit: Set(autoQuit.map { $0.lowercased() })
        )
    }

    func toggleIgnore(_ key: String) {
        let k = key.lowercased()
        if let i = ignoreLeaks.firstIndex(of: k) { ignoreLeaks.remove(at: i) } else { ignoreLeaks.append(k) }
    }
}

extension NSColor {
    convenience init?(hex: String) {
        let s = hex.trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        guard s.count == 6, let v = UInt32(s, radix: 16) else { return nil }
        self.init(srgbRed: CGFloat((v >> 16) & 0xFF) / 255, green: CGFloat((v >> 8) & 0xFF) / 255,
                  blue: CGFloat(v & 0xFF) / 255, alpha: 1)
    }

    var hexString: String {
        let c = usingColorSpace(.sRGB) ?? self
        let r = Int((c.redComponent * 255).rounded())
        let g = Int((c.greenComponent * 255).rounded())
        let b = Int((c.blueComponent * 255).rounded())
        return String(format: "#%02X%02X%02X", r, g, b)
    }
}
