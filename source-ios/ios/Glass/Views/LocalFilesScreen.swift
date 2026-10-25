import SwiftUI
import UniformTypeIdentifiers
import PhotosUI

/// Local Files (desktop local_library.rs): "Local Files" with counts and total time,
/// play, import and rescan; the folders it reads; the views Albums, Songs, Artists,
/// Genres, Recently Added and Playlists (the choice is kept); a filter. Albums are filed
/// like iTunes by the core (src/local_index.rs). On iOS the music lives in Glass's own
/// folder (Files › On My iPhone › Glass), and in folders added from Files.
struct LocalFilesScreen: View {
    @Environment(Store.self) private var store
    @State private var filter = ""
    @State private var importing = false
    @State private var addingFolder = false
    @State private var songSort: SongSort = .artist
    @State private var descending = false
    @State private var artist: String?
    @State private var genre: String?
    @State private var editing: String?

    var body: some View {
        let look = Look.shared
        let lib = store.local
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 8) {
                header(lib)
                if lib == nil || (lib?.loaded == false && lib?.scanning == true) {
                    LoadingNote()
                } else if let lib, lib.tracks.isEmpty && view != .playlists {
                    Unavailable(title: "listen to local files", detail: "Import songs from Files, or put them in Files › On My iPhone › Glass. Add a folder from Files to play music where it is.")
                } else if let lib {
                    tabs
                    FilterField(text: $filter, prompt: "filter")
                    content(lib)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 40)
        }
        .scrollIndicators(.hidden)
        .scrollDismissesKeyboard(.interactively)
        .refreshable { store.core.rescanLocal() }
        .safeAreaInset(edge: .bottom) {
            if store.appleMusicPicks != nil, let lib {
                AppleMusicPickBar(shown: view == .playlists ? [] : shownPaths(lib))
            }
        }
        .glassNav("music / local")
        .fileImporter(isPresented: $importing, allowedContentTypes: Self.audioTypes, allowsMultipleSelection: true) { r in
            if case let .success(urls) = r { LocalImport.importFiles(urls, store: store) }
        }
        .fileImporter(isPresented: $addingFolder, allowedContentTypes: [.folder], allowsMultipleSelection: false) { r in
            if case let .success(urls) = r, let u = urls.first { LocalImport.addFolder(u, store: store) }
        }
        .sheet(item: Binding(get: { editing.map { EditPath(path: $0) } }, set: { editing = $0?.path })) { e in
            TagEditor(path: e.path).environment(store)
        }
        .foregroundStyle(look.foreground)
    }

    static let audioTypes: [UTType] = [.mp3, .mpeg4Audio, .wav, .aiff, UTType("org.xiph.flac") ?? .audio, UTType("org.xiph.ogg") ?? .audio, UTType(filenameExtension: "flac") ?? .audio, UTType(filenameExtension: "ogg") ?? .audio, UTType(filenameExtension: "oga") ?? .audio, UTType(filenameExtension: "aac") ?? .audio, .audio]

    private var view: LocalFilesView { store.settings?.localView ?? .recent }

    @ViewBuilder private func header(_ lib: LocalModel?) -> some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 6) {
            BlinkTitle(text: "local files")
            if let lib, !lib.tracks.isEmpty {
                let total = lib.tracks.reduce(0) { $0 + $1.durationMs }
                Text("\(lib.tracks.count == 1 ? "1 song" : "\(lib.tracks.count) songs") · \(lib.albums.count == 1 ? "1 album" : "\(lib.albums.count) albums") · \(Self.long(total))").gFont(12.5).foregroundStyle(look.sub)
            }
            FlowRow(spacing: 4) {
                if let lib, !lib.songs.isEmpty, view != .playlists {
                    PlayCircle(playing: store.playback?.localFile == true && store.isPlaying, size: 44) {
                        if store.playback?.localFile == true { store.core.togglePlay() } else { playList(shownPaths(lib), 0) }
                    }
                    .accessibilityLabel("Play the list shown")
                }
                BracketButton(title: "import songs") { importing = true }
                BracketButton(title: "add a folder") { addingFolder = true }
                BracketButton(title: "rescan") { store.core.rescanLocal() }
                if let lib, !lib.songs.isEmpty, store.appleMusicPicks == nil {
                    BracketButton(title: "add to Apple Music", icon: "music.note") { store.appleMusicPicks = [] }
                        .accessibilityHint("Pick songs to put in the Music app's library")
                }
            }
            if store.appleMusicPicks != nil {
                Text("Tap songs and albums to pick them for Apple Music.").gFont(11.5).foregroundStyle(look.sub)
            }
            if let lib {
                Text("Showing songs from: " + (lib.sources.isEmpty ? "no folders" : lib.sources.map(LocalImport.display).joined(separator: ", "))).gFont(11.5).foregroundStyle(look.muted)
            }
            ShareStrip()
        }
    }

    private var tabs: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach([(LocalFilesView.albums, "Albums"), (.songs, "Songs"), (.artists, "Artists"), (.genres, "Genres"), (.recent, "Recently Added"), (.playlists, "Playlists")], id: \.1) { v, name in
                    Chip(title: name, on: view == v) { store.editSettings { $0.localView = v } }
                }
            }
        }
        .scrollIndicators(.hidden)
    }

    @ViewBuilder private func content(_ lib: LocalModel) -> some View {
        let q = filter.trimmingCharacters(in: .whitespaces).lowercased()
        switch view {
        case .albums:
            AlbumGrid(lib: lib, albums: lib.albums.indices.filter { albumMatches(lib, $0, q) })
        case .recent:
            AlbumGrid(lib: lib, albums: lib.albums.indices.filter { albumMatches(lib, $0, q) }.sorted { lib.albums[$0].added > lib.albums[$1].added })
        case .songs:
            songs(lib, q)
        case .artists:
            artists(lib, q)
        case .genres:
            genres(lib, q)
        case .playlists:
            playlists(lib)
        }
        if let lib = store.local, lib.tracks.isEmpty == false, view != .playlists, shownCount(lib, q) == 0 {
            Text("Nothing matches the filter.").gFont(13).foregroundStyle(Look.shared.sub)
        }
    }

    private func shownCount(_ lib: LocalModel, _ q: String) -> Int {
        lib.songs.filter { songMatches(lib.tracks[Int($0)], q) }.count
    }

    private func albumMatches(_ lib: LocalModel, _ i: Int, _ q: String) -> Bool {
        q.isEmpty || "\(lib.albums[i].artist)\u{1f}\(lib.albums[i].title)".lowercased().contains(q)
    }

    private func songMatches(_ t: LocalTrackModel, _ q: String) -> Bool {
        q.isEmpty || t.title.lowercased().contains(q) || t.artist.lowercased().contains(q) || t.album.lowercased().contains(q) || t.genre.lowercased().contains(q)
    }

    /// The songs of the view shown, in its order (the header's play button).
    private func shownPaths(_ lib: LocalModel) -> [String] {
        let q = filter.lowercased()
        return songOrder(lib, q).map { lib.tracks[$0].path }
    }

    private func playList(_ paths: [String], _ start: Int) {
        guard !paths.isEmpty else { return }
        store.core.playLocal(paths: paths, start: UInt32(start))
    }

    // MARK: Songs

    enum SongSort: String, CaseIterable { case artist = "Artist", title = "Title", album = "Album", genre = "Genre", year = "Year", time = "Time" }

    /// Desktop `songs_order`: ties keep the artist / album / track order.
    private func songOrder(_ lib: LocalModel, _ q: String) -> [Int] {
        var order = lib.songs.map(Int.init).filter { songMatches(lib.tracks[$0], q) }
        let rank = Dictionary(uniqueKeysWithValues: lib.songs.enumerated().map { (Int($0.element), $0.offset) })
        func key(_ s: String) -> String { let l = s.trimmingCharacters(in: .whitespaces).lowercased(); return l.hasPrefix("the ") ? String(l.dropFirst(4)) : l }
        switch songSort {
        case .artist: break
        case .title: order.sort { key(lib.tracks[$0].title) < key(lib.tracks[$1].title) }
        case .album: order.sort { (key(lib.tracks[$0].album), rank[$0]!) < (key(lib.tracks[$1].album), rank[$1]!) }
        case .genre: order.sort { (lib.tracks[$0].genre.isEmpty ? 1 : 0, key(lib.tracks[$0].genre), rank[$0]!) < (lib.tracks[$1].genre.isEmpty ? 1 : 0, key(lib.tracks[$1].genre), rank[$1]!) }
        case .year: order.sort { (lib.tracks[$0].year == nil ? 1 : 0, Int(lib.tracks[$0].year ?? 0), rank[$0]!) < (lib.tracks[$1].year == nil ? 1 : 0, Int(lib.tracks[$1].year ?? 0), rank[$1]!) }
        case .time: order.sort { lib.tracks[$0].durationMs < lib.tracks[$1].durationMs }
        }
        return descending ? order.reversed() : order
    }

    @ViewBuilder private func songs(_ lib: LocalModel, _ q: String) -> some View {
        let order = songOrder(lib, q)
        let paths = order.map { lib.tracks[$0].path }
        Menu {
            ForEach(SongSort.allCases, id: \.self) { s in
                Button { if songSort == s { descending.toggle() } else { songSort = s; descending = false } } label: {
                    if songSort == s { Label("Sort by \(s.rawValue)" + (descending ? " ▾" : " ▴"), systemImage: "checkmark") } else { Text("Sort by \(s.rawValue)") }
                }
            }
        } label: {
            Text("sorted by \(songSort.rawValue.lowercased()) \(descending ? "▾" : "▴")").gFont(12.5).foregroundStyle(Look.shared.sub).frame(minHeight: 44)
        }
        ForEach(Array(order.enumerated()), id: \.element) { pos, i in
            LocalSongRow(track: lib.tracks[i], number: pos + 1) { playList(paths, pos) } edit: { editing = lib.tracks[i].path }
        }
    }

    // MARK: Artists, Genres

    @ViewBuilder private func artists(_ lib: LocalModel, _ q: String) -> some View {
        let look = Look.shared
        let shown = lib.artists.filter { a in q.isEmpty || a.name.lowercased().contains(q) || a.albums.contains { albumMatches(lib, Int($0), q) } }
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(shown, id: \.name) { a in
                    Chip(title: a.name, on: (artist ?? shown.first?.name) == a.name) { artist = a.name }
                }
            }
        }
        .scrollIndicators(.hidden)
        if let a = shown.first(where: { $0.name == (artist ?? shown.first?.name) }) {
            let songCount = a.albums.reduce(0) { $0 + lib.albums[Int($1)].tracks.count }
            Text("\(a.name)").gFont(22).padding(.top, 6)
            Text("\(a.albums.count == 1 ? "1 album" : "\(a.albums.count) albums") · \(songCount == 1 ? "1 song" : "\(songCount) songs")").gFont(12).foregroundStyle(look.sub)
            ForEach(a.albums.map(Int.init), id: \.self) { ai in
                let al = lib.albums[ai]
                Button { store.push(.localAlbum(artist: al.artist, album: al.title)) } label: {
                    HStack(spacing: 12) {
                        Art(url: al.cover.map { "localcover://\($0)" }, size: 72)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(al.title).gFont(15).foregroundStyle(look.foreground)
                            Text([al.genre, al.year.map(String.init) ?? ""].filter { !$0.isEmpty }.joined(separator: " · ")).gFont(12).foregroundStyle(look.sub)
                        }
                        Spacer()
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                let paths = al.tracks.map { lib.tracks[Int($0)].path }
                ForEach(Array(al.tracks.enumerated()), id: \.element) { pos, ti in
                    LocalSongRow(track: lib.tracks[Int(ti)], number: Int(lib.tracks[Int(ti)].trackNo == 0 ? UInt32(pos + 1) : lib.tracks[Int(ti)].trackNo), compact: true) { playList(paths, pos) } edit: { editing = lib.tracks[Int(ti)].path }
                }
            }
        }
    }

    @ViewBuilder private func genres(_ lib: LocalModel, _ q: String) -> some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(lib.genres, id: \.name) { g in
                    let n = g.albums.reduce(0) { $0 + lib.albums[Int($1)].tracks.count }
                    Chip(title: "\(g.name)  \(n)", on: (genre ?? lib.genres.first?.name) == g.name) { genre = g.name }
                }
            }
        }
        .scrollIndicators(.hidden)
        if let g = lib.genres.first(where: { $0.name == (genre ?? lib.genres.first?.name) }) {
            AlbumGrid(lib: lib, albums: g.albums.map(Int.init).filter { albumMatches(lib, $0, q) })
        }
    }

    // MARK: Playlists

    @ViewBuilder private func playlists(_ lib: LocalModel) -> some View {
        let look = Look.shared
        if lib.playlists.isEmpty {
            Unavailable(title: "no local playlists yet", detail: "Long-press a song and choose Add to playlist › New local playlist. A Spotify playlist downloaded as files (Share › Download as) shows up here too.")
        }
        ForEach(lib.playlists, id: \.id) { p in
            Button { store.push(.localPlaylist(id: p.id)) } label: {
                HStack(spacing: 12) {
                    Button { store.core.playLocalPlaylist(id: p.id, index: 0) } label: {
                        Image(systemName: "play.fill").foregroundStyle(look.muted).frame(width: 36, height: 44)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Play \(p.name)")
                    if let cover = p.cover { Art(url: cover, size: 40) }
                    Text(p.name).gFont(14.5).foregroundStyle(look.foreground)
                    Spacer()
                    Text(p.paths.count == 1 ? "1 song" : "\(p.paths.count) songs").gFont(12).foregroundStyle(look.sub)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .contextMenu {
                Button("Play") { store.core.playLocalPlaylist(id: p.id, index: 0) }
                AppleMusicButton(paths: p.paths, playlist: p.name)
            }
        }
        if lib.canUndo { BracketButton(title: "undo last playlist change") { store.core.undoPlaylistChange() } }
    }

    static func long(_ ms: UInt64) -> String {
        let m = ms / 60000
        if m < 60 { return "\(m) min" }
        if m < 60 * 48 { return "\(m / 60) hr \(m % 60) min" }
        return "\(m / 1440) days"
    }
}

private struct EditPath: Identifiable {
    let path: String
    var id: String { path }
}

/// Album cards (cover, title, album artist); tap for the album, play button plays it.
struct AlbumGrid: View {
    @Environment(Store.self) private var store
    let lib: LocalModel
    let albums: [Int]
    var body: some View {
        let look = Look.shared
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 140), spacing: 12, alignment: .top)], spacing: 14) {
            ForEach(albums, id: \.self) { i in
                let al = lib.albums[i]
                let paths = al.tracks.map { lib.tracks[Int($0)].path }
                let picked = store.appleMusicPicks.map { Set(paths).isSubset(of: $0) }
                Button {
                    if let picks = store.appleMusicPicks {
                        store.appleMusicPicks = picked == true ? picks.subtracting(paths) : picks.union(paths)
                    } else {
                        store.push(.localAlbum(artist: al.artist, album: al.title))
                    }
                } label: {
                    VStack(alignment: .leading, spacing: 5) {
                        Art(url: al.cover.map { "localcover://\($0)" })
                            .overlay(alignment: .bottomTrailing) {
                                if let picked {
                                    PickMark(on: picked).padding(8)
                                } else {
                                    PlayCircle(playing: false, size: 36) { store.core.playLocal(paths: paths, start: 0) }.padding(6)
                                }
                            }
                        Text(al.title).gFont(13).foregroundStyle(look.foreground).lineLimit(2)
                        Text(al.artist).gFont(11.5).foregroundStyle(look.sub).lineLimit(1)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(picked == true ? .isSelected : [])
                .contextMenu {
                    Button("Play") { store.core.playLocal(paths: paths, start: 0) }
                    LocalAddMenu(paths: paths)
                    SendToMenu(paths: paths)
                    AppleMusicButton(paths: paths)
                    Button("Go to album") { store.push(.localAlbum(artist: al.artist, album: al.title)) }
                }
            }
        }
    }
}

