// MemManager for macOS — entry point.
//
// Scaffolding only. Planned components (docs/architecture/macos.md):
// StatusItemController, PopoverController, Sampler, ProcessMonitor, Actions,
// HelperClient, SettingsWindow (SwiftUI, created on demand), Notifications.
// This binary currently builds the toolchain and module graph end to end and
// exits immediately.

import AppKit
import MemCore

_ = MemCore.specVersion
_ = NSApplication.shared
