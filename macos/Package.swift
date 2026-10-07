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
        // C shim over libproc, Mach VM statistics and sysctl.
        .target(name: "MemSys"),
        // XPC protocol and constants shared by the app and the helper.
        .target(name: "MemShared"),
        // Unprivileged LSUIElement agent: status item, popover, sampler, actions.
        .executableTarget(name: "MemManager", dependencies: ["MemCore", "MemSys", "MemShared"]),
        // Optional root LaunchDaemon registered via SMAppService (Nudge, Purge).
        .executableTarget(name: "MemManagerHelper", dependencies: ["MemShared", "MemSys"]),
        .testTarget(name: "MemCoreTests", dependencies: ["MemCore"]),
    ]
)
