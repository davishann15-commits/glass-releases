import SwiftUI
import UIKit

/// The track menu (desktop `menus::track`), for long-press and ⋯ on songs,
/// episodes and chapters.
struct TrackMenu: View {
    @Environment(Store.self) private var store
    let row: Row
    var playlist: String? = nil
    var index: Int = 0
    var route: Route? = nil

    var body: some View {
        let episode = row.kind == .episode
        let local = row.kind == .localLink || row.kind == .localFile
        if !local || row.kind == .localLink {
            AddToPlaylistMenu(uris: [row.uri])
        }
        if let playlist {
            if case .localPlaylist = route {
                Button("Remove from this playlist", role: .destructive) { store.core.removeFromLocalPlaylist(id: playlist, index: UInt32(index)) }
            } else {
                Button("Remove from this playlist", role: .destructive) { store.core.removeFromPlaylist(playlist: playlist, uris: [row.uri]) }
            }
        }
        if !local {
            let saved = store.isSaved(row.uri)
            Button(episode ? (saved ? "Remove from Your Episodes" : "Save to Your Episodes") : (saved ? "Remove from Liked Songs" : "Save to Liked Songs"), systemImage: saved ? "heart.slash" : "heart") {
                store.toggleSaved(row.uri)
            }
            .onAppear { store.wantSaved(row.uri) }
            Button("Add to queue", systemImage: "text.badge.plus") { store.core.queue(uris: [row.uri]) }
        } else if row.kind == .localFile, let path = row.path {
            Button("Play", systemImage: "play") { store.core.playLocal(paths: [path], start: 0) }
            LocalPlaylistMenu(paths: [path])
        }
        Divider()
        if episode {
            if let id = row.uri.split(separator: ":").last { Button("Go to episode") { store.push(.episode(id: String(id))) } }
            if let show = row.parent, let r = routeFor(uri: show.uri) { Button("Go to podcast") { store.push(r) } }
        } else if !local {
            if let id = row.uri.split(separator: ":").last { Button("Go to song") { store.push(.track(id: String(id))) } }
            Button("Go to song radio") { store.core.radio(uri: row.uri) }
            let artists = row.artists.filter { !$0.uri.isEmpty }
            if artists.count == 1, let r = routeFor(uri: artists[0].uri) {
                Button("Go to artist") { store.push(r) }
            } else if artists.count > 1 {
                Menu("Go to artist") {
                    ForEach(artists, id: \.uri) { a in Button(a.name) { if let r = routeFor(uri: a.uri) { store.push(r) } } }
                }
            }
            if let al = row.album, let r = routeFor(uri: al.uri) { Button("Go to album") { store.push(r) } }
        }
        if row.kind == .localLink, !row.playable, !(store.share?.peers.isEmpty ?? true) {
            Button("Get from your other devices", systemImage: "arrow.down.to.line") { store.core.getFromDevices(uris: [row.uri]) }
        }
        if row.kind == .localLink, row.playable, let path = row.path { SendToMenu(paths: [path]) }
        if let note = row.note, local { Text(note) }
        if !local { ShareMenu(uri: row.uri, kind: episode ? "episode" : "song") }
    }
}

/// Add to playlist ▸ ＋ New playlist, then the playlists you can edit (owned or collaborative).
struct AddToPlaylistMenu: View {
    @Environment(Store.self) private var store
    let uris: [String]
    var body: some View {
        Menu("Add to playlist", systemImage: "plus") {
            Button("＋  New playlist") { store.sheet = .newPlaylist(uris: uris) }
            Divider()
            ForEach(writable, id: \.uri) { p in
                Button(p.title) { store.core.addToPlaylist(playlist: String(p.uri.split(separator: ":").last ?? ""), uris: uris) }
            }
        }
    }

    private var writable: [LibItem] {
        (store.library?.items ?? []).filter { $0.kind == .playlist && ($0.editable || $0.collaborative) }
    }
}

/// Glass local playlists for local files ("only on this device").
struct LocalPlaylistMenu: View {
    @Environment(Store.self) private var store
    let paths: [String]
    var body: some View {
        Menu("Add to local playlist", systemImage: "music.note.list") {
            Button("＋  New local playlist") { store.core.createLocalPlaylist(name: "Local Playlist #\((store.local?.playlists.count ?? 0) + 1)", paths: paths) }
            ForEach(store.local?.playlists ?? [], id: \.id) { p in
                Button(p.name) { store.core.addToLocalPlaylist(id: p.id, paths: paths) }
            }
        }
    }
}

