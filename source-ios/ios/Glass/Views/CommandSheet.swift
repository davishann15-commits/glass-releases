import SwiftUI
import GameController

/// The command bar (desktop command.rs, ⌘K): typing lists matching commands and
/// library items; return runs the first, or searches Spotify for the text when
/// nothing matches; "? …" asks the assistant. On iPhone and iPad it's a sheet,
/// opened from the navigation bar or with ⌘K on a keyboard. The commands are the
/// desktop's, minus window ones (mini player, full screen) that iOS doesn't have.
struct CommandSheet: View {
    @Environment(Store.self) private var store
    @State private var text = ""
    @State private var index = 0
    @FocusState private var focused: Bool

    struct Command {
        let name: String
        let words: String
        let key: String
        let run: @MainActor (Store) -> Void
    }

    enum Hit {
        case command(Command)
        case item(LibItem)
        case search(String)
        case ask(String)
    }

    static let commands: [Command] = [
        Command(name: "play / pause", words: "resume stop", key: "space") { $0.core.togglePlay() },
        Command(name: "next song", words: "skip forward track", key: "⌘ →") { $0.core.next() },
        Command(name: "previous song", words: "back track", key: "⌘ ←") { $0.core.previous() },
        Command(name: "shuffle on/off", words: "random", key: "⌘ S") { s in s.core.setShuffle(on: !(s.playback?.shuffle ?? false)) },
        Command(name: "repeat: off / all / one", words: "loop", key: "⌘ R") { $0.core.cycleRepeat() },
        Command(name: "like current song", words: "save heart", key: "⌥ ⇧ B") { s in
            if let uri = s.playback?.item?.uri, !uri.isEmpty { s.toggleSaved(uri) }
        },
        Command(name: "queue", words: "up next", key: "⌥ ⇧ Q") { s in s.nowPlayingView = .queue; s.nowPlayingOpen = true },
        Command(name: "devices", words: "connect speaker phone", key: "⌥ ⇧ D") { s in s.nowPlayingView = .devices; s.nowPlayingOpen = true },
        Command(name: "lyrics", words: "words karaoke", key: "") { s in s.nowPlayingView = .lyrics; s.nowPlayingOpen = true },
        Command(name: "now playing", words: "current track screen", key: "⌥ ⇧ J") { $0.nowPlayingOpen = true },
        Command(name: "theme panel", words: "colors appearance opacity", key: "⌘ T") { $0.themePanel = true },
        Command(name: "library", words: "music playlists", key: "⌥ ⇧ 0") { $0.push(.library) },
        Command(name: "home", words: "start", key: "⌥ ⇧ H") { $0.push(.home) },
        Command(name: "search Spotify", words: "find", key: "⌘ L") { s in s.tab = .search; s.paths[.search] = [] },
        Command(name: "liked songs", words: "favorites favourites hearts", key: "⌥ ⇧ S") { $0.push(.likedSongs) },
        Command(name: "your episodes", words: "podcasts saved", key: "") { $0.push(.yourEpisodes) },
        Command(name: "albums", words: "library", key: "") { s in s.libTab = 0; s.libChip = .albums; s.push(.library) },
        Command(name: "artists", words: "library following", key: "") { s in s.libTab = 0; s.libChip = .artists; s.push(.library) },
        Command(name: "podcasts", words: "shows library", key: "") { s in s.libTab = 1; s.libChip = .shows; s.push(.library) },
        Command(name: "downloads", words: "offline downloaded", key: "") { s in s.libTab = 0; s.libChip = .downloaded; s.push(.library) },
        Command(name: "recents", words: "history recently played", key: "⌥ ⇧ R") { $0.push(.recents) },
        Command(name: "local files", words: "mp3 files device", key: "") { $0.push(.localFiles) },
        Command(name: "dj", words: "ai dj x mix spotify play", key: "") { $0.core.playContext(context: "spotify:playlist:37i9dQZF1EYkqdzj48dyYq", track: nil) },
        Command(name: "profile", words: "account me", key: "") { $0.push(.profile(id: "")) },
        Command(name: "settings", words: "preferences", key: "⌘ ,") { $0.push(.settings) },
        Command(name: "activity log", words: "network requests", key: "") { s in s.push(.settings); s.push(.activity) },
        Command(name: "assistant", words: "ai chatgpt ask", key: "⌘ J") { $0.assistantOpen = true },
        Command(name: "new playlist", words: "create", key: "⌘ N") { $0.sheet = .newPlaylist(uris: []) },
        Command(name: "download what's playing", words: "offline save", key: "") { s in
            guard let p = s.playback else { return }
            let uri = p.contextUri.isEmpty ? (p.item?.uri ?? "") : p.contextUri
            if !uri.isEmpty { s.core.download(uri: uri, name: p.contextName.isEmpty ? (p.item?.title ?? "") : p.contextName) }
        },
        Command(name: "keyboard shortcuts", words: "keys help", key: "⌘ /") { $0.sheet = .shortcuts },
        Command(name: "about glass", words: "version", key: "") { $0.sheet = .about },
        Command(name: "log out", words: "sign out", key: "") { s in
            s.confirm = Confirm(title: "Log out?", body: "You'll need to sign in again to play music.", button: "Log out") { s.core.logOut() }
        },
    ]

