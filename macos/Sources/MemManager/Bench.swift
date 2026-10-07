import Foundation
import MemCore
import MemShared
import MemSys

/// Reproducible memory-bloat benchmark (macOS): creates realistic bloat,
/// runs each MemManager action through the same kernel paths the helper and
/// app use, and measures what it freed and what it cost.
/// `sudo MemManager --bench [--out=PATH]` (Nudge and purge need root).
enum Bench {
    struct Scenario {
        var id: String
        var name: String
        var action: String
        var freed: Int64 = 0
        var cost = ""
        var details: [(String, String)] = []
        var skipped: String?

        func json(total: UInt64) -> String {
            let d = details.map { "\(SelfTest.json($0.0)):\($0.1)" }.joined(separator: ",")
            let pct = Double(freed) * 100 / Double(max(total, 1))
            return "{\"id\":\(SelfTest.json(id)),\"name\":\(SelfTest.json(name)),\"action\":\(SelfTest.json(action)),"
                + "\"freed_bytes\":\(freed),\"freed_pct_ram\":\(String(format: "%.2f", pct)),\"cost\":\(SelfTest.json(cost)),"
                + "\"skipped\":\(skipped.map(SelfTest.json) ?? "null"),\"details\":{\(d)}}"
        }
    }

    static let parent = getpid()
    static func marker(_ id: Int, _ what: String) -> String { "/tmp/memmanager-bench-\(parent)-\(id).\(what)" }

    // MARK: child

    /// Child: allocate and touch `mib` MiB; if cooperative, free it on memory pressure like a well-behaved app.
    static func child(mib: Int, id: Int, cooperative: Bool, parent: String) -> Never {
        let size = mib << 20
        var buf: UnsafeMutableRawPointer? = malloc(size)
        if let b = buf {
            var i = 0
            while i < size {
                b.storeBytes(of: UInt64(i) &* 0x9E37_79B9_7F4A_7C15 ^ UInt64(id), toByteOffset: i, as: UInt64.self)
                i += 4096
            }
        }
        let base = "/tmp/memmanager-bench-\(parent)-\(id)"
        var src: DispatchSourceMemoryPressure?
        if cooperative {
            let s = DispatchSource.makeMemoryPressureSource(eventMask: [.warning, .critical], queue: .main)
            s.setEventHandler {
                if let b = buf {
                    free(b)
                    buf = nil
                    malloc_zone_pressure_relief(nil, 0)
                    FileManager.default.createFile(atPath: base + ".released", contents: nil)
                }
            }
            s.resume()
            src = s
        }
        _ = src
        FileManager.default.createFile(atPath: base + ".ready", contents: nil)
        dispatchMain()
    }

    // MARK: helpers

    static func vm() -> mm_vmstats {
        var s = mm_vmstats()
        _ = mm_vm_stats(&s)
        return s
    }

    static func footprint(_ pid: Int32) -> UInt64 { mm_proc_rusage(pid).phys_footprint }

    static func level() -> Int32 {
        var l: Int32 = 0
        _ = mm_sysctl_int("kern.memorystatus_vm_pressure_level", &l)
        return l
    }

    static func readFileMs(_ path: String) -> Double {
        let t = Date()
        guard let h = FileHandle(forReadingAtPath: path) else { return -1 }
        while autoreleasepool(invoking: { (try? h.read(upToCount: 4 << 20))?.isEmpty == false }) {}
        try? h.close()
        return Date().timeIntervalSince(t) * 1000
    }

