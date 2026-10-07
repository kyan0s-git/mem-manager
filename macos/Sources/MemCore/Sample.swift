/// System sample (policy-engine §2) and the macOS pressure score (§3).
public struct Sample: Equatable, Sendable {
    public var tMs: UInt64 = 0
    public var pageSize: UInt64 = 16384
    public var totalBytes: UInt64 = 0
    /// macOS: free (incl. speculative).
    public var fastAvailBytes: UInt64 = 0
    /// macOS: external + purgeable.
    public var cacheBytes: UInt64 = 0
    /// macOS: decompressions (cumulative).
    public var churnPagesCum: UInt64 = 0
    /// macOS: pageins (cumulative).
    public var hardFaultsCum: UInt64 = 0
    /// 1 | 2 | 4 from kern.memorystatus_vm_pressure_level.
    public var kernelLevel: Int = 1
    /// kern.memorystatus_level.
    public var freePct: Int = 100
    public var swapUsedBytes: UInt64 = 0

    public init() {}
}

@inline(__always) func clamp01(_ x: Double) -> Double { min(1, max(0, x)) }

/// Floor contributed by the kernel level (dispatch values 1/2/4).
public func macLevelFloor(_ level: Int) -> Double {
    switch level {
    case 4: return 0.90
    case 2: return 0.60
    default: return 0.0
    }
}

public func pressureMacOS(_ cur: Sample, prev: Sample?) -> Double {
    let floor = macLevelFloor(cur.kernelLevel)
    let avail = clamp01((60.0 - Double(cur.freePct)) / 60.0)
    var swapGrowth = 0.0
    if let prev, cur.tMs > prev.tMs {
        let dt = Double(cur.tMs - prev.tMs) / 1000.0
        let grew = cur.swapUsedBytes > prev.swapUsedBytes ? Double(cur.swapUsedBytes - prev.swapUsedBytes) : 0
        swapGrowth = clamp01(grew / dt / (64.0 * MemCore.mib))
    }
    return max(floor, 0.8 * avail, swapGrowth)
}
