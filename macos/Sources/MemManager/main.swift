// MemManager for macOS — a complementary memory manager in the menu bar.
// See docs/architecture/macos.md.

import AppKit

let args = CommandLine.arguments.dropFirst()
func argValue(_ prefix: String) -> String? {
    args.first(where: { $0.hasPrefix(prefix) }).map { String($0.dropFirst(prefix.count)) }
}
if let mib = argValue("--bench-child=").flatMap({ Int($0) }) {
    Bench.child(mib: mib, id: argValue("--bench-id=").flatMap { Int($0) } ?? 0,
                cooperative: args.contains("--cooperative"), parent: argValue("--bench-parent=") ?? "0")
}
if args.contains("--bench") {
    exit(Bench.run(out: argValue("--out=")))
}
if let st = args.first(where: { $0.hasPrefix("--selftest") }) {
    let secs = st.split(separator: "=").dropFirst().first.flatMap { Int($0) } ?? 10
    let out = args.first(where: { $0.hasPrefix("--out=") }).map { String($0.dropFirst(6)) }
    exit(SelfTest.run(seconds: secs, out: out))
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let controller = AppController()
app.delegate = controller
app.run()
