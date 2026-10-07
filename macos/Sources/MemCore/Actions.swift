import Foundation

/// Tiered actions, token buckets, ledger and regret (policy-engine §5).
public struct ActionSpec: Equatable, Sendable {
    public var id: Int
    public var tier: Int
    public var minState: PressureState
    public var cooldownS: Double
    public var bucketCapacity: Double
    public var refillPerHour: Double
    public init(id: Int, tier: Int, minState: PressureState, cooldownS: Double, bucketCapacity: Double, refillPerHour: Double) {
        self.id = id
        self.tier = tier
        self.minState = minState
        self.cooldownS = cooldownS
        self.bucketCapacity = bucketCapacity
        self.refillPerHour = refillPerHour
    }
}

public struct TokenBucket: Equatable, Sendable {
    public private(set) var tokens: Double
    public let capacity: Double
    public let refillPerHour: Double
    private var lastMs: UInt64

    public init(capacity: Double, refillPerHour: Double, nowMs: UInt64) {
        tokens = capacity
        self.capacity = capacity
        self.refillPerHour = refillPerHour
        lastMs = nowMs
    }

    public mutating func refill(_ now: UInt64) {
        let dtH = Double(now >= lastMs ? now - lastMs : 0) / 3_600_000.0
        tokens = min(capacity, tokens + dtH * refillPerHour)
        lastMs = now
    }

    public mutating func available(_ now: UInt64) -> Bool {
        refill(now)
        return tokens >= 1.0
    }

    @discardableResult
    public mutating func take(_ now: UInt64) -> Bool {
        guard available(now) else { return false }
        tokens -= 1.0
        return true
    }
}

public struct ActionRuntime: Sendable {
    public static let maxPenalty = 16.0
    public let spec: ActionSpec
    public var bucket: TokenBucket
    public var lastRunMs: UInt64?
    public var penalty = 1.0
    private var goodStreak = 0
    public var enabled = true

    public init(spec: ActionSpec, nowMs: UInt64) {
        self.spec = spec
        bucket = TokenBucket(capacity: spec.bucketCapacity, refillPerHour: spec.refillPerHour, nowMs: nowMs)
    }

    public var threshold: Double { spec.minState.enterThreshold + 0.05 * log2(penalty) }

    public func cooldownOK(_ now: UInt64) -> Bool {
        guard let t = lastRunMs else { return true }
        return Double(now >= t ? now - t : 0) >= spec.cooldownS * penalty * 1000.0
    }

    public mutating func eligible(state: PressureState, s: Double, now: UInt64) -> Bool {
        enabled && state >= spec.minState && s >= threshold && cooldownOK(now) && bucket.available(now)
    }

    public mutating func markRun(_ now: UInt64) {
        bucket.take(now)
        lastRunMs = now
    }

    @discardableResult
    public mutating func applyRegret(_ ratio: Double) -> Double {
        if ratio > 0.25 {
            penalty = min(penalty * 2, ActionRuntime.maxPenalty)
            goodStreak = 0
        } else if ratio < 0.05 {
            goodStreak += 1
            if goodStreak >= 3 {
                penalty = max(penalty / 2, 1)
                goodStreak = 0
            }
        } else {
            goodStreak = 0
        }
        return penalty
    }
}

public struct Scheduler: Sendable {
    public var actions: [ActionRuntime]

    public init(specs: [ActionSpec], nowMs: UInt64) {
        actions = specs.map { ActionRuntime(spec: $0, nowMs: nowMs) }.sorted { $0.spec.tier < $1.spec.tier }
    }

    public func index(of id: Int) -> Int? { actions.firstIndex { $0.spec.id == id } }

    /// Lowest-tier eligible action whose platform predicate holds.
    public mutating func pick(state: PressureState, s: Double, now: UInt64, maxTier: Int,
                              predicate: (Int) -> Bool) -> Int? {
        for i in actions.indices {
            let tier = actions[i].spec.tier
            if tier > maxTier || tier == 0 { continue }
            if actions[i].eligible(state: state, s: s, now: now) && predicate(actions[i].spec.id) {
                return actions[i].spec.id
            }
        }
        return nil
    }
}