/// Share ▸ Copy link, Copy Spotify URI, Open in browser (and the system share sheet).
struct ShareMenu: View {
    @Environment(Store.self) private var store
    let uri: String
    let kind: String
    var body: some View {
        if let link = webLink(uri) {
            Menu("Share", systemImage: "square.and.arrow.up") {
                Button("Copy link to \(kind)") { UIPasteboard.general.string = link; store.show(toast: "Copied to clipboard") }
                Button("Copy Spotify URI") { UIPasteboard.general.string = uri; store.show(toast: "Copied to clipboard") }
                Button("Open in browser") { if let u = URL(string: link) { UIApplication.shared.open(u) } }
                if let u = URL(string: link) { ShareLink("Share…", item: u) }
                SaveAsFileItems(uri: uri, kind: kind)
            }
        }
    }
}

/// Share ▸ "Download as M4A/MP3" and its progress (desktop menus.rs): a song, or a
/// playlist, album or artist's top songs into a folder of their own.
struct SaveAsFileItems: View {
    @Environment(Store.self) private var store
    let uri: String
    let kind: String
    var body: some View {
        let format = store.saves?.format ?? (store.settings?.saveFormat == .mp3 ? "MP3" : "M4A")
        if kind == "song", uri.hasPrefix("spotify:track:") {
            Divider()
            if let job = store.saves?.jobs.first(where: { $0.key == uri }) {
                switch job.status {
                case .queued, .finding: Text("Finding it to download as \(format)…")
                case let .downloading(p): Text("Downloading as \(format)… \(p)%")
                case .converting: Text("Saving as \(format)…")
                case .done: ShareLink(item: URL(fileURLWithPath: job.path)) { Label("Downloaded · Share file…", systemImage: "checkmark") }
                case .failed, .stopped: Button("Download as \(format) (try again)") { store.core.saveSong(uri: uri) }
                }
            } else {
                Button("Download as \(format)", systemImage: "arrow.down.doc") { store.core.saveSong(uri: uri) }
            }
        } else if ["playlist", "album", "artist"].contains(kind) {
            Divider()
            let noun = kind == "artist" ? "top songs" : kind
            if let set = store.saves?.sets.first(where: { $0.uri == uri }), !set.finished {
                Text("Downloading \(noun) as \(format)… \(set.saved + set.failed + set.stopped) of \(set.saved + set.failed + set.stopped + set.going)")
                Button("Stop downloading", role: .destructive) { store.core.stopSaveCollection(uri: uri) }
            } else if let set = store.saves?.sets.first(where: { $0.uri == uri }), set.saved > 0 {
                ShareLink(item: URL(fileURLWithPath: set.folder)) { Label("Downloaded · Share folder…", systemImage: "checkmark") }
            } else {
                Button("Download \(noun) as \(format)", systemImage: "arrow.down.doc") { store.core.saveCollection(uri: uri, name: title) }
            }
        }
    }

    private var title: String {
        store.library?.items.first { $0.uri == uri }?.title ?? store.pages.values.compactMap(\.header).first { $0.uri == uri }?.title ?? "Glass"
    }
}

/// open.spotify.com links (desktop `model::web_link`).
func webLink(_ uri: String) -> String? {
    let p = uri.split(separator: ":").map(String.init)
    guard p.count >= 3, p[0] == "spotify" else { return nil }
    switch p[1] {
    case "track", "album", "artist", "playlist", "show", "episode", "audiobook", "user":
        return "https://open.spotify.com/\(p[1])/\(p[2])"
    default: return nil
    }
}

/// Menus for cards (desktop `menus::any`: playlist, album, artist, show, audiobook, profile).
struct CardMenu: View {
    @Environment(Store.self) private var store
    let card: Card
    var body: some View {
        switch card.kind {
        case "playlist": PlaylistMenu(uri: card.uri, name: card.title, editable: isMine, collaborative: false)
        case "album": AlbumMenu(uri: card.uri, name: card.title)
        case "artist": ArtistMenu(uri: card.uri)
        case "show": ShowMenu(uri: card.uri)
        case "audiobook": AudiobookMenu(uri: card.uri)
        case "user": ShareMenu(uri: card.uri, kind: "profile")
        case "track", "episode":
            Button("Play") { store.core.playUris(uris: [card.uri], index: 0) }
            Button("Add to queue") { store.core.queue(uris: [card.uri]) }
            ShareMenu(uri: card.uri, kind: card.kind == "track" ? "song" : "episode")
        default: EmptyView()
        }
    }

