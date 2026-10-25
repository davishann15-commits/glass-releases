import SwiftUI

/// Now Playing (desktop nowplaying.rs): artwork and controls, and beside or below
/// them the lyrics, queue, the playlist that's playing, or devices. Portrait stacks
/// them; landscape and iPad put them side by side as on the desktop.
struct NowPlayingView: View {
    @Environment(Store.self) private var store
    @Environment(\.horizontalSizeClass) private var hSize
    @Environment(\.verticalSizeClass) private var vSize
    /// Portrait: the artwork large (tap it to go back to the lyrics).
    @State private var bigArt = false
    /// How far it's been pulled down (it closes past a point).
    @State private var pull: CGFloat = 0

    private func dismiss() { store.nowPlayingOpen = false }

    var body: some View {
        @Bindable var store = store
        ZStack {
            GlassBackground(wash: true)
            GeometryReader { g in
                let side = g.size.width > g.size.height || hSize == .regular && g.size.width > 700
                VStack(spacing: 0) {
                    topBar
                        .modifier(PullDown(y: $pull, done: dismiss))
                    if side {
                        HStack(alignment: .top, spacing: 24) {
                            ScrollView { NowControls(compactArt: false, pull: $pull).padding(.horizontal, 8) }
                                .scrollIndicators(.hidden)
                                .frame(width: min(g.size.width * 0.42, 460))
                            VStack(spacing: 8) {
                                PanePicker()
                                PaneContent()
                            }
                        }
                        .padding(.horizontal, 20)
                    } else if bigArt {
                        ScrollView {
                            VStack(spacing: 14) {
                                NowControls(compactArt: true, onArt: { withAnimation(.easeInOut(duration: 0.3)) { bigArt = false } }, pull: $pull)
                                PanePicker(onPick: { withAnimation(.easeInOut(duration: 0.3)) { bigArt = false } })
                            }
                            .padding(.horizontal, 20)
                            .padding(.bottom, 30)
                        }
                        .scrollIndicators(.hidden)
                    } else {
                        // Portrait: the controls stay at the top, compact, and the lyrics (or
                        // queue, playlist, devices) fill the rest, as the desktop's right column does.
                        VStack(spacing: 10) {
                            CompactControls(onArt: { withAnimation(.easeInOut(duration: 0.3)) { bigArt = true } })
                                .modifier(PullDown(y: $pull, done: dismiss))
                            PanePicker()
                            PaneContent()
                        }
                        .padding(.horizontal, 16)
                    }
                }
            }
        }
        .offset(y: pull)
        // Nothing to swipe back to here: songs swipe right to join the queue, as in Spotify (and left).
        .environment(\.queueSwipe, [.left, .right])
        .albumTinted()
        .onAppear { pull = 0; updatePanels() }
        .onChange(of: store.nowPlayingView) { _, _ in updatePanels() }
        .onDisappear { store.core.setPanels(queue: false, devices: false, nowPlaying: false) }
    }

    private func updatePanels() {
        store.core.setPanels(queue: store.nowPlayingView == .queue, devices: store.nowPlayingView == .devices, nowPlaying: true)
    }

    private var topBar: some View {
        let look = Look.shared
        return HStack {
            Button { dismiss() } label: {
                Image(systemName: "chevron.down").font(.system(size: 17, weight: .medium)).foregroundStyle(look.foreground).frame(width: 44, height: 44)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Close Now Playing")
            Spacer()
            Breadcrumb(path: "music / now-playing")
            Spacer()
            if let item = store.playback?.item {
                Menu {
                    TrackMenu(row: rowFor(item))
                } label: {
                    Image(systemName: "ellipsis").font(.system(size: 17)).foregroundStyle(look.foreground).frame(width: 44, height: 44)
                }
                .accessibilityLabel("More options")
            } else {
                Color.clear.frame(width: 44, height: 44)
            }
        }
        .padding(.horizontal, 8)
    }
}

/// The left column: "now playing_", artwork, title and artists, heart and menu,
/// the seek rail, transport, volume and devices.
struct NowControls: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let compactArt: Bool
    var onArt: (() -> Void)? = nil
    var pull: Binding<CGFloat>? = nil
    @State private var liked = false