/// A Local Files song: number (bars while it plays), cover, title, artist · album, time; edit.
struct LocalSongRow: View {
    @Environment(Store.self) private var store
    let track: LocalTrackModel
    let number: Int
    var compact = false
    let play: () -> Void
    let edit: () -> Void
    var body: some View {
        let look = Look.shared
        let current = store.playback?.item?.path == track.path
        let picked = store.appleMusicPicks?.contains(track.path)
        Button {
            // The end of a swipe on the row isn't a tap.
            if SwipeGuard.active { return }
            if let picks = store.appleMusicPicks {
                store.appleMusicPicks = picked == true ? picks.subtracting([track.path]) : picks.union([track.path])
            } else {
                play()
            }
        } label: {
            HStack(spacing: 10) {
                ZStack {
                    if let picked { PickMark(on: picked) }
                    else if current && store.isPlaying { PlayingBars() } else { Text(String(format: "%02d", number)).gFont(12).foregroundStyle(current ? look.accent : look.muted) }
                }
                .frame(width: 26)
                if !compact { Art(url: track.hasCover ? "localcover://\(track.path)" : nil, size: 40) }
                VStack(alignment: .leading, spacing: 2) {
                    Text(track.title).gFont(14).foregroundStyle(current ? look.accent : look.foreground).lineLimit(1)
                    Text([track.artist, compact ? "" : track.album].filter { !$0.isEmpty }.joined(separator: " · ")).gFont(12).foregroundStyle(look.sub).lineLimit(1)
                }
                Spacer(minLength: 4)
                Text(Fmt.ms(track.durationMs)).gFont(12).foregroundStyle(look.sub).monospacedDigit()
            }
            .padding(.vertical, 5)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(picked == true ? .isSelected : [])
        // While Local Files plays, swipe a song left to queue it there.
        .modifier(SwipeToQueue(uri: nil, queue: store.playback?.localFile == true && picked == nil ? { store.core.queueLocal(paths: [track.path]) } : nil))
        .contextMenu {
            Button("Play") { play() }
            LocalAddMenu(paths: [track.path])
            if !track.album.isEmpty || !track.artist.isEmpty {
                Button("Go to album") { store.push(.localAlbum(artist: track.albumArtist.isEmpty ? (track.artist.isEmpty ? "Unknown Artist" : track.artist) : track.albumArtist, album: track.album.isEmpty ? "Unknown Album" : track.album)) }
            }
            SendToMenu(paths: [track.path])
            AppleMusicButton(paths: [track.path])
            Button("Edit song info…", systemImage: "pencil") { edit() }
            ShareLink(item: URL(fileURLWithPath: track.path)) { Label("Share file…", systemImage: "square.and.arrow.up") }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(track.title), \(track.artist), \(Fmt.ms(track.durationMs))\(current ? ", playing" : "")")
        .accessibilityAction(named: "Edit song info") { edit() }
    }
}

/// "Add to Apple Music" in a menu: the Add to Apple Music sheet for these songs.
struct AppleMusicButton: View {
    @Environment(Store.self) private var store
    let paths: [String]
    var playlist: String? = nil
    var body: some View {
        Button(playlist == nil ? "Add to Apple Music" : "Add to Apple Music as a playlist", systemImage: "music.note") {
            store.sheet = .appleMusic(paths: paths, playlist: playlist)
        }
    }
}

/// A song or album picked (or not) for Apple Music.
struct PickMark: View {
    @GlassAccent private var accent
    let on: Bool
    var body: some View {
        Image(systemName: on ? "checkmark.circle.fill" : "circle")
            .font(.system(size: 20))
            .foregroundStyle(on ? accent : Look.shared.muted)
            .background(Circle().fill(.black.opacity(on ? 0.25 : 0.15)))
            .accessibilityHidden(true)
    }
}

/// Add to playlist ▸ for local songs: your editable Spotify playlists (as Spotify's
/// local-file entries), then "only on this device": Glass's local playlists.
struct LocalAddMenu: View {
    @Environment(Store.self) private var store
    let paths: [String]
    var body: some View {
        Menu(paths.count == 1 ? "Add to playlist" : "Add \(paths.count) songs to playlist", systemImage: "plus") {
            if store.account.phase == .signedIn || store.demo {
                let lists = (store.library?.items ?? []).filter { $0.kind == .playlist && ($0.editable || $0.collaborative) }
                ForEach(lists, id: \.uri) { p in
                    Button(p.title) { store.core.addLocalToPlaylist(playlist: String(p.uri.split(separator: ":").last ?? ""), paths: paths) }
                }
                Divider()
            }
            Section("only on this device") {
                Button("＋  New local playlist") {
                    let n = (store.local?.playlists.count ?? 0) + 1
                    store.core.createLocalPlaylist(name: "Local Playlist #\(n)", paths: paths)
                }
                ForEach(store.local?.playlists ?? [], id: \.id) { p in
                    Button(p.name) { store.core.addToLocalPlaylist(id: p.id, paths: paths) }
                }
            }
        }
    }
}

/// A local album (desktop `album_page`): title, artist · genre · year · songs and time,
/// play, and the songs disc by disc.
struct LocalAlbumScreen: View {
    @Environment(Store.self) private var store
    let artist: String
    let album: String
    @State private var editing: String?
    var body: some View {
        let look = Look.shared
        let lib = store.local
        let al = lib?.albums.first { $0.artist.lowercased() == artist.lowercased() && $0.title.lowercased() == album.lowercased() }
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 8) {
                if let lib, let al {
                    HStack(alignment: .bottom, spacing: 14) {
                        Art(url: al.cover.map { "localcover://\($0)" }, size: 132)
                        VStack(alignment: .leading, spacing: 4) {
                            Caps(text: "album")
                            Text(al.title).gFont(22).foregroundStyle(look.foreground).lineLimit(3)
                        }
                    }
                    Text([al.artist, al.genre, al.year.map(String.init) ?? "", "\(al.tracks.count == 1 ? "1 song" : "\(al.tracks.count) songs"), \(LocalFilesScreen.long(al.durationMs))"].filter { !$0.isEmpty }.joined(separator: " · ")).gFont(12.5).foregroundStyle(look.sub)
                    let paths = al.tracks.map { lib.tracks[Int($0)].path }
                    HStack {
                        BracketButton(title: "play", icon: "play.fill", primary: true) { store.core.playLocal(paths: paths, start: 0) }
                        Spacer()
                        Menu { LocalAddMenu(paths: paths); AppleMusicButton(paths: paths) } label: { Image(systemName: "ellipsis").font(.system(size: 18)).foregroundStyle(look.sub).frame(width: 44, height: 44) }
                    }
                    let discs = Set(al.tracks.map { lib.tracks[Int($0)].discNo }).count > 1
                    ForEach(Array(al.tracks.enumerated()), id: \.element) { pos, ti in
                        let t = lib.tracks[Int(ti)]
                        if discs, pos == 0 || lib.tracks[Int(al.tracks[pos - 1])].discNo != t.discNo {
                            Text("Disc \(max(t.discNo, 1))").gFont(12).foregroundStyle(look.sub).padding(.top, 8)
                        }
                        LocalSongRow(track: t, number: Int(t.trackNo == 0 ? UInt32(pos + 1) : t.trackNo), compact: true) { store.core.playLocal(paths: paths, start: UInt32(pos)) } edit: { editing = t.path }
                    }
                } else {
                    Unavailable(title: "this album isn't in Local Files any more", detail: "Its files may have moved; pull to rescan.")
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 40)
        }
        .refreshable { store.core.rescanLocal() }
        .safeAreaInset(edge: .bottom) {
            if store.appleMusicPicks != nil, let lib, let al {
                AppleMusicPickBar(shown: al.tracks.map { lib.tracks[Int($0)].path })
            }
        }
        .glassNav("music / local / album")
        .sheet(item: Binding(get: { editing.map { EditPath(path: $0) } }, set: { editing = $0?.path })) { e in
            TagEditor(path: e.path).environment(store)
        }
    }
}

/// Imports and folders from Files.
enum LocalImport {
    /// Copies the picked files into Glass's folder (Documents/Imported): they stay
    /// playable however the originals move. The originals aren't changed.
    @MainActor static func importFiles(_ urls: [URL], store: Store, target: FileTarget = .localFiles) {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        var copied: [String] = []
        for u in urls {
            let ok = u.startAccessingSecurityScopedResource()
            defer { if ok { u.stopAccessingSecurityScopedResource() } }
            // A file still in iCloud is downloaded by the coordinated read.
            var err: NSError?
            NSFileCoordinator().coordinate(readingItemAt: u, options: [], error: &err) { readable in
                if let p = copyIn(readable, name: u.lastPathComponent) { copied.append(p) }
            }
        }
        if case .localFiles = target {
            let n = copied.count
            store.core.filesAdded(dirs: [docs.path])
            store.show(toast: n == 1 ? "Added to Local Files" : n == 0 ? "Nothing there Glass can play (it takes MP3, M4A, AAC, FLAC, Ogg, WAV and AIFF files)" : "Added \(n) songs to Local Files")
        } else {
            // Read, then added where they were meant to go (the core's toast says so).
            store.core.addFiles(paths: copied, target: target)
        }
    }

