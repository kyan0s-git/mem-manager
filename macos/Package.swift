// swift-tools-version:5.9
// MemManager for macOS — see docs/architecture/macos.md §2.
import PackageDescription

let package = Package(
    name: "MemManager",
    platforms: [.macOS(.v13)],
    products: [
        .executable(name: "MemManager", targets: ["MemManager"]),
        .executable(name: "MemManagerHelper", targets: ["MemManagerHelper"]),
        .library(name: "MemCore", targets: ["MemCore"]),
    ],
    targets: [
        // Platform-neutral policy engine + leak detector (docs/architecture/policy-engine.md).
        .target(name: "MemCore"),
        // Unprivileged LSUIElement agent: status item, popover, sampler, actions.
        .executableTarget(name: "MemManager", dependencies: ["MemCore"]),
        // Optional root LaunchDaemon registered via SMAppService (Nudge, Purge).
        .executableTarget(name: "MemManagerHelper"),
        .testTarget(name: "MemCoreTests", dependencies: ["MemCore"]),
    ]
)
