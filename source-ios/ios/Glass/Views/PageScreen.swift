import SwiftUI

/// Any Spotify page (playlist, album, artist, show, episode, audiobook, song,
/// profile, Liked Songs, Your Episodes, Recents, folders, local playlists, Home,
/// a category): the core builds a `PageModel` (header + blocks) from the same
/// data the desktop page reads, and rebuilds it as it changes.
struct PageScreen: View {
    @Environment(Store.self) private var store
    let route: Route

    var body: some View {
        let look = Look.shared
        let page = store.page(route)
        ScrollView {
            LazyVStack(alignment: .leading, spacing: look.sp(1.5)) {
                if case .home = route { HomeTop() }
                if let h = page?.header { PageHeader(header: h, route: route) }
                switch page?.state {
                case .none, .loading?:
                    if let page { BlocksView(page: page) }
                    LoadingNote()
                case let .failed(message)?:
                    if let page { BlocksView(page: page) }
                    ErrorBox(message: message) { store.core.refresh(key: refreshKey) }
                case let .missing(title, detail)?:
                    Unavailable(title: title, detail: detail)
                    if case .localPlaylist = route {
                        BracketButton(title: "undo last playlist change") { store.core.undoPlaylistChange() }.frame(maxWidth: .infinity)
                    }
                case .ready?:
                    if let page { BlocksView(page: page) }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .padding(.bottom, 40)
        }
        .scrollIndicators(.hidden)
        .refreshable { store.core.refresh(key: refreshKey) }
        .glassNav(breadcrumb)
        .onAppear { store.subscribe(route) }
        .onDisappear { store.unsubscribe(route) }
    }

    /// What [try again] / pull-to-refresh invalidates (any cached path containing it).
    private var refreshKey: String {
        switch route {
        case let .playlist(id), let .album(id), let .artist(id), let .show(id), let .episode(id), let .audiobook(id), let .track(id), let .discography(id): id
        case .likedSongs: "/me/tracks"
        case .yourEpisodes: "/me/episodes"
        case .recents: "/me/player/recently-played"
        case .home: "/me"
        case let .profile(id): id.isEmpty ? "/me" : id
        case let .genre(id, _): id
        default: "/"
        }
    }

    private var breadcrumb: String {
        switch route {
        case .home: "home"
        case .playlist: "music / playlist"
        case .album: "music / album"
        case .artist, .discography: "music / artist"
        case .track: "music / song"
        case .show, .episode: "podcasts"
        case .audiobook: "audiobooks"
        case .profile: "profile"
        case .likedSongs: "music / liked"
        case .yourEpisodes: "podcasts / saved"
        case .recents: "music / recents"
        case .folder: "music / folder"
        case .localPlaylist: "music / local / playlist"
        case .localAlbum: "music / local / album"
        case .device: "music / local / devices"
        case .genre, .search: "search"
        case .activity: "settings / activity"
        default: "music"
        }
    }
}

/// Home's chips, greeting and "home_" (desktop pages.rs `home`).
struct HomeTop: View {
    @Environment(Store.self) private var store
    @AppStorage("homeFilter") private var filter = 0
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 6) {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    ForEach(Array(["all", "music", "podcasts", "audiobooks"].enumerated()), id: \.offset) { i, n in
                        Chip(title: n, on: filter == i) { filter = i }
                    }
                }
            }
            .scrollIndicators(.hidden)
            Text(store.library?.greeting ?? "").gFont(14).foregroundStyle(look.sub)
            BlinkTitle(text: "home")
        }
    }
}

/// "library_": a title with the terminal's blinking cursor (steady with Reduce Motion).
struct BlinkTitle: View {
    @GlassAccent private var accent
    let text: String
    @Environment(\.accessibilityReduceMotion) private var reduce
    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.6)) { ctx in
            let on = reduce || Int(ctx.date.timeIntervalSinceReferenceDate / 0.6) % 2 == 0
            (Text(text) + Text("_").foregroundColor(on ? accent : .clear))
                .gFont(34).foregroundStyle(Look.shared.foreground)
        }
        .accessibilityLabel(text)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The top of a page: artwork, kind, title, links, meta, description and the action row.
struct PageHeader: View {
    @Environment(Store.self) private var store
    let header: Header
    let route: Route

