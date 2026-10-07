import Foundation

/// Identifiers shared by the app and the privileged helper.
public enum HelperConstants {
    public static let machService = "dev.memmanager.helper"
    public static let plistName = "dev.memmanager.helper.plist"
    public static let appBundleID = "dev.memmanager.MemManager"
    public static let version = "1"

    /// TEST_LOW_MEMORY_PURGEABLE_TRIGGER_ALL (xnu kern_memorystatus_notify.c).
    public static let triggerAll: Int32 = 6
    /// NOTE_MEMORYSTATUS_PRESSURE_* (xnu bsd/sys/event_private.h).
    public static let levelNormal: Int32 = 0x1
    public static let levelWarn: Int32 = 0x2
    public static let levelCritical: Int32 = 0x4

    public static let minNudgeMs = 500
    public static let maxNudgeMs = 5_000
    /// Helper-side rate limit, independent of the app's own policy.
    public static let minNudgeIntervalS: TimeInterval = 300
}

/// The privileged helper's XPC interface: integers only, no paths or strings
/// to execute (docs/architecture/macos.md §5).
@objc public protocol MemHelperProtocol {
    /// Briefly raises the kernel pressure level so apps drop caches and
    /// purgeable memory is emptied, then always restores it.
    func nudge(level: Int32, durationMs: Int, reply: @escaping (Int32, String) -> Void)
    /// Runs /usr/sbin/purge (flushes the disk buffer cache). Manual only.
    func purgeDiskCache(reply: @escaping (Int32, String) -> Void)
    func version(reply: @escaping (String) -> Void)
}
