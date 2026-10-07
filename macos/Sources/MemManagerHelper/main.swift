// MemManagerHelper — optional privileged LaunchDaemon (docs/architecture/macos.md §5).
//
// Registered by the app through SMAppService.daemon. Exposes exactly two
// privileged operations over XPC to a code-signing-validated client.

import Foundation
import MemShared

let nudger = Nudger()
// Reset any pressure level a previous crash may have left latched.
nudger.recoverIfNeeded()
nudger.installSignalHandlers()

final class HelperService: NSObject, MemHelperProtocol {
    func nudge(level: Int32, durationMs: Int, reply: @escaping (Int32, String) -> Void) {
        let (code, msg) = nudger.nudge(level: level, durationMs: durationMs)
        reply(code, msg)
    }

    func purgeDiskCache(reply: @escaping (Int32, String) -> Void) {
        let (code, msg) = Purger.run()
        reply(code, msg)
    }

    func version(reply: @escaping (String) -> Void) {
        reply(HelperConstants.version)
    }
}

final class ListenerDelegate: NSObject, NSXPCListenerDelegate {
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection c: NSXPCConnection) -> Bool {
        c.setCodeSigningRequirement(ClientValidation.requirement())
        c.exportedInterface = NSXPCInterface(with: MemHelperProtocol.self)
        c.exportedObject = HelperService()
        c.invalidationHandler = { IdleExit.touch() }
        c.resume()
        IdleExit.touch()
        return true
    }
}

let delegate = ListenerDelegate()
let listener = NSXPCListener(machServiceName: HelperConstants.machService)
listener.delegate = delegate
listener.resume()
IdleExit.start()
RunLoop.main.run()