    /// Copies a file into Documents/Imported under a free name; its new path.
    nonisolated static func copyIn(_ url: URL, name: String) -> String? {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let dest = docs.appendingPathComponent("Imported", isDirectory: true)
        try? FileManager.default.createDirectory(at: dest, withIntermediateDirectories: true)
        let base = URL(fileURLWithPath: name)
        var target = dest.appendingPathComponent(name)
        var k = 2
        while FileManager.default.fileExists(atPath: target.path) {
            target = dest.appendingPathComponent("\(base.deletingPathExtension().lastPathComponent) (\(k)).\(base.pathExtension)")
            k += 1
        }
        return (try? FileManager.default.copyItem(at: url, to: target)) != nil ? target.path : nil
    }

    /// Dropped audio files (their providers hand over temporary copies, kept here).
    @MainActor static func drop(_ providers: [NSItemProvider], store: Store) -> Bool {
        let target = store.dropTarget
        let group = DispatchGroup()
        let lock = NSLock()
        var copied: [String] = []
        for p in providers where p.hasItemConformingToTypeIdentifier(UTType.audio.identifier) {
            group.enter()
            _ = p.loadFileRepresentation(forTypeIdentifier: UTType.audio.identifier) { url, _ in
                defer { group.leave() }
                // The temporary file is gone once this returns: copy it now.
                guard let url, let path = copyIn(url, name: p.suggestedName.map { $0 + "." + url.pathExtension } ?? url.lastPathComponent) else { return }
                lock.lock(); copied.append(path); lock.unlock()
            }
        }
        group.notify(queue: .main) {
            if copied.isEmpty { store.show(toast: "Nothing there Glass can play (it takes MP3, M4A, AAC, FLAC, Ogg, WAV and AIFF files)"); return }
            store.core.addFiles(paths: copied, target: target)
        }
        return !providers.isEmpty
    }

