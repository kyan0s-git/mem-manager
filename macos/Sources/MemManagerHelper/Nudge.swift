import Darwin
import Foundation
import MemShared
import MemSys

/// Latch-safe wrapper around kern.memorypressure_manual_trigger.
///
/// The sysctl latches the system pressure level until NORMAL is written, so
/// every path must restore it: bounded duration, a marker file checked on
/// start, launchd KeepAlive-on-crash, and signal handlers.
final class Nudger {
    private let queue = DispatchQueue(label: "dev.memmanager.helper.nudge")
    private let markerDir = "/var/db/dev.memmanager.helper"
    private var markerPath: String { markerDir + "/nudge-active" }
    private var lastNudge: Date?
    private var active = false
    private var signalSources: [DispatchSourceSignal] = []

    private func value(_ level: Int32) -> Int32 { (HelperConstants.triggerAll << 16) | level }

    private func writeMarker() {
        try? FileManager.default.createDirectory(atPath: markerDir, withIntermediateDirectories: true,
                                                 attributes: [.posixPermissions: 0o700])
        let fd = open(markerPath, O_WRONLY | O_CREAT | O_TRUNC, 0o600)
        if fd >= 0 {
            _ = "1".withCString { write(fd, $0, 1) }
            fsync(fd)
            close(fd)
        }
    }

    private func removeMarker() {
        unlink(markerPath)
    }

    /// Restores NORMAL (always safe to call).
    @discardableResult
    func reset() -> Int32 {
        let rc = mm_pressure_trigger(value(HelperConstants.levelNormal))
        removeMarker()
        active = false
        return rc
    }

    func recoverIfNeeded() {
        if FileManager.default.fileExists(atPath: markerPath) {
            reset()
        }
    }

    func installSignalHandlers() {
        for sig in [SIGTERM, SIGINT, SIGHUP] {
            signal(sig, SIG_IGN)
            let src = DispatchSource.makeSignalSource(signal: sig, queue: queue)
            src.setEventHandler { [weak self] in
                self?.reset()
                exit(0)
            }
            src.resume()
            signalSources.append(src)
        }
    }

    func nudge(level: Int32, durationMs: Int) -> (Int32, String) {
        queue.sync { () -> (Int32, String) in
            guard level == HelperConstants.levelWarn || level == HelperConstants.levelCritical else {
                return (EINVAL, "unsupported level")
            }
            if active { return (EBUSY, "a nudge is already running") }
            if let last = lastNudge, Date().timeIntervalSince(last) < HelperConstants.minNudgeIntervalS {
                return (EAGAIN, "rate limited")
            }
            let ms = min(max(durationMs, HelperConstants.minNudgeMs), HelperConstants.maxNudgeMs)
            writeMarker()
            active = true
            lastNudge = Date()
            // Schedule the reset before raising the level.
            queue.asyncAfter(deadline: .now() + .milliseconds(ms)) { [weak self] in
                self?.reset()
            }
            let rc = mm_pressure_trigger(value(level))
            if rc != 0 {
                reset()
                return (rc, String(cString: strerror(rc)))
            }
            return (0, "ok")
        }
    }
}

enum Purger {
    static func run() -> (Int32, String) {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/sbin/purge")
        do {
            try p.run()
            p.waitUntilExit()
            return (p.terminationStatus, p.terminationStatus == 0 ? "ok" : "purge failed")
        } catch {
            return (EIO, error.localizedDescription)
        }
    }
}

/// Exits after 60 s without clients; launchd relaunches on demand.
enum IdleExit {
    private static var last = Date()
    private static let lock = NSLock()

    static func touch() {
        lock.lock()
        last = Date()
        lock.unlock()
    }

    static func start() {
        let t = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        t.schedule(deadline: .now() + 60, repeating: 30, leeway: .seconds(5))
        t.setEventHandler {
            lock.lock()
            let idle = Date().timeIntervalSince(last)
            lock.unlock()
            if idle > 60 {
                nudger.reset()
                exit(0)
            }
        }
        t.resume()
        timer = t
    }

    private static var timer: DispatchSourceTimer?
}
