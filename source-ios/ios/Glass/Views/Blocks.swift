import SwiftUI

/// A page's blocks, top to bottom.
struct BlocksView: View {
    @Environment(Store.self) private var store
    @AppStorage("homeFilter") private var homeFilter = 0
    let page: PageModel

    var body: some View {
        ForEach(Array(page.blocks.enumerated()), id: \.offset) { _, b in
            if shows(b) { BlockView(block: b, route: page.route) }
        }
    }

    /// Home's chips pick sections, as the desktop's do.
    private func shows(_ b: Block) -> Bool {
        guard case .home = page.route else { return true }
        let id: String
        switch b {
        case let .cards(i, _, _, _, _): id = i
        case let .rows(i, _, _, _, _, _, _, _): id = i
        case .dj, .shortcuts: id = "music."
        case .empty: id = "audiobooks."
        default: id = ""
        }
        switch homeFilter {
        case 1: return id.hasPrefix("music.")
        case 2: return id.hasPrefix("podcasts.")
        case 3: return id.hasPrefix("audiobooks.")
        default: return !(id.hasPrefix("audiobooks.") && { if case .empty = b { return true }; return false }())
        }
    }
}

struct BlockView: View {
    @Environment(Store.self) private var store
    let block: Block
    let route: Route

    var body: some View {
        let look = Look.shared
        switch block {
        case let .rows(id, title, rows, style, context, playlist, more, collapsed):
            RowsBlock(title: title, rows: rows, style: style, context: context, playlist: playlist, more: more, collapsed: Int(collapsed), route: route)
            if id == "top-songs" { OnlyYou() }
        case let .cards(id, title, cards, style, more):
            if !title.isEmpty { SectionTitle(title: title, more: more.map { r in { store.push(r) } }) }
            if id == "top-artists" { OnlyYou() }
            if id == "discography" {
                // Desktop `disco_chips`: All releases / Albums / Singles and EPs / Compilations.
                DiscographyChips()
                let shown = cards.filter { DiscographyChips.keeps($0, store.discoFilter) }
                if shown.isEmpty {
                    Text("No \(DiscographyChips.names[store.discoFilter].lowercased()) here.").gFont(12.5).foregroundStyle(look.muted)
                } else {
                    CardsBlock(cards: shown, style: style)
                }
            } else {
                CardsBlock(cards: cards, style: style)
            }
        case let .text(title, body):
            if !title.isEmpty { SectionTitle(title: title) }
            Text(body).gFont(13.5).foregroundStyle(look.sub).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
        case let .lines(lines):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(lines, id: \.self) { Text($0).gFont(11.5).foregroundStyle(look.sub) }
            }
            .padding(.top, 8)
        case let .links(title, links):
            ForEach(links, id: \.uri) { l in
                Button { if let r = routeFor(uri: l.uri) { store.push(r) } } label: {
                    HStack(spacing: 12) {
                        ArtistAvatar(uri: l.uri)
                        VStack(alignment: .leading) {
                            Text(title).gFont(12).foregroundStyle(look.sub)
                            Text(l.name).gFont(16).foregroundStyle(look.foreground)
                        }
                        Spacer()
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        case let .empty(title, body):
            Unavailable(title: title, detail: body)
        case let .dj(caption, voiceOn, playingHere):
            if case .home = route { DjBanner(caption: caption, voiceOn: voiceOn, playing: playingHere) } else { DjBody(caption: caption, voiceOn: voiceOn) }
        case let .shortcuts(cards):
            ShortcutGrid(cards: cards)
        case let .topResult(card):
            SectionTitle(title: "Top result")
            TopResultCard(card: card)
        case let .recentSearches(queries):
            RecentSearches(queries: queries)
        case let .categories(categories):
            SectionTitle(title: "Browse all")
            CategoryGrid(categories: categories)
        case let .findMore(playlist, empty):
            FindMore(playlist: playlist, empty: empty)
        case let .searchInstead(query):
            BracketButton(title: "Search for “\(query)”") {
                store.searchText = query
                store.searchFilter = .all
                store.tab = .search
            }
        }
    }
}

/// An artist's round picture, fetched by the core with the page (track page "Artist" rows).
struct ArtistAvatar: View {
    let uri: String
    var body: some View {
        Art(url: nil, size: 56, round: true, placeholder: "person")
    }
}

// MARK: rows

/// A list of songs/episodes/chapters with the desktop's columns folded for a phone:
/// number (or track number), cover, title and artists, album, length; long-press
/// for the track menu; tap plays (in the context, or the list from there).
struct RowsBlock: View {
    @Environment(Store.self) private var store
    let title: String
    let rows: [Row]
    let style: RowsStyle
    let context: String
    let playlist: String?
    let more: Route?
    let collapsed: Int
    let route: Route
    @State private var expanded = false
    @State private var filter = ""
    @State private var sort: TrackSort = .custom
    @State private var descending = false
    @State private var selecting = false
    @State private var selection: Set<Int> = []

    var body: some View {
        let look = Look.shared
        let table = style == .table && rows.count > 8
        VStack(alignment: .leading, spacing: 0) {
            if !title.isEmpty { SectionTitle(title: title, more: more.map { r in { store.push(r) } }) }
            if table {
                TableControls(filter: $filter, sort: $sort, descending: $descending, selecting: $selecting, editable: playlist != nil)
            }
            if selecting {
                SelectionBar(rows: rows, selection: $selection, selecting: $selecting, playlist: playlist, route: route)
            }
            let order = visibleOrder()
            let shown = collapsed > 0 && !expanded ? Array(order.prefix(collapsed)) : order
            ForEach(Array(shown.enumerated()), id: \.element) { pos, i in
                if style == .album, sort == .custom, filter.isEmpty, discBreak(i) {
                    Text("Disc \(rows[i].discNumber)").gFont(12).foregroundStyle(look.sub).padding(.top, 10).padding(.bottom, 2)
                }
                TrackRow(row: rows[i], number: number(i, pos), style: style, selected: selection.contains(i), selecting: selecting) {
                    // The end of a swipe on the row isn't a tap.
                    if SwipeGuard.active { return }
                    if selecting {
                        if selection.contains(i) { selection.remove(i) } else { selection.insert(i) }
                    } else if rows[i].playable {
                        store.play(rows: rows, index: i, context: context)
                    }
                }
                .contextMenu { TrackMenu(row: rows[i], playlist: playlist, index: i, route: route) }
                .modifier(Reorder(index: i, playlist: reorderable ? playlist : nil))
                // Swipe a song to add it to the queue (Spotify songs and episodes).
                .modifier(SwipeToQueue(uri: !selecting && rows[i].playable && (rows[i].kind == .track || rows[i].kind == .episode) && rows[i].uri.hasPrefix("spotify:") ? rows[i].uri : nil))
            }
            if collapsed > 0, rows.count > collapsed {
                Button(expanded ? "Show less" : "See more") { expanded.toggle() }
                    .buttonStyle(.plain).gFont(13).foregroundStyle(look.sub).frame(minHeight: 44)
            }
        }
    }

    /// Rows can be dragged into a new order (desktop tracklist drag): a Spotify playlist
    /// you can edit, in its own order, unfiltered.
    private var reorderable: Bool {
        guard playlist != nil, sort == .custom, filter.isEmpty, !selecting else { return false }
        if case .playlist = route { return true }
        return false
    }

    private func discBreak(_ i: Int) -> Bool {
        let multi = Set(rows.map(\.discNumber)).count > 1
        return multi && (i == 0 || rows[i - 1].discNumber != rows[i].discNumber)
    }

    private func number(_ i: Int, _ pos: Int) -> String {
        switch style {
        case .album: return rows[i].trackNumber > 0 ? "\(rows[i].trackNumber)" : "\(i + 1)"
        default: return String(format: "%02d", pos + 1)
        }
    }

    /// Filter and sort as the desktop table does (`visible_order`).
    private func visibleOrder() -> [Int] {
        let q = filter.lowercased()
        var idx = rows.indices.filter { i in
            q.isEmpty || rows[i].title.lowercased().contains(q) || Fmt.artists(rows[i].artists).lowercased().contains(q) || (rows[i].album?.name.lowercased().contains(q) ?? false)
        }
        if sort != .custom {
            idx.sort { a, b in
                let ra = rows[a], rb = rows[b]
                switch sort {
                case .title: return ra.title.lowercased() < rb.title.lowercased()
                case .artist: return Fmt.artists(ra.artists).lowercased() < Fmt.artists(rb.artists).lowercased()
                case .album: return (ra.album?.name ?? "").lowercased() < (rb.album?.name ?? "").lowercased()
                case .added: return ra.addedAt < rb.addedAt
                case .duration: return ra.durationMs < rb.durationMs
                case .custom: return a < b
                }
            }
            if descending { idx.reverse() }
        }
        return idx
    }
}

enum TrackSort: String, CaseIterable {
    case custom = "Custom order", title = "Title", artist = "Artist", album = "Album", added = "Date added", duration = "Duration"
}

/// "filter list" and "Sort and view" above a long list.
struct TableControls: View {
    @GlassAccent private var accent
    @Binding var filter: String
    @Binding var sort: TrackSort
    @Binding var descending: Bool
    @Binding var selecting: Bool
    let editable: Bool
    var body: some View {
        let look = Look.shared
        HStack(spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "line.3.horizontal.decrease").font(.system(size: 12)).foregroundStyle(look.muted)
                TextField("", text: $filter, prompt: Text("filter list").foregroundColor(look.muted))
                    .gFont(13).foregroundStyle(look.foreground)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
            }
            .padding(.horizontal, 10).frame(height: 36)
            .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
            .overlay(RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(look.divider))
            Menu {
                Section("Sort by") {
                    ForEach(TrackSort.allCases, id: \.self) { s in
                        Button { if sort == s { descending.toggle() } else { sort = s; descending = false } } label: {
                            if sort == s { Label(s.rawValue + (descending ? " ▾" : " ▴"), systemImage: "checkmark") } else { Text(s.rawValue) }
                        }
                    }
                }
            } label: {
                Text(sort.rawValue.lowercased() + " ⌄").gFont(12.5).foregroundStyle(look.sub).frame(minHeight: 44)
            }
            .accessibilityLabel("Sort and view")
            Button(selecting ? "done" : "select") { selecting.toggle() }
                .buttonStyle(.plain).gFont(12.5).foregroundStyle(accent).frame(minHeight: 44)
        }
        .padding(.bottom, 4)
    }
}

