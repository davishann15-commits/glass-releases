import AVFoundation
import MediaPlayer
import SwiftUI

/// The iPhone's own volume, which is Glass's volume while music plays on this iPhone:
/// the audio goes out at full scale, as other apps' does, so the side buttons, Control
/// Center and Glass's volume rail all move the same thing. Spotify Connect is told, so
/// other devices show it; a volume set from elsewhere (another device, the assistant,
/// mute) moves the iPhone's.
@MainActor
@Observable
final class SystemVolume {
    static let shared = SystemVolume()
    /// 0–1, as the audio session reports it.
    private(set) var level: Float = AVAudioSession.sharedInstance().outputVolume
    @ObservationIgnored private var observation: NSKeyValueObservation?
    @ObservationIgnored private weak var store: Store?
    /// The last level told to the core, and when.
    @ObservationIgnored private var sent: (level: Float, at: Date) = (-1, .distantPast)
    /// The core's volume in the last playback model.
    @ObservationIgnored private var published: Float?
    @ObservationIgnored private var here = false
    /// iOS lets an app set the volume only through a system volume view's slider: one
    /// is kept off screen while it's in use (which also keeps the system's volume
    /// overlay away while Glass shows the level itself).
    @ObservationIgnored private var setter: MPVolumeView?
    @ObservationIgnored private var removal: Task<Void, Never>?

    func start(store: Store) {
        self.store = store
        observation = AVAudioSession.sharedInstance().observe(\.outputVolume, options: [.new]) { [weak self] _, change in
            guard let v = change.newValue else { return }
            Task { @MainActor in self?.changed(v) }
        }
    }

    private func changed(_ v: Float) {
        level = v
        if here { tell(v) }
    }

    private func tell(_ v: Float) {
        sent = (v, Date())
        store?.core.setVolume(volume: v, commit: true)
    }

    /// Each playback model: while it plays here, the core's volume follows the iPhone's,
    /// and a change the core made (another device, the assistant, mute) moves the iPhone's.
    func follow(_ p: PlaybackModel?) {
        let wasHere = here
        here = p?.here ?? false
        defer { published = p?.volume }
        guard let p, here else { return }
        if !wasHere {
            level = AVAudioSession.sharedInstance().outputVolume
            tell(level)
            return
        }
        // Only a change the core made (not one it's echoing back from here).
        guard let before = published, p.volume != before, abs(p.volume - sent.level) > 0.01, Date().timeIntervalSince(sent.at) > 0.6 else { return }
        set(p.volume)
    }

    /// The level before muting from Glass's mute button.
    @ObservationIgnored private var unmuted: Float = 0.5

    func toggleMute() {
        if level > 0 {
            unmuted = level
            set(0)
        } else {
            set(unmuted)
        }
    }

    /// Moves the iPhone's volume.
    func set(_ v: Float) {
        let v = min(max(v, 0), 1)
        level = v
        sent = (v, Date())
        let view = setter ?? {
            let m = MPVolumeView(frame: CGRect(x: -2000, y: -2000, width: 120, height: 40))
            let window = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
            window?.addSubview(m)
            m.layoutIfNeeded()
            setter = m
            return m
        }()
        func apply() -> Bool {
            guard let slider = view.subviews.compactMap({ $0 as? UISlider }).first else { return false }
            slider.setValue(v, animated: false)
            slider.sendActions(for: .valueChanged)
            return true
        }
        // A new view makes its slider on its first layout pass.
        if !apply() {
            Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(60))
                _ = apply()
            }
        }
        if here { store?.core.setVolume(volume: v, commit: true) }
        removal?.cancel()
        removal = Task { @MainActor in
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            self.setter?.removeFromSuperview()
            self.setter = nil
        }
    }
}
