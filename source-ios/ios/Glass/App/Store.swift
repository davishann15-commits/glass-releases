import SwiftUI
import Observation

/// The app's one store. Every screen reads from it; it changes only by applying
/// the core's events (on the main actor). Commands go to `core`; the core owns
/// the real state (playback, library, caches) and sends back what changed.
@Observable
@MainActor
final class Store {
    let core: GlassCore
    let demo: Bool

    var account = AccountModel(phase: .signedOut, error: nil, userId: "", displayName: "", email: "", country: "", product: "", image: nil, demo: false)
    var playback: PlaybackModel?
    var queue: QueueModel?
    var devices: DevicesModel?
    var library: LibraryModel?
    /// Pages on screen, by route key.
    var pages: [String: PageModel] = [:]
    /// Saved / liked / followed, by URI (missing: not known yet).
    var saved: [String: Bool] = [:]
    var toast: Toast?
    var lyrics: LyricsModel?
    var theme: ThemeModel?
    var settings: SettingsModel?
    var local: LocalModel?
    var saves: SavesModel?
    var share: ShareModel?
    /// The system's side of each playback change (audio session, lock screen, volume).
    @ObservationIgnored var onPlayback: ((PlaybackModel) -> Void)?
    /// Another device's Local Files, while its page is open.
    var deviceLibrary: DeviceLibraryModel?
    /// Add to Apple Music: the pairing file, a run's progress, its result.
    var appleMusic: AppleMusicModel?
    /// Local Files songs picked for Add to Apple Music (nil: not picking).
    var appleMusicPicks: Set<String>?
    var assistant: AssistantModel?
    /// The assistant's sheet (iPhone) or column (iPad).
    var assistantOpen = false
    /// The command bar (desktop ⌘K), as a sheet.
    var commandOpen = false
    /// The ChatGPT sign-in page to show in the web sign-in sheet.
    var assistantSignIn: SignInRequest?
    /// The tag editor's last save (the editor closes, or shows the error).
    var tagsResult: TagsResult?
    var config: ConfigModel?
    var status: StatusModel?

    // Navigation: one stack per tab, so each keeps its place.
    var tab: Tab = .library
    var paths: [Tab: [Route]] = [:]
    var nowPlayingOpen = false
    /// The theme panel (desktop ⌘T): theme.toml's [theme], [lyrics] and [visualizer].
    var themePanel = false
    var nowPlayingView: NowPlayingPane = .lyrics
    /// A modal sheet the app shows (dialogs ported from the desktop's modals).
    var sheet: SheetKind?
    /// "Add local files…" is choosing files for this.
    var addFilesTarget: FileTarget?
    var confirm: Confirm?
    /// Sign-in requested by the core: SignInSession presents it.
    var signInRequest: SignInRequest?

    // Library screen state (kept across navigation, as the desktop keeps it).
    var libTab: Int = 0
    var libChip: LibChip = .playlists
    var libQuery = ""
    var expandedFolders: Set<String> = []

    /// The discography chips' choice (artist page and Discography, as the desktop keeps it).
    var discoFilter = 0

    // Search screen state.
    var searchText = ""
    var searchFilter: SearchFilter = .all

    // Settings screen state.
    var settingsQuery = ""

    private var subscriptions: [String: Int] = [:]
    private var pendingSaved: Set<String> = []
    private var savedFlush: Task<Void, Never>?

    init(demo: Bool) {
        self.demo = demo
        let fm = FileManager.default
        let support = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Glass")
        let caches = fm.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("Glass")
        let docs = fm.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let config = CoreConfig(dataDir: support.path, cacheDir: caches.path, documentsDir: docs.path, demo: demo, deviceName: UIDevice.current.model == "iPad" ? "Glass iPad" : "Glass iPhone", language: Locale.preferredLanguages.first ?? "en", reduceMotion: UIAccessibility.isReduceMotionEnabled, pickedFolders: demo ? PickedFolders.openForDemo() : PickedFolders.open())
        let listener = Listener()
        core = GlassCore(config: config, listener: listener, keychain: demo ? nil : KeychainStore(), translator: AppleTranslator())
        listener.store = self
    }

    // MARK: events

