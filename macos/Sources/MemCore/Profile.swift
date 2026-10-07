/// Profile tables (policy-engine §6).
public enum Profile: String, CaseIterable, Sendable {
    case conservative, balanced, aggressive

    public var label: String {
        switch self {
        case .conservative: return "Conservative"
        case .balanced: return "Balanced"
        case .aggressive: return "Aggressive"
        }
    }

    public var leakThresholds: LeakThresholds {
        switch self {
        case .conservative: return LeakThresholds(slopeMiBPerHour: 200, absMiB: 512, rel: 0.40)
        case .balanced: return LeakThresholds(slopeMiBPerHour: 100, absMiB: 256, rel: 0.25)
        case .aggressive: return LeakThresholds(slopeMiBPerHour: 50, absMiB: 128, rel: 0.15)
        }
    }

    /// macOS: automatic Nudge at High (helper required).
    public var autoNudge: Bool { self == .aggressive }

    /// Idle-app suggestions: minimum hours since last activation.
    public var idleAppHours: Double {
        switch self {
        case .conservative: return 4
        case .balanced: return 2
        case .aggressive: return 1
        }
    }

    /// Idle-app suggestions: minimum footprint.
    public var idleAppMinBytes: UInt64 {
        switch self {
        case .conservative: return 1 << 30
        case .balanced: return 500 << 20
        case .aggressive: return 250 << 20
        }
    }
}

public struct LeakThresholds: Equatable, Sendable {
    public var slopeMiBPerHour: Double
    public var absMiB: Double
    public var rel: Double
    public init(slopeMiBPerHour: Double, absMiB: Double, rel: Double) {
        self.slopeMiBPerHour = slopeMiBPerHour
        self.absMiB = absMiB
        self.rel = rel
    }
}
