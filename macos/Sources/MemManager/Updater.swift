import AppKit
import CryptoKit
import Foundation
import MemCore

/// In-app updates: ask GitHub for a newer release, download the universal zip,
/// verify it against the published SHA-256, then swap the app bundle in place
/// and relaunch. Network work happens only during a check or an install.
struct UpdateError: LocalizedError {
    let message: String
    init(_ s: String) { message = s }
    var errorDescription: String? { message }
}

final class Updater: ObservableObject {
    static let shared = Updater()

    enum Status: Equatable {
        case idle
        case checking
        case upToDate(Date)
        case available(UpdateOffer)
        case installing(AppVersion)
        case failed(String)

        var busy: Bool {
            switch self {
            case .checking, .installing: return true
            default: return false
            }
        }
    }

    @Published private(set) var status: Status = .idle
    /// Called on the main thread when an automatic check finds a new version.
    var onFound: ((UpdateOffer) -> Void)?
    private var timer: Timer?

    /// Set by `--check-update --pretend-version=X` (CI).
    static var versionOverride: AppVersion?

    static var currentVersion: AppVersion {
        versionOverride
            ?? AppVersion(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "")
            ?? AppVersion("0.0.0")!
    }

    static func assetName(_ v: AppVersion) -> String { "MemManager-\(v)-macos-universal.zip" }

    private func set(_ s: Status) {
        if Thread.isMainThread { status = s } else { DispatchQueue.main.async { self.status = s } }
    }

    // MARK: schedule

    /// Daily automatic checks while `enabled()` is true (first one shortly after launch).
    func schedule(enabled: @escaping () -> Bool) {
        timer?.invalidate()
        let t = Timer(timeInterval: Updates.firstCheckDelay, repeats: false) { [weak self] _ in
            guard let self else { return }
            if enabled() { self.check(manual: false) }
            let daily = Timer(timeInterval: Updates.checkInterval, repeats: true) { [weak self] _ in
                if enabled() { self?.check(manual: false) }
            }
            daily.tolerance = 600
            RunLoop.main.add(daily, forMode: .common)
            self.timer = daily
        }
        t.tolerance = 30
        RunLoop.main.add(t, forMode: .common)
        timer = t
    }

    // MARK: check

    private func request(_ url: String, accept: String) -> URLRequest? {
        guard let u = URL(string: url), u.scheme == "https" else { return nil }
        var r = URLRequest(url: u, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30)
        r.setValue(accept, forHTTPHeaderField: "Accept")
        r.setValue("MemManager/\(Updater.currentVersion) (+https://github.com/kyan0s-git/mem-manager)", forHTTPHeaderField: "User-Agent")
        // CI only: authenticated API calls avoid shared-runner rate limits. Never sent elsewhere.
        if u.host == "api.github.com", let t = ProcessInfo.processInfo.environment["MEMMANAGER_GITHUB_TOKEN"], !t.isEmpty {
            r.setValue("Bearer \(t)", forHTTPHeaderField: "Authorization")
        }
        return r
    }

    private func fetch(_ url: String, accept: String, completion: @escaping (Result<Data, UpdateError>) -> Void) {
        guard let req = request(url, accept: accept) else { return completion(.failure(UpdateError("bad URL"))) }
        let cfg = URLSessionConfiguration.ephemeral
        cfg.urlCache = nil
        let session = URLSession(configuration: cfg)
        session.dataTask(with: req) { data, resp, err in
            defer { session.finishTasksAndInvalidate() }
            if let err { return completion(.failure(UpdateError(err.localizedDescription))) }
            let code = (resp as? HTTPURLResponse)?.statusCode ?? 0
            guard code == 200, let data else {
                return completion(.failure(UpdateError(code == 403 || code == 429
                    ? "GitHub is rate-limiting requests; try again later" : "HTTP \(code)")))
            }
            completion(.success(data))
        }.resume()
    }

    func check(manual: Bool) {
        if status.busy { return }
        let hadOffer: Bool
        if case .available = status { hadOffer = true } else { hadOffer = false }
        set(.checking)
        fetch(Updates.releasesURL, accept: "application/vnd.github+json") { [weak self] r in
            guard let self else { return }
            switch r {
            case let .failure(e):
                self.set(.failed(e.message))
            case let .success(data):
                guard let releases = Updates.parse(data) else { return self.set(.failed("unexpected response from GitHub")) }
                if let o = Updates.choose(releases, current: Updater.currentVersion, assetName: Updater.assetName) {
                    self.set(.available(o))
                    if !manual && !hadOffer { DispatchQueue.main.async { self.onFound?(o) } }
                } else {
                    self.set(.upToDate(Date()))
                }
            }
        }
    }

    // MARK: install