    /// A folder from Files is read where it is (tag edits change its files): kept as a
    /// security-scoped bookmark, opened again at each launch.
    @MainActor static func addFolder(_ url: URL, store: Store) {
        guard url.startAccessingSecurityScopedResource() else {
            store.show(toast: "Glass wasn't given access to that folder")
            return
        }
        if let data = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
            var marks = UserDefaults.standard.array(forKey: key) as? [Data] ?? []
            marks.append(data)
            UserDefaults.standard.set(marks, forKey: key)
        }
        store.core.filesAdded(dirs: [url.path])
        store.show(toast: "Now showing songs from \(url.lastPathComponent)")
    }

    private static let key = "localFolderBookmarks"

    /// At launch: open the folders added from Files again (bookmarks can go stale when a
    /// folder moves; they're refreshed, or dropped when the folder is gone).
    @MainActor static func restoreFolders(store: Store) {
        var marks = UserDefaults.standard.array(forKey: key) as? [Data] ?? []
        var paths: [String] = []
        marks = marks.compactMap { data in
            var stale = false
            guard let url = try? URL(resolvingBookmarkData: data, options: [], relativeTo: nil, bookmarkDataIsStale: &stale) else { return nil }
            guard url.startAccessingSecurityScopedResource() else { return nil }
            paths.append(url.path)
            if stale, let fresh = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) { return fresh }
            return data
        }
        UserDefaults.standard.set(marks, forKey: key)
        if !paths.isEmpty { store.core.filesAdded(dirs: paths) }
    }

    static func display(_ path: String) -> String {
        if let r = path.range(of: "/Documents") , path.hasSuffix("/Documents") || path.contains("/Documents/") {
            let rest = path[r.upperBound...]
            return "On My iPhone › Glass" + rest.replacingOccurrences(of: "/", with: " › ")
        }
        return URL(fileURLWithPath: path).lastPathComponent
    }
}

