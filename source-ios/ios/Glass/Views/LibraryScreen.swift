import SwiftUI

/// Your Library (desktop library.rs): the greeting and "library_", tabs
/// music / podcasts / audiobooks with their chips, a filter, sort and view, and the
/// numbered list. Phones open an item's page on tap (the page carries what the
/// desktop's detail pane shows); wide layouts keep the desktop's list + detail.
struct LibraryScreen: View {
    @Environment(Store.self) private var store
    @Environment(\.horizontalSizeClass) private var hSize
    @State private var selected: LibItem?

    var body: some View {
        if hSize == .regular {
            HStack(spacing: 0) {
                list.frame(width: 380)
                Rectangle().fill(Look.shared.divider).frame(width: 1)
                LibraryDetail(item: selected ?? rows.first?.item)
                    .frame(maxWidth: .infinity)
            }
            .glassNav("music")
        } else {
            list.glassNav("music")
        }
    }

    private var list: some View {
        @Bindable var store = store
        let look = Look.shared
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 4) {
                Text(store.library?.greeting ?? "").gFont(14).foregroundStyle(look.sub)
                BlinkTitle(text: "library")
                HStack(spacing: 8) {
                    ForEach(Array(["music", "podcasts", "audiobooks"].enumerated()), id: \.offset) { i, n in
                        Chip(title: n, on: store.libTab == i) {
                            store.libTab = i
                            store.libChip = chips(i)[0]
                        }
                    }
                }
                ScrollView(.horizontal) {
                    HStack(spacing: 8) {
                        ForEach(chips(store.libTab), id: \.self) { c in
                            Chip(title: chipTitle(c), on: store.libChip == c) { store.libChip = c }
                        }
                        Spacer(minLength: 8)
                        SortViewMenu()
                    }
                }
                .scrollIndicators(.hidden)
                FilterField(text: $store.libQuery, prompt: "search your library")
                    .padding(.bottom, 6)
                switch store.library?.state {
                case let .failed(message)?:
                    ErrorBox(message: message) { store.core.refresh(key: "/me/playlists") }
                case .loading?, nil:
                    LoadingNote()
                default:
                    if rows.isEmpty { empty } else { items }
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 40)
        }
        .scrollIndicators(.hidden)
        .refreshable { store.core.refresh(key: "/me/") }
        .onAppear { store.core.setPanels(queue: false, devices: false, nowPlaying: false) }
    }

    @ViewBuilder private var items: some View {
        let view = store.library?.view ?? .list
        if view == .grid {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 12, alignment: .top)], spacing: 14) {
                ForEach(rows, id: \.item.uri) { r in
                    LibGridCell(item: r.item) { open(r.item) }
                }
            }
        } else {
            ForEach(Array(rows.enumerated()), id: \.element.item.uri) { i, r in
                LibRow(item: r.item, number: i + 1, depth: r.depth, compact: view == .compact, expanded: store.expandedFolders.contains(folderId(r.item)), selected: selected?.uri == r.item.uri && hSize == .regular) {
                    if r.item.kind == .folder {
                        let id = folderId(r.item)
                        if store.expandedFolders.contains(id) { store.expandedFolders.remove(id) } else { store.expandedFolders.insert(id) }
                        if hSize == .regular { selected = r.item }
                    } else if hSize == .regular {
                        selected = r.item
                    } else {
                        open(r.item)
                    }
                }
                .modifier(FolderDrag(item: r.item))
            }
        }
    }

    @ViewBuilder private var empty: some View {
        if store.libChip == .downloaded {
            Unavailable(title: "nothing downloaded yet", detail: "Choose download on a playlist, album or podcast to keep it on this device.")
        } else if !store.libQuery.isEmpty {
            Unavailable(title: "no matches", detail: "Nothing in your library matches the filter.")
        } else {
            Unavailable(title: "nothing here yet", detail: "Things you save on Spotify show up here.")
        }
    }

    private func chips(_ tab: Int) -> [LibChip] {
        switch tab {
        case 0: [.playlists, .albums, .artists, .downloaded]
        case 1: [.shows, .downloaded]
        default: [.audiobooks]
        }
    }

    private func chipTitle(_ c: LibChip) -> String { c.rawValue }

    private func open(_ item: LibItem) {
        if let r = libRoute(item) { store.push(r) }
    }

    /// Desktop `build_rows`: specials, pins, folders (children under expanded ones), then the rest.
    private var rows: [(item: LibItem, depth: Int)] {
        guard let lib = store.library else { return [] }
        let q = store.libQuery.trimmingCharacters(in: .whitespaces).lowercased()
        let words = q.split(separator: " ").map(String.init)
        func hit(_ e: LibItem) -> Bool { words.allSatisfy { (e.title + " " + e.subtitle).lowercased().contains($0) } }
        func inChip(_ e: LibItem) -> Bool {
            switch store.libChip {
            case .playlists: [.playlist, .likedSongs, .localFiles].contains(e.kind)
            case .albums: e.kind == .album
            case .artists: e.kind == .artist
            case .shows: [.show, .yourEpisodes].contains(e.kind)
            case .audiobooks: e.kind == .audiobook
            case .downloaded: false
            }
        }
        if store.libChip == .downloaded {
            return lib.downloads.compactMap { d in
                let e = lib.items.first { $0.uri == d.uri } ?? LibItem(uri: d.uri, kind: .playlist, title: d.name, subtitle: "Download", image: nil, ownerId: "", addedAt: "", recentRank: .max, pin: nil, folder: nil, children: 0, editable: false, collaborative: false, downloaded: true, detail: "")
                let podcast = [.show, .yourEpisodes].contains(e.kind)
                let inTab = store.libTab == 1 ? podcast : !(podcast || e.kind == .audiobook)
                return inTab && hit(e) ? (e, 0) : nil
            }
        }
        var out: [(LibItem, Int)] = []
        let specials = lib.items.filter { [.likedSongs, .yourEpisodes, .localFiles].contains($0.kind) }
        out += specials.filter { inChip($0) && hit($0) }.map { ($0, 0) }
        let rest = lib.items.filter { ![.likedSongs, .yourEpisodes, .localFiles, .folder].contains($0.kind) }
        let pinned = rest.filter { $0.pin != nil }.sorted { ($0.pin ?? 0) < ($1.pin ?? 0) }
        out += pinned.filter { inChip($0) && hit($0) }.map { ($0, 0) }
        if store.libChip == .playlists {
            for f in lib.items where f.kind == .folder {
                let fid = folderId(f)
                let children = rest.filter { $0.folder == fid && $0.pin == nil && hit($0) }
                if !hit(f) && children.isEmpty { continue }
                out.append((f, 0))
                if store.expandedFolders.contains(fid) || !q.isEmpty { out += children.map { ($0, 1) } }
            }
        }
        out += rest.filter { inChip($0) && $0.pin == nil && $0.folder == nil && hit($0) }.map { ($0, 0) }
        return out
    }
}

