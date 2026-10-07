import Foundation
import MemShared
import ServiceManagement

/// Registers and talks to the optional privileged helper (SMAppService.daemon + XPC).
final class HelperClient {
    private var connection: NSXPCConnection?

    var service: SMAppService { SMAppService.daemon(plistName: HelperConstants.plistName) }
    var status: SMAppService.Status { service.status }
    var isInstalled: Bool { status == .enabled }

    var statusText: String {
        switch status {
        case .enabled: return "Installed"
        case .requiresApproval: return "Waiting for approval in System Settings › Login Items"
        case .notRegistered: return "Not installed"
        case .notFound: return "Unavailable in this build (needs a signed app bundle)"
        @unknown default: return "Unknown"
        }
    }

    func install() -> String? {
        do {
            try service.register()
            if service.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
            return nil
        } catch {
            return error.localizedDescription
        }
    }

    func uninstall(_ done: @escaping (String?) -> Void) {
        connection?.invalidate()
        connection = nil
        service.unregister { error in DispatchQueue.main.async { done(error?.localizedDescription) } }
    }

    private func proxy(_ onError: @escaping (String) -> Void) -> MemHelperProtocol? {
        if connection == nil {
            let c = NSXPCConnection(machServiceName: HelperConstants.machService, options: .privileged)
            c.remoteObjectInterface = NSXPCInterface(with: MemHelperProtocol.self)
            c.invalidationHandler = { [weak self] in DispatchQueue.main.async { self?.connection = nil } }
            c.resume()
            connection = c
        }
        return connection?.remoteObjectProxyWithErrorHandler { err in
            DispatchQueue.main.async { onError(err.localizedDescription) }
        } as? MemHelperProtocol
    }

    func nudge(level: Int32, durationMs: Int = 3_000, _ done: @escaping (Bool, String) -> Void) {
        guard isInstalled else { done(false, "helper not installed"); return }
        proxy { done(false, $0) }?.nudge(level: level, durationMs: durationMs) { code, msg in
            DispatchQueue.main.async { done(code == 0, msg) }
        }
    }

    func purge(_ done: @escaping (Bool, String) -> Void) {
        guard isInstalled else { done(false, "helper not installed"); return }
        proxy { done(false, $0) }?.purgeDiskCache { code, msg in
            DispatchQueue.main.async { done(code == 0, msg) }
        }
    }
}
