import SwiftUI
import AVFoundation

@main
struct GlassApp: App {
    @State private var store: Store
    @State private var audio: SystemAudio
    @Environment(\.scenePhase) private var phase

    init() {
        // `-demo` (or GLASS_DEMO=1): the desktop's sample data, no account, no network.
        let args = ProcessInfo.processInfo
        let demo = args.arguments.contains("-demo") || args.environment["GLASS_DEMO"] != nil
        let store = Store(demo: demo)
        _store = State(initialValue: store)
        let audio = SystemAudio(store: store)
        _audio = State(initialValue: audio)
        SystemVolume.shared.start(store: store)
        store.onPlayback = { [weak store] p in
            TrackNotifier.shared.update(p, enabled: store?.settings?.trackNotifications ?? false)
            audio.update(playingHere: p.here && p.isPlaying)
            audio.keepAwake(reconnecting: p.reconnecting)
            audio.publish(p)
            SystemVolume.shared.follow(p)
        }
        Fixture.apply(to: store)
        #if DEBUG
        // `-probeSpotify <credentials.json>`: the read-only feasibility check (core probe.rs).
        let a = ProcessInfo.processInfo.arguments
        if let i = a.firstIndex(of: "-probeSpotify"), a.indices.contains(i + 1) {
            let path = a[i + 1]
            let engine = a.contains("-probeEngine")
            let core = store.core
            let play = a.contains("-probePlay")
            if play { try? AVAudioSession.sharedInstance().setCategory(.playback); try? AVAudioSession.sharedInstance().setActive(true) }
            Task.detached {
                let r = await core.debugProbeSpotify(credentialsPath: path, withEngine: engine, play: play)
                let out = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("probe.txt")
                try? r.write(to: out, atomically: true, encoding: .utf8)
                NSLog("PROBE done:\n%@", r)
            }
        }
        if let i = a.firstIndex(of: "-assistantMock"), i + 1 < a.count {
            store.core.debugAssistantMock(base: a[i + 1])
        }
        if a.contains("-probeTranslate") {
            Task.detached { await TranslateProbe.run() }
        }
        if a.contains("-probeHashes") {
            let core = store.core
            Task.detached { _ = await core.debugProbeHashes() }
        }
        if a.contains("-probeBonjour") {
            let core = store.core
            Task.detached { _ = await core.debugProbeBonjour(secs: 25) }
        }
        if a.contains("-probeSave") {
            let core = store.core
            let mp3 = a.contains("-mp3")
            Task.detached { _ = await core.debugProbeSave(mp3: mp3) }
        }
        #endif
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(store)
                .preferredColorScheme(.dark)
                .onChange(of: phase) { _, p in
                    store.core.setForeground(foreground: p == .active)
                }
                .onAppear {
                    store.core.setAccessibility(reduceMotion: UIAccessibility.isReduceMotionEnabled)
                    // Songs in picked folders that are only in iCloud (opened before the core started).
                    if !store.demo { CloudSongs.shared.fetch(PickedFolders.sources, store: store) }
                }
                .onReceive(NotificationCenter.default.publisher(for: UIAccessibility.reduceMotionStatusDidChangeNotification)) { _ in
                    store.core.setAccessibility(reduceMotion: UIAccessibility.isReduceMotionEnabled)
                }
        }
    }
}

/// Launch fixtures for checking layouts (like the desktop's GLASS_ROUTE, GLASS_RIGHT
/// and GLASS_NP_VIEW): `-route playlist|artist|album|home|search|liked|local|settings|nowplaying`,
/// `-pane lyrics|queue|playlist|devices`.
enum Fixture {
    @MainActor static func apply(to store: Store) {
        let a = ProcessInfo.processInfo.arguments
        func value(_ flag: String) -> String? { a.firstIndex(of: flag).flatMap { a.indices.contains($0 + 1) ? a[$0 + 1] : nil } }
        if let pane = value("-pane"), let p = NowPlayingPane(rawValue: pane) { store.nowPlayingView = p }
        // `-autoSignIn`: start Spotify's sign-in at launch (to check the sheet without tapping).
        if a.contains("-autoSignIn"), !store.demo {
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(1))
                store.core.signIn()
            }
        }
        switch value("-route") {
        case "home": store.tab = .home
        case "search": store.tab = .search
        case "local": store.tab = .local
        case "settings": store.tab = .settings
        case "playlist": store.paths[.library] = [.playlist(id: "p0")]
        case "artist": store.paths[.library] = [.artist(id: "a0")]
        case "album": store.paths[.library] = [.album(id: "al0")]
        case "liked": store.paths[.library] = [.likedSongs]
        case "nowplaying": store.nowPlayingOpen = true
        default: break
        }
    }
}
