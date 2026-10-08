import Foundation

/// What the numbers mean to a person (docs/architecture/ui-design.md §7).
///
/// The default "friendly" presentation leads with memory that is ready for
/// apps (free memory plus cached files macOS hands out instantly) and shows the
/// cache as a speed-up. Every figure is a real macOS value; only the grouping
/// and the words differ. "Activity Monitor terms" keeps Apple's labels.
public enum Presentation {
    /// "1.4 GB", "820 MB".
    public static func bytes(_ b: UInt64) -> String {
        let d = Double(b)
        let g = d / 1_073_741_824
        if g >= 100 { return String(format: "%.0f GB", g) }
        if g >= 1 { return String(format: "%.1f GB", g) }
        return String(format: "%.0f MB", d / 1_048_576)
    }

    /// Calm state names by default; the engine's terms otherwise.
    public static func stateName(_ s: PressureState, friendly: Bool) -> String {
        guard friendly else { return s.name }
        switch s {
        case .normal: return "Comfortable"
        case .elevated: return "Busy"
        case .high: return "Tight"
        case .critical: return "Critical"
        }
    }

    /// New app memory the cache made room for between two (apps, cache) fraction pairs.
    public static func roomMade(first: (Double, Double), last: (Double, Double), total: UInt64) -> UInt64 {
        let grew = last.0 - first.0
        let gave = first.1 - last.1
        guard grew > 0, gave > 0 else { return 0 }
        return UInt64(min(grew, gave) * Double(total))
    }

    /// "10 minutes", "hour".
    public static func window(_ ms: UInt64) -> String {
        let m = max(1, (ms + 30_000) / 60_000)
        if m == 1 { return "minute" }
        if m >= 59 { return "hour" }
        return "\(m) minutes"
    }

    public static func roomLine(bytes b: UInt64, windowMs: UInt64) -> String? {
        guard b >= 128 << 20 else { return nil }
        return "Cache made room for \(bytes(b)) of app memory in the last \(window(windowMs))."
    }

    /// Shown before a manual Nudge when memory is comfortable (friendly mode only).
    public static func nudgePreview(_ s: PressureState, split: MemorySplit, friendly: Bool) -> String? {
        guard friendly, s == .normal else { return nil }
        return "Memory is comfortable: \(bytes(split.ready)) is ready for apps, including \(bytes(split.cache)) of speed-up cache.\n\n"
            + "A Nudge asks apps to drop caches they would otherwise reuse, so things you open next may load a little slower. "
            + "It won't make anything faster right now, and macOS already frees memory on its own when it gets tight."
    }
}

/// Physical memory as a person thinks about it.
public struct MemorySplit: Equatable {
    public var total: UInt64
    /// Activity Monitor's "Memory Used": app memory, wired and compressed.
    public var apps: UInt64
    /// "Cached Files" (file cache and purgeable memory), handed out instantly.
    public var cache: UInt64
    public var free: UInt64

    public init(total: UInt64, used: UInt64, cached: UInt64) {
        self.total = total
        apps = min(used, total)
        cache = min(cached, total - apps)
        free = total - apps - cache
    }

    /// Ready for apps right now: free plus cached files.
    public var ready: UInt64 { cache + free }
    public var appsFraction: Double { total > 0 ? Double(apps) / Double(total) : 0 }
    public var cacheFraction: Double { total > 0 ? Double(cache) / Double(total) : 0 }
}

// MARK: - Updates

/// A semantic version: `0.2.0`, `v1.0.0-rc1`.
public struct AppVersion: Comparable, CustomStringConvertible, Equatable {
    public var major: Int
    public var minor: Int
    public var patch: Int
    public var pre: String?

