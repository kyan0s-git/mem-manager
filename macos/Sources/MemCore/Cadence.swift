/// Sampling cadence (policy-engine §8).
public struct Cadence: Equatable, Sendable {
    public var systemMs: UInt64
    public var processMs: UInt64

    /// `nil` = paused (only kernel events wake the sampler).
    public static func forState(popupVisible: Bool, state: PressureState, displayOff: Bool, onBattery: Bool) -> Cadence? {
        if displayOff && !popupVisible { return nil }
        var c: Cadence
        if popupVisible {
            c = Cadence(systemMs: 1_000, processMs: 2_000)
        } else if state >= .elevated {
            c = Cadence(systemMs: 5_000, processMs: 15_000)
        } else {
            c = Cadence(systemMs: 10_000, processMs: 30_000)
        }
        if onBattery && !popupVisible {
            c.systemMs *= 2
            c.processMs *= 2
        }
        return c
    }
}