func folderId(_ item: LibItem) -> String { String(item.uri.dropFirst("glass:folder:".count)) }

func libRoute(_ item: LibItem) -> Route? {
    switch item.kind {
    case .likedSongs: .likedSongs
    case .yourEpisodes: .yourEpisodes
    case .localFiles: .localFiles
    case .folder: .folder(id: folderId(item))
    default: routeFor(uri: item.uri)
    }
}

/// One numbered row: "01" (bars while its context plays), cover, title, subtitle.
struct LibRow: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let item: LibItem
    let number: Int
    let depth: Int
    let compact: Bool
    let expanded: Bool
    let selected: Bool
    let tap: () -> Void

    var body: some View {
        let look = Look.shared
        let playing = store.isPlayingContext(item.uri)
        Button(action: tap) {
            HStack(spacing: 10) {
                ZStack {
                    if playing && store.isPlaying { PlayingBars() } else { Text(String(format: "%02d", number)).gFont(12).foregroundStyle(look.muted).monospacedDigit() }
                }
                .frame(width: 24)
                if !compact {
                    LibArt(item: item).frame(width: 52, height: 52)
                }
                if compact {
                    HStack(spacing: 8) {
                        Text(item.title).gFont(14).foregroundStyle(playing ? accent : look.foreground).lineLimit(1)
                        Text(item.subtitle).gFont(12).foregroundStyle(look.sub).lineLimit(1)
                    }
                } else {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(item.title).gFont(14.5).foregroundStyle(playing ? accent : look.foreground).lineLimit(1)
                        HStack(spacing: 4) {
                            if item.pin != nil { Image(systemName: "pin.fill").font(.system(size: 9)).foregroundStyle(accent) }
                            if item.downloaded { Image(systemName: "arrow.down.circle.fill").font(.system(size: 10)).foregroundStyle(accent) }
                            Text(item.subtitle).gFont(12).foregroundStyle(look.sub).lineLimit(1)
                        }
                    }
                }
                Spacer(minLength: 0)
                if item.kind == .folder {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right").font(.system(size: 11)).foregroundStyle(look.muted)
                } else {
                    Image(systemName: "chevron.right").font(.system(size: 11)).foregroundStyle(look.muted.opacity(0.6))
                }
            }
            .padding(.leading, CGFloat(depth) * 22)
            .padding(.vertical, compact ? 8 : 5)
            .padding(.horizontal, 6)
            .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(selected ? look.selection : .clear))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { LibMenu(item: item) }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(item.title), \(item.subtitle)\(playing ? ", playing" : "")")
        .accessibilityHint(item.kind == .folder ? (expanded ? "Collapses the folder" : "Shows the folder's playlists") : "Opens it")
    }
}

