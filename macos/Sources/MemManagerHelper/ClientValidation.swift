import Foundation
import MemShared
import Security

/// Builds the code-signing requirement clients must satisfy: the MemManager
/// app identifier, signed by the same team as this helper.
enum ClientValidation {
    static func ownTeamID() -> String? {
        var code: SecCode?
        guard SecCodeCopySelf([], &code) == errSecSuccess, let code else { return nil }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else { return nil }
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
              let dict = info as? [String: Any] else { return nil }
        return dict[kSecCodeInfoTeamIdentifier as String] as? String
    }

    static func requirement() -> String {
        let id = "identifier \"\(HelperConstants.appBundleID)\""
        if let team = ownTeamID() {
            return "anchor apple generic and \(id) and certificate leaf[subject.OU] = \"\(team)\""
        }
        // Unsigned/ad-hoc development builds: identifier only.
        return id
    }
}