    private func openArt(_ item: NowItem?) {
        if let onArt { onArt() } else if let al = item?.album, let r = routeFor(uri: al.uri) { store.nowPlayingOpen = false; store.push(r) }
    }

    /// Double-tap likes (never unlikes, so a stray double-tap can't undo a like).
    private func like(_ item: NowItem?) {
        guard let item, item.kind == .track || item.kind == .episode else { return }
        if !store.isSaved(item.uri) { store.toggleSaved(item.uri) }
        Haptic.commit()
        liked = true
        Task { try? await Task.sleep(for: .milliseconds(650)); liked = false }
    }

    var body: some View {
        let look = Look.shared
        let p = store.playback
        let item = p?.item
        VStack(alignment: .leading, spacing: 14) {
            Text("now playing_").gFont(12).foregroundStyle(look.muted)
            // Tap: the album (or back to the lyrics); double-tap: like; swipe: next or
            // previous song; pull down: close.
            Art(url: item?.image, radius: look.radius, fallback: item?.imageSmall)
                .frame(maxWidth: compactArt ? 340 : .infinity)
                .frame(maxWidth: .infinity)
                .shadow(color: .black.opacity(0.35), radius: 24, y: 10)
                .overlay { LikePop(shown: liked) }
                .contentShape(Rectangle())
                .gesture(TapGesture(count: 2).onEnded { like(item) }.exclusively(before: TapGesture().onEnded { openArt(item) }))
                .modifier(HorizontalSwipe(left: { store.core.next() }, right: { store.core.previous() }, threshold: 60))
                .modifier(PullDownIf(pull: pull, done: { store.nowPlayingOpen = false }))
                .accessibilityElement()
                .accessibilityAddTraits(.isButton)
                .accessibilityLabel(item?.album.map { "Artwork. Go to album \($0.name)" } ?? "Artwork")
                .accessibilityAction { openArt(item) }
                .accessibilityAction(named: "Like") { like(item) }
                .accessibilityAction(named: "Next song") { store.core.next() }
                .accessibilityAction(named: "Previous song") { store.core.previous() }
            if let item {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(item.title).gFont(21).foregroundStyle(look.foreground).lineLimit(2)
                        if let dj = p?.djCaption {
                            Text("DJ X · " + dj).gFont(13).foregroundStyle(accent).lineLimit(3)
                        }
                        if item.kind == .episode, let show = item.show {
                            LinkLine(links: [show], meta: "")
                        } else if item.kind == .localFile || item.kind == .localLink {
                            Text(Fmt.artists(item.artists)).gFont(13.5).foregroundStyle(look.sub)
                        } else {
                            LinkLine(links: item.artists, meta: "")
                        }
                    }
                    Spacer()
                    if item.kind == .track || item.kind == .episode {
                        HeartButton(uri: item.uri, episode: item.kind == .episode, size: 22)
                    }
                }
            } else {
                Unavailable(title: "nothing playing", detail: "Pick something to play.")
            }
            SeekRail()
            Transport()
            HStack(spacing: 4) {
                VolumeRail()
                IconButton(icon: "hifispeaker.2", size: 17, on: !(p?.here ?? true), label: "Devices") { store.nowPlayingView = .devices }
            }
            if let p, !p.here, let d = p.device {
                Button { store.nowPlayingView = .devices } label: {
                    Text("playing on \(d.name)").gFont(12).foregroundStyle(accent)
                }
                .buttonStyle(.plain)
            }
        }
    }
}