    static func waitFor(_ path: String, seconds: Double) -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            if FileManager.default.fileExists(atPath: path) { return true }
            Thread.sleep(forTimeInterval: 0.1)
        }
        return false
    }

    static func spawn(mib: Int, id: Int, cooperative: Bool) -> Process? {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: Bundle.main.executablePath ?? CommandLine.arguments[0])
        var args = ["--bench-child=\(mib)", "--bench-id=\(id)", "--bench-parent=\(parent)"]
        if cooperative { args.append("--cooperative") }
        p.arguments = args
        do { try p.run() } catch { return nil }
        return p
    }

    // MARK: run

    static func run(out: String?) -> Int32 {
        let root = geteuid() == 0
        let total = mm_memsize()
        let page = vm().page_size
        let size = min(UInt64(768) << 20, total / 10)
        let mib = Int(size >> 20)
        var failures: [String] = []
        var scen: [Scenario] = []

        let coop = spawn(mib: mib, id: 0, cooperative: true)
        let idle = spawn(mib: mib, id: 1, cooperative: false)
        if coop == nil || idle == nil { failures.append("could not spawn children") }
        if !waitFor(marker(0, "ready"), seconds: 60) || !waitFor(marker(1, "ready"), seconds: 60) {
            failures.append("children did not become ready")
        }
        Thread.sleep(forTimeInterval: 1)

        // A + B: Nudge (purgeable memory and cooperative apps).
        let addr = mm_purgeable_alloc(size)
        if addr != 0, let p = UnsafeMutableRawPointer(bitPattern: UInt(addr)) {
            var i = 0
            while i < Int(size) {
                p.storeBytes(of: UInt8(truncatingIfNeeded: i >> 12), toByteOffset: i, as: UInt8.self)
                i += Int(page)
            }
            _ = mm_purgeable_set_volatile(addr)
        } else {
            failures.append("purgeable allocation failed")
        }
        var a = Scenario(id: "A", name: "Volatile purgeable memory (app caches)", action: "Nudge (helper)")
        var b = Scenario(id: "B", name: "App holding a cache, cooperative", action: "Nudge (helper)")
        if root {
            let v0 = vm()
            let coopBefore = coop.map { footprint($0.processIdentifier) } ?? 0
            let stateBefore = mm_purgeable_state(addr)
            let rc = mm_pressure_trigger((HelperConstants.triggerAll << 16) | HelperConstants.levelWarn)
            Thread.sleep(forTimeInterval: 3)
            let reset = mm_pressure_trigger((HelperConstants.triggerAll << 16) | HelperConstants.levelNormal)
            Thread.sleep(forTimeInterval: 1)
            let v1 = vm()
            let coopAfter = coop.map { footprint($0.processIdentifier) } ?? 0
            let released = FileManager.default.fileExists(atPath: marker(0, "released"))
            let lvl = level()
            a.freed = Int64(v0.purgeable_count * page) - Int64(v1.purgeable_count * page)
            a.cost = "apps regenerate purged caches on next use"
            a.details = [
                ("bloat_bytes", "\(size)"),
                ("purgeable_before", "\(v0.purgeable_count * page)"),
                ("purgeable_after", "\(v1.purgeable_count * page)"),
                ("region_state_before", "\(stateBefore)"),
                ("region_state_after", "\(mm_purgeable_state(addr))"),
                ("free_before", "\(v0.free_count * page)"),
                ("free_after", "\(v1.free_count * page)"),
                ("trigger_rc", "\(rc)"),
                ("reset_rc", "\(reset)"),
                ("kernel_level_after_reset", "\(lvl)"),
            ]
            if rc != 0 { a.skipped = "trigger failed (\(rc))" }
            if reset != 0 { failures.append("pressure reset failed: \(reset)") }
            b.freed = Int64(coopBefore) - Int64(coopAfter)
            b.cost = "the app rebuilds its cache when needed"
            b.details = [("footprint_before", "\(coopBefore)"), ("footprint_after", "\(coopAfter)"),
                         ("handler_ran", released ? "true" : "false")]
        } else {
            a.skipped = "needs root (run with sudo or the helper)"
            b.skipped = a.skipped
        }
        scen.append(a)
        scen.append(b)
        if addr != 0 { mm_purgeable_free(addr, size) }

        // C: file cache.
        let path = "/tmp/memmanager-bench-\(parent).dat"
        var c = Scenario(id: "C", name: "File cache after reading a large file", action: "Purge disk cache (manual, helper)")
        do {
            FileManager.default.createFile(atPath: path, contents: nil)
            let h = try FileHandle(forWritingTo: URL(fileURLWithPath: path))
            var chunk = Data(count: 4 << 20)
            var seed: UInt64 = 0x1234_5678
            var written: UInt64 = 0
            while written < size {
                chunk.withUnsafeMutableBytes { raw in
                    var j = 0
                    while j < raw.count {
                        seed = seed &* 6364136223846793005 &+ 1
                        raw[j] = UInt8(truncatingIfNeeded: seed >> 33)
                        j += 512
                    }
                }
                try h.write(contentsOf: chunk)
                written += UInt64(chunk.count)
            }
            try h.synchronize()
            try h.close()
        } catch {
            failures.append("temp file: \(error)")
        }
        let warm = readFileMs(path)
        if root {
            let v0 = vm()
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/usr/sbin/purge")
            try? p.run()
            p.waitUntilExit()
            let v1 = vm()
            let cold = readFileMs(path)
            c.freed = Int64(v0.external_page_count * page) - Int64(v1.external_page_count * page)
            c.cost = String(format: "re-reading the file took %.0f ms vs %.0f ms from cache", cold, warm)
            c.details = [("file_bytes", "\(size)"), ("cached_files_before", "\(v0.external_page_count * page)"),
                         ("cached_files_after", "\(v1.external_page_count * page)"),
                         ("read_ms_cached", String(format: "%.1f", warm)), ("read_ms_after_purge", String(format: "%.1f", cold)),
                         ("purge_status", "\(p.terminationStatus)")]
        } else {
            c.skipped = "needs root (run with sudo or the helper)"
        }
        scen.append(c)
        try? FileManager.default.removeItem(atPath: path)

        // D: quitting an idle app that holds memory.
        var d = Scenario(id: "D", name: "Idle app holding memory", action: "Quit idle app (suggestion / allowlist)")
        if let idle {
            let fp = footprint(idle.processIdentifier)
            let v0 = vm()
            idle.terminate()
            idle.waitUntilExit()
            Thread.sleep(forTimeInterval: 1)
            let v1 = vm()
            d.freed = Int64(fp)
            d.cost = "the app must be reopened"
            d.details = [("app_footprint", "\(fp)"), ("free_before", "\(v0.free_count * page)"),
                         ("free_after", "\(v1.free_count * page)")]
        } else {
            d.skipped = "child not running"
        }
        scen.append(d)

        coop?.terminate()
        for id in 0...1 {
            for what in ["ready", "released"] { try? FileManager.default.removeItem(atPath: marker(id, what)) }
        }

        for s in scen {
            let gb = String(format: "%.2f", Double(s.freed) / 1_073_741_824)
            FileHandle.standardError.write("\(s.id) \(s.action): freed \(gb) GiB | \(s.skipped ?? s.cost)\n".data(using: .utf8)!)
        }
        let report = "{\"os\":\"macos\",\"total_ram\":\(total),\"root\":\(root),\"scenarios\":["
            + scen.map { $0.json(total: total) }.joined(separator: ",")
            + "],\"failures\":[\(failures.map(SelfTest.json).joined(separator: ","))]}"
        if let out { try? report.write(toFile: out, atomically: true, encoding: .utf8) }
        print(report)
        return failures.isEmpty ? 0 : 1
    }
}
