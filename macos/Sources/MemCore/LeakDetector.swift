import Foundation

/// Theil–Sen + Mann–Kendall leak and runaway detector (policy-engine §7).
public struct LeakResult: Equatable, Sendable {
    public var eligible = false
    /// Theil–Sen slope, MiB per hour.
    public var slope = 0.0
    public var tau = 0.0
    public var z = 0.0
    public var plateau = false
    public var leak = false
    public var runaway = false
    public init() {}
}

public enum LeakDetector {
    public static let maxPoints = 64
    public static let minSamples = 40
    public static let minSpanHours = 0.5
    public static let heavyMultiplier = 2.0
    public static let runawayMiBPerHour = 1024.0 * 60.0
    static let runawayWindowHours = 2.0 / 60.0
    static let runawayMinSamples = 5

    static func median(_ xs: [Double]) -> Double {
        if xs.isEmpty { return 0 }
        let s = xs.sorted()
        let mid = s.count / 2
        return s.count % 2 == 1 ? s[mid] : (s[mid - 1] + s[mid]) / 2.0
    }

    static func theilSen(_ pts: ArraySlice<(Double, Double)>) -> Double {
        let a = Array(pts)
        var slopes: [Double] = []
        slopes.reserveCapacity(a.count * (a.count - 1) / 2)
        for i in 0..<a.count {
            for j in (i + 1)..<a.count where a[j].0 > a[i].0 {
                slopes.append((a[j].1 - a[i].1) / (a[j].0 - a[i].0))
            }
        }
        return median(slopes)
    }

    static func mannKendall(_ pts: [(Double, Double)]) -> (tau: Double, z: Double) {
        let n = pts.count
        var s = 0
        for i in 0..<n {
            for j in (i + 1)..<n {
                let d = pts[j].1 - pts[i].1
                s += (d > 0 ? 1 : 0) - (d < 0 ? 1 : 0)
            }
        }
        let ys = pts.map { $0.1 }.sorted()
        var tieTerm = 0.0
        var i = 0
        while i < n {
            var j = i + 1
            while j < n && ys[j] == ys[i] { j += 1 }
            let t = Double(j - i)
            if t > 1 { tieTerm += t * (t - 1) * (2 * t + 5) }
            i = j
        }
        let nf = Double(n)
        let variance = (nf * (nf - 1) * (2 * nf + 5) - tieTerm) / 18.0
        let sf = Double(s)
        let z: Double
        if s > 0 { z = (sf - 1) / variance.squareRoot() }
        else if s < 0 { z = (sf + 1) / variance.squareRoot() }
        else { z = 0 }
        return (sf / (nf * (nf - 1) / 2.0), z)
    }

    static func decimate(_ series: [(Double, Double)]) -> [(Double, Double)] {
        let n = series.count
        if n <= maxPoints { return series }
        let k = (n + maxPoints - 1) / maxPoints
        var idx: [Int] = []
        var i = n - 1
        while i >= 0 {
            idx.append(i)
            i -= k
        }
        return idx.reversed().map { series[$0] }
    }

    /// Analyses one process series `(t_hours, MiB)`, oldest first.
    public static func analyze(_ series: [(Double, Double)], thresholds th: LeakThresholds,
                               heavy: Bool, processAgeOK: Bool = true) -> LeakResult {
        var out = LeakResult()
        let mult = heavy ? heavyMultiplier : 1.0

        if let last = series.last {
            let from = max(0, series.count - maxPoints)
            let tail = series[from...].filter { $0.0 >= last.0 - runawayWindowHours }
            if tail.count >= runawayMinSamples {
                out.runaway = theilSen(tail[...]) >= runawayMiBPerHour
            }
        }

        let nRaw = series.count
        guard nRaw >= minSamples, series[nRaw - 1].0 - series[0].0 >= minSpanHours, processAgeOK else {
            return out
        }
        out.eligible = true
        let d = decimate(series)
        let n = d.count
        let slope = theilSen(d[...])
        let mk = mannKendall(d)
        let m = min(n, max(8, (n + 3) / 4))
        let sTail = theilSen(d[(n - m)...])
        let plateau = !(sTail >= 0.5 * slope)
        let growth = d[n - 1].1 - d[0].1
        let baseline = d[0].1
        out.slope = slope
        out.tau = mk.tau
        out.z = mk.z
        out.plateau = plateau
        out.leak = slope >= th.slopeMiBPerHour * mult
            && growth >= max(th.absMiB * mult, th.rel * baseline)
            && mk.tau >= 0.6 && mk.z > 2.33 && !plateau
        return out
    }

    /// Hours until `headroomMiB` is consumed at `slope` MiB/h.
    public static func etaHours(headroomMiB: Double, slope: Double) -> Double? {
        guard slope > 0, headroomMiB.isFinite else { return nil }
        return max(0, headroomMiB) / slope
    }
}