/// Acting on several songs (the desktop's ⌘/⇧-click selection, made explicit for touch).
struct SelectionBar: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let rows: [Row]
    @Binding var selection: Set<Int>
    @Binding var selecting: Bool
    let playlist: String?
    let route: Route
    var body: some View {
        let look = Look.shared
        let uris = selection.sorted().map { rows[$0].uri }.filter { !$0.isEmpty }
        HStack(spacing: 4) {
            Text("\(selection.count) selected").gFont(12.5).foregroundStyle(look.sub)
            Spacer()
            Button("all") { selection = Set(rows.indices) }.buttonStyle(.plain).gFont(12.5).foregroundStyle(look.sub).frame(minHeight: 44)
            Menu {
                Button("Add to queue") { store.core.queue(uris: uris); selecting = false }
                Button("Add to playlist…") { store.sheet = .addToPlaylist(uris: uris); selecting = false }
                Button("Save to Liked Songs") { store.core.save(uris: uris.filter { $0.hasPrefix("spotify:track:") || $0.hasPrefix("spotify:episode:") }, save: true); selecting = false }
                Button("Remove from Liked Songs") { store.core.save(uris: uris.filter { $0.hasPrefix("spotify:track:") || $0.hasPrefix("spotify:episode:") }, save: false); selecting = false }
                if let playlist {
                    if case .localPlaylist = route {
                        Button("Remove from this playlist", role: .destructive) {
                            for i in selection.sorted(by: >) { store.core.removeFromLocalPlaylist(id: playlist, index: UInt32(i)) }
                            selection = []; selecting = false
                        }
                    } else {
                        Button("Remove from this playlist", role: .destructive) { store.core.removeFromPlaylist(playlist: playlist, uris: uris); selection = []; selecting = false }
                    }
                }
            } label: {
                Text("[ actions ]").gFont(12.5).foregroundStyle(selection.isEmpty ? look.muted : accent).frame(minHeight: 44)
            }
            .disabled(selection.isEmpty)
        }
        .padding(.horizontal, 10)
        .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.selection))
    }
}

