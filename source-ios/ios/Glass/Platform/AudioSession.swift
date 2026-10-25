import AVFoundation
import MediaPlayer
import UIKit

/// iOS's side of playback: the audio session, interruptions and route changes,
/// and the system's Now Playing info and remote commands (lock screen, Control
/// Center, AirPods, CarPlay, Bluetooth). The audio itself is Glass's own: the
/// Rust core decodes and plays it (librespot → DSP → rodio → RemoteIO).
@MainActor
final class SystemAudio {
    private let store: Store
    private var observers: [NSObjectProtocol] = []
    private var lastInfo: [String: Any] = [:]
    private var artworkFor = ""
    private var commandsOn = false

    init(store: Store) {
        self.store = store
        configureSession()
        let nc = NotificationCenter.default
        let s = AVAudioSession.sharedInstance()
        observers.append(nc.addObserver(forName: AVAudioSession.interruptionNotification, object: s, queue: .main) { [weak self] n in
            MainActor.assumeIsolated { self?.interrupted(n) }
        })
        observers.append(nc.addObserver(forName: AVAudioSession.routeChangeNotification, object: s, queue: .main) { [weak self] n in
            MainActor.assumeIsolated { self?.routeChanged(n) }
        })
        observers.append(nc.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: s, queue: .main) { [weak self] _ in
            // Everything audio was torn down: set the session up again, and reopen outputs.
            MainActor.assumeIsolated {
                self?.configureSession()
                self?.store.core.audioRouteChanged(gone: false)
            }
        })
    }

    /// Playback (not mixable): Glass is a music player, keeps playing locked or in
    /// the background (UIBackgroundModes audio), and stops others when it starts.
    private func configureSession() {
        let s = AVAudioSession.sharedInstance()
        do {
            try s.setCategory(.playback, mode: .default, policy: .longFormAudio)
        } catch {
            NSLog("Glass: audio session category: \(error)")
        }
    }

    /// Active while this device plays; let go when it doesn't (so other apps can resume
    /// and iOS can suspend Glass). Not at once: a pause or a track change is often
    /// followed by playing again, and a paused local file keeps its output for 30 s
    /// (the core lets it go then; no silent output is kept to stay awake).
    func update(playingHere: Bool) {
        let s = AVAudioSession.sharedInstance()
        release?.cancel()
        release = nil
        if playingHere {
            try? s.setActive(true)
            return
        }
        release = Task { @MainActor in
            try? await Task.sleep(for: .seconds(35))
            guard !Task.isCancelled else { return }
            do {
                try s.setActive(false, options: .notifyOthersOnDeactivation)
            } catch {
                // Still busy (something reopened the output): try again on the next pause.
                NSLog("Glass: audio session stays active: \(error)")
            }
        }
    }

    private var release: Task<Void, Never>?
    private var reconnect: UIBackgroundTaskIdentifier = .invalid

    /// While playback here waits for a new engine (its connection died, e.g. after a
    /// long time in the background), Glass asks iOS for time: with nothing playing,
    /// iOS would suspend it before it reconnects and carries on.
    func keepAwake(reconnecting: Bool) {
        if reconnecting, reconnect == .invalid {
            reconnect = UIApplication.shared.beginBackgroundTask(withName: "Reconnect playback") { [weak self] in
                MainActor.assumeIsolated { self?.endReconnect() }
            }
        } else if !reconnecting {
            endReconnect()
        }
    }

    private func endReconnect() {
        guard reconnect != .invalid else { return }
        UIApplication.shared.endBackgroundTask(reconnect)
        reconnect = .invalid
    }

    private func interrupted(_ n: Notification) {
        guard let info = n.userInfo, let raw = info[AVAudioSessionInterruptionTypeKey] as? UInt, let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
        switch type {
        case .began:
            store.core.audioInterrupted(began: true, resume: false)
        case .ended:
            let opts = (info[AVAudioSessionInterruptionOptionKey] as? UInt).map(AVAudioSession.InterruptionOptions.init) ?? []
            if opts.contains(.shouldResume) { try? AVAudioSession.sharedInstance().setActive(true) }
            store.core.audioInterrupted(began: false, resume: opts.contains(.shouldResume))
        @unknown default:
            break
        }
    }

    /// Headphones out (or AirPods gone): pause, don't carry on through the speaker.
    private func routeChanged(_ n: Notification) {
        guard let raw = n.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt, let reason = AVAudioSession.RouteChangeReason(rawValue: raw) else { return }
        switch reason {
        case .oldDeviceUnavailable:
            store.core.audioRouteChanged(gone: true)
        case .newDeviceAvailable, .override, .routeConfigurationChange:
            store.core.audioRouteChanged(gone: false)
        default:
            break
        }
    }

    // MARK: Now Playing and remote commands

    /// Owns the system's Now Playing only while this device plays (here or a local
    /// file): when another device plays, the system's controls belong to that
    /// device's app, and Glass mustn't claim them.
    func publish(_ p: PlaybackModel?) {
        let center = MPNowPlayingInfoCenter.default()
        guard let p, p.here, let item = p.item else {
            if !lastInfo.isEmpty {
                center.nowPlayingInfo = nil
                center.playbackState = .stopped
                lastInfo = [:]
            }
            return
        }
        enableCommands()
        var info: [String: Any] = [
            MPMediaItemPropertyTitle: item.title,
            MPMediaItemPropertyArtist: item.kind == .episode ? (item.show?.name ?? "") : item.artists.map(\.name).joined(separator: ", "),
            MPMediaItemPropertyPlaybackDuration: Double(item.durationMs) / 1000,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: Double(store.position()) / 1000,
            MPNowPlayingInfoPropertyPlaybackRate: p.isPlaying ? 1.0 : 0.0,
            MPNowPlayingInfoPropertyDefaultPlaybackRate: 1.0,
            MPNowPlayingInfoPropertyMediaType: MPNowPlayingInfoMediaType.audio.rawValue,
        ]
        if let album = item.album?.name { info[MPMediaItemPropertyAlbumTitle] = album }
        if let art = lastInfo[MPMediaItemPropertyArtwork], artworkFor == item.uri { info[MPMediaItemPropertyArtwork] = art }
        center.nowPlayingInfo = info
        center.playbackState = p.isPlaying ? .playing : .paused
        lastInfo = info
        let commands = MPRemoteCommandCenter.shared()
        let episode = item.kind == .episode
        commands.skipForwardCommand.isEnabled = episode
        commands.skipBackwardCommand.isEnabled = episode
        commands.nextTrackCommand.isEnabled = !episode
        commands.previousTrackCommand.isEnabled = !episode
        if artworkFor != item.uri, let url = item.image {
            artworkFor = item.uri
            let uri = item.uri
            Task { @MainActor in
                guard let img = await ImageCache.shared.load(url, px: 600), self.artworkFor == uri else { return }
                let art = MPMediaItemArtwork(boundsSize: img.size) { _ in img }
                var info = MPNowPlayingInfoCenter.default().nowPlayingInfo ?? [:]
                info[MPMediaItemPropertyArtwork] = art
                MPNowPlayingInfoCenter.default().nowPlayingInfo = info
                self.lastInfo = info
            }
        }
    }

    /// Play and Pause mean exactly that (AirPods send Pause when they come out, and
    /// that must never start paused music); toggle toggles. Seeks go to the core.
    private func enableCommands() {
        guard !commandsOn else { return }
        commandsOn = true
        let c = MPRemoteCommandCenter.shared()
        let core = store.core
        c.playCommand.addTarget { _ in core.play(); return .success }
        c.pauseCommand.addTarget { _ in core.pause(); return .success }
        c.togglePlayPauseCommand.addTarget { _ in core.togglePlay(); return .success }
        c.stopCommand.addTarget { _ in core.pause(); return .success }
        c.nextTrackCommand.addTarget { _ in core.next(); return .success }
        c.previousTrackCommand.addTarget { _ in core.previous(); return .success }
        c.skipForwardCommand.preferredIntervals = [15]
        c.skipBackwardCommand.preferredIntervals = [15]
        c.skipForwardCommand.addTarget { _ in core.seekBy(deltaMs: 15000); return .success }
        c.skipBackwardCommand.addTarget { _ in core.seekBy(deltaMs: -15000); return .success }
        c.changePlaybackPositionCommand.addTarget { e in
            guard let e = e as? MPChangePlaybackPositionCommandEvent else { return .commandFailed }
            core.seek(positionMs: UInt64(max(e.positionTime, 0) * 1000))
            return .success
        }
        c.likeCommand.isEnabled = false
        c.dislikeCommand.isEnabled = false
        c.bookmarkCommand.isEnabled = false
        c.changePlaybackRateCommand.isEnabled = false
    }
}
