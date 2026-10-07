import AppKit
import Foundation
import MemCore
import MemSys

/// Headless self-test (CI runs this on macos-latest):
/// `MemManager --selftest[=SECONDS] [--out=PATH]`
enum SelfTest {
    static func json(_ s: String) -> String {
        "\"" + s.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }

    static func run(seconds: Int, out: String?) -> Int32 {
        var failures: [String] = []
        let engine = Engine(settings: EngineSettings())
        var samples: [String] = []
        let deadline = Date().addingTimeInterval(TimeInterval(max(3, seconds)))
        while Date() < deadline {
            let (r, s, st) = engine.sampleOnce()
            samples.append(String(format: "{\"used\":%.4f,\"app\":%llu,\"wired\":%llu,\"compressed\":%llu,\"cached\":%llu,\"swap\":%llu,\"kernel_level\":%ld,\"free_pct\":%ld,\"s\":%.4f,\"state\":\"%@\"}",
                                  r.usedFraction, r.app, r.wired, r.compressed, r.cached, r.swapUsed,
                                  r.kernelLevel, r.freePct, s, st.name))
            if ![1, 2, 4].contains(r.kernelLevel) { failures.append("unexpected kernel level \(r.kernelLevel)") }
            Thread.sleep(forTimeInterval: 1)
        }
        let r = engine.last
        if r.total == 0 || r.used == 0 { failures.append("VM statistics returned zeros") }
        if engine.procs.visibleProcesses < 3 { failures.append("too few visible processes (\(engine.procs.visibleProcesses))") }

        // Every icon style renders.
        for style in IconStyle.allCases {
            let spec = IconSpec(style: style, fraction: 0.62, text: "62%", state: .high, color: .systemBlue, template: false,
                                paused: false, history: [0.2, 0.4, 0.3, 0.6], height: 22)
            let img = IconRenderer.image(spec)
            guard let rep = img.bitmapImageRepForCachingDisplay(in: NSRect(origin: .zero, size: img.size)) else {
                failures.append("icon \(style.rawValue): no bitmap"); continue
            }
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
            img.draw(in: NSRect(origin: .zero, size: img.size))
            NSGraphicsContext.restoreGraphicsState()
            var opaque = 0
            for x in 0..<rep.pixelsWide { for y in 0..<rep.pixelsHigh where (rep.colorAt(x: x, y: y)?.alphaComponent ?? 0) > 0.5 { opaque += 1 } }
            if opaque == 0 { failures.append("icon \(style.rawValue) drew nothing") }
        }

        let me = mm_proc_rusage(getpid())
        let top = engine.procs.top(5).map { "{\"name\":\(json($0.name)),\"footprint\":\($0.footprint)}" }.joined(separator: ",")
        let report = """
        {"total":\(r.total),"processes_visible":\(engine.procs.visibleProcesses),"top":[\(top)],\
        "self":{"footprint":\(me.phys_footprint),"peak":\(me.lifetime_max_phys_footprint)},\
        "samples":[\(samples.joined(separator: ","))],"failures":[\(failures.map(json).joined(separator: ","))]}
        """
        if let out { try? report.write(toFile: out, atomically: true, encoding: .utf8) }
        print(report)
        return failures.isEmpty ? 0 : 1
    }
}