    func apply(_ e: CoreEvent) {
        switch e {
        case let .account(account):
            self.account = account
        case let .playback(playback):
            if playback.item?.uri != self.playback?.item?.uri { prefetchCovers(playback.item) }
            self.playback = playback
            // Here, not in a view: views don't update while Glass is in the background,
            // and the lock screen, audio session and volume must follow every change.
            onPlayback?(playback)
        case let .queue(queue):
            self.queue = queue
            // The next songs' covers, at Now Playing's and the player bar's sizes.
            for (i, url) in queue.prefetch.enumerated() {
                ImageCache.shared.prefetch(url, px: i % 2 == 0 ? Store.nowPlayingArtPx : Store.barArtPx)
            }
        case let .devices(devices):
            self.devices = devices
        case let .library(library):
            self.library = library
        case let .page(key, page):
            pages[key] = page
        case let .saved(changes):
            for c in changes { saved[c.uri] = c.saved }
        case let .toast(text):
            show(toast: text)
        case let .navigate(route):
            nowPlayingOpen = false
            push(route)
        case let .left(route):
            for t in Tab.allCases {
                paths[t]?.removeAll { $0 == route }
            }
        case .showNowPlaying:
            nowPlayingOpen = true
        case let .lyrics(lyrics):
            self.lyrics = lyrics
        case let .share(share):
            self.share = share
        case let .deviceLibrary(library):
            deviceLibrary = library
        case let .appleMusic(appleMusic):
            self.appleMusic = appleMusic
        case let .assistant(assistant):
            self.assistant = assistant
        case let .openAssistantSignIn(request):
            assistantSignIn = request
        case let .saves(saves):
            self.saves = saves
        case let .local(local):
            self.local = local
        case let .tagsSaved(path, error):
            tagsResult = TagsResult(path: path, error: error)
        case let .settings(settings, config, status):
            self.settings = settings
            self.config = config
            self.status = status
        case let .albumColors(colors):
            if Look.shared.album != colors {
                withAnimation(Look.shared.colorAnimation) { Look.shared.album = colors }
            }
        case let .theme(theme):
            self.theme = theme
            Look.shared.apply(theme)
        case let .openSignIn(request):
            signInRequest = request
        case .signedOut:
            pages = [:]
            saved = [:]
            playback = nil
            queue = nil
            devices = nil
            library = nil
            paths = [:]
        }
    }

    func show(toast text: String) {
        let t = Toast(text: text)
        toast = t
        // Long enough to read: 3 s, more for a longer one (up to 7 s).
        let secs = min(7, max(3, Double(text.count) / 14))
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(secs))
            if toast?.id == t.id { toast = nil }
        }
    }

    // MARK: navigation

    func push(_ r: Route) {
        switch r {
        case .library: tab = .library; paths[.library] = []
        case .home: tab = .home; paths[.home] = []
        case .localFiles: tab = .local; paths[.local] = []
        case .settings: tab = .settings; paths[.settings] = []
        case let .search(query, filter) where (paths[tab] ?? []).isEmpty && tab == .search:
            searchText = query
            searchFilter = filter
        default:
            paths[tab, default: []].append(r)
        }
    }

    func path(_ t: Tab) -> Binding<[Route]> {
        Binding(get: { self.paths[t] ?? [] }, set: { self.paths[t] = $0 })
    }

    // MARK: pages

    /// A page view appeared: the core builds it now and rebuilds it as its data changes.
    /// Counted, so two views of one page (a split view) share a subscription.
    func subscribe(_ r: Route) {
        let k = routeKey(route: r)
        subscriptions[k, default: 0] += 1
        if subscriptions[k] == 1 { core.subscribe(route: r) }
    }

    func unsubscribe(_ r: Route) {
        let k = routeKey(route: r)
        subscriptions[k, default: 1] -= 1
        if subscriptions[k] == 0 {
            subscriptions[k] = nil
            core.unsubscribe(route: r)
        }
    }

    func page(_ r: Route) -> PageModel? { pages[routeKey(route: r)] }

    /// A row with a heart (or a Save/Follow button) came on screen: ask once, in batches.
    func wantSaved(_ uri: String) {
        guard !uri.isEmpty, saved[uri] == nil, !pendingSaved.contains(uri) else { return }
        pendingSaved.insert(uri)
        savedFlush?.cancel()
        savedFlush = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled else { return }
            let uris = Array(pendingSaved)
            pendingSaved.removeAll()
            core.checkSaved(uris: uris)
        }
    }

    func isSaved(_ uri: String) -> Bool { saved[uri] ?? false }

    func toggleSaved(_ uri: String) {
        core.save(uris: [uri], save: !isSaved(uri))
    }

    // MARK: settings

    /// Edits settings.json through the core (the whole model goes back, changed).
    func editSettings(_ change: (inout SettingsModel) -> Void) {
        guard var s = settings else { return }
        change(&s)
        settings = s
        core.setSettings(settings: s)
    }

    // MARK: playback helpers

    var isPlaying: Bool { playback?.isPlaying ?? false }

    /// The position now (the core sends a position and when it was taken).
    func position(at date: Date = .now) -> UInt64 {
        guard let p = playback else { return 0 }
        var pos = p.positionMs
        if p.isPlaying, p.positionAt > 0 {
            let now = UInt64(date.timeIntervalSince1970 * 1000)
            pos += now > p.positionAt ? now - p.positionAt : 0
        }
        if let d = p.item?.durationMs, d > 0 { pos = min(pos, d) }
        return pos
    }

    func isCurrent(_ uri: String) -> Bool { !uri.isEmpty && playback?.item?.uri == uri }

    func isPlayingContext(_ uri: String) -> Bool { !uri.isEmpty && playback?.contextUri == uri }

    /// Play/pause a context: toggles when it's the one playing (desktop action row).
    func playContext(_ uri: String) {
        if isPlayingContext(uri) { core.togglePlay() } else { core.playContext(context: uri, track: nil) }
    }

    /// Play a row from a list: in its context when there is one, else the list's songs from it.
    func play(rows: [Row], index: Int, context: String) {
        guard index < rows.count else { return }
        let r = rows[index]
        if isCurrent(r.uri) { core.togglePlay(); return }
        if r.kind == .localFile, let path = r.path {
            let paths = rows.compactMap { $0.kind == .localFile ? $0.path : nil }
            core.playLocal(paths: paths, start: UInt32(paths.firstIndex(of: path) ?? 0))
            return
        }
        if !context.isEmpty {
            core.playContext(context: context, track: r.uri)
        } else {
            let playable = rows.filter { $0.playable && !$0.uri.isEmpty }
            core.playUris(uris: playable.map(\.uri), index: UInt32(playable.firstIndex { $0.uri == r.uri } ?? 0))
        }
    }

    // MARK: listener

    final class Listener: CoreListener, @unchecked Sendable {
        weak var store: Store?
        func onEvent(event: CoreEvent) {
            Task { @MainActor [weak self] in self?.store?.apply(event) }
        }
    }
}