    private var isMine: Bool {
        store.library?.items.first { $0.uri == card.uri }?.editable ?? false
    }
}

/// The menu on a page's ⋯ (desktop: the page's own entity menu).
struct PageMenu: View {
    @Environment(Store.self) private var store
    let header: Header
    let route: Route
    var body: some View {
        switch route {
        case .playlist:
            PlaylistMenu(uri: header.uri, name: header.title, editable: header.editable, collaborative: header.collaborative, header: header)
        case .album: AlbumMenu(uri: header.uri, name: header.title)
        case .artist: ArtistMenu(uri: header.uri)
        case .show: ShowMenu(uri: header.uri)
        case .audiobook: AudiobookMenu(uri: header.uri)
        case .likedSongs:
            Button("Add all to queue") {
                if let rows = store.page(route)?.blocks.firstRows { store.core.queue(uris: rows.map(\.uri)) }
            }
        case let .track(id):
            if let rows = store.page(route)?.blocks.firstRows, let row = rows.first { TrackMenu(row: row) } else { ShareMenu(uri: "spotify:track:\(id)", kind: "song") }
        case .episode:
            if let rows = store.page(route)?.blocks.firstRows, let row = rows.first { TrackMenu(row: row) }
        case .profile:
            if header.editable {
                Button("Edit profile") { if let u = URL(string: "https://www.spotify.com/account/profile/") { UIApplication.shared.open(u) } }
            }
            ShareMenu(uri: header.uri, kind: "profile")
        case let .folder(id):
            Button("Rename") { store.sheet = .renameFolder(id: id, name: header.title) }
            Button("Delete folder", role: .destructive) {
                store.confirm = Confirm(title: "Delete folder?", body: "This will delete “\(header.title)”. Its playlists stay in Your Library.", button: "Delete") { store.core.deleteFolder(id: id) }
            }
        case let .localPlaylist(id):
            Button("Add files…", systemImage: "doc.badge.plus") { store.addFilesTarget = .local(id: id, name: header.title) }
            if let p = store.local?.playlists.first(where: { $0.id == id }), !p.paths.isEmpty {
                AppleMusicButton(paths: p.paths, playlist: p.name)
            }
            Button("Undo last change") { store.core.undoPlaylistChange() }
            Button("Delete", role: .destructive) {
                store.confirm = Confirm(title: "Delete “\(header.title)”?", body: "The playlist is removed from Glass (you can undo). Your music files aren't touched.", button: "Delete") { store.core.deleteLocalPlaylist(id: id) }
            }
        default: EmptyView()
        }
    }
}

/// Desktop `menus::playlist`.
struct PlaylistMenu: View {
    @Environment(Store.self) private var store
    let uri: String
    let name: String
    let editable: Bool
    let collaborative: Bool
    var header: Header? = nil
    var body: some View {
        let id = String(uri.split(separator: ":").last ?? "")
        Button("Add to queue", systemImage: "text.badge.plus") { store.core.queueCollection(uri: uri) }
        if editable {
            Button("Edit details", systemImage: "pencil") {
                store.sheet = .editPlaylist(id: id, name: header?.title ?? name, description: header?.description ?? "", isPublic: header?.public ?? false, collaborative: collaborative, image: header?.image)
            }
            Button("Delete", systemImage: "trash", role: .destructive) {
                store.confirm = Confirm(title: "Delete from Your Library?", body: "This will delete \(name) from Your Library.", button: "Delete") { store.core.deletePlaylist(id: id) }
            }
        } else {
            let saved = store.isSaved(uri)
            Button(saved ? "Remove from Your Library" : "Save to Your Library") { store.toggleSaved(uri) }.onAppear { store.wantSaved(uri) }
        }
        // Only when another Glass is around to ask.
        if !(store.share?.peers.isEmpty ?? true) {
            if let header {
                if header.missingLocal > 0 {
                    Button("Get \(header.missingLocal) missing local \(header.missingLocal == 1 ? "song" : "songs") from your other devices", systemImage: "arrow.down.to.line") { store.core.getMissingFromDevices(playlist: id) }
                }
            } else {
                Button("Get missing local songs from your other devices", systemImage: "arrow.down.to.line") { store.core.getMissingFromDevices(playlist: id) }
            }
        }
        if editable || collaborative {
            Button("Add local files…", systemImage: "doc.badge.plus") { store.addFilesTarget = .spotify(id: id, name: header?.title ?? name) }
        }
        DownloadMenuItem(uri: uri, name: name)
        Divider()
        Button("Create playlist") { store.sheet = .newPlaylist(uris: []) }
        Button("Create folder") { store.sheet = .newFolder(add: uri) }
        if editable, let header {
            if header.collaborative {
                Button("Make non-collaborative") { store.core.updatePlaylist(id: id, name: header.title, description: header.description, public: header.public, collaborative: false) }
            } else {
                Button("Invite collaborators") {
                    store.core.updatePlaylist(id: id, name: header.title, description: header.description, public: false, collaborative: true)
                    if let l = webLink(uri) { UIPasteboard.general.string = l }
                    store.show(toast: "Playlist is now collaborative — link copied")
                }
                Button(header.public ? "Remove from profile" : "Add to profile") { store.core.updatePlaylist(id: id, name: header.title, description: header.description, public: !header.public, collaborative: false) }
            }
        }
        Button("Move to folder…") { store.sheet = .moveToFolder(playlist: uri) }
        PinItem(uri: uri, noun: "playlist")
        ShareMenu(uri: uri, kind: "playlist")
    }
}

