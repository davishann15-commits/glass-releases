import SwiftUI
import UniformTypeIdentifiers

/// Add to Apple Music (core src/apple_music, a port of ByeTunes): puts Local Files
/// songs into this iPhone's own music library, through LocalDevVPN and a pairing
/// file. Shows the setup, the pairing file, a run's progress and its result.
/// `paths`: the songs to add (none: just the setup, from Settings).
struct AppleMusicSheet: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let paths: [String]
    let playlist: String?
    @State private var importing = false
    @State private var inPlaylist = false
    @State private var playlistName = ""
    @State private var asked = false
    @State private var showDetails = false
    @State private var showSetup = false

    var body: some View {
        let look = Look.shared
        let m = store.appleMusic
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Add to Apple Music").gFont(20).foregroundStyle(look.foreground)
                if let m, m.busy {
                    progress(m)
                } else if let r = m?.result, asked || paths.isEmpty {
                    result(r)
                }
                if !paths.isEmpty, !(m?.busy ?? false), !(asked && m?.result?.ok == true) {
                    add(m)
                }
                pairing(m)
                setup(m)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .scrollIndicators(.hidden)
        .tint(accent)
        .onAppear {
            playlistName = playlist ?? ""
            inPlaylist = playlist != nil
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.propertyList, .xml, .data, .item]) { r in
            guard case let .success(url) = r else { return }
            let ok = url.startAccessingSecurityScopedResource()
            defer { if ok { url.stopAccessingSecurityScopedResource() } }
            // Read here and handed over as bytes: the core keeps it in the Keychain only.
            if let data = try? Data(contentsOf: url) { store.core.appleMusicImportPairing(file: data) } else { store.show(toast: "That file couldn't be read") }
        }
    }

    // MARK: Adding

    @ViewBuilder private func add(_ m: AppleMusicModel?) -> some View {
        let look = Look.shared
        let n = paths.count
        Text("Adds \(n == 1 ? "1 song" : "\(n) songs") to your Apple Music library, as your own files (even where Apple Music has the song). Ones Glass added before are skipped.")
            .gFont(13).foregroundStyle(look.sub)
        Toggle(isOn: $inPlaylist) { Text("Add to a playlist too").gFont(13.5) }
        if inPlaylist {
            TextField("", text: $playlistName, prompt: Text("Playlist name").foregroundColor(look.muted))
                .gFont(14).padding(10)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
            Text("If this playlist exists, the songs are added to it.").gFont(11.5).foregroundStyle(look.muted)
        }
        BracketButton(title: n == 1 ? "Add 1 song" : "Add \(n) songs", icon: "plus", primary: true) {
            asked = true
            showDetails = false
            store.core.appleMusicAdd(paths: paths, playlist: inPlaylist ? playlistName.trimmingCharacters(in: .whitespaces) : nil)
            store.appleMusicPicks = nil
        }
        .disabled((m?.pairing == nil && !store.demo) || (inPlaylist && playlistName.trimmingCharacters(in: .whitespaces).isEmpty))
        if m?.pairing == nil && !store.demo {
            Text("Set up below first (once).").gFont(12).foregroundStyle(look.sub)
        }
    }

    @ViewBuilder private func progress(_ m: AppleMusicModel) -> some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 8) {
            Text(m.step.isEmpty ? "Working…" : m.step).gFont(14).foregroundStyle(look.foreground)
            if let p = m.progress {
                ProgressView(value: Double(p)).tint(accent)
            } else {
                ProgressView().controlSize(.small).tint(look.muted)
            }
            Text("Keep Glass open until this finishes.").gFont(11.5).foregroundStyle(look.muted)
            if m.canStop { BracketButton(title: "Stop") { store.core.appleMusicStop() } }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: look.radius).fill(look.surface))
    }

    @ViewBuilder private func result(_ r: AppleMusicResult) -> some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 6) {
            Text(r.text).gFont(14).foregroundStyle(r.ok ? look.foreground : look.error).textSelection(.enabled)
            if !r.details.isEmpty {
                Button { showDetails.toggle() } label: {
                    Text("\(r.details.count) skipped \(showDetails ? "▾" : "▸")").gFont(12.5).foregroundStyle(look.sub).frame(minHeight: 32)
                }
                .buttonStyle(.plain)
                if showDetails {
                    ForEach(Array(r.details.enumerated()), id: \.offset) { _, d in Text(d).gFont(12).foregroundStyle(look.muted) }
                }
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: look.radius).fill(look.surface))
        .accessibilityElement(children: .combine)
    }

    // MARK: Pairing file and setup

    @ViewBuilder private func pairing(_ m: AppleMusicModel?) -> some View {
        let look = Look.shared
        SectionTitle(title: "Setup")
        Text(m?.pairing == nil ? "Not set up yet." : "Set up for this iPhone.").gFont(13).foregroundStyle(look.sub)
        FlowRow(spacing: 4) {
            BracketButton(title: m?.pairing == nil ? "Choose pairing file…" : "Replace…", icon: "key") { importing = true }
            if m?.pairing != nil {
                BracketButton(title: "Test connection") { asked = true; store.core.appleMusicCheck() }.disabled(m?.busy ?? false)
                BracketButton(title: "Remove") {
                    store.confirm = Confirm(title: "Remove the pairing file?", body: "You'll need to choose it again to add songs.", button: "Remove") { store.core.appleMusicForgetPairing() }
                }
            }
        }
        if m?.canRestore == true {
            BracketButton(title: "Undo last add…", icon: "arrow.uturn.backward") {
                store.confirm = Confirm(title: "Undo the last add?", body: "Your Apple Music library goes back to how it was before Glass last added songs. Anything added since then is removed.", button: "Undo") {
                    asked = true
                    store.core.appleMusicRestore()
                }
            }
            .disabled((m?.busy ?? false) || m?.pairing == nil)
        }
    }

    @ViewBuilder private func setup(_ m: AppleMusicModel?) -> some View {
        let look = Look.shared
        let rp = m?.needsRemotePairing ?? true
        if m?.pairing == nil || showSetup {
            SectionTitle(title: "How to set up")
            VStack(alignment: .leading, spacing: 8) {
                step(1, "Get LocalDevVPN from the App Store, open it and tap Connect. Keep it on while songs are added.")
                step(2, "On a computer, use idevice_pair (github.com/jkcoxson/idevice_pair) to save \(rp ? "an RP pairing file" : "a pairing file") for this iPhone, then AirDrop it here.")
                step(3, "Tap Choose pairing file above.")
                step(4, "Pick songs in Local Files and tap Add to Apple Music. Restart your iPhone to see them in the Music app.")
            }
        } else {
            Button { showSetup = true } label: { Text("How to set up ▸").gFont(12.5).foregroundStyle(look.sub).frame(minHeight: 32) }.buttonStyle(.plain)
        }
        Text("Glass keeps a backup, so you can undo the last add.").gFont(11.5).foregroundStyle(look.muted)
    }

    private func step(_ n: Int, _ text: String) -> some View {
        let look = Look.shared
        return HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text("\(n).").gFont(13).foregroundStyle(look.muted).frame(width: 18, alignment: .trailing)
            Text(text).gFont(13).foregroundStyle(look.sub).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The picking bar at the bottom of Local Files while songs are picked for Apple Music.
struct AppleMusicPickBar: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    /// The songs shown, in order (for "all shown").
    let shown: [String]

    var body: some View {
        let look = Look.shared
        let picks = store.appleMusicPicks ?? []
        HStack(spacing: 6) {
            Text(picks.count == 1 ? "1 song" : "\(picks.count) songs").gFont(13).foregroundStyle(look.sub).monospacedDigit()
            Spacer(minLength: 4)
            if !shown.isEmpty, !Set(shown).isSubset(of: picks) {
                BracketButton(title: "all") { store.appleMusicPicks = picks.union(shown) }
            } else if !picks.isEmpty {
                BracketButton(title: "none") { store.appleMusicPicks = [] }
            }
            BracketButton(title: "add", icon: "plus", primary: true) {
                let order = store.local?.tracks.map(\.path) ?? []
                store.sheet = .appleMusic(paths: order.filter { picks.contains($0) }, playlist: nil)
            }
            .disabled(picks.isEmpty)
            BracketButton(title: "done") { store.appleMusicPicks = nil }
        }
        .padding(.horizontal, 14).padding(.vertical, 4)
        .panel()
        .padding(.horizontal, 12)
        .accessibilityElement(children: .contain)
    }
}
