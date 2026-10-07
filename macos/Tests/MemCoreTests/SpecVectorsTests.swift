import Foundation
import XCTest
@testable import MemCore

/// Cross-implementation conformance with spec/test-vectors (same vectors as
/// the Rust core and the Python reference).
final class SpecVectorsTests: XCTestCase {
    private func vectorsDir() -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // MemCoreTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // macos
            .deletingLastPathComponent()   // repo root
            .appendingPathComponent("spec/test-vectors")
    }

    private func load(prefix: String) throws -> [(String, [String: Any])] {
        let files = try FileManager.default.contentsOfDirectory(at: vectorsDir(), includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" && $0.lastPathComponent.hasPrefix(prefix) }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        XCTAssertFalse(files.isEmpty)
        return try files.map { url in
            let obj = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [String: Any]
            return (url.lastPathComponent, obj)
        }
    }

    private func close(_ a: Double, _ b: Double) -> Bool {
        abs(a - b) <= 1e-9 * max(abs(a), abs(b), 1)
    }

    func testLeakVectors() throws {
        for (name, v) in try load(prefix: "leak-") {
            let profile = Profile(rawValue: v["profile"] as! String)!
            let heavy = v["heavy"] as! Bool
            let series = (v["series"] as! [[Double]]).map { ($0[0], $0[1]) }
            let got = LeakDetector.analyze(series, thresholds: profile.leakThresholds, heavy: heavy)
            let e = v["expect"] as! [String: Any]
            XCTAssertEqual(got.eligible, e["eligible"] as! Bool, "\(name) eligible")
            XCTAssertEqual(got.plateau, e["plateau"] as! Bool, "\(name) plateau")
            XCTAssertEqual(got.leak, e["leak"] as! Bool, "\(name) leak")
            XCTAssertEqual(got.runaway, e["runaway"] as! Bool, "\(name) runaway")
            for (k, g) in [("slope", got.slope), ("tau", got.tau), ("z", got.z)] {
                let want = (e[k] as! NSNumber).doubleValue
                XCTAssertTrue(close(g, want), "\(name) \(k): got \(g) want \(want)")
            }
        }
    }

    func testStateVectors() throws {
        for (name, v) in try load(prefix: "state-") {
            var sm = StateMachine()
            let samples = v["samples"] as! [[Any]]
            let got = samples.map { s -> String in
                let t = (s[0] as! NSNumber).uint64Value
                let p = (s[1] as! NSNumber).doubleValue
                let ev = s.count > 2 ? (s[2] as! Bool) : false
                return sm.step(tMs: t, p: p, event: ev).name
            }
            XCTAssertEqual(got, v["expect_states"] as! [String], name)
        }
    }

    func testRegretTracker() {
        var ledger = Ring<LedgerEntry>(capacity: 64)
        var r = RegretTracker(pageSize: 4096)
        for i: UInt64 in 0...12 {
            _ = r.observe(tMs: i * 10_000, hardCum: i * 100, fastAvail: 1 << 30, ledger: &ledger)
        }
        XCTAssertEqual(r.baselineRate, 10, accuracy: 1e-6)
        let t0: UInt64 = 120_000
        r.begin(tMs: t0, actionId: 2, manual: false, state: .high, hardCum: 1200, fastAvail: 1 << 30, ledger: &ledger)
        let after: UInt64 = (1 << 30) + (256 << 20)
        XCTAssertFalse(r.observe(tMs: t0 + 2_000, hardCum: 1220, fastAvail: after, ledger: &ledger))
        XCTAssertTrue(r.observe(tMs: t0 + 60_000, hardCum: 1200 + 600 + 16_384, fastAvail: after, ledger: &ledger))
        let e = ledger.last!
        XCTAssertFalse(e.pending)
        XCTAssertEqual(e.freedPages, 65_536)
        XCTAssertEqual(e.refaultRatio, 0.25, accuracy: 1e-9)
    }

    func testSchedulerPicksLowestTier() {
        let mk = { (id: Int, tier: Int, st: PressureState) in
            ActionSpec(id: id, tier: tier, minState: st, cooldownS: 30, bucketCapacity: 2, refillPerHour: 4)
        }
        var s = Scheduler(specs: [mk(3, 3, .high), mk(2, 2, .elevated)], nowMs: 0)
        XCTAssertEqual(s.pick(state: .high, s: 0.7, now: 0, maxTier: 5) { _ in true }, 2)
        XCTAssertEqual(s.pick(state: .high, s: 0.7, now: 0, maxTier: 5) { $0 != 2 }, 3)
        XCTAssertNil(s.pick(state: .elevated, s: 0.45, now: 0, maxTier: 5) { $0 != 2 })
    }

    func testMacPressure() {
        var prev = Sample()
        prev.freePct = 70
        var cur = prev
        cur.tMs = 1000
        XCTAssertEqual(pressureMacOS(cur, prev: prev), 0)
        cur.kernelLevel = 2
        XCTAssertEqual(pressureMacOS(cur, prev: prev), 0.6)
        cur.kernelLevel = 4
        XCTAssertEqual(pressureMacOS(cur, prev: prev), 0.9)
    }
}