/// One row: number (the playing song shows bars), cover, title, artists, length.
struct TrackRow: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let row: Row
    let number: String
    let style: RowsStyle
    var selected = false
    var selecting = false
    let tap: () -> Void

    var body: some View {
        let look = Look.shared
        let current = store.isCurrent(row.uri) || (row.path != nil && store.playback?.item?.path == row.path)
        let dim = row.playable ? 1 : 0.45
        Button(action: tap) {
            if style == .episodes {
                EpisodeRowBody(row: row, current: current)
            } else {
                HStack(spacing: 10) {
                    if selecting {
                        Image(systemName: selected ? "checkmark.circle.fill" : "circle").foregroundStyle(selected ? accent : look.muted).font(.system(size: 18))
                    }
                    ZStack {
                        if current && store.isPlaying { PlayingBars() } else { Text(number).gFont(12).foregroundStyle(current ? accent : look.muted).monospacedDigit() }
                    }
                    .frame(width: 26)
                    if style != .album && style != .chapters {
                        Art(url: row.image, size: 40)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(row.title).gFont(14).foregroundStyle(current ? accent : look.foreground).lineLimit(1)
                            if row.explicit { Text("E").gFont(9, .bold).foregroundStyle(look.background).padding(.horizontal, 3).background(RoundedRectangle(cornerRadius: 2).fill(look.muted)) }
                        }
                        Text(subtitle).gFont(12).foregroundStyle(look.sub).lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    if row.kind == .localLink || row.kind == .localFile {
                        Image(systemName: row.playable ? "internaldrive" : "exclamationmark.triangle").font(.system(size: 11)).foregroundStyle(look.muted)
                    }
                    Text(Fmt.ms(row.durationMs)).gFont(12).foregroundStyle(look.sub).monospacedDigit()
                }
                .opacity(dim)
                .padding(.vertical, 6)
                .padding(.horizontal, 6)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(selected ? look.selection : .clear))
                .contentShape(Rectangle())
            }
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(row.title), \(subtitle), \(Fmt.ms(row.durationMs))\(current ? ", playing" : "")\(row.playable ? "" : ", unavailable")")
        .accessibilityHint(row.note ?? "")
        .accessibilityAddTraits(selected ? .isSelected : [])
        // Now Playing's playlist pane scrolls to the song that's playing.
        .background { if current { Color.clear.id(NowRow.id) } }
    }

    private var subtitle: String {
        switch row.kind {
        case .episode: return row.parent?.name ?? ""
        case .chapter: return Fmt.ms(row.durationMs)
        default:
            let a = Fmt.artists(row.artists)
            if style == .album || style == .popular { return a }
            if let al = row.album?.name, !al.isEmpty { return a.isEmpty ? al : "\(a) · \(al)" }
            return a
        }
    }
}

