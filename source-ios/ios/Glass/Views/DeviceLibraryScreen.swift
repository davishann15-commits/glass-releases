import SwiftUI

/// Another device's Local Files (local files sharing, core share_app.rs): its songs,
/// listed by that device, to pick and get, as songs are picked for Apple Music. Songs
/// already on this iPhone are marked and left out. Nothing to set up: Glass on your
/// other devices, signed in to the same account and on the same Wi-Fi, shows up in
/// Local Files by itself.
struct DeviceLibraryScreen: View {
    @Environment(Store.self) private var store
    let peer: String
    let name: String
    @State private var filter = ""
    @State private var picks: Set<String> = []

    var body: some View {
        let look = Look.shared
        let lib = store.deviceLibrary?.peer == peer ? store.deviceLibrary : nil
        let shown = lib.map { shownSongs($0) } ?? []
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 8) {
                BlinkTitle(text: name)
                if let lib, !lib.loading, lib.error == nil {
                    let missing = lib.songs.filter { !$0.here }.count
                    Text("\(Self.songs(lib.songs.count)) · \(missing == 0 ? "all on this iPhone" : "\(Self.songs(missing)) not on this iPhone")").gFont(12.5).foregroundStyle(look.sub)
                }
                if let t = store.share?.transfers.first {
                    HStack {
                        Text(t.text).gFont(12).foregroundStyle(look.sub)
                        if let p = t.progress { ProgressView(value: Double(p)).frame(width: 110).tint(look.accent) }
                    }
                }
                if lib == nil || lib?.loading == true {
                    LoadingNote()
                } else if let e = lib?.error {
                    Unavailable(title: "couldn't load the songs", detail: e)
                    BracketButton(title: "try again") { store.core.shareBrowse(peer: peer) }.frame(maxWidth: .infinity)
                } else if let lib, lib.songs.isEmpty {
                    Unavailable(title: "no songs", detail: "\(name) has nothing in Local Files.")
                } else {
                    FilterField(text: $filter, prompt: "filter")
                    ForEach(shown, id: \.id) { s in
                        DeviceSongRow(song: s, picked: picks.contains(s.id)) {
                            if picks.contains(s.id) { picks.remove(s.id) } else { picks.insert(s.id) }
                        }
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 40)
        }
        .scrollIndicators(.hidden)
        .scrollDismissesKeyboard(.interactively)
        .refreshable { store.core.shareBrowse(peer: peer) }
        .safeAreaInset(edge: .bottom) {
            if let lib, !lib.songs.isEmpty {
                pickBar(shown.filter { !$0.here }.map(\.id), all: lib.songs)
            }
        }
        .glassNav("music / local / devices")
        .foregroundStyle(look.foreground)
        .onAppear { store.core.shareBrowse(peer: peer) }
        // Songs that arrived are here now: they can't be picked any more.
        .onChange(of: store.deviceLibrary?.songs.filter(\.here).count) { _, _ in
            let here = Set(store.deviceLibrary?.songs.filter(\.here).map(\.id) ?? [])
            picks.subtract(here)
        }
    }

    private func shownSongs(_ lib: DeviceLibraryModel) -> [DeviceSongModel] {
        let q = filter.trimmingCharacters(in: .whitespaces).lowercased()
        let songs = q.isEmpty ? lib.songs : lib.songs.filter { "\($0.title) \($0.artist) \($0.album)".lowercased().contains(q) }
        return songs.sorted { ($0.artist.lowercased(), $0.album.lowercased(), $0.title.lowercased()) < ($1.artist.lowercased(), $1.album.lowercased(), $1.title.lowercased()) }
    }

    /// "2 songs · 31 MB" [all shown] [get]: what's picked, at the bottom.
    private func pickBar(_ shown: [String], all: [DeviceSongModel]) -> some View {
        let look = Look.shared
        let bytes = all.filter { picks.contains($0.id) }.reduce(UInt64(0)) { $0 + $1.size }
        return HStack(spacing: 6) {
            Text(picks.isEmpty ? "Tap songs to pick them" : "\(Self.songs(picks.count)) · \(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file))")
                .gFont(13).foregroundStyle(look.sub).monospacedDigit().lineLimit(1)
            Spacer(minLength: 4)
            if !shown.isEmpty, !Set(shown).isSubset(of: picks) {
                BracketButton(title: "all") { picks.formUnion(shown) }
            } else if !picks.isEmpty {
                BracketButton(title: "none") { picks = [] }
            }
            BracketButton(title: "get", icon: "arrow.down", primary: true) {
                store.core.shareGet(peer: peer, ids: all.map(\.id).filter { picks.contains($0) })
                picks = []
            }
            .disabled(picks.isEmpty)
            .accessibilityLabel(picks.count == 1 ? "Get 1 song" : "Get \(picks.count) songs")
        }
        .padding(.horizontal, 14).padding(.vertical, 4)
        .panel()
        .padding(.horizontal, 12)
        .accessibilityElement(children: .contain)
    }

    static func songs(_ n: Int) -> String { n == 1 ? "1 song" : "\(n) songs" }
}

/// One of another device's songs: tap to pick it (songs already here can't be).
struct DeviceSongRow: View {
    let song: DeviceSongModel
    let picked: Bool
    let toggle: () -> Void
    var body: some View {
        let look = Look.shared
        Button(action: toggle) {
            HStack(spacing: 10) {
                Group {
                    if song.here {
                        Image(systemName: "checkmark").font(.system(size: 13, weight: .semibold)).foregroundStyle(look.muted)
                    } else {
                        PickMark(on: picked)
                    }
                }
                .frame(width: 26)
                VStack(alignment: .leading, spacing: 2) {
                    Text(song.title.isEmpty ? "Untitled" : song.title).gFont(14).foregroundStyle(song.here ? look.muted : look.foreground).lineLimit(1)
                    let about = song.here ? "on this iPhone" : [song.artist, song.album].filter { !$0.isEmpty }.joined(separator: " · ")
                    if !about.isEmpty { Text(about).gFont(12).foregroundStyle(look.muted).lineLimit(1) }
                }
                Spacer(minLength: 4)
                Text(Fmt.ms(song.durationMs)).gFont(12).foregroundStyle(look.sub).monospacedDigit()
            }
            .padding(.vertical, 5)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(song.here)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(picked ? .isSelected : [])
        .accessibilityHint(song.here ? "Already on this iPhone" : "Picks it to get")
    }
}