/// Portrait Now Playing's top: a small cover (tap for the large one), title and
/// artists, heart; the seek rail; transport; volume and devices.
struct CompactControls: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let onArt: () -> Void
    var body: some View {
        let look = Look.shared
        let p = store.playback
        let item = p?.item
        VStack(spacing: 6) {
            HStack(spacing: 12) {
                Button(action: onArt) { Art(url: item?.image, size: 76, radius: look.smallRadius, fallback: item?.imageSmall) }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Show the artwork")
                VStack(alignment: .leading, spacing: 3) {
                    Text(item?.title ?? "nothing playing").gFont(17).foregroundStyle(look.foreground).lineLimit(2)
                    if let dj = p?.djCaption {
                        Text("DJ X · " + dj).gFont(12).foregroundStyle(accent).lineLimit(2)
                    } else if let item {
                        if item.kind == .episode, let show = item.show { LinkLine(links: [show], meta: "") }
                        else if item.kind == .localFile || item.kind == .localLink { Text(Fmt.artists(item.artists)).gFont(12.5).foregroundStyle(look.sub) }
                        else { LinkLine(links: item.artists, meta: "") }
                    }
                }
                Spacer(minLength: 0)
                if let item, item.kind == .track || item.kind == .episode {
                    HeartButton(uri: item.uri, episode: item.kind == .episode, size: 20)
                }
            }
            // Swipe the song left or right for the next or previous one.
            .contentShape(Rectangle())
            .modifier(HorizontalSwipe(left: { store.core.next() }, right: { store.core.previous() }, threshold: 60))
            .accessibilityAction(named: "Next song") { store.core.next() }
            .accessibilityAction(named: "Previous song") { store.core.previous() }
            SeekRail()
            Transport()
            if let p, !p.here, let d = p.device {
                Button { store.nowPlayingView = .devices } label: { Text("playing on \(d.name)").gFont(11.5).foregroundStyle(accent) }
                    .buttonStyle(.plain)
            }
        }
    }
}

/// Shuffle, previous, play, next, repeat (or ±15 s for podcasts).
struct Transport: View {
    @Environment(Store.self) private var store
    var body: some View {
        let p = store.playback
        let episode = p?.item?.kind == .episode
        HStack {
            if episode {
                IconButton(icon: "gobackward.15", size: 22, label: "Back 15 seconds") { store.core.seekBy(deltaMs: -15000) }
            } else {
                IconButton(icon: "shuffle", size: 19, on: p?.shuffle ?? false, label: (p?.shuffle ?? false) ? "Shuffle is on" : "Shuffle") { store.core.setShuffle(on: !(p?.shuffle ?? false)) }
            }
            Spacer()
            IconButton(icon: "backward.fill", size: 24, label: "Previous") { store.core.previous() }
            Spacer()
            PlayCircle(playing: store.isPlaying, size: 72) { store.core.togglePlay() }
            Spacer()
            IconButton(icon: "forward.fill", size: 24, label: "Next") { store.core.next() }
            Spacer()
            if episode {
                IconButton(icon: "goforward.15", size: 22, label: "Forward 15 seconds") { store.core.seekBy(deltaMs: 15000) }
            } else {
                let r = p?.repeat ?? .off
                IconButton(icon: r == .track ? "repeat.1" : "repeat", size: 19, on: r != .off, label: r == .track ? "Repeat one" : r == .context ? "Repeat all" : "Repeat") { store.core.cycleRepeat() }
            }
        }
    }
}

/// lyrics · queue · playlist · devices (the desktop's L / Q / P / D views), and V for the visualizer.
struct PanePicker: View {
    @Environment(Store.self) private var store
    var onPick: (() -> Void)? = nil
    var body: some View {
        HStack(spacing: 4) {
            ScrollView(.horizontal) {
                HStack(spacing: 6) {
                    ForEach(NowPlayingPane.allCases, id: \.self) { p in
                        Chip(title: p.rawValue, on: store.nowPlayingView == p) { store.nowPlayingView = p; onPick?() }
                    }
                }
            }
            .scrollIndicators(.hidden)
            // V: the visualizer under the lyrics (Settings: visualizer).
            let on = store.settings?.visualizer ?? false
            IconButton(icon: "waveform", size: 15, on: on, label: on ? "Hide the visualizer" : "Show the visualizer") {
                store.editSettings { $0.visualizer.toggle() }
                if !on { store.nowPlayingView = .lyrics; onPick?() }
            }
        }
    }
}

