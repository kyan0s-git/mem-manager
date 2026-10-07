import AppKit
import MemCore
import ServiceManagement
import SwiftUI

/// SwiftUI settings, hosted in a window that is created on open and released
/// on close so the SwiftUI view graph doesn't stay resident.
struct SettingsView: View {
    @ObservedObject var settings = AppSettings.shared
    let helper: HelperClient
    let onNudge: () -> Void
    let onPurge: () -> Void
    @State private var helperStatus = ""
    @State private var helperError: String?
    @State private var loginItem = SMAppService.mainApp.status == .enabled
    @State private var confirmPurge = false

    private func colorBinding(_ i: Int) -> Binding<Color> {
        Binding(
            get: { Color(nsColor: settings.customColors[i]) },
            set: { c in
                var cs = settings.customColors
                cs[i] = NSColor(c)
                settings.customColors = cs
                settings.preset = .custom
            }
        )
    }

    private func listBinding(_ kp: ReferenceWritableKeyPath<AppSettings, [String]>) -> Binding<String> {
        Binding(
            get: { settings[keyPath: kp].joined(separator: ", ") },
            set: { settings[keyPath: kp] = $0.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces).lowercased() }.filter { !$0.isEmpty } }
        )
    }

    var body: some View {
        Form {
            Section("Menu bar") {
                Picker("Style", selection: $settings.iconStyle) {
                    ForEach(IconStyle.allCases) { Text($0.label).tag($0) }
                }
                Picker("Shows", selection: $settings.iconMetric) {
                    ForEach(IconMetric.allCases) { Text($0.label).tag($0) }
                }
                Picker("Colors", selection: $settings.preset) {
                    ForEach(ColorPreset.allCases) { Text($0.label).tag($0) }
                }
                HStack {
                    Text("State colors")
                    Spacer()
                    ForEach(0..<4, id: \.self) { i in
                        ColorPicker(PressureState(rawValue: i)!.name, selection: colorBinding(i), supportsOpacity: false)
                            .labelsHidden()
                            .help(PressureState(rawValue: i)!.name)
                    }
                }
            }
            Section {
                Picker("Profile", selection: $settings.profile) {
                    ForEach(Profile.allCases, id: \.self) { Text($0.label).tag($0) }
                }
                .pickerStyle(.segmented)
                Toggle("Nudge automatically at high pressure (Aggressive, needs helper)", isOn: $settings.autoNudge)
                    .disabled(settings.profile != .aggressive)
                TextField("Quit when idle under pressure", text: listBinding(\.autoQuit), prompt: Text("app names, comma separated"))
                TextField("Never alert about leaks", text: listBinding(\.ignoreLeaks), prompt: Text("app names, comma separated"))
            } header: {
                Text("Behavior")
            } footer: {
                Text("macOS already manages memory well. MemManager watches pressure, finds leaking apps, and only nudges the system cooperatively.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Notifications") {
                Toggle("Memory leak alerts", isOn: $settings.notifyLeaks)
                Toggle("Swap growth warnings", isOn: $settings.notifySwap)
            }
            Section {
                LabeledContent("Status", value: helperStatus)
                if let helperError { Text(helperError).font(.caption).foregroundStyle(.red) }
                HStack {
                    if helper.isInstalled {
                        Button("Nudge now", action: onNudge)
                        Button("Purge disk cache…") { confirmPurge = true }
                        Spacer()
                        Button("Remove helper") {
                            helper.uninstall { err in helperError = err; refresh() }
                        }
                    } else {
                        Button("Install helper…") { helperError = helper.install(); refresh() }
                    }
                }
            } header: {
                Text("Privileged helper (optional)")
            } footer: {
                Text("Nudge briefly signals memory pressure so apps drop their caches and purgeable memory is emptied, then always restores the real level. Purge flushes the disk cache — useful for benchmarks, not for speed.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("General") {
                Toggle("Open at login", isOn: $loginItem)
                    .onChange(of: loginItem) { on in
                        do {
                            if on { try SMAppService.mainApp.register() } else { try SMAppService.mainApp.unregister() }
                        } catch {
                            helperError = error.localizedDescription
                        }
                    }
                LabeledContent("Version", value: Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "dev")
            }
        }
        .formStyle(.grouped)
        .frame(width: 480)
        .frame(minHeight: 560)
        .onAppear(perform: refresh)
        .confirmationDialog("Purge the disk cache?", isPresented: $confirmPurge) {
            Button("Purge", role: .destructive, action: onPurge)
        } message: {
            Text("Files will be read from disk again the next time apps use them.")
        }
    }

    private func refresh() {
        helperStatus = helper.statusText
        loginItem = SMAppService.mainApp.status == .enabled
    }
}

final class SettingsWindowController: NSObject, NSWindowDelegate {
    private var window: NSWindow?
    var onClose: (() -> Void)?

    func show(helper: HelperClient, onNudge: @escaping () -> Void, onPurge: @escaping () -> Void) {
        if window == nil {
            let host = NSHostingController(rootView: SettingsView(helper: helper, onNudge: onNudge, onPurge: onPurge))
            let w = NSWindow(contentViewController: host)
            w.title = "MemManager Settings"
            w.styleMask = [.titled, .closable]
            w.isReleasedWhenClosed = false
            w.delegate = self
            w.center()
            window = w
        }
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        // Drop the SwiftUI hierarchy entirely.
        window?.contentViewController = nil
        window = nil
        onClose?()
    }
}
