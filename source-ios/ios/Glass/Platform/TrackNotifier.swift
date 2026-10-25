import UserNotifications

/// "Show notifications when the song changes" (desktop `on_track_changed` → the
/// system's notifications): the title and artists of each new song, not the first
/// one. iOS asks for permission when the setting is turned on.
@MainActor
final class TrackNotifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = TrackNotifier()
    private var lastURI = ""

    override init() {
        super.init()
        UNUserNotificationCenter.current().delegate = self
    }

    func requestPermission() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert]) { _, _ in }
    }

    /// Called with each playback update.
    func update(_ p: PlaybackModel?, enabled: Bool) {
        guard let item = p?.item, !item.uri.isEmpty else { return }
        let uri = item.uri
        defer { lastURI = uri }
        guard enabled, !lastURI.isEmpty, uri != lastURI else { return }
        let c = UNMutableNotificationContent()
        c.title = item.title
        c.body = item.artists.map(\.name).joined(separator: ", ")
        // One at a time: each song replaces the last one's notification.
        let r = UNNotificationRequest(identifier: "glass-track", content: c, trigger: nil)
        UNUserNotificationCenter.current().add(r)
    }

    /// Not while Glass is open: the song is on screen already (it goes to the
    /// notification list only).
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.list]
    }
}