    private static let start = ["play / pause", "queue", "devices", "lyrics", "search Spotify", "liked songs"]

    private var hits: [Hit] {
        let raw = text.trimmingCharacters(in: .whitespaces)
        if raw.hasPrefix("?") { return [.ask(String(raw.dropFirst()).trimmingCharacters(in: .whitespaces))] }
        let q = raw.trimmingCharacters(in: CharacterSet(charactersIn: ">")).trimmingCharacters(in: .whitespaces).lowercased()
        if q.isEmpty { return Self.start.compactMap { n in Self.commands.first { $0.name == n } }.map(Hit.command) }
        let words = q.split(separator: " ").map(String.init)
        let match = { (hay: String) in words.allSatisfy { hay.lowercased().contains($0) } }
        // The shortcuts list only means something with a keyboard attached.
        var out: [Hit] = Self.commands.filter { ($0.name != "keyboard shortcuts" || GCKeyboard.coalesced != nil) && match("\($0.name) \($0.words)") }.prefix(5).map(Hit.command)
        let specials: [LibItem] = store.library?.items.filter { $0.kind == .likedSongs || $0.kind == .yourEpisodes || $0.kind == .localFiles } ?? []
        let items = (specials + (store.library?.items ?? []).filter { !specials.map(\.uri).contains($0.uri) })
        out += items.filter { match("\($0.title) \($0.subtitle)") }.prefix(6).map(Hit.item)
        out.append(.search(raw.trimmingCharacters(in: CharacterSet(charactersIn: ">")).trimmingCharacters(in: .whitespaces)))
        return out
    }

    var body: some View {
        let look = Look.shared
        let list = hits
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Text(">").gFont(13.5).foregroundStyle(look.accent)
                TextField("search, go to, or run a command", text: $text)
                    .gFont(13.5)
                    .focused($focused)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                    .submitLabel(.go)
                    .onSubmit { if let h = list[safe: min(index, list.count - 1)] { run(h) } }
                    .onChange(of: text) { index = 0 }
                    .onKeyPress(.downArrow) { index = min(index + 1, max(list.count - 1, 0)); return .handled }
                    .onKeyPress(.upArrow) { index = max(index - 1, 0); return .handled }
                    .accessibilityIdentifier("command-input")
            }
            .padding(12)
            .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.raised))
            .overlay(RoundedRectangle(cornerRadius: look.smallRadius).stroke(look.accent.opacity(0.8), lineWidth: 1))
            ScrollView {
                VStack(spacing: 2) {
                    ForEach(Array(list.enumerated()), id: \.offset) { i, h in
                        Button { run(h) } label: { row(h, on: i == index) }.buttonStyle(.plain)
                    }
                }
            }
            Text("start with ? to ask the assistant").gFont(11).foregroundStyle(look.muted)
        }
        .padding(16)
        .background(look.background.ignoresSafeArea())
        .foregroundStyle(look.foreground)
        .onAppear { focused = true }
        .presentationDetents([.medium, .large])
    }

    private func row(_ h: Hit, on: Bool) -> some View {
        let look = Look.shared
        let (glyph, name, sub, key): (String, String, String, String) = switch h {
        case let .command(c): (">", c.name, "", c.key)
        case let .item(i): ("♪", i.title, i.subtitle, "")
        case let .search(q): ("⌕", "search Spotify for “\(q)”", "", "")
        case let .ask(q): ("?", q.isEmpty ? "ask the assistant, or tell it what to play" : "ask the assistant: “\(q)”", "", q.isEmpty ? "⌘ J" : "")
        }
        return HStack(spacing: 10) {
            Text(glyph).foregroundStyle(on ? look.accent : look.muted).frame(width: 14)
            Text(name).lineLimit(1)
            if !sub.isEmpty { Text(sub).gFont(12).foregroundStyle(look.muted).lineLimit(1) }
            Spacer(minLength: 4)
            // Keys only with a keyboard attached (an iPad's, say).
            if !key.isEmpty, GCKeyboard.coalesced != nil { Text(key).gFont(11.5).foregroundStyle(look.muted) }
        }
        .gFont(13)
        .padding(.horizontal, 10)
        .frame(minHeight: 40)
        .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(on ? look.selection : .clear))
        .contentShape(Rectangle())
    }

    private func run(_ h: Hit) {
        store.commandOpen = false
        switch h {
        case let .command(c): c.run(store)
        case let .item(i):
            if let r = routeFor(uri: i.uri) ?? libRoute(i) { store.push(r) }
        case let .search(q):
            store.tab = .search
            store.paths[.search] = []
            store.searchText = q
        case let .ask(q):
            // A request runs in the background (the answer arrives as a toast); the panel
            // opens only to type, or to show why it can't answer.
            if !q.isEmpty, store.assistant?.notReady == nil, store.assistant?.enabled == true {
                store.core.aiSend(text: q)
                if !store.assistantOpen { store.show(toast: "Asking the assistant…") }
            } else {
                store.nowPlayingOpen = false
                store.assistantOpen = true
                if !q.isEmpty { store.core.aiSend(text: q) }
            }
        }
        text = ""
    }

    private func libRoute(_ i: LibItem) -> Route? {
        switch i.kind {
        case .likedSongs: .likedSongs
        case .yourEpisodes: .yourEpisodes
        case .localFiles: .localFiles
        case .folder: .folder(id: i.uri)
        default: nil
        }
    }
}

extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}

/// The navigation bar's way in.
struct CommandButton: View {
    @Environment(Store.self) private var store
    var body: some View {
        IconButton(icon: "chevron.right.square", size: 16, label: "Command bar") { store.commandOpen = true }
            .accessibilityIdentifier("command-button")
    }
}

/// Keyboard shortcuts (desktop ⌘/), for an iPad or iPhone with a keyboard.
struct ShortcutsSheet: View {
    static let keys: [(String, String)] = [
        ("⌘ K", "command bar"), ("⌘ J", "assistant"), ("space", "play / pause"), ("⌘ →  ⌘ ←", "next / previous song"),
        ("⌘ S", "shuffle"), ("⌘ R", "repeat"), ("⌥ ⇧ B", "like current song"), ("⌥ ⇧ J", "now playing"),
        ("⌥ ⇧ Q", "queue"), ("⌥ ⇧ D", "devices"), ("⌘ L", "search Spotify"), ("⌥ ⇧ H", "home"), ("⌥ ⇧ 0", "library"),
        ("⌥ ⇧ S", "liked songs"), ("⌥ ⇧ R", "recents"), ("⌘ ,", "settings"), ("⌘ N", "new playlist"), ("⌘ T", "theme panel"), ("⌘ /", "this list"),
    ]
    var body: some View {
        let look = Look.shared
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                Caps(text: "keyboard shortcuts")
                ForEach(Self.keys, id: \.0) { k, what in
                    HStack {
                        Text(k).gFont(13).foregroundStyle(look.accent).frame(width: 110, alignment: .leading)
                        Text(what).gFont(13)
                    }
                }
                Text("With a keyboard attached. Hold ⌘ to see the system's list too.").gFont(11.5).foregroundStyle(look.muted).padding(.top, 8)
            }
            .padding(20)
        }
        .background(look.background.ignoresSafeArea())
        .foregroundStyle(look.foreground)
    }
}

/// The shortcuts themselves: hidden buttons with key equivalents, in the root view.
struct KeyCommands: View {
    @Environment(Store.self) private var store
    var body: some View {
        Group {
            key("Command bar", "k", .command) { store.commandOpen.toggle() }
            key("Play or pause", .space, []) { store.core.togglePlay() }
            key("Next song", .rightArrow, .command) { store.core.next() }
            key("Previous song", .leftArrow, .command) { store.core.previous() }
            key("Shuffle", "s", .command) { store.core.setShuffle(on: !(store.playback?.shuffle ?? false)) }
            key("Repeat", "r", .command) { store.core.cycleRepeat() }
            key("Like current song", "b", [.option, .shift]) { if let u = store.playback?.item?.uri, !u.isEmpty { store.toggleSaved(u) } }
            key("Now Playing", "j", [.option, .shift]) { store.nowPlayingOpen.toggle() }
            key("Queue", "q", [.option, .shift]) { store.nowPlayingView = .queue; store.nowPlayingOpen = true }
            key("Devices", "d", [.option, .shift]) { store.nowPlayingView = .devices; store.nowPlayingOpen = true }
            key("Search Spotify", "l", .command) { store.tab = .search; store.paths[.search] = [] }
            key("Home", "h", [.option, .shift]) { store.push(.home) }
            key("Library", "0", [.option, .shift]) { store.push(.library) }
            key("Liked Songs", "s", [.option, .shift]) { store.push(.likedSongs) }
            key("Recents", "r", [.option, .shift]) { store.push(.recents) }
            key("Settings", ",", .command) { store.push(.settings) }
            key("New playlist", "n", .command) { store.sheet = .newPlaylist(uris: []) }
            key("Theme panel", "t", .command) { store.themePanel = true }
            key("Keyboard shortcuts", "/", .command) { store.sheet = .shortcuts }
        }
        .frame(width: 0, height: 0)
        .opacity(0)
        .accessibilityHidden(true)
    }

    private func key(_ title: String, _ k: KeyEquivalent, _ m: EventModifiers, _ action: @escaping () -> Void) -> some View {
        Button(title, action: action).keyboardShortcut(k, modifiers: m).accessibilityHidden(true)
    }
}
