import AppKit
import Combine
import MemCore
import MemShared

/// Owns the status item, popover, settings window and the engine.
final class AppController: NSObject, NSApplicationDelegate, NSPopoverDelegate {
    let settings = AppSettings.shared
    let engine: Engine
    let statusItem = StatusItemController()
    let helper = HelperClient()
    let notifier = Notifier()
    private var popover: NSPopover?
    private var dashboard: DashboardViewController?
    private let settingsWindow = SettingsWindowController()
    private var lastSnapshot = Snapshot()
    private var selfPressure: DispatchSourceMemoryPressure?
    private var updateSub: AnyCancellable?
    static let explainerURL = URL(string: "https://github.com/kyan0s-git/mem-manager/blob/HEAD/docs/understanding-memory.md")!

    override init() {
        engine = Engine(settings: AppSettings.shared.engine)
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem.onLeftClick = { [weak self] in self?.togglePopover() }
        statusItem.onRightClick = { [weak self] in self?.showMenu() }
        settings.onChange = { [weak self] in
            guard let self else { return }
            self.engine.setSettings(self.settings.engine)
            self.statusItem.invalidate()
            self.render(self.lastSnapshot)
        }
        engine.onSnapshot = { [weak self] in self?.render($0) }
        engine.onNotice = { [weak self] in self?.handle($0) }
        engine.helperAvailable = { [weak self] in
            // Read on the main thread when the engine was created; refreshed below.
            self?.helperInstalledCache ?? false
        }
        engine.runNudge = { [weak self] level, done in
            self?.helper.nudge(level: level) { ok, msg in done(ok, msg) }
        }
        engine.quitApp = { [weak self] target in self?.quit(target) ?? false }
        notifier.setUp()
        notifier.onQuit = { [weak self] in _ = self?.quit($0) }
        notifier.onRelaunch = { [weak self] in self?.relaunch($0) }
        notifier.onIgnore = { [weak self] in self?.settings.toggleIgnore(($0 as NSString).lastPathComponent.replacingOccurrences(of: ".app", with: "")) }

        let ws = NSWorkspace.shared.notificationCenter
        ws.addObserver(forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main) { [weak self] n in
            if let app = n.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication, let name = app.localizedName {
                self?.engine.appActivated(name)
            }
        }
        ws.addObserver(forName: NSWorkspace.screensDidSleepNotification, object: nil, queue: .main) { [weak self] _ in
            self?.engine.setDisplayOff(true)
        }
        ws.addObserver(forName: NSWorkspace.screensDidWakeNotification, object: nil, queue: .main) { [weak self] _ in
            self?.engine.setDisplayOff(false)
        }
        // Our own good citizenship: drop rebuildable state under pressure.
        let src = DispatchSource.makeMemoryPressureSource(eventMask: [.warning, .critical], queue: .main)
        src.setEventHandler { [weak self] in
            self?.engine.trimSelf()
            if self?.popover?.isShown != true { self?.dashboard = nil }
        }
        src.resume()
        selfPressure = src
        refreshHelperCache()
        engine.start()

        let updater = Updater.shared
        updater.onFound = { [weak self] o in
            self?.notifier.post(title: "MemManager \(o.version) is available",
                                body: "Open MemManager to install it. It takes a few seconds and keeps your settings.")
        }
        updateSub = updater.$status.receive(on: RunLoop.main).sink { [weak self] _ in
            guard let self else { return }
            self.render(self.lastSnapshot)
        }
        updater.schedule { AppSettings.shared.autoUpdate }
    }

    private var helperInstalledCache = false
    private func refreshHelperCache() { helperInstalledCache = helper.isInstalled }

    private func render(_ snap: Snapshot) {
        lastSnapshot = snap
        statusItem.update(snap, settings: settings)
        if popover?.isShown == true {
            dashboard?.render(snap, settings: settings, helperInstalled: helperInstalledCache)
        }
    }

    // MARK: popover

    func togglePopover() {
        if let p = popover, p.isShown {
            p.performClose(nil)
            return
        }
        guard let button = statusItem.item.button else { return }
        refreshHelperCache()
        let vc = DashboardViewController()
        vc.controller = self
        dashboard = vc
        let p = NSPopover()
        p.behavior = .transient
        p.animates = true
        p.contentViewController = vc
        p.delegate = self
        popover = p
        vc.render(lastSnapshot, settings: settings, helperInstalled: helperInstalledCache)
        p.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
        engine.setPopupVisible(true)
    }

    func popoverDidClose(_ notification: Notification) {
        engine.setPopupVisible(false)
        popover?.contentViewController = nil
        popover = nil
        dashboard = nil
    }

    // MARK: actions

    func showSettings() {
        popover?.performClose(nil)
        settingsWindow.show(helper: helper, onNudge: { [weak self] in self?.nudge() }, onPurge: { [weak self] in self?.purge() })
        settingsWindow.onClose = { [weak self] in self?.refreshHelperCache() }
    }

    func togglePause() { engine.setPaused(!lastSnapshot.paused) }

    func nudge() {
        refreshHelperCache()
        guard helperInstalledCache else { showSettings(); return }
        // While memory is comfortable, explain what a Nudge would cost first.
        if let text = Presentation.nudgePreview(lastSnapshot.state, split: lastSnapshot.reading.split,
                                                friendly: settings.friendlyValues) {
            popover?.performClose(nil)
            let a = NSAlert()
            a.messageText = "Nudge anyway?"
            a.informativeText = text
            a.addButton(withTitle: "Nudge")
            a.addButton(withTitle: "Cancel")
            NSApp.activate(ignoringOtherApps: true)
            guard a.runModal() == .alertFirstButtonReturn else { return }
        }
        engine.nudgeNow()
    }

