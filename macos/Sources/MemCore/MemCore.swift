// Platform-neutral core of the macOS memory manager.
//
// Implements docs/architecture/policy-engine.md and must agree with the Rust
// implementation (windows/core) on every vector in spec/test-vectors/.
//
// Scaffolding only: the planned files are Ring.swift, Sample.swift,
// StateMachine.swift, Actions.swift, Profile.swift and LeakDetector.swift.

public enum MemCore {
    /// Version of the policy-engine spec this module implements.
    public static let specVersion = 1
}