/// A library item's artwork, with the special tiles.
struct LibArt: View {
    @GlassAccent private var accent
    let item: LibItem
    var body: some View {
        let look = Look.shared
        switch item.kind {
        case .likedSongs, .yourEpisodes, .localFiles, .folder:
            ZStack {
                RoundedRectangle(cornerRadius: look.smallRadius).fill(accent.opacity(0.14))
                RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(accent.opacity(0.45))
                Image(systemName: item.kind == .likedSongs ? "heart.fill" : item.kind == .yourEpisodes ? "bell.fill" : item.kind == .folder ? "folder.fill" : "arrow.down.circle")
                    .font(.system(size: 18)).foregroundStyle(accent)
            }
        default:
            Art(url: item.image, round: item.kind == .artist, placeholder: item.kind == .artist ? "person" : "music.note")
        }
    }
}

struct LibGridCell: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let item: LibItem
    let tap: () -> Void
    var body: some View {
        let look = Look.shared
        Button(action: tap) {
            VStack(alignment: .leading, spacing: 6) {
                LibArt(item: item).aspectRatio(1, contentMode: .fit)
                Text(item.title).gFont(13).foregroundStyle(store.isPlayingContext(item.uri) ? accent : look.foreground).lineLimit(2)
                Text(item.subtitle).gFont(11.5).foregroundStyle(look.sub).lineLimit(1)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { LibMenu(item: item) }
    }
}

/// Library row menus (desktop library.rs: folders, specials, else the entity's menu).
struct LibMenu: View {
    @Environment(Store.self) private var store
    let item: LibItem
    var body: some View {
        switch item.kind {
        case .folder:
            Button("Rename") { store.sheet = .renameFolder(id: folderId(item), name: item.title) }
            Button("Delete", role: .destructive) {
                store.confirm = Confirm(title: "Delete folder?", body: "This will delete “\(item.title)”. Its playlists stay in Your Library.", button: "Delete") { store.core.deleteFolder(id: folderId(item)) }
            }
        case .likedSongs, .yourEpisodes:
            Button("Open") { if let r = libRoute(item) { store.push(r) } }
            DownloadMenuItem(uri: item.uri, name: item.title)
        case .localFiles:
            Button("Open") { store.push(.localFiles) }
        case .playlist:
            PlaylistMenu(uri: item.uri, name: item.title, editable: item.editable, collaborative: item.collaborative)
        case .album: AlbumMenu(uri: item.uri, name: item.title)
        case .artist: ArtistMenu(uri: item.uri)
        case .show: ShowMenu(uri: item.uri)
        case .audiobook: AudiobookMenu(uri: item.uri)
        }
    }
}

/// Sort by / View as (desktop library sort/view dropdown), plus new playlist / folder.
struct SortViewMenu: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        Menu {
            Section("sort by") {
                ForEach([("recents", LibrarySort.recents), ("recently added", .recentlyAdded), ("alphabetical", .alphabetical), ("creator", .creator)], id: \.0) { name, s in
                    Button { store.setLibrary(sort: s) } label: { if store.library?.sort == s { Label(name, systemImage: "checkmark") } else { Text(name) } }
                }
            }
            Section("view as") {
                ForEach([("list", LibraryView.list), ("compact", .compact), ("grid", .grid)], id: \.0) { name, v in
                    Button { store.setLibrary(view: v) } label: { if store.library?.view == v { Label(name, systemImage: "checkmark") } else { Text(name) } }
                }
            }
            Divider()
            Button("new playlist") { store.sheet = .newPlaylist(uris: []) }
            Button("new folder") { store.sheet = .newFolder(add: nil) }
        } label: {
            HStack(spacing: 4) {
                Text(sortName).gFont(12.5)
                Image(systemName: "chevron.down").font(.system(size: 9))
            }
            .foregroundStyle(look.sub)
            .frame(minHeight: 44)
        }
        .accessibilityLabel("Sort and view")
    }

    private var sortName: String {
        switch store.library?.sort {
        case .recentlyAdded?: "recently added"
        case .alphabetical?: "alphabetical"
        case .creator?: "creator"
        default: "recents"
        }
    }
}