enum Tab: String, CaseIterable, Hashable {
    case home, library, search, local, settings
    var title: String {
        switch self {
        case .home: "home"
        case .library: "library"
        case .search: "search"
        case .local: "local"
        case .settings: "settings"
        }
    }
    var icon: String {
        switch self {
        case .home: "house"
        case .library: "square.stack"
        case .search: "magnifyingglass"
        case .local: "externaldrive"
        case .settings: "gearshape"
        }
    }
}

enum LibChip: String, CaseIterable, Hashable {
    case playlists, albums, artists, shows, audiobooks, downloaded
}

enum NowPlayingPane: String, CaseIterable, Hashable {
    case lyrics, queue, playlist, devices
}

struct TagsResult: Equatable {
    let path: String
    let error: String?
}

struct Toast: Equatable {
    let id = UUID()
    let text: String
}

/// A question before something that can't be undone (desktop `Modal::Confirm`).
struct Confirm: Identifiable {
    let id = UUID()
    let title: String
    let body: String
    let button: String
    let run: () -> Void
}

enum SheetKind: Identifiable {
    case editPlaylist(id: String, name: String, description: String, isPublic: Bool, collaborative: Bool, image: String?)
    case newFolder(add: String?)
    case renameFolder(id: String, name: String)
    case newPlaylist(uris: [String])
    case addToPlaylist(uris: [String])
    case moveToFolder(playlist: String)
    case about
    case shortcuts
    /// Add to Apple Music for these songs (none: the setup), into `playlist` if named.
    case appleMusic(paths: [String], playlist: String?)

    var id: String {
        switch self {
        case let .editPlaylist(id, _, _, _, _, _): "edit:\(id)"
        case let .newFolder(add): "folder:\(add ?? "")"
        case let .renameFolder(id, _): "rename:\(id)"
        case let .newPlaylist(uris): "new:\(uris.count)"
        case let .addToPlaylist(uris): "add:\(uris.joined())"
        case let .moveToFolder(p): "move:\(p)"
        case .about: "about"
        case .shortcuts: "shortcuts"
        case let .appleMusic(paths, playlist): "applemusic:\(paths.count):\(playlist ?? "")"
        }
    }
}

extension Store {
    /// Where files dropped now would go (desktop `file_target`): the playlist on
    /// screen when you can add to it, otherwise Local Files.
    var dropTarget: FileTarget {
        switch (paths[tab] ?? []).last {
        case let .playlist(id)?:
            if let h = page(.playlist(id: id))?.header, h.canAdd { return .spotify(id: id, name: h.title) }
            return .localFiles
        case let .localPlaylist(id)?:
            if let p = local?.playlists.first(where: { $0.id == id }) { return .local(id: id, name: p.name) }
            return .localFiles
        default:
            return .localFiles
        }
    }
}

extension Store {
    /// The page on top of the current tab (the assistant's context note names it).
    var topRoute: Route? {
        (paths[tab] ?? []).last ?? {
            switch tab {
            case .library: .library
            case .home: .home
            case .search: nil
            case .local: .localFiles
            case .settings: .settings
            }
        }()
    }
}

extension Store {
    /// Now Playing's large cover and the player bar's, in pixels on this screen.
    static var nowPlayingArtPx: CGFloat { min(UIScreen.main.bounds.width - 40, 460) * UIScreen.main.scale }
    static var barArtPx: CGFloat { 56 * UIScreen.main.scale }

    /// The playing song's covers, fetched as soon as it's known (before Now Playing opens).
    func prefetchCovers(_ item: NowItem?) {
        ImageCache.shared.prefetch(item?.imageSmall, px: Store.barArtPx)
        ImageCache.shared.prefetch(item?.image, px: Store.nowPlayingArtPx)
    }
}
