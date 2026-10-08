import Foundation
import MemCore
import MemSys

/// One system reading, with Activity-Monitor-consistent figures
/// (docs/research/macos-memory-internals.md §5).
struct MacReading: Equatable {
    var sample = Sample()
    var total: UInt64 = 0
    var used: UInt64 = 0
    var app: UInt64 = 0
    var wired: UInt64 = 0
    var compressed: UInt64 = 0
    var cached: UInt64 = 0
    var free: UInt64 = 0
    var swapTotal: UInt64 = 0
    var swapUsed: UInt64 = 0
    /// Dispatch values: 1 normal, 2 warning, 4 critical.
    var kernelLevel: Int = 1
    var freePct: Int = 100
    var compressionRatio: Double = 1
    var pageinRate: Double = 0
    var swapoutRate: Double = 0

    var usedFraction: Double { total > 0 ? Double(used) / Double(total) : 0 }
    /// Apps & macOS / speed-up cache (Cached Files) / free.
    var split: MemorySplit { MemorySplit(total: total, used: used, cached: cached) }
    var kernelLevelName: String {
        switch kernelLevel {
        case 4: return "Critical"
        case 2: return "Warning"
        default: return "Normal"
        }
    }
}

final class MacSampler {
    private var last: (t: UInt64, s: mm_vmstats)?
    let total = mm_memsize()

    func sample(tMs: UInt64) -> MacReading? {
        var s = mm_vmstats()
        guard mm_vm_stats(&s) == 0 else { return nil }
        let page = s.page_size
        var r = MacReading()
        r.total = total
        let active = s.active_count * page
        let inactive = s.inactive_count * page
        let speculative = s.speculative_count * page
        r.wired = s.wire_count * page
        r.compressed = s.compressor_page_count * page
        let purgeable = s.purgeable_count * page
        let external = s.external_page_count * page
        let sum = active + inactive + speculative + r.wired + r.compressed
        let notUsed = purgeable + external
        r.used = sum > notUsed ? sum - notUsed : 0
        r.app = r.used > r.wired + r.compressed ? r.used - r.wired - r.compressed : 0
        r.cached = purgeable + external
        r.free = total > r.used ? total - r.used : 0
        if s.compressor_page_count > 0 {
            r.compressionRatio = Double(s.total_uncompressed_pages_in_compressor) / Double(s.compressor_page_count)
        }
        var level: Int32 = 1
        if mm_sysctl_int("kern.memorystatus_vm_pressure_level", &level) == 0 { r.kernelLevel = Int(level) }
        var pct: Int32 = 100
        if mm_sysctl_int("kern.memorystatus_level", &pct) == 0 { r.freePct = Int(pct) }
        var st: UInt64 = 0, su: UInt64 = 0
        if mm_swap_usage(&st, &su) == 0 {
            r.swapTotal = st
            r.swapUsed = su
        }
        if let l = last, tMs > l.t {
            let dt = Double(tMs - l.t) / 1000
            r.pageinRate = Double(s.pageins &- l.s.pageins) / dt
            r.swapoutRate = Double(s.swapouts &- l.s.swapouts) / dt
        }
        last = (tMs, s)

        var smp = Sample()
        smp.tMs = tMs
        smp.pageSize = page
        smp.totalBytes = total
        smp.fastAvailBytes = s.free_count * page
        smp.cacheBytes = r.cached
        smp.churnPagesCum = s.decompressions
        smp.hardFaultsCum = s.pageins
        smp.kernelLevel = r.kernelLevel
        smp.freePct = r.freePct
        smp.swapUsedBytes = r.swapUsed
        r.sample = smp
        return r
    }
}

/// Milliseconds on a monotonic clock.
func monotonicMs() -> UInt64 {
    DispatchTime.now().uptimeNanoseconds / 1_000_000
}

func fmtBytes(_ b: UInt64) -> String { Presentation.bytes(b) }

func fmtAgo(_ ms: UInt64) -> String {
    let s = ms / 1000
    if s < 45 { return "just now" }
    if s < 3600 { return "\((s + 30) / 60) min ago" }
    if s < 86_400 { return "\((s + 1800) / 3600) h ago" }
    return "\(s / 86_400) d ago"
}

func fmtHours(_ h: Double) -> String {
    if h < 1 { return "\(max(1, Int((h * 60).rounded()))) min" }
    if h < 48 { return "\(Int(h.rounded())) h" }
    return "\(Int((h / 24).rounded())) days"
}
