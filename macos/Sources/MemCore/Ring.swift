/// Fixed-capacity ring buffer: storage is allocated once.
public struct Ring<T> {
    private var buf: [T?]
    private var head = 0
    public private(set) var count = 0

    public init(capacity: Int) {
        precondition(capacity > 0)
        buf = Array(repeating: nil, count: capacity)
    }

    public var capacity: Int { buf.count }
    public var isEmpty: Bool { count == 0 }

    public mutating func push(_ v: T) {
        buf[head] = v
        head = (head + 1) % buf.count
        if count < buf.count { count += 1 }
    }

    public mutating func removeAll() {
        for i in buf.indices { buf[i] = nil }
        head = 0
        count = 0
    }

    /// Element `i` from the oldest (0) to the newest (count - 1).
    public subscript(i: Int) -> T {
        precondition(i >= 0 && i < count)
        let start = (head - count + buf.count) % buf.count
        return buf[(start + i) % buf.count]!
    }

    public var last: T? { count == 0 ? nil : self[count - 1] }
    public var first: T? { count == 0 ? nil : self[0] }

    public mutating func updateLast(_ f: (inout T) -> Void) {
        guard count > 0 else { return }
        let i = (head - 1 + buf.count) % buf.count
        var v = buf[i]!
        f(&v)
        buf[i] = v
    }

    /// Oldest → newest.
    public var elements: [T] { (0..<count).map { self[$0] } }

    /// Keeps only the newest `keep` elements.
    public mutating func truncateOldest(keep: Int) {
        if keep < count { count = keep }
    }
}