public struct LedgerEntry: Sendable {
    public var tMs: UInt64 = 0
    public var actionId = 0
    public var manual = false
    public var state: PressureState = .normal
    public var fastAvailBefore: UInt64 = 0
    public var fastAvailAfter: UInt64 = 0
    public var baselineHardRate = 0.0
    public var postHardPages = 0.0
    public var freedPages = 0.0
    public var refaultRatio = 0.0
    public var penaltyAfter = 1.0
    public var pending = false
    public var detail = ""
    public init() {}
    public var freedBytes: UInt64 { fastAvailAfter > fastAvailBefore ? fastAvailAfter - fastAvailBefore : 0 }
}

/// Measures each action's effect (policy-engine §5.1).
public struct RegretTracker: Sendable {
    public static let postWindowMs: UInt64 = 60_000
    public static let afterDelayMs: UInt64 = 2_000
    public static let baselineTauS = 120.0

    private var baseline: Double?
    private var last: (UInt64, UInt64)?
    private var pendingStart: UInt64?
    private var hardAtStart: UInt64 = 0
    private var afterSet = false
    private let pageSize: UInt64

    public init(pageSize: UInt64) { self.pageSize = max(1, pageSize) }

    public var baselineRate: Double { baseline ?? 0 }
    public var isPending: Bool { pendingStart != nil }

    /// Returns true when the pending measurement finished (see `ledger.last`).
    public mutating func observe(tMs: UInt64, hardCum: UInt64, fastAvail: UInt64,
                                 ledger: inout Ring<LedgerEntry>) -> Bool {
        if let l = last, pendingStart == nil {
            let (pt, ph) = l
            let dt = Double(tMs >= pt ? tMs - pt : 0) / 1000.0
            if dt > 0 {
                let rate = Double(hardCum >= ph ? hardCum - ph : 0) / dt
                if let b = baseline {
                    baseline = b + (1 - exp(-dt / RegretTracker.baselineTauS)) * (rate - b)
                } else {
                    baseline = rate
                }
            }
        }
        last = (tMs, hardCum)
        guard let start = pendingStart, !ledger.isEmpty else { return false }
        let elapsed = tMs >= start ? tMs - start : 0
        if !afterSet && elapsed >= RegretTracker.afterDelayMs {
            ledger.updateLast { $0.fastAvailAfter = fastAvail }
            afterSet = true
        }
        if elapsed >= RegretTracker.postWindowMs {
            finish(tMs: tMs, hardCum: hardCum, fastAvail: fastAvail, ledger: &ledger)
            return true
        }
        return false
    }

    private mutating func finish(tMs: UInt64, hardCum: UInt64, fastAvail: UInt64, ledger: inout Ring<LedgerEntry>) {
        guard let start = pendingStart else { return }
        let elapsedS = Double(tMs >= start ? tMs - start : 0) / 1000.0
        let raw = Double(hardCum >= hardAtStart ? hardCum - hardAtStart : 0)
        let setAfter = !afterSet
        let ps = Double(pageSize)
        ledger.updateLast { e in
            if setAfter { e.fastAvailAfter = fastAvail }
            e.postHardPages = max(0, raw - e.baselineHardRate * elapsedS)
            e.freedPages = Double(e.freedBytes) / ps
            e.refaultRatio = e.postHardPages / max(e.freedPages, 1)
            e.pending = false
        }
        pendingStart = nil
    }

    /// Records an action about to run; closes a pending measurement early.
    public mutating func begin(tMs: UInt64, actionId: Int, manual: Bool, state: PressureState,
                               hardCum: UInt64, fastAvail: UInt64, detail: String = "",
                               ledger: inout Ring<LedgerEntry>) {
        if pendingStart != nil {
            finish(tMs: tMs, hardCum: hardCum, fastAvail: fastAvail, ledger: &ledger)
        }
        var e = LedgerEntry()
        e.tMs = tMs
        e.actionId = actionId
        e.manual = manual
        e.state = state
        e.fastAvailBefore = fastAvail
        e.fastAvailAfter = fastAvail
        e.baselineHardRate = baselineRate
        e.pending = true
        e.detail = detail
        ledger.push(e)
        pendingStart = tMs
        hardAtStart = hardCum
        afterSet = false
    }
}
