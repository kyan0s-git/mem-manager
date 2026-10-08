import AppKit
import Foundation
import MemCore

/// Rows shown in the popover.
struct AppRow: Equatable {
    var key: String
    var name: String
    var bundlePath: String?
    var pids: [Int32]
    var footprint: UInt64
    var slope: Double
    var leak: Bool
    var runaway: Bool
    var idleHours: Double?
}

/// One activity entry: what MemManager did (or warned about), why, to which
/// apps, and — for actions — what it measurably changed (from the ledger).
struct ActivityRow: Equatable {
    var tMs: UInt64
    var title: String
    var why: String
    var detail: String = ""
    var manual = false
    var alert = false
    /// Links the entry to its ledger entry (actions only).
    var isAction = false
    var measured = false
    var freed: UInt64 = 0
    var refaultRatio = 0.0
    var pending = false
}

struct Snapshot: Equatable {
    var tMs: UInt64 = 0
    var reading = MacReading()
    var s = 0.0
    var state: PressureState = .normal
    var history: [Double] = []
    /// App and cache fractions at the same points as `history`.
    var historyApps: [Double] = []
    var historyCache: [Double] = []
    var apps: [AppRow] = []
    var idleHeavy: [AppRow] = []
    /// Newest first. `apps`, `idleHeavy` and `activity` are filled only while the
    /// popover is open, so a closed app does no per-tick work building them.
    var activity: [ActivityRow] = []
    var paused = false
    var acting = false
    var visibleProcesses = 0
}

enum ActionID {
    static let nudge = 1
    static let nudgeManual = 20
    static let purgeManual = 21
    static let quitIdle = 22

    static func label(_ id: Int) -> String {
        switch id {
        case nudge: return "Asked apps to release caches"
        case nudgeManual: return "Nudge"
        case purgeManual: return "Purged disk cache"
        case quitIdle: return "Quit idle app"
        default: return "Action"
        }
    }
}

/// Notices delivered to the UI (notifications).
enum Notice {
    case leak(key: String, name: String, slope: Double, etaH: Double?, runaway: Bool, bundlePath: String?)
    case swap(text: String)
    case info(title: String, text: String)
}

/// Runs on its own serial queue: pressure events, adaptive sampling, the
/// policy engine and leak detection. Publishes snapshots to the main thread.
final class Engine {
    let queue = DispatchQueue(label: "dev.memmanager.engine", qos: .utility)
    private let sampler = MacSampler()
    let procs: ProcessMonitor
    private var sm = StateMachine()
    private var scheduler: Scheduler
    private var regret: RegretTracker
    private var ledger = Ring<LedgerEntry>(capacity: 64)
    private var activity: [ActivityRow] = []
    private var history = Ring<Double>(capacity: 120)
    private var historyApps = Ring<Double>(capacity: 120)
    private var historyCache = Ring<Double>(capacity: 120)
    private var lastHistoryMs: UInt64 = 0
    private var prevSample: Sample?
    private(set) var last = MacReading()
    private var settings: EngineSettings
    private var pressureSource: DispatchSourceMemoryPressure?
    private var timer: DispatchSourceTimer?
    private var nextSysMs: UInt64 = 0
    private var nextProcMs: UInt64 = 0
    private var popupVisible = false
    private var displayOff = false
    private var pausedUntil: UInt64?
    private var actingUntil: UInt64 = 0
    private var elevatedSince: UInt64?
    private var swapHist = Ring<(UInt64, UInt64)>(capacity: 64)
    private var lastSwapNoticeMs: UInt64?
    private var lastActivated: [String: Date] = [:]
    private let launchDate = Date()

    /// Called on the main thread.
    var onSnapshot: ((Snapshot) -> Void)?
    var onNotice: ((Notice) -> Void)?
    /// Asks the main thread to run a Nudge through the helper; returns via completion.
    var runNudge: ((Int32, @escaping (Bool, String) -> Void) -> Void)?
    var helperAvailable: () -> Bool = { false }
    /// Asks the main thread to quit an app gracefully (auto-quit allowlist).
    var quitApp: ((String) -> Bool)?

    init(settings: EngineSettings) {
        let now = monotonicMs()
        self.settings = settings
        procs = ProcessMonitor(nowMs: now)
        scheduler = Scheduler(specs: Engine.specs(), nowMs: now)
        regret = RegretTracker(pageSize: 16384)
    }