/// An episode (desktop `episode_row`): cover, name, show, two lines of description,
/// date and time left, a resume bar, and save.
struct EpisodeRowBody: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let row: Row
    let current: Bool
    var body: some View {
        let look = Look.shared
        HStack(alignment: .top, spacing: 12) {
            Art(url: row.image, size: 72)
            VStack(alignment: .leading, spacing: 4) {
                Text(row.title).gFont(14.5).foregroundStyle(current ? accent : look.foreground).lineLimit(2)
                if let p = row.parent { Text(p.name).gFont(12).foregroundStyle(look.foreground).lineLimit(1) }
                if !row.description.isEmpty { Text(row.description).gFont(11.5).foregroundStyle(look.sub).lineLimit(2) }
                HStack(spacing: 8) {
                    Image(systemName: current && store.isPlaying ? "pause.circle" : "play.circle").foregroundStyle(accent)
                    Text(meta).gFont(11.5).foregroundStyle(look.sub)
                    if row.resumeMs > 0, row.durationMs > 0, !row.played {
                        ProgressView(value: Double(row.resumeMs), total: Double(row.durationMs)).tint(look.foreground).frame(width: 60)
                    }
                    Spacer()
                    HeartButton(uri: row.uri, episode: true, size: 16)
                }
            }
        }
        .padding(.vertical, 8)
        .contentShape(Rectangle())
        .overlay(alignment: .bottom) { Hairline() }
    }

    private var meta: String {
        let date = row.releaseDate
        if row.played { return "\(date) • Played" }
        if row.resumeMs > 0 { return "\(date) • \(Fmt.ms(row.durationMs - min(row.resumeMs, row.durationMs))) left" }
        return "\(date) • \(Fmt.ms(row.durationMs))"
    }
}