struct PaneContent: View {
    @Environment(Store.self) private var store
    var embedded = false
    var body: some View {
        switch store.nowPlayingView {
        case .lyrics: LyricsPane(embedded: embedded)
        case .queue: QueuePane(embedded: embedded)
        case .playlist: PlayingFromPane(embedded: embedded)
        case .devices: DevicesPane(embedded: embedded)
        }
    }
}

/// Wraps a pane in a scroll view when it stands on its own (side by side).
struct PaneScroll<Content: View>: View {
    let embedded: Bool
    /// Each new value scrolls to the row marked `NowRow.id` (the song that's playing).
    var follow: String? = nil
    @ViewBuilder let content: Content
    var body: some View {
        if embedded {
            VStack(alignment: .leading, spacing: 6) { content }
        } else {
            ScrollViewReader { proxy in
                ScrollView { VStack(alignment: .leading, spacing: 6) { content }.padding(.bottom, 30) }
                    .scrollIndicators(.hidden)
                    .onChange(of: follow, initial: true) { old, new in
                        guard new != nil else { return }
                        // After the rows are laid out; a jump when the pane opens, a glide as songs change.
                        let glide = old != nil && old != new
                        DispatchQueue.main.async {
                            if glide {
                                withAnimation(.easeInOut(duration: 0.35)) { proxy.scrollTo(NowRow.id, anchor: .center) }
                            } else {
                                proxy.scrollTo(NowRow.id, anchor: .center)
                            }
                        }
                    }
            }
        }
    }
}

/// The song that's playing, in a list of rows: Now Playing's playlist pane scrolls to it.
enum NowRow {
    static let id = "now-playing-row"
}

/// The queue (desktop right.rs `queue`): Now playing; Next in queue (the songs you
/// queued) and Next from / Next up, with Clear queue; tapping a song plays from it.
/// Playing on this iPhone (or from Local Files), drag a song's ≡ handle to reorder;
/// swipe a song that isn't queued (right or left) to queue it.
struct QueuePane: View {
    @Environment(Store.self) private var store
    let embedded: Bool
    /// The rows' frames in the pane (where a dragged song would land).
    @State private var frames: [Int: CGRect] = [:]
    /// The song being dragged by its handle.
    @State private var drag: QueueDrag?

    var body: some View {
        let look = Look.shared
        let q = store.queue
        PaneScroll(embedded: embedded) {
            Text("queue / up next").gFont(12).foregroundStyle(look.muted)
            if let q {
                if let e = q.error {
                    Text("couldn't load the queue").gFont(14).foregroundStyle(look.foreground)
                    Text(e).gFont(12).foregroundStyle(look.error)
                } else if q.loading && q.current == nil {
                    LoadingNote()
                } else if q.current == nil && q.next.isEmpty {
                    Unavailable(title: "your queue is empty", detail: "Swipe a song, or long-press it and choose Add to queue.")
                } else {
                    if let c = q.current {
                        Text("Now playing").gFont(14).foregroundStyle(look.foreground).padding(.top, 6)
                        TrackRow(row: c, number: "01", style: .plain) { store.core.togglePlay() }
                            .contextMenu { TrackMenu(row: c) }
                    }
                    let queued = min(Int(q.queued), q.next.count)
                    if queued > 0 {
                        header("Next in queue", q, clear: true)
                        ForEach(0..<queued, id: \.self) { i in row(q, i) }
                        if queued < q.next.count { header(q.nextFrom, q, clear: false) }
                        ForEach(queued..<q.next.count, id: \.self) { i in row(q, i) }
                    } else {
                        header(q.nextFrom, q, clear: true)
                        ForEach(0..<q.next.count, id: \.self) { i in row(q, i) }
                    }
                    if Self.reorderable(q), q.next.count > 1 {
                        Text("Drag a song's ≡ to change the order.").gFont(11.5).foregroundStyle(look.muted).padding(.top, 4)
                    }
                }
            } else {
                LoadingNote()
            }
        }
        .coordinateSpace(name: "queue")
        .onPreferenceChange(QueueRowFrames.self) { frames = $0 }
    }

