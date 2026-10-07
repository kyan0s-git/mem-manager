// MemManager for macOS — a complementary memory manager in the menu bar.
// See docs/architecture/macos.md.

import AppKit

let args = CommandLine.arguments.dropFirst()
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