    var body: some View {
        let look = Look.shared
        let h = header
        VStack(alignment: .leading, spacing: 10) {
            if h.art == .banner {
                ZStack(alignment: .bottomLeading) {
                    Art(url: h.image, radius: look.radius, fill: true)
                        .frame(maxWidth: .infinity)
                        .frame(height: 230)
                        .clipShape(RoundedRectangle(cornerRadius: look.radius))
                    LinearGradient(colors: [.clear, .black.opacity(0.6)], startPoint: .center, endPoint: .bottom)
                        .clipShape(RoundedRectangle(cornerRadius: look.radius))
                    VStack(alignment: .leading, spacing: 4) {
                        Caps(text: h.kindLabel, color: look.sub)
                        Text(h.title).gFont(h.title.count > 20 ? 26 : 32).foregroundStyle(look.foreground).lineLimit(2).minimumScaleFactor(0.6)
                        Text(h.listeners).gFont(12.5).foregroundStyle(look.sub)
                    }
                    .padding(16)
                }
            } else {
                HStack(alignment: .bottom, spacing: 14) {
                    HeaderArt(header: h)
                        .frame(width: 132, height: 132)
                        .onTapGesture { if h.editable, case .playlist = route { editDetails() } }
                    VStack(alignment: .leading, spacing: 4) {
                        if !h.kindLabel.isEmpty { Caps(text: h.kindLabel) }
                        Text(h.title).gFont(h.title.count > 24 ? 20 : 24).foregroundStyle(look.foreground).lineLimit(3).minimumScaleFactor(0.7)
                            .accessibilityAddTraits(.isHeader)
                    }
                }
                if !h.description.isEmpty {
                    Text(h.description).gFont(13).foregroundStyle(look.sub).lineLimit(4)
                }
                if !h.links.isEmpty || !h.meta.isEmpty {
                    LinkLine(links: h.links, meta: h.meta)
                }
            }
            ActionRow(header: h, route: route)
        }
        .padding(.bottom, 4)
    }

    private func editDetails() {
        store.sheet = .editPlaylist(id: idOf(h: header), name: header.title, description: header.description, isPublic: header.public, collaborative: header.collaborative, image: header.image)
    }

    private func idOf(h: Header) -> String { h.uri.split(separator: ":").last.map(String.init) ?? "" }
}

/// Special artwork tiles (Liked Songs' heart, Your Episodes' bell, folders, Local Files).
struct HeaderArt: View {
    @GlassAccent private var accent
    let header: Header
    var body: some View {
        let look = Look.shared
        switch header.art {
        case .likedSongs, .yourEpisodes, .folder, .localFiles:
            ZStack {
                RoundedRectangle(cornerRadius: look.smallRadius).fill(accent.opacity(0.14))
                RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(accent.opacity(0.5))
                Image(systemName: header.art == .likedSongs ? "heart.fill" : header.art == .yourEpisodes ? "bell.fill" : header.art == .folder ? "folder.fill" : "arrow.down.circle")
                    .font(.system(size: 40)).foregroundStyle(accent)
            }
        case .round:
            Art(url: header.image, round: true, placeholder: "person")
        default:
            Art(url: header.image)
        }
    }
}

/// "Davis • 12 saves • 15 songs, 48 min": tappable links, then the plain meta.
struct LinkLine: View {
    @Environment(Store.self) private var store
    let links: [Link]
    let meta: String
    var body: some View {
        let look = Look.shared
        FlowRow(spacing: 6) {
            ForEach(Array(links.enumerated()), id: \.offset) { i, l in
                HStack(spacing: 6) {
                    if i > 0 { Text("•").gFont(12).foregroundStyle(look.muted) }
                    if let r = routeFor(uri: l.uri) {
                        Button { store.push(r) } label: { Text(l.name).gFont(13).foregroundStyle(look.foreground).underline(false) }
                            .buttonStyle(.plain)
                            .frame(minHeight: 32)
                    } else {
                        Text(l.name).gFont(13).foregroundStyle(look.foreground)
                    }
                }
            }
            if !meta.isEmpty {
                HStack(spacing: 6) {
                    if !links.isEmpty { Text("•").gFont(12).foregroundStyle(look.muted) }
                    Text(meta).gFont(12.5).foregroundStyle(look.sub)
                }
            }
        }
    }
}

/// `[ play ]  [ shuffle ]`, save, download and more (desktop `action_row`).
struct ActionRow: View {
    @Environment(Store.self) private var store
    let header: Header
    let route: Route

    var body: some View {
        let look = Look.shared
        let h = header
        let here = store.isPlayingContext(h.uri)
        let playing = here && store.isPlaying
        HStack(spacing: 6) {
            if h.canPlay {
                BracketButton(title: playing ? "pause" : "play", icon: playing ? "pause.fill" : "play.fill", primary: true) { play() }
            }
            if h.canShuffle {
                let on = store.playback?.shuffle ?? false
                BracketButton(title: on ? "shuffle on" : "shuffle", icon: "shuffle") { store.core.setShuffle(on: !on) }
            }
            Spacer(minLength: 0)
            switch h.save {
            case .library:
                IconButton(icon: store.isSaved(h.uri) ? "checkmark.circle.fill" : "plus.circle", size: 20, on: store.isSaved(h.uri), label: store.isSaved(h.uri) ? "Remove from Your Library" : "Save to Your Library") { store.toggleSaved(h.uri) }
                    .onAppear { store.wantSaved(h.uri) }
            case .follow:
                OutlineButton(title: store.isSaved(h.uri) ? "Following" : "Follow", on: store.isSaved(h.uri)) { store.toggleSaved(h.uri) }
                    .onAppear { store.wantSaved(h.uri) }
            case .like:
                HeartButton(uri: routeUri, episode: { if case .episode = route { return true }; return false }(), size: 20)
            case .none:
                EmptyView()
            }
            if h.canDownload, !h.uri.isEmpty {
                DownloadToggle(uri: h.uri, name: h.title)
            }
            Menu {
                PageMenu(header: h, route: route)
            } label: {
                Image(systemName: "ellipsis").font(.system(size: 18)).foregroundStyle(look.sub).frame(width: 44, height: 44).contentShape(Rectangle())
            }
            .accessibilityLabel("More options")
        }
    }

