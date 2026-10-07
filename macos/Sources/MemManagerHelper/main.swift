// MemManagerHelper — optional privileged LaunchDaemon (docs/architecture/macos.md §5).
//
// Scaffolding only. Planned: NSXPCListener on "dev.memmanager.helper" with a
// code-signing requirement on clients, exposing nudge(level:durationMs:),
// purgeDiskCache() and version(). Nudge must be latch-safe: marker file,
// bounded duration, KeepAlive-on-crash reset, and signal handlers.

import Foundation