    /// Where the dragged song lands: the row under the finger, as the rows stood when the
    /// drag began (the first or last beyond them).
    private func target(_ d: QueueDrag, count: Int) -> Int {
        let y = d.y
        let rows = d.rows.filter { $0.key < count }.sorted { $0.key < $1.key }
        guard let first = rows.first, let last = rows.last else { return 0 }
        if y < first.value.minY { return first.key }
        if y > last.value.maxY { return last.key }
        return rows.min { abs($0.value.midY - y) < abs($1.value.midY - y) }?.key ?? 0
    }

    /// The order can change: playing on this iPhone, or Local Files' own queue.
    static func reorderable(_ q: QueueModel) -> Bool {
        !q.positions.isEmpty && q.positions.count == q.next.count
    }

    private func header(_ title: String, _ q: QueueModel, clear: Bool) -> some View {
        let look = Look.shared
        return HStack {
            Text(title).gFont(14).foregroundStyle(look.foreground)
            Spacer()
            if clear, !q.local, !q.next.isEmpty {
                Button("Clear queue") {
                    store.confirm = Confirm(title: "Clear queue?", body: "You can't undo this.", button: "Clear") { store.core.clearQueue() }
                }
                .buttonStyle(.plain).gFont(12.5).foregroundStyle(look.sub).frame(minHeight: 44)
            }
        }
        .padding(.top, 8)
        .accessibilityAddTraits(.isHeader)
    }

    @ViewBuilder private func row(_ q: QueueModel, _ i: Int) -> some View {
        let r = q.next[i]
        let reorder = Self.reorderable(q)
        let look = Look.shared
        let dragged = drag?.from == i
        let landing = drag.map { $0.from != i && target($0, count: q.next.count) == i } ?? false
        HStack(spacing: 0) {
            TrackRow(row: r, number: String(format: "%02d", i + 2), style: .plain) {
                if SwipeGuard.active || drag != nil { return }
                if q.local {
                    store.core.playLocal(paths: q.next[i...].compactMap(\.path), start: 0)
                } else {
                    store.core.playUris(uris: q.next[i...].map(\.uri), index: 0)
                }
            }
            .contextMenu { TrackMenu(row: r) }
            // Songs that aren't queued yet can be swiped into the queue.
            .modifier(i < Int(q.queued) || drag != nil ? SwipeToQueue(uri: nil) : swipe(q, r))
            if reorder {
                Image(systemName: "line.3.horizontal")
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(dragged ? look.accent : look.muted)
                    .frame(width: 40, height: 44)
                    .contentShape(Rectangle())
                    .highPriorityGesture(
                        DragGesture(minimumDistance: 0, coordinateSpace: .named("queue"))
                            .onChanged { v in
                                if drag == nil {
                                    Haptic.tick()
                                    drag = QueueDrag(from: i, y: v.location.y, dy: 0, rows: frames)
                                }
                                drag?.y = v.location.y
                                drag?.dy = v.translation.height
                            }
                            .onEnded { v in
                                guard var d = drag else { return }
                                d.y = v.location.y
                                let to = target(d, count: q.next.count)
                                drag = nil
                                if to != i { Haptic.commit(); move(q, i, to) }
                            }
                    )
                    .accessibilityIdentifier("queue-handle-\(i)")
                    .accessibilityLabel("Reorder \(r.title)")
            }
        }
        .background(GeometryReader { g in Color.clear.preference(key: QueueRowFrames.self, value: [i: g.frame(in: .named("queue"))]) })
        .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.raised).opacity(dragged ? 1 : 0))
        .offset(y: dragged ? drag!.dy : 0)
        .zIndex(dragged ? 1 : 0)
        .overlay(alignment: (drag.map { $0.from > i } ?? true) ? .top : .bottom) {
            if landing { Rectangle().fill(look.accent).frame(height: 2) }
        }
        .accessibilityActions {
            if reorder, i > 0 { Button("Move up") { move(q, i, i - 1) } }
            if reorder, i + 1 < q.next.count { Button("Move down") { move(q, i, i + 1) } }
        }
    }

    private func swipe(_ q: QueueModel, _ r: Row) -> SwipeToQueue {
        if q.local, let path = r.path { return SwipeToQueue(uri: nil) { store.core.queueLocal(paths: [path]) } }
        let spotify = r.playable && (r.kind == .track || r.kind == .episode) && r.uri.hasPrefix("spotify:")
        return SwipeToQueue(uri: spotify ? r.uri : nil)
    }

    private func move(_ q: QueueModel, _ from: Int, _ to: Int) {
        guard Self.reorderable(q), q.next.indices.contains(from), q.next.indices.contains(to), from != to else { return }
        store.core.moveInQueue(uri: q.next[from].uri, from: q.positions[from], to: q.positions[to])
    }
}

