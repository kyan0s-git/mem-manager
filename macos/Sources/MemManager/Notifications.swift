import AppKit
import UserNotifications

/// Leak / swap notifications with Quit, Relaunch and Ignore actions.
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let leakCategory = "LEAK"
    private var authorized: Bool?
    var onQuit: ((String) -> Void)?
    var onRelaunch: ((String) -> Void)?
    var onIgnore: ((String) -> Void)?

    /// UNUserNotificationCenter requires a bundled app.
    private var available: Bool { Bundle.main.bundleIdentifier != nil }

    func setUp() {
        guard available else { return }
        let c = UNUserNotificationCenter.current()
        c.delegate = self
        let quit = UNNotificationAction(identifier: "QUIT", title: "Quit", options: [.destructive])
        let relaunch = UNNotificationAction(identifier: "RELAUNCH", title: "Relaunch", options: [])
        let ignore = UNNotificationAction(identifier: "IGNORE", title: "Ignore this app", options: [])
        c.setNotificationCategories([
            UNNotificationCategory(identifier: Notifier.leakCategory, actions: [relaunch, quit, ignore], intentIdentifiers: []),
        ])
    }

    private func withAuthorization(_ f: @escaping () -> Void) {
        guard available else { return }
        if authorized == true { f(); return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { ok, _ in
            DispatchQueue.main.async {
                self.authorized = ok
                if ok { f() }
            }
        }
    }

    func post(title: String, body: String, category: String? = nil, target: String? = nil) {
        withAuthorization {
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            if let category { content.categoryIdentifier = category }
            if let target { content.userInfo = ["target": target] }
            let req = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
            UNUserNotificationCenter.current().add(req)
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler done: @escaping () -> Void) {
        let target = response.notification.request.content.userInfo["target"] as? String ?? ""
        DispatchQueue.main.async {
            switch response.actionIdentifier {
            case "QUIT": self.onQuit?(target)
            case "RELAUNCH": self.onRelaunch?(target)
            case "IGNORE": self.onIgnore?(target)
            default: break
            }
            done()
        }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .list])
    }
}
