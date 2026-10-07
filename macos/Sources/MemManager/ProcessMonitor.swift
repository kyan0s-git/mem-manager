import AppKit
import Foundation
import MemCore
import MemSys

/// Per-app memory (summed over the app's processes) with bounded leak
/// histories. Only processes of the current user are visible (proc_info
/// CHECK_SAME_USER), which covers every GUI app and its helpers.
struct AppGroup {
    let key: String
    var name: String
    var bundlePath: String?
    var pids: [Int32] = []
    var footprint: UInt64 = 0
    var prevFootprint: UInt64 = 0
    var cpuNs: UInt64 = 0
    var firstSeenMs: UInt64
    var seenGen = 0
    var hist: Ring<(Double, Double)>?
    var lastHistMs: UInt64 = 0
    var leak = LeakResult()
    var leakNotifiedMs: UInt64?
    var notifiedEtaH = Double.infinity
}

final class ProcessMonitor {
    static let histLen = 360       // 2 h at 20 s
    static let histSpacingMs: UInt64 = 20_000
    static let topK = 24
    static let warmupMs: UInt64 = 10 * 60_000

    private(set) var groups: [String: AppGroup] = [:]
    private var foreign = Set<Int32>()
    private var pidBuf = [Int32](repeating: 0, count: 4096)
    private var gen = 0
    private let epochMs: UInt64
    private let ownPid = getpid()
    private(set) var visibleProcesses = 0

    init(nowMs: UInt64) { epochMs = nowMs }

    static func groupKey(path: String, name: String) -> (key: String, display: String, bundle: String?) {
        if let r = path.range(of: ".app/") {
            let bundle = String(path[..<r.upperBound].dropLast())
            let display = (bundle as NSString).lastPathComponent.replacingOccurrences(of: ".app", with: "")
            return (bundle.lowercased(), display, bundle)
        }
        let n = name.isEmpty ? (path as NSString).lastPathComponent : name
        return (n.lowercased(), n, nil)
    }

    /// Refreshes all groups; returns the keys newly flagged as leaking/runaway.
    func update(nowMs: UInt64, settings: EngineSettings) -> [String] {
        let n = Int(mm_list_pids(&pidBuf, Int32(pidBuf.count)))
        guard n > 0 else { return [] }
        gen += 1
        var seen = Set<Int32>()
        var pathBuf = [CChar](repeating: 0, count: 4096)
        var nameBuf = [CChar](repeating: 0, count: 256)
        var visible = 0
        for i in 0..<min(n, pidBuf.count) {
            let pid = pidBuf[i]
            if pid <= 0 { continue }
            seen.insert(pid)
            if foreign.contains(pid) { continue }
            let ru = mm_proc_rusage(pid)
            if ru.ok == 0 {
                if ru.err == EPERM { foreign.insert(pid) }
                continue
            }
            visible += 1
            let plen = mm_proc_path(pid, &pathBuf, Int32(pathBuf.count))
            let path = plen > 0 ? String(cString: pathBuf) : ""
            let nlen = mm_proc_name(pid, &nameBuf, Int32(nameBuf.count))
            let name = nlen > 0 ? String(cString: nameBuf) : "pid \(pid)"
            let g = ProcessMonitor.groupKey(path: path, name: name)
            if groups[g.key] == nil {
                groups[g.key] = AppGroup(key: g.key, name: g.display, bundlePath: g.bundle, firstSeenMs: nowMs)
            }
            if groups[g.key]!.seenGen != gen {
                groups[g.key]!.seenGen = gen
                groups[g.key]!.prevFootprint = groups[g.key]!.footprint
                groups[g.key]!.footprint = 0
                groups[g.key]!.cpuNs = 0
                groups[g.key]!.pids.removeAll(keepingCapacity: true)
            }
            groups[g.key]!.footprint += ru.phys_footprint
            groups[g.key]!.cpuNs += ru.user_time_ns + ru.system_time_ns
            groups[g.key]!.pids.append(pid)
        }
        visibleProcesses = visible
        foreign = foreign.intersection(seen)
        groups = groups.filter { $0.value.seenGen == gen }
        return updateHistories(nowMs: nowMs, settings: settings)
    }

    private func updateHistories(nowMs: UInt64, settings: EngineSettings) -> [String] {
        var want = groups.values.sorted { $0.footprint > $1.footprint }.prefix(ProcessMonitor.topK).map(\.key)
        for g in groups.values where g.footprint > g.prevFootprint + (512 << 20) && !want.contains(g.key) {
            want.append(g.key)
        }
        let wantSet = Set(want)
        for k in groups.keys where !wantSet.contains(k) && groups[k]!.hist != nil {
            if groups.values.filter({ $0.hist != nil }).count > ProcessMonitor.topK + 8 {
                groups[k]!.hist = nil
                groups[k]!.leak = LeakResult()
            }
        }
        let tH = Double(nowMs &- epochMs) / 3_600_000
        let th = settings.profile.leakThresholds
        var flagged: [String] = []
        for k in want {
            guard var g = groups[k] else { continue }
            if nowMs &- g.firstSeenMs < ProcessMonitor.warmupMs && g.hist == nil { continue }
            if g.hist == nil { g.hist = Ring(capacity: ProcessMonitor.histLen) }
            if g.lastHistMs != 0 && nowMs &- g.lastHistMs < ProcessMonitor.histSpacingMs {
                groups[k] = g
                continue
            }
            g.lastHistMs = nowMs
            g.hist!.push((tH, Double(g.footprint) / MemCore.mib))
            let was = g.leak.leak || g.leak.runaway
            g.leak = LeakDetector.analyze(g.hist!.elements, thresholds: th, heavy: settings.isHeavy(g.name))
            if (g.leak.leak || g.leak.runaway) && !was { flagged.append(k) }
            groups[k] = g
        }
        return flagged
    }

    func markNotified(_ key: String, nowMs: UInt64, eta: Double?) {
        groups[key]?.leakNotifiedMs = nowMs
        groups[key]?.notifiedEtaH = eta ?? .infinity
    }

    func top(_ n: Int) -> [AppGroup] {
        Array(groups.values.filter { $0.pids.first != ownPid || $0.pids.count > 1 }
            .sorted { $0.footprint > $1.footprint }.prefix(n))
    }

    func group(_ key: String) -> AppGroup? { groups[key] }
}