struct AlbumMenu: View {
    @Environment(Store.self) private var store
    let uri: String
    let name: String
    var body: some View {
        let saved = store.isSaved(uri)
        Button(saved ? "Remove from Your Library" : "Save to Your Library") { store.toggleSaved(uri) }.onAppear { store.wantSaved(uri) }
        Button("Add to queue") { store.core.queueCollection(uri: uri) }
        if let artist = store.page(.album(id: String(uri.split(separator: ":").last ?? "")))?.header?.links.first, !artist.uri.isEmpty {
            Button("Go to artist radio") { store.core.radio(uri: artist.uri) }
            Button("Go to artist") { if let r = routeFor(uri: artist.uri) { store.push(r) } }
        }
        PinItem(uri: uri, noun: "album")
        DownloadMenuItem(uri: uri, name: name)
        ShareMenu(uri: uri, kind: "album")
    }
}

struct ArtistMenu: View {
    @Environment(Store.self) private var store
    let uri: String
    var body: some View {
        let saved = store.isSaved(uri)
        Button(saved ? "Unfollow" : "Follow") { store.toggleSaved(uri) }.onAppear { store.wantSaved(uri) }
        Button("Go to artist radio") { store.core.radio(uri: uri) }
        PinItem(uri: uri, noun: "artist")
        ShareMenu(uri: uri, kind: "artist")
    }
}

struct ShowMenu: View {
    @Environment(Store.self) private var store
    let uri: String
    var body: some View {
        let saved = store.isSaved(uri)
        Button(saved ? "Unfollow" : "Follow") { store.toggleSaved(uri) }.onAppear { store.wantSaved(uri) }
        PinItem(uri: uri, noun: "podcast")
        ShareMenu(uri: uri, kind: "podcast")
    }
}

struct AudiobookMenu: View {
    @Environment(Store.self) private var store
    let uri: String
    var body: some View {
        let saved = store.isSaved(uri)
        Button(saved ? "Remove from Your Library" : "Save to Your Library") { store.toggleSaved(uri) }.onAppear { store.wantSaved(uri) }
        PinItem(uri: uri, noun: "audiobook")
        ShareMenu(uri: uri, kind: "audiobook")
    }
}

struct PinItem: View {
    @Environment(Store.self) private var store
    let uri: String
    let noun: String
    var body: some View {
        let pinned = store.library?.items.first { $0.uri == uri }?.pin != nil
        Button(pinned ? "Unpin \(noun)" : "Pin \(noun)", systemImage: pinned ? "pin.slash" : "pin") { store.core.pin(uri: uri, pinned: !pinned) }
    }
}

struct DownloadMenuItem: View {
    @Environment(Store.self) private var store
    let uri: String
    let name: String
    var body: some View {
        let kept = store.library?.downloads.contains { $0.uri == uri } ?? false
        Button(kept ? "Remove download" : "Download", systemImage: kept ? "arrow.down.circle.fill" : "arrow.down.circle") {
            if kept { store.core.removeDownload(uri: uri) } else { store.core.download(uri: uri, name: name) }
        }
    }
}