    /// The song or episode itself (a track page's header plays its album).
    private var routeUri: String {
        switch route {
        case let .track(id): "spotify:track:\(id)"
        case let .episode(id): "spotify:episode:\(id)"
        default: header.uri
        }
    }

    private func play() {
        switch route {
        case let .track(id):
            let uri = "spotify:track:\(id)"
            if store.isCurrent(uri) { store.core.togglePlay() }
            else if !header.uri.isEmpty { store.core.playContext(context: header.uri, track: uri) }
            else { store.core.playUris(uris: [uri], index: 0) }
        case let .episode(id):
            let uri = "spotify:episode:\(id)"
            if store.isCurrent(uri) { store.core.togglePlay() } else { store.core.playUris(uris: [uri], index: 0) }
        case let .localPlaylist(id):
            store.core.playLocalPlaylist(id: id, index: 0)
        case .yourEpisodes:
            if let rows = store.page(route)?.blocks.firstRows {
                store.core.playUris(uris: rows.map(\.uri), index: 0)
            }
        default:
            store.playContext(header.uri)
        }
    }
}

/// Download to this device, or remove it; shows progress (desktop `download_toggle`).
struct DownloadToggle: View {
    @Environment(Store.self) private var store
    let uri: String
    let name: String
    var body: some View {
        let kept = store.library?.downloads.contains { $0.uri == uri } ?? false
        let p = store.library?.downloadProgress.first { $0.uri == uri }
        let going = kept && p.map { $0.total > 0 && $0.done < $0.total } ?? false
        HStack(spacing: 2) {
            IconButton(icon: kept ? "arrow.down.circle.fill" : "arrow.down.circle", size: 20, on: kept, label: going ? "Downloading \(p!.done) of \(p!.total) — remove" : kept ? "Downloaded — remove" : "Download to this device") {
                if kept { store.core.removeDownload(uri: uri) } else { store.core.download(uri: uri, name: name) }
            }
            // "d/t" while going, as on the desktop.
            if going, let p { Text("\(p.done)/\(p.total)").gFont(11).foregroundStyle(Look.shared.muted) }
        }
    }
}

extension Array where Element == Block {
    var firstRows: [Row]? {
        for b in self { if case let .rows(_, _, rows, _, _, _, _, _) = b { return rows } }
        return nil
    }
}

/// The page for a Spotify URI (a link in a header or a row).
func routeFor(uri: String) -> Route? {
    let parts = uri.split(separator: ":").map(String.init)
    guard parts.count >= 3, parts[0] == "spotify" else { return nil }
    let id = parts[2]
    switch parts[1] {
    case "playlist": return .playlist(id: id)
    case "album": return .album(id: id)
    case "artist": return .artist(id: id)
    case "show": return .show(id: id)
    case "episode": return .episode(id: id)
    case "audiobook": return .audiobook(id: id)
    case "track": return .track(id: id)
    case "user": return parts.count > 3 && parts[3] == "collection" ? .likedSongs : .profile(id: id)
    default: return nil
    }
}

func routeFor(card: Card) -> Route? {
    switch card.kind {
    case "collection": return .likedSongs
    case "category": return .genre(id: card.uri, name: card.title)
    default: return routeFor(uri: card.uri)
    }
}

/// A simple wrapping row (links and meta).
struct FlowRow: Layout {
    var spacing: CGFloat = 6
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let w = proposal.width ?? .infinity
        var x: CGFloat = 0, y: CGFloat = 0, line: CGFloat = 0, maxX: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > 0, x + sz.width > w { x = 0; y += line + 2; line = 0 }
            x += sz.width + spacing
            maxX = max(maxX, x)
            line = max(line, sz.height)
        }
        return CGSize(width: min(maxX, w), height: y + line)
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var x = bounds.minX, y = bounds.minY, line: CGFloat = 0
        for s in subviews {
            let sz = s.sizeThatFits(.unspecified)
            if x > bounds.minX, x + sz.width > bounds.maxX { x = bounds.minX; y += line + 2; line = 0 }
            s.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sz))
            x += sz.width + spacing
            line = max(line, sz.height)
        }
    }
}