    static func specs() -> [ActionSpec] {
        [
            ActionSpec(id: ActionID.nudge, tier: 3, minState: .high, cooldownS: 1800, bucketCapacity: 1, refillPerHour: 2),
            ActionSpec(id: ActionID.quitIdle, tier: 4, minState: .high, cooldownS: 600, bucketCapacity: 2, refillPerHour: 4),
        ]
    }

    func start() {
        queue.async { [self] in
            let src = DispatchSource.makeMemoryPressureSource(eventMask: [.normal, .warning, .critical], queue: queue)
            src.setEventHandler { [weak self] in
                guard let self, let s = self.pressureSource else { return }
                let critical = s.data.contains(.critical)
                self.tickSystem(event: critical)
                self.publish()
            }
            src.resume()
            pressureSource = src
            let t = DispatchSource.makeTimerSource(queue: queue)
            t.setEventHandler { [weak self] in self?.onTimer() }
            timer = t
            t.resume()
            nextSysMs = 0
            nextProcMs = 0
            onTimer()
        }
    }

    // MARK: inputs from the UI

    func setSettings(_ s: EngineSettings) { queue.async { self.settings = s } }
    func setPopupVisible(_ v: Bool) {
        queue.async {
            self.popupVisible = v
            if v { self.nextSysMs = 0; self.nextProcMs = 0; self.onTimer() } else { self.reschedule() }
        }
    }
    func setDisplayOff(_ off: Bool) { queue.async { self.displayOff = off; if !off { self.onTimer() } else { self.reschedule() } } }
    func setPaused(_ p: Bool) { queue.async { self.pausedUntil = p ? monotonicMs() + 3_600_000 : nil; self.publish() } }
    func appActivated(_ key: String) { queue.async { self.lastActivated[key.lowercased()] = Date() } }

    // MARK: loop

    private func cadence() -> Cadence? {
        Cadence.forState(popupVisible: popupVisible, state: sm.state, displayOff: displayOff, onBattery: false)
    }

    private func reschedule() {
        guard let t = timer else { return }
        guard let c = cadence() else {
            t.schedule(deadline: .distantFuture)
            return
        }
        let now = monotonicMs()
        let due = min(nextSysMs, nextProcMs)
        let delay = due > now ? due - now : 1
        t.schedule(deadline: .now() + .milliseconds(Int(delay)), repeating: .never,
                   leeway: .milliseconds(Int(max(100, c.systemMs / 10))))
    }

    private func onTimer() {
        let now = monotonicMs()
        if let c = cadence() {
            if now >= nextProcMs {
                tickProcesses(now)
                nextProcMs = now + c.processMs
            }
            if now >= nextSysMs {
                tickSystem(event: false)
                nextSysMs = now + c.systemMs
            }
        }
        publish()
        reschedule()
    }

    func tickSystem(event: Bool) {
        let now = monotonicMs()
        guard let r = sampler.sample(tMs: now) else { return }
        if prevSample == nil { regret = RegretTracker(pageSize: r.sample.pageSize) }
        let p = pressureMacOS(r.sample, prev: prevSample)
        let state = sm.step(tMs: now, p: p, event: event)
        if regret.observe(tMs: now, hardCum: r.sample.hardFaultsCum, fastAvail: r.sample.fastAvailBytes, ledger: &ledger),
           let e = ledger.last, let i = scheduler.index(of: e.actionId) {
            let pen = scheduler.actions[i].applyRegret(e.refaultRatio)
            ledger.updateLast { $0.penaltyAfter = pen }
        }
        prevSample = r.sample
        last = r
        if lastHistoryMs == 0 || now &- lastHistoryMs >= 5_000 {
            history.push(r.usedFraction)
            let sp = r.split
            historyApps.push(sp.appsFraction)
            historyCache.push(sp.cacheFraction)
            lastHistoryMs = now
        }
        if state >= .elevated {
            if elevatedSince == nil { elevatedSince = now }
        } else {
            elevatedSince = nil
        }
        swapHist.push((now, r.swapUsed))
        checkSwap(now: now)
        if !paused(now) { maybeAct(now: now, state: state) }
    }

    private func paused(_ now: UInt64) -> Bool { pausedUntil.map { now < $0 } ?? false }

    private func sustained(_ ms: UInt64, _ now: UInt64) -> Bool {
        elevatedSince.map { now &- $0 >= ms } ?? false
    }