/// Equalizer bars for the row that's playing.
struct PlayingBars: View {
    @GlassAccent private var accent
    @Environment(\.accessibilityReduceMotion) private var reduce
    var body: some View {
        TimelineView(.periodic(from: .now, by: reduce ? 60 : 0.18)) { ctx in
            let t = ctx.date.timeIntervalSinceReferenceDate
            HStack(alignment: .bottom, spacing: 2) {
                ForEach(0..<3) { i in
                    RoundedRectangle(cornerRadius: 1).fill(accent)
                        .frame(width: 3, height: reduce ? 8 : 4 + 9 * abs(sin(t * 3.1 + Double(i) * 1.7)))
                }
            }
            .frame(height: 14, alignment: .bottom)
        }
        .accessibilityHidden(true)
    }
}

// MARK: cards

struct CardsBlock: View {
    @Environment(Store.self) private var store
    let cards: [Card]
    let style: CardsStyle
    var body: some View {
        switch style {
        case .shelf:
            ScrollView(.horizontal) {
                LazyHStack(alignment: .top, spacing: 12) {
                    ForEach(cards, id: \.uri) { c in CardView(card: c, width: 140) }
                }
            }
            .scrollIndicators(.hidden)
            .scrollClipDisabled()
        case .grid:
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 12, alignment: .top)], spacing: 14) {
                ForEach(cards, id: \.uri) { c in CardView(card: c, width: nil) }
            }
        }
    }
}

/// A tile: cover (round for artists), title, subtitle; long-press for its menu.
struct CardView: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let card: Card
    let width: CGFloat?
    var body: some View {
        let look = Look.shared
        let playing = store.isPlayingContext(card.uri)
        Button { if let r = routeFor(card: card) { store.push(r) } } label: {
            VStack(alignment: .leading, spacing: 6) {
                Art(url: card.image, round: card.round, placeholder: card.round ? "person" : "music.note")
                    .overlay(alignment: .bottomTrailing) {
                        if playing { PlayingBars().padding(8) }
                    }
                Text(card.title).gFont(13).foregroundStyle(playing ? accent : look.foreground).lineLimit(2)
                Text(card.subtitle).gFont(11.5).foregroundStyle(look.sub).lineLimit(2)
            }
            .frame(width: width)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { CardMenu(card: card) }
        .accessibilityLabel("\(card.title), \(card.subtitle)")
    }
}

/// Home's grid of recent contexts (desktop `shortcut_tile`).
struct ShortcutGrid: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let cards: [Card]
    var body: some View {
        let look = Look.shared
        LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
            ForEach(cards, id: \.uri) { c in
                let here = store.isPlayingContext(c.uri)
                Button { if let r = routeFor(card: c) { store.push(r) } } label: {
                    HStack(spacing: 10) {
                        if c.kind == "collection" {
                            ZStack { Color(red: 110 / 255, green: 80 / 255, blue: 230 / 255); Image(systemName: "heart.fill").foregroundStyle(look.foreground) }
                                .frame(width: 52, height: 52)
                        } else {
                            Art(url: c.image, size: 52, round: c.kind == "artist", radius: 0)
                        }
                        Text(c.title).gFont(12.5).foregroundStyle(here ? accent : look.foreground).lineLimit(2)
                        Spacer(minLength: 0)
                    }
                    .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.raised))
                    .clipShape(RoundedRectangle(cornerRadius: look.smallRadius))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .contextMenu { CardMenu(card: c) }
            }
        }
    }
}