/// A search/filter field in Glass's style: "⌕ > text".
struct FilterField: View {
    @Binding var text: String
    let prompt: String
    var body: some View {
        let look = Look.shared
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").font(.system(size: 13)).foregroundStyle(look.sub)
            Text(">").gFont(13).foregroundStyle(look.sub)
            TextField("", text: $text, prompt: Text(prompt).foregroundColor(look.muted))
                .gFont(13.5).foregroundStyle(look.foreground)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
                .submitLabel(.search)
            if !text.isEmpty {
                Button { text = "" } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(look.muted) }.buttonStyle(.plain).accessibilityLabel("Clear")
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 44)
        .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
        .overlay(RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(look.divider))
    }
}

/// The desktop's detail pane, for wide layouts: artwork, kind, name, subtitle,
/// [play] [shuffle], download, ⋯, and the item's page below.
struct LibraryDetail: View {
    @Environment(Store.self) private var store
    let item: LibItem?
    var body: some View {
        if let item, let r = libRoute(item), item.kind != .localFiles {
            PageScreen(route: r).id(item.uri)
        } else if let item, item.kind == .localFiles {
            LocalFilesScreen()
        } else {
            Unavailable(title: "nothing selected", detail: "Pick something in your library.")
        }
    }
}

extension Store {
    func setLibrary(sort: LibrarySort) {
        core.setLibrarySort(sort: sort)
    }
    func setLibrary(view: LibraryView) {
        core.setLibraryView(view: view)
    }
}

/// Desktop drag-onto-folder: a playlist dragged onto a Glass folder moves into it;
/// dropped on another row it joins that row's folder (or the top level).
private struct FolderDrag: ViewModifier {
    @Environment(Store.self) private var store
    let item: LibItem
    @State private var targeted = false
    func body(content: Content) -> some View {
        let look = Look.shared
        let base = item.kind == .playlist ? AnyView(content.draggable("glass-playlist:\(item.uri)")) : AnyView(content)
        base
            .dropDestination(for: String.self) { items, _ in
                guard let s = items.first, s.hasPrefix("glass-playlist:") else { return false }
                let uri = String(s.dropFirst("glass-playlist:".count))
                guard uri != item.uri else { return false }
                let current = store.library?.items.first(where: { $0.uri == uri })?.folder
                // Onto a folder: into it. Onto another row: into that row's folder (none at the top level).
                let target = item.kind == .folder ? folderId(item) : item.folder
                guard target != current else { return false }
                store.core.moveToFolder(playlist: uri, folder: target)
                return true
            } isTargeted: { targeted = $0 && item.kind == .folder }
            .background(RoundedRectangle(cornerRadius: look.smallRadius).stroke(targeted ? look.accent : .clear, lineWidth: 1.5))
    }
}