    public init?(_ text: String) {
        var s = text.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("v") || s.hasPrefix("V") { s.removeFirst() }
        let core: Substring
        if let dash = s.firstIndex(of: "-") {
            core = s[..<dash]
            pre = String(s[s.index(after: dash)...])
        } else {
            core = Substring(s)
            pre = nil
        }
        let parts = core.split(separator: ".", omittingEmptySubsequences: false).map { Int($0) }
        guard (1...3).contains(parts.count), parts.allSatisfy({ $0 != nil }) else { return nil }
        major = parts[0]!
        minor = parts.count > 1 ? parts[1]! : 0
        patch = parts.count > 2 ? parts[2]! : 0
    }

    /// 0.x and suffixed versions are pre-releases.
    public var isPrerelease: Bool { major == 0 || pre != nil }

    public var description: String {
        "\(major).\(minor).\(patch)" + (pre.map { "-\($0)" } ?? "")
    }

    public static func < (a: AppVersion, b: AppVersion) -> Bool {
        if (a.major, a.minor, a.patch) != (b.major, b.minor, b.patch) {
            return (a.major, a.minor, a.patch) < (b.major, b.minor, b.patch)
        }
        switch (a.pre, b.pre) {
        case (nil, nil): return false
        case (nil, _): return false
        case (_, nil): return true
        case let (x?, y?): return x < y
        }
    }
}

public struct ReleaseAsset: Equatable {
    public var name: String
    public var url: String
    public var size: UInt64
}

public struct ReleaseInfo: Equatable {
    public var version: AppVersion
    public var prerelease: Bool
    public var draft: Bool
    public var htmlURL: String
    public var assets: [ReleaseAsset]
}

public struct UpdateOffer: Equatable {
    public var version: AppVersion
    public var htmlURL: String
    public var asset: ReleaseAsset
    public var checksum: ReleaseAsset
}

public enum Updates {
    public static let releasesURL = "https://api.github.com/repos/kyan0s-git/mem-manager/releases?per_page=20"
    public static let firstCheckDelay: TimeInterval = 120
    public static let checkInterval: TimeInterval = 24 * 3600

    /// Parses the GitHub releases API response.
    public static func parse(_ data: Data) -> [ReleaseInfo]? {
        guard let items = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return nil }
        return items.compactMap { r in
            guard let tag = r["tag_name"] as? String, let v = AppVersion(tag) else { return nil }
            let assets = (r["assets"] as? [[String: Any]] ?? []).compactMap { a -> ReleaseAsset? in
                guard let n = a["name"] as? String, let u = a["browser_download_url"] as? String else { return nil }
                return ReleaseAsset(name: n, url: u, size: (a["size"] as? NSNumber)?.uint64Value ?? 0)
            }
            return ReleaseInfo(version: v, prerelease: r["prerelease"] as? Bool ?? false, draft: r["draft"] as? Bool ?? false,
                               htmlURL: r["html_url"] as? String ?? "", assets: assets)
        }
    }

    /// The newest non-draft release newer than `current` carrying `assetName(version)` and its
    /// `.sha256`. Pre-releases are offered only to pre-release installs.
    public static func choose(_ releases: [ReleaseInfo], current: AppVersion,
                              assetName: (AppVersion) -> String) -> UpdateOffer? {
        let allowPre = current.isPrerelease
        return releases
            .filter { !$0.draft && (allowPre || !$0.prerelease) && $0.version > current }
            .compactMap { r -> UpdateOffer? in
                let name = assetName(r.version)
                guard let a = r.assets.first(where: { $0.name == name }),
                      let s = r.assets.first(where: { $0.name == name + ".sha256" }) else { return nil }
                return UpdateOffer(version: r.version, htmlURL: r.htmlURL, asset: a, checksum: s)
            }
            .max { $0.version < $1.version }
    }

    /// The digest in a `sha256sum`-style file.
    public static func checksum(in text: String) -> String? {
        guard let token = text.split(whereSeparator: { $0 == " " || $0 == "\n" || $0 == "\t" }).first?.lowercased(),
              token.count == 64, token.allSatisfy({ $0.isHexDigit }) else { return nil }
        return token
    }
}
