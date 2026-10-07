import Foundation

/// Pressure states (policy-engine §4).
public enum PressureState: Int, Comparable, CaseIterable, Sendable {
    case normal = 0, elevated, high, critical

    public static func < (a: PressureState, b: PressureState) -> Bool { a.rawValue < b.rawValue }

    public var name: String {
        switch self {
        case .normal: return "Normal"
        case .elevated: return "Elevated"
        case .high: return "High"
        case .critical: return "Critical"
        }
    }

    public var enterThreshold: Double { StateMachine.enter[rawValue] }
}

/// EWMA smoothing (τ = 20 s) and the hysteresis state machine.
public struct StateMachine {
    static let enter: [Double] = [0.0, 0.40, 0.65, 0.85]
    static let exit: [Double] = [0.0, 0.30, 0.55, 0.75]
    static let dwellS: [Double] = [0.0, 30.0, 30.0, 60.0]
    public static let tauS = 20.0
    public static let pEvent = 0.9

    private var s: Double?
    private var tMs: UInt64 = 0
    public private(set) var state: PressureState = .normal
    private var enteredMs: UInt64 = 0

    public init() {}

    public var smoothed: Double { s ?? 0 }

    @discardableResult
    public mutating func step(tMs t: UInt64, p: Double, event: Bool = false) -> PressureState {
        var v: Double
        if let prev = s {
            let dt = Double(t >= tMs ? t - tMs : 0) / 1000.0
            let alpha = 1.0 - exp(-dt / StateMachine.tauS)
            v = prev + alpha * (p - prev)
        } else {
            v = p
        }
        if event { v = max(v, StateMachine.pEvent) }
        s = v
        tMs = t

        let cur = state.rawValue
        var target = cur
        if cur < 3 {
            for i in stride(from: 3, to: cur, by: -1) where v >= StateMachine.enter[i] {
                target = i
                break
            }
        }
        if target > cur {
            state = PressureState(rawValue: target)!
            enteredMs = t
        } else if cur > 0, v < StateMachine.exit[cur],
                  Double(t >= enteredMs ? t - enteredMs : 0) / 1000.0 >= StateMachine.dwellS[cur] {
            state = PressureState(rawValue: cur - 1)!
            enteredMs = t
        }
        return state
    }
}