    /// Downloads, verifies and installs the offered version, then relaunches.
    func install() {
        guard case let .available(o) = status else { return }
        let app = Bundle.main.bundleURL
        guard app.pathExtension == "app" else { return set(.failed("not running from an app bundle")) }
        let parent = app.deletingLastPathComponent()
        guard FileManager.default.isWritableFile(atPath: parent.path),
              FileManager.default.isWritableFile(atPath: app.path) else {
            return set(.failed("can't replace the app in \(parent.path); download the new version from the releases page"))
        }
        set(.installing(o.version))
        downloadVerified(o) { [weak self] r in
            guard let self else { return }
            switch r {
            case let .failure(e):
                self.set(.failed(e.message))
            case let .success(zip):
                do {
                    let staged = try self.stage(zip)
                    try self.swapAndRelaunch(staged: staged, into: app)
                } catch {
                    self.set(.failed(error.localizedDescription))
                }
            }
        }
    }

    /// Downloads the offer's asset and keeps it only if it matches the published SHA-256.
    func downloadVerified(_ o: UpdateOffer, completion: @escaping (Result<Data, UpdateError>) -> Void) {
        fetch(o.checksum.url, accept: "application/octet-stream") { [weak self] r in
            guard let self else { return }
            guard case let .success(sumData) = r,
                  let want = Updates.checksum(in: String(decoding: sumData, as: UTF8.self)) else {
                return completion(.failure(UpdateError("couldn't read the published checksum")))
            }
            self.fetch(o.asset.url, accept: "application/octet-stream") { r in
                guard case let .success(data) = r else {
                    if case let .failure(e) = r { completion(.failure(e)) }
                    return
                }
                let got = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
                guard got == want else {
                    return completion(.failure(UpdateError("the download did not match its published checksum, so it was discarded")))
                }
                completion(.success(data))
            }
        }
    }

    /// `--check-update [--pretend-version=X] [--download] [--out=PATH]`: JSON report for CI
    /// (real GitHub API, download and checksum; never installs).
    static func cli(pretend: String?, download: Bool, out: String?) -> Int32 {
        if let p = pretend { versionOverride = AppVersion(p) }
        let u = Updater.shared
        u.check(manual: true)
        func spin(until done: () -> Bool) {
            let deadline = Date().addingTimeInterval(120)
            while !done() && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.1)) }
        }
        spin { !u.status.busy }
        var report = "{\"current\":\"\(currentVersion)\",\"asset_pattern\":\"\(assetName(currentVersion))\""
        var code: Int32 = 0
        switch u.status {
        case let .available(o):
            report += ",\"offer\":{\"version\":\"\(o.version)\",\"asset\":\"\(o.asset.name)\",\"size\":\(o.asset.size)}"
            if download {
                var result: Result<Data, UpdateError>?
                u.downloadVerified(o) { r in DispatchQueue.main.async { result = r } }
                spin { result != nil }
                switch result {
                case .success?: report += ",\"verified\":true"
                case let .failure(e)?: report += ",\"verified\":false,\"error\":\"\(e.message)\""; code = 1
                case nil: report += ",\"verified\":false,\"error\":\"timeout\""; code = 1
                }
            }
        case .upToDate:
            report += ",\"offer\":null"
        case let .failed(e):
            report += ",\"error\":\"\(e)\""
            code = 1
        default:
            report += ",\"error\":\"timeout\""
            code = 1
        }
        report += "}"
        if let out { try? report.write(toFile: out, atomically: true, encoding: .utf8) }
        print(report)
        return code
    }


    /// Unzips into a fresh temporary folder and returns the new app bundle.
    private func stage(_ zip: Data) throws -> URL {
        let fm = FileManager.default
        let dir = fm.temporaryDirectory.appendingPathComponent("MemManager-update-\(UUID().uuidString)")
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let zipURL = dir.appendingPathComponent("update.zip")
        try zip.write(to: zipURL)
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/ditto")
        p.arguments = ["-x", "-k", zipURL.path, dir.path]
        try p.run()
        p.waitUntilExit()
        guard p.terminationStatus == 0 else { throw UpdateError("couldn't unpack the update") }
        let new = dir.appendingPathComponent("MemManager.app")
        guard let b = Bundle(url: new), b.bundleIdentifier == Bundle.main.bundleIdentifier else {
            throw UpdateError("the update doesn't contain MemManager.app")
        }
        return new
    }

    /// Waits for this process to exit, swaps the bundles (restoring the old one
    /// on failure) and opens the app again.
    private func swapAndRelaunch(staged: URL, into app: URL) throws {
        let script = """
        while /bin/kill -0 "$1" 2>/dev/null; do /bin/sleep 0.2; done
        old="$3.updating-old"
        /bin/rm -rf "$old"
        if /bin/mv "$3" "$old" && /bin/mv "$2" "$3"; then
          /bin/rm -rf "$old"
        else
          [ -d "$3" ] || /bin/mv "$old" "$3"
        fi
        /bin/rm -rf "$(/usr/bin/dirname "$2")"
        /usr/bin/open "$3"
        """
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = ["-c", script, "memmanager-update", String(getpid()), staged.path, app.path]
        try p.run()
        DispatchQueue.main.async { NSApp.terminate(nil) }
    }
}