    private func checkSwap(now: UInt64) {
        guard settings.notifySwap, sm.state >= .elevated, sustained(5 * 60_000, now),
              let oldest = swapHist.elements.first(where: { now &- $0.0 <= 5 * 60_000 }) else { return }
        let minutes = Double(now &- oldest.0) / 60_000
        guard minutes >= 2 else { return }
        let grew = last.swapUsed > oldest.1 ? Double(last.swapUsed - oldest.1) : 0
        let perMin = grew / minutes / MemCore.mib
        guard perMin >= 64 else { return }
        if let t = lastSwapNoticeMs, now &- t < 3_600_000 { return }
        lastSwapNoticeMs = now
        let top = procs.top(3).map { "\($0.name) (\(fmtBytes($0.footprint)))" }.joined(separator: ", ")
        onMain { $0.onNotice?(.swap(text: "Swap is growing about \(Int(perMin)) MB/min. Largest apps: \(top).")) }
        log(ActivityRow(tMs: now, title: "Warned: your Mac is swapping heavily",
                        why: "Swap grew about \(Int(perMin)) MB per minute while memory was busy",
                        detail: "Largest: \(top)", alert: true))
    }

    private func maybeAct(now: UInt64, state: PressureState) {
        guard !regret.isPending else { return }
        let s = sm.smoothed
        let helper = helperAvailable()
        let idle = idleHeavy(now: now).filter { settings.autoQuit.contains($0.name.lowercased()) }
        let allowNudge = settings.autoNudge && settings.profile.autoNudge && helper
        let pick = scheduler.pick(state: state, s: s, now: now, maxTier: 5) { id in
            switch id {
            case ActionID.nudge: return allowNudge
            case ActionID.quitIdle: return !idle.isEmpty && self.sustained(2 * 60_000, now)
            default: return false
            }
        }
        guard let id = pick else { return }
        execute(id, now: now, manual: false, target: idle.first)
    }

    /// Runs an action; nudges complete asynchronously through the helper.
    func execute(_ id: Int, now: UInt64, manual: Bool, target: AppRow? = nil) {
        if let i = scheduler.index(of: id) { scheduler.actions[i].markRun(now) }
        let why: String
        var title = ActionID.label(id)
        if manual {
            why = "You asked"
        } else if id == ActionID.quitIdle, let t = target {
            title = "Quit \(t.name)"
            why = "Idle for \(fmtHours(t.idleHours ?? 0)) holding \(fmtBytes(t.footprint)), and on your auto-quit list"
        } else {
            why = "Memory pressure stayed high; asked apps to release caches they can rebuild"
        }
        log(ActivityRow(tMs: now, title: title, why: why, manual: manual, isAction: true))
        let before = last
        regret.begin(tMs: now, actionId: id, manual: manual, state: sm.state,
                     hardCum: before.sample.hardFaultsCum, fastAvail: before.sample.fastAvailBytes, ledger: &ledger)
        actingUntil = now + 1_500
        switch id {
        case ActionID.nudge, ActionID.nudgeManual:
            onMain { ui in
                ui.runNudge?(0x2) { ok, msg in
                    self.queue.async {
                        self.ledger.updateLast { $0.detail = ok ? "apps notified, purgeable memory emptied" : msg }
                        self.nextSysMs = monotonicMs() + 2_000
                        self.reschedule()
                        self.publish()
                    }
                }
            }
        case ActionID.quitIdle:
            if let t = target {
                onMain { ui in
                    let ok = ui.quitApp?(t.bundlePath ?? t.name) ?? false
                    self.queue.async { self.ledger.updateLast { $0.detail = ok ? "quit \(t.name)" : "\(t.name) refused to quit" } }
                }
            }
        default:
            break
        }
        publish()
    }

    /// Records a manual action performed elsewhere (e.g. purge via helper).
    func recordManual(_ id: Int, detail: String) {
        queue.async {
            let now = monotonicMs()
            self.regret.begin(tMs: now, actionId: id, manual: true, state: self.sm.state,
                              hardCum: self.last.sample.hardFaultsCum, fastAvail: self.last.sample.fastAvailBytes,
                              detail: detail, ledger: &self.ledger)
            self.log(ActivityRow(tMs: now, title: ActionID.label(id), why: "You asked", manual: true, isAction: true))
            self.nextSysMs = now + 2_000
            self.reschedule()
            self.publish()
        }
    }

    private func log(_ row: ActivityRow) {
        activity.append(row)
        if activity.count > 40 { activity.removeFirst(activity.count - 40) }
    }