/// A song being dragged by its ≡ handle: the finger's height and travel in the pane,
/// and the rows' frames when it began.
private struct QueueDrag {
    let from: Int
    var y: CGFloat
    var dy: CGFloat
    let rows: [Int: CGRect]
}

/// The queue rows' frames in the pane, by row.
private struct QueueRowFrames: PreferenceKey {
    static let defaultValue: [Int: CGRect] = [:]
    static func reduce(value: inout [Int: CGRect], nextValue: () -> [Int: CGRect]) {
        value.merge(nextValue()) { $1 }
    }
}

/// "playing from {name}": the playlist, album, artist or Liked Songs that's playing (P).
struct PlayingFromPane: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let embedded: Bool
    var body: some View {
        let look = Look.shared
        let ctx = store.playback?.contextUri ?? ""
        let route = contextRoute(ctx)
        PaneScroll(embedded: embedded, follow: route.flatMap { playingRowLoaded($0) ? "\(ctx)|\(store.playback?.item?.uri ?? "")" : nil }) {
            if store.playback?.localFile == true {
                Text("playing from / local files").gFont(12).foregroundStyle(look.muted)
                QueuePane(embedded: true)
            } else if let route {
                HStack(spacing: 6) {
                    Text("playing from").gFont(12).foregroundStyle(look.muted)
                    Button { store.nowPlayingOpen = false; store.push(route) } label: {
                        Text(store.playback?.contextName.isEmpty == false ? store.playback!.contextName : "this list").gFont(12).foregroundStyle(accent)
                    }
                    .buttonStyle(.plain)
                }
                ContextRows(route: route)
            } else {
                Unavailable(title: "not from a playlist", detail: "This isn't playing from a playlist, album, artist or Liked Songs. The queue shows what's next.")
                BracketButton(title: "show the queue") { store.nowPlayingView = .queue }.frame(maxWidth: .infinity)
            }
        }
    }

    /// Whether the song that's playing is among the context's rows yet (they load after the pane opens).
    private func playingRowLoaded(_ route: Route) -> Bool {
        guard let blocks = store.page(route)?.blocks else { return false }
        return blocks.contains { b in
            if case let .rows(id, _, rows, _, _, _, _, _) = b, id == "tracks" || id == "popular" { return rows.contains { store.isCurrent($0.uri) } }
            return false
        }
    }

    private func contextRoute(_ uri: String) -> Route? {
        if uri.hasSuffix(":collection") { return .likedSongs }
        guard let r = routeFor(uri: uri) else { return nil }
        switch r {
        case .playlist, .album, .artist: return r
        default: return nil
        }
    }
}