/// The DJ banner at the top of Home.
struct DjBanner: View {
    @Environment(Store.self) private var store
    let caption: String?
    let voiceOn: Bool
    let playing: Bool
    var body: some View {
        let look = Look.shared
        let djUri = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq"
        HStack(spacing: 12) {
            Art(url: "https://lexicon-assets.spotifycdn.com/Your-DJ-Cover-Art-300.png", size: 56)
            VStack(alignment: .leading, spacing: 3) {
                Text("DJ").gFont(17).foregroundStyle(look.foreground)
                Text(caption.map { "X: \($0)" } ?? (voiceOn ? "Songs picked for you, with X talking between them" : "Songs picked for you")).gFont(12).foregroundStyle(look.sub).lineLimit(2)
            }
            Spacer()
            PlayCircle(playing: playing, size: 40) { store.playContext(djUri) }
        }
        .padding(8)
        .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.raised))
        .contentShape(Rectangle())
        .onTapGesture { store.push(.playlist(id: "37i9dQZF1EYkqdzj48dyYq")) }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        // The play button is inside the banner's one element: offered as an action.
        .accessibilityAction(named: playing ? "Pause" : "Play") { store.playContext(djUri) }
    }
}

/// The DJ page's body: "Next pick" and what X is saying.
struct DjBody: View {
    @Environment(Store.self) private var store
    let caption: String?
    let voiceOn: Bool
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 10) {
            if store.isPlayingContext("spotify:playlist:37i9dQZF1EYkqdzj48dyYq") {
                OutlineButton(title: "Next pick") { store.core.next() }
            }
            if let caption {
                Text("“\(caption)”").gFont(16).foregroundStyle(look.foreground)
                Text("— X").gFont(12.5).foregroundStyle(look.sub)
            } else if !voiceOn {
                Text("X's voice is off (Settings › Playback): the DJ plays its picks without talking.").gFont(12.5).foregroundStyle(look.sub)
            }
        }
    }
}

struct TopResultCard: View {
    @Environment(Store.self) private var store
    let card: Card
    var body: some View {
        let look = Look.shared
        Button { if let r = routeFor(card: card) { store.push(r) } } label: {
            HStack(spacing: 14) {
                Art(url: card.image, size: 84, round: card.round)
                VStack(alignment: .leading, spacing: 4) {
                    Text(card.title).gFont(22).foregroundStyle(look.foreground).lineLimit(2)
                    Text(card.subtitle).gFont(12.5).foregroundStyle(look.sub).lineLimit(1)
                }
                Spacer()
                PlayCircle(playing: store.isPlayingContext(card.uri) && store.isPlaying, size: 44) {
                    if card.kind == "track" { store.core.playUris(uris: [card.uri], index: 0) } else { store.playContext(card.uri) }
                }
            }
            .padding(14)
            .background(RoundedRectangle(cornerRadius: look.radius).fill(look.raised))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu { CardMenu(card: card) }
    }
}

struct RecentSearches: View {
    @Environment(Store.self) private var store
    let queries: [String]
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                SectionTitle(title: "Recent searches")
                Spacer()
                Button("Clear recent searches") { store.core.clearRecentSearches() }.gFont(12).foregroundStyle(Look.shared.sub)
            }
            FlowRow(spacing: 8) {
                ForEach(queries, id: \.self) { q in
                    Chip(title: q, on: false) { store.searchText = q }
                }
            }
        }
    }
}

struct CategoryGrid: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let categories: [Category]
    var body: some View {
        let look = Look.shared
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 10)], spacing: 10) {
            ForEach(categories, id: \.id) { c in
                Button { store.push(.genre(id: c.id, name: c.name)) } label: {
                    ZStack(alignment: .topLeading) {
                        RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface)
                        HStack(spacing: 0) {
                            Rectangle().fill(Color(css: c.color)?.opacity(0.66) ?? accent.opacity(0.5)).frame(width: 3)
                            Spacer()
                        }
                        if let img = c.image {
                            Art(url: img, size: 56).rotationEffect(.radians(0.4)).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing).offset(x: 10, y: 10)
                        }
                        Text(c.name.lowercased()).gFont(14).foregroundStyle(look.foreground).padding(12)
                    }
                    .frame(height: 84)
                    .clipShape(RoundedRectangle(cornerRadius: look.smallRadius))
                    .overlay(RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(look.divider))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }
}