    func openExplainer() {
        settings.explained = true
        popover?.performClose(nil)
        NSWorkspace.shared.open(AppController.explainerURL)
    }

    func installUpdate() {
        guard case let .available(o) = Updater.shared.status else { return }
        popover?.performClose(nil)
        let a = NSAlert()
        a.messageText = "Install MemManager \(o.version)?"
        a.informativeText = "MemManager downloads the update, checks it against its published SHA-256, replaces itself and opens again with your settings."
        a.addButton(withTitle: "Install")
        a.addButton(withTitle: "Cancel")
        NSApp.activate(ignoringOtherApps: true)
        if a.runModal() == .alertFirstButtonReturn { Updater.shared.install() }
    }

    func purge() {
        helper.purge { [weak self] ok, msg in
            self?.engine.recordManual(ActionID.purgeManual, detail: ok ? "disk cache flushed" : msg)
        }
    }

    private func runningApp(_ target: String) -> NSRunningApplication? {
        NSWorkspace.shared.runningApplications.first {
            $0.bundleURL?.path == target || $0.localizedName == target
                || ($0.bundleURL?.path.lowercased() == target.lowercased())
        }
    }

    @discardableResult
    func quit(_ target: String) -> Bool {
        runningApp(target)?.terminate() ?? false
    }

    func relaunch(_ target: String) {
        guard let app = runningApp(target), let url = app.bundleURL else { return }
        app.terminate()
        var tries = 0
        Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { t in
            tries += 1
            if app.isTerminated || tries > 40 {
                t.invalidate()
                if app.isTerminated {
                    NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
                }
            }
        }
    }

    func showAppMenu(_ row: AppRow, from view: NSView) {
        let menu = NSMenu()
        let header = NSMenuItem(title: "\(row.name) — \(fmtBytes(row.footprint)) in \(row.pids.count) process(es)", action: nil, keyEquivalent: "")
        header.isEnabled = false
        menu.addItem(header)
        menu.addItem(.separator())
        let target = row.bundlePath ?? row.name
        if row.bundlePath != nil {
            menu.addItem(ClosureMenuItem("Quit") { [weak self] in _ = self?.quit(target) })
            menu.addItem(ClosureMenuItem("Relaunch") { [weak self] in self?.relaunch(target) })
        }
        let ignored = settings.ignoreLeaks.contains(row.name.lowercased())
        let ignore = ClosureMenuItem(ignored ? "Alert about leaks in this app" : "Don't alert about leaks in this app") { [weak self] in
            self?.settings.toggleIgnore(row.name)
        }
        menu.addItem(ignore)
        menu.addItem(ClosureMenuItem("Open Activity Monitor") {
            NSWorkspace.shared.openApplication(at: URL(fileURLWithPath: "/System/Applications/Utilities/Activity Monitor.app"),
                                               configuration: NSWorkspace.OpenConfiguration())
        })
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: view.bounds.height), in: view)
    }

    func showMenu() {
        let menu = NSMenu()
        menu.addItem(ClosureMenuItem("Open MemManager") { [weak self] in self?.togglePopover() })
        menu.addItem(.separator())
        let nudge = ClosureMenuItem("Nudge apps to release caches") { [weak self] in self?.nudge() }
        nudge.isEnabled = helper.isInstalled
        menu.addItem(nudge)
        menu.addItem(ClosureMenuItem(lastSnapshot.paused ? "Resume automatic actions" : "Pause for 1 hour") { [weak self] in
            self?.togglePause()
        })
        let profiles = NSMenu()
        for p in Profile.allCases {
            let item = ClosureMenuItem(p.label) { [weak self] in self?.settings.profile = p }
            item.state = settings.profile == p ? .on : .off
            profiles.addItem(item)
        }
        let profileItem = NSMenuItem(title: "Profile", action: nil, keyEquivalent: "")
        profileItem.submenu = profiles
        menu.addItem(profileItem)
        menu.addItem(.separator())
        if case let .available(o) = Updater.shared.status {
            menu.addItem(ClosureMenuItem("Install MemManager \(o.version)…") { [weak self] in self?.installUpdate() })
        } else {
            menu.addItem(ClosureMenuItem("Check for Updates") { Updater.shared.check(manual: true) })
        }
        menu.addItem(ClosureMenuItem("Settings…") { [weak self] in self?.showSettings() })
        menu.addItem(ClosureMenuItem("Quit MemManager") { NSApp.terminate(nil) })
        statusItem.item.menu = menu
        statusItem.item.button?.performClick(nil)
        statusItem.item.menu = nil
    }

    private func handle(_ n: Notice) {
        switch n {
        case let .leak(_, name, slope, eta, runaway, bundlePath):
            if runaway {
                notifier.post(title: "\(name) is allocating memory very fast",
                              body: "Its memory use is climbing by the second.", category: Notifier.leakCategory,
                              target: bundlePath ?? name)
            } else {
                let etaText = eta.map { " At this rate memory pressure turns critical in about \(fmtHours($0))." } ?? ""
                notifier.post(title: "\(name) looks like it's leaking memory",
                              body: "It has grown steadily by about \(Int(slope)) MB per hour.\(etaText) Relaunching it frees the memory.",
                              category: Notifier.leakCategory, target: bundlePath ?? name)
            }
        case let .swap(text):
            notifier.post(title: "Your Mac is swapping heavily", body: text)
        case let .info(title, text):
            notifier.post(title: title, body: text)
        }
    }
}

/// NSMenuItem that runs a closure.
final class ClosureMenuItem: NSMenuItem {
    private let handler: () -> Void

    init(_ title: String, _ handler: @escaping () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
    }

    required init(coder: NSCoder) { fatalError() }

    @objc private func run() { handler() }
}