/// The rows of the context that's playing, in its own order.
struct ContextRows: View {
    @Environment(Store.self) private var store
    let route: Route
    var body: some View {
        let page = store.page(route)
        Group {
            if let page {
                ForEach(Array(page.blocks.enumerated()), id: \.offset) { _, b in
                    if case let .rows(id, _, rows, style, context, playlist, _, _) = b, id == "tracks" || id == "popular" {
                        RowsBlock(title: "", rows: rows, style: style == .album ? .album : .plain, context: context, playlist: playlist, more: nil, collapsed: 0, route: route)
                    }
                }
            } else {
                LoadingNote()
            }
        }
        .onAppear { store.subscribe(route) }
        .onDisappear { store.unsubscribe(route) }
    }
}

/// Devices (desktop right.rs `devices`): the current device and engine status, then
/// "Select another device"; tap to transfer.
struct DevicesPane: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let embedded: Bool
    var body: some View {
        let look = Look.shared
        let d = store.devices
        let current = store.playback?.device
        PaneScroll(embedded: embedded) {
            Text("devices").gFont(12).foregroundStyle(look.muted)
            VStack(alignment: .leading, spacing: 6) {
                Caps(text: "current device")
                HStack(spacing: 10) {
                    Image(systemName: icon(current?.kind ?? "Smartphone")).font(.system(size: 22)).foregroundStyle(accent)
                    Text(current.map { $0.isThis ? "\($0.name) (this device)" : $0.name } ?? "No active device").gFont(16).foregroundStyle(look.foreground)
                }
                // Only when it isn't simply connected.
                if let st = d?.engineStatus ?? store.playback?.engineStatus, !st.hasPrefix("Connected") {
                    Text(st).gFont(11.5).foregroundStyle(look.sub)
                }
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: look.radius).fill(look.surface))
            Text("Select another device").gFont(14).foregroundStyle(look.foreground).padding(.top, 8)
            let others = (d?.devices ?? []).filter { $0.id != current?.id }
            if others.isEmpty {
                Text(d?.loaded == false ? "looking…" : "No other devices. Open Spotify on another device to see it here.").gFont(12.5).foregroundStyle(look.sub)
            }
            ForEach(others, id: \.id) { dev in
                Button { if !dev.restricted { store.core.transfer(deviceId: dev.id) } } label: {
                    HStack(spacing: 12) {
                        Image(systemName: icon(dev.kind)).font(.system(size: 18)).foregroundStyle(look.sub).frame(width: 28)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(dev.isThis ? "\(dev.name) (this device)" : dev.name).gFont(14).foregroundStyle(look.foreground)
                            Text(sub(dev)).gFont(11.5).foregroundStyle(look.sub)
                        }
                        Spacer()
                    }
                    .padding(.vertical, 8)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(dev.restricted)
            }
        }
    }

    private func sub(_ d: DeviceModel) -> String {
        var parts = [d.kind]
        if let v = d.volumePercent { parts.append("volume \(v)%") }
        if d.restricted { parts.append("can't be controlled remotely") }
        return parts.joined(separator: " • ")
    }

    private func icon(_ kind: String) -> String {
        switch kind.lowercased() {
        case "computer": "desktopcomputer"
        case "smartphone": "iphone"
        case "speaker": "hifispeaker"
        case "tv": "tv"
        case "tablet": "ipad"
        case "automobile": "car"
        case "castaudio", "castvideo": "airplayaudio"
        default: "speaker.wave.2"
        }
    }
}

/// A NowItem as a row (for its menu).
func rowFor(_ item: NowItem) -> Row {
    Row(uri: item.uri, kind: item.kind, title: item.title, artists: item.artists, album: item.album, image: item.imageSmall, durationMs: item.durationMs, explicit: item.explicit, playable: true, note: nil, addedAt: "", trackNumber: 0, discNumber: 1, path: item.path, releaseDate: "", description: "", resumeMs: 0, played: false, parent: item.show)
}

/// `PullDown` when there's somewhere to report the pull to (alongside scrolling).
private struct PullDownIf: ViewModifier {
    let pull: Binding<CGFloat>?
    let done: () -> Void
    func body(content: Content) -> some View {
        if let pull { content.modifier(PullDown(y: pull, simultaneous: true, done: done)) } else { content }
    }
}