/// Your own playlist: search Spotify for songs to add (desktop `find_for_playlist`).
struct FindMore: View {
    @Environment(Store.self) private var store
    let playlist: String
    let empty: Bool
    @State private var q = ""
    @State private var committed = ""
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 8) {
            Hairline().padding(.top, 8)
            Text(empty ? "Let's find something for your playlist" : "Find more").gFont(18).foregroundStyle(look.foreground)
            TextField("", text: $q, prompt: Text("Search for songs").foregroundColor(look.muted))
                .gFont(13.5).padding(10)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                .overlay(RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(look.divider))
                .textInputAutocapitalization(.never).autocorrectionDisabled()
                .task(id: q) {
                    try? await Task.sleep(for: .milliseconds(350))
                    if !Task.isCancelled { committed = q.trimmingCharacters(in: .whitespaces) }
                }
            if committed.count >= 2 {
                FindResults(query: committed, playlist: playlist)
            }
        }
    }
}

struct FindResults: View {
    @Environment(Store.self) private var store
    let query: String
    let playlist: String
    var body: some View {
        let route = Route.search(query: query, filter: .songs)
        let rows = store.page(route)?.blocks.firstRows ?? []
        VStack(spacing: 0) {
            if store.page(route) == nil { LoadingNote() }
            ForEach(rows.prefix(10), id: \.uri) { r in
                HStack(spacing: 10) {
                    Art(url: r.image, size: 40)
                    VStack(alignment: .leading) {
                        Text(r.title).gFont(13.5).foregroundStyle(Look.shared.foreground).lineLimit(1)
                        Text(Fmt.artists(r.artists)).gFont(12).foregroundStyle(Look.shared.sub).lineLimit(1)
                    }
                    Spacer()
                    OutlineButton(title: "Add") { store.core.addToPlaylist(playlist: playlist, uris: [r.uri]) }
                }
            }
        }
        .onAppear { store.subscribe(route) }
        .onDisappear { store.unsubscribe(route) }
    }
}


struct DiscographyChips: View {
    @Environment(Store.self) private var store
    static let names = ["All releases", "Albums", "Singles and EPs", "Compilations"]
    static func keeps(_ c: Card, _ f: Int) -> Bool {
        switch f {
        case 1: c.group == "album"
        case 2: c.group == "single" || c.group == "ep"
        case 3: c.group == "compilation"
        default: true
        }
    }
    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                ForEach(Array(Self.names.enumerated()), id: \.offset) { i, n in
                    Chip(title: n, on: store.discoFilter == i) { store.discoFilter = i }
                }
            }
        }
        .scrollIndicators(.hidden)
    }
}

/// Your own profile's top lists (desktop: "Only visible to you").
struct OnlyYou: View {
    var body: some View {
        Text("Only visible to you").gFont(11.5).foregroundStyle(Look.shared.muted)
    }
}

/// Drag a row onto another to move it there (the core sends Spotify's reorder).
private struct Reorder: ViewModifier {
    @Environment(Store.self) private var store
    let index: Int
    let playlist: String?
    @State private var targeted = false
    func body(content: Content) -> some View {
        if let playlist {
            content
                .draggable("glass-row:\(index)")
                .dropDestination(for: String.self) { items, _ in
                    guard let s = items.first, s.hasPrefix("glass-row:"), let from = Int(s.dropFirst(10)), from != index else { return false }
                    store.core.movePlaylistItem(playlist: playlist, from: UInt32(from), to: UInt32(index))
                    return true
                } isTargeted: { targeted = $0 }
                .overlay(alignment: .top) { if targeted { Rectangle().fill(Look.shared.accent).frame(height: 2) } }
        } else {
            content
        }
    }
}
