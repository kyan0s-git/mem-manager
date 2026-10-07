// Platform-neutral core of the macOS memory manager.
//
// A 1:1 port of windows/core (Rust). Implements
// docs/architecture/policy-engine.md and must agree with the Rust
// implementation and spec/reference/policy_ref.py on every vector in
// spec/test-vectors/.

public enum MemCore {
    /// Version of the policy-engine spec this module implements.
    public static let specVersion = 1
    public static let mib: Double = 1024 * 1024
}