/// Edit song info (desktop `Modal::EditTags`): title, artist, album, year, genre and the
/// cover, written into the file itself on a copy that then replaces it.
struct TagEditor: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let path: String
    @State private var tags = TagsModel(title: "", artist: "", album: "", year: "", genre: "", hasCover: false)
    @State private var cover: CoverChange = .keep
    @State private var preview: UIImage?
    @State private var error: String?
    @State private var saving = false
    @State private var pick: PhotosPickerItem?
    @State private var pickingFile = false

    var body: some View {
        let look = Look.shared
        NavigationStack {
            ZStack {
                GlassBackground()
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("Edit song info").gFont(20)
                        Text(URL(fileURLWithPath: path).lastPathComponent).gFont(11.5).foregroundStyle(look.muted)
                        HStack(alignment: .top, spacing: 14) {
                            Group {
                                if let preview { Image(uiImage: preview).resizable().scaledToFill() }
                                else if case .remove = cover { look.raised }
                                else { Art(url: tags.hasCover ? "localcover://\(path)" : nil) }
                            }
                            .frame(width: 120, height: 120)
                            .clipShape(RoundedRectangle(cornerRadius: look.smallRadius))
                            VStack(alignment: .leading, spacing: 6) {
                                PhotosPicker(selection: $pick, matching: .images) { Text("[ Choose image… ]").gFont(13) }
                                Button("[ From Files… ]") { pickingFile = true }.gFont(13).buttonStyle(.plain)
                                Button("[ Remove ]") { cover = .remove; preview = nil }.gFont(13).buttonStyle(.plain)
                                    .accessibilityLabel("Remove the cover from the file")
                            }
                        }
                        field("Title", $tags.title)
                        field("Artist", $tags.artist)
                        field("Album", $tags.album)
                        field("Year", $tags.year, hint: "e.g. 2019")
                        field("Genre", $tags.genre)
                        Text("Playlists find local songs by title, artist and length, so changing the title or artist can stop a playlist from finding this file.").gFont(11.5).foregroundStyle(look.sub)
                        if path.contains("/Documents/") {
                            Text("This edits Glass's copy of the file.").gFont(11.5).foregroundStyle(look.muted)
                        } else {
                            Text("This edits the file in its folder in Files.").gFont(11.5).foregroundStyle(look.muted)
                        }
                        if let error { Text(error).gFont(12).foregroundStyle(look.error) }
                        BracketButton(title: saving ? "Saving…" : "Save", primary: true) { save() }.disabled(saving)
                    }
                    .padding(20)
                }
            }
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.gFont(14) } }
            .toolbarBackground(.hidden, for: .navigationBar)
        }
        .presentationBackground(.clear)
        .foregroundStyle(look.foreground)
        .task {
            let p = path
            if let t = await Task.detached(operation: { readSongTags(path: p) }).value { tags = t } else { error = "That file isn't there any more" }
        }
        .onChange(of: pick) { _, item in
            Task { if let item, let data = try? await item.loadTransferable(type: Data.self) { useImage(data) } }
        }
        .fileImporter(isPresented: $pickingFile, allowedContentTypes: [.jpeg, .png]) { r in
            if case let .success(u) = r {
                let ok = u.startAccessingSecurityScopedResource()
                defer { if ok { u.stopAccessingSecurityScopedResource() } }
                if let data = try? Data(contentsOf: u) { useImage(data) }
            }
        }
        .onChange(of: store.tagsResult?.path) { _, _ in
            guard let r = store.tagsResult, r.path == path else { return }
            saving = false
            if let e = r.error { error = e } else { dismiss() }
            store.tagsResult = nil
        }
    }

    private func field(_ label: String, _ text: Binding<String>, hint: String = "") -> some View {
        let look = Look.shared
        return VStack(alignment: .leading, spacing: 3) {
            Text(label).gFont(12).foregroundStyle(look.sub)
            TextField("", text: text, prompt: Text(hint).foregroundColor(look.muted))
                .gFont(14).padding(10)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                .onSubmit { save() }
        }
    }

    /// The picked image made into Glass's cover (≤ 1000 px JPEG) by the core.
    private func useImage(_ data: Data) {
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("cover-\(UUID().uuidString)")
        guard (try? data.write(to: tmp)) != nil else { return }
        defer { try? FileManager.default.removeItem(at: tmp) }
        do {
            let jpeg = try prepareCover(imagePath: tmp.path)
            cover = .set(jpeg: jpeg)
            preview = UIImage(data: Data(jpeg))
            error = nil
        } catch {
            self.error = error.plain
        }
    }

    private func save() {
        if let e = validateTags(tags: tags) { error = e; return }
        saving = true
        store.core.saveTags(path: path, tags: tags, cover: cover)
    }
}