    func nudgeNow() {
        queue.async { self.execute(ActionID.nudgeManual, now: monotonicMs(), manual: true) }
    }

    private func tickProcesses(_ now: UInt64) {
        let flagged = procs.update(nowMs: now, settings: settings)
        let headroomMiB = last.total > 0 ? Double(last.free + last.cached) / MemCore.mib : .infinity
        for key in flagged {
            guard let g = procs.group(key), settings.notifyLeaks, !settings.ignores(g.name) else { continue }
            let eta = LeakDetector.etaHours(headroomMiB: headroomMiB, slope: g.leak.slope)
            let due: Bool
            if let t = g.leakNotifiedMs {
                due = now &- t >= 6 * 3_600_000 || (eta.map { $0 <= g.notifiedEtaH / 2 } ?? false)
            } else {
                due = true
            }
            guard due else { continue }
            procs.markNotified(key, nowMs: now, eta: eta)
            let runaway = g.leak.runaway && !g.leak.leak
            let n = Notice.leak(key: key, name: g.name, slope: g.leak.slope, etaH: eta,
                                runaway: runaway, bundlePath: g.bundlePath)
            onMain { $0.onNotice?(n) }
            log(ActivityRow(tMs: now,
                            title: runaway ? "Warned: \(g.name) is allocating very fast" : "Warned: \(g.name) looks like it's leaking",
                            why: "Grew steadily by about \(Int(max(0, g.leak.slope))) MB per hour",
                            detail: "Relaunching it frees the memory", alert: true))
        }
    }

    private func idleHeavy(now: UInt64) -> [AppRow] {
        let minBytes = settings.profile.idleAppMinBytes
        let hours = settings.profile.idleAppHours
        return procs.top(12).compactMap { g -> AppRow? in
            guard g.bundlePath != nil, g.footprint >= minBytes else { return nil }
            let lastActive = lastActivated[g.name.lowercased()] ?? launchDate
            let idleH = Date().timeIntervalSince(lastActive) / 3600
            guard idleH >= hours else { return nil }
            var row = Engine.row(g)
            row.idleHours = idleH
            return row
        }
    }

    static func row(_ g: AppGroup) -> AppRow {
        let enough = (g.hist?.count ?? 0) >= 10
        return AppRow(key: g.key, name: g.name, bundlePath: g.bundlePath, pids: g.pids, footprint: g.footprint,
                      slope: enough ? g.leak.slope : 0, leak: g.leak.leak, runaway: g.leak.runaway, idleHours: nil)
    }

    func snapshot() -> Snapshot {
        let now = monotonicMs()
        var s = Snapshot()
        s.tMs = now
        s.reading = last
        s.s = sm.smoothed
        s.state = sm.state
        s.history = history.elements
        s.historyApps = historyApps.elements
        s.historyCache = historyCache.elements
        if popupVisible {
            s.apps = procs.top(6).map { g in
                var r = Engine.row(g)
                if settings.ignores(g.name) { r.leak = false }
                return r
            }
            s.idleHeavy = sustained(2 * 60_000, now) ? idleHeavy(now: now) : []
            let entries = ledger.elements
            s.activity = activity.reversed().map { a in
                var r = a
                if a.isAction, let e = entries.last(where: { $0.tMs == a.tMs }) {
                    r.measured = true
                    r.freed = e.freedBytes
                    r.refaultRatio = e.refaultRatio
                    r.pending = e.pending
                    if r.detail.isEmpty { r.detail = e.detail }
                }
                return r
            }
        }
        s.paused = paused(now)
        s.acting = now < actingUntil
        s.visibleProcesses = procs.visibleProcesses
        return s
    }

    private func publish() {
        let snap = snapshot()
        onMain { $0.onSnapshot?(snap) }
    }

    private func onMain(_ f: @escaping (Engine) -> Void) {
        DispatchQueue.main.async { f(self) }
    }

    /// Our own memory pressure: drop history we can rebuild.
    func trimSelf() {
        queue.async {
            self.history.truncateOldest(keep: 24)
            self.historyApps.truncateOldest(keep: 24)
            self.historyCache.truncateOldest(keep: 24)
            self.swapHist.truncateOldest(keep: 8)
        }
    }

    /// Synchronous sampling for the self-test.
    func sampleOnce() -> (MacReading, Double, PressureState) {
        queue.sync {
            tickProcesses(monotonicMs())
            tickSystem(event: false)
            return (last, sm.smoothed, sm.state)
        }
    }
}