/// Send to ▸ your other devices with Glass on this network (sharing on).
struct SendToMenu: View {
    @Environment(Store.self) private var store
    let paths: [String]
    var body: some View {
        if let peers = store.share?.peers, !peers.isEmpty {
            Menu(paths.count == 1 ? "Send to" : "Send \(paths.count) songs to", systemImage: "paperplane") {
                ForEach(peers, id: \.id) { p in Button(p.name) { store.core.sendToDevice(peer: p.id, paths: paths) } }
            }
        }
    }
}

/// Your other devices with Glass, at the top of Local Files (desktop pages.rs
/// `devices`): found on the Wi-Fi by themselves; tap one to see its songs and get
/// some. Transfers under way show under them.
struct ShareStrip: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        if let sh = store.share, sh.on {
            VStack(alignment: .leading, spacing: 4) {
                if sh.error != nil {
                    Text("Glass can't look for your other devices. Allow Local Network for Glass in Settings › Privacy & Security.").gFont(11.5).foregroundStyle(look.sub)
                } else if !sh.peers.isEmpty {
                    ForEach(sh.peers, id: \.id) { p in
                        Button { store.push(.device(id: p.id, name: p.name)) } label: {
                            HStack(spacing: 10) {
                                Image(systemName: "laptopcomputer.and.iphone").font(.system(size: 15)).foregroundStyle(look.accent).frame(width: 26)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(p.name).gFont(14).foregroundStyle(look.foreground).lineLimit(1)
                                    Text("see its songs and get them here").gFont(11.5).foregroundStyle(look.muted)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").font(.system(size: 12)).foregroundStyle(look.muted)
                            }
                            .padding(.vertical, 6)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityHint("Shows the songs on \(p.name)")
                    }
                }
                ForEach(Array(sh.transfers.enumerated()), id: \.offset) { _, t in
                    HStack {
                        Text(t.text).gFont(11.5).foregroundStyle(look.sub)
                        if let p = t.progress { ProgressView(value: Double(p)).frame(width: 120).tint(look.accent) }
                    }
                }
            }
        }
    }
}
