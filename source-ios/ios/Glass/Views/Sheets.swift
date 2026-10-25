import SwiftUI

/// The desktop's dialogs (modals.rs) as sheets: Edit details, Create folder,
/// Rename folder, a new playlist, Add to playlist, Move to folder, About.
struct SheetHost: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let kind: SheetKind

    var body: some View {
        NavigationStack {
            ZStack {
                GlassBackground()
                content.padding(20)
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(kind.closes ? "Close" : "Cancel") { dismiss() }.gFont(14) }
            }
            .toolbarBackground(.hidden, for: .navigationBar)
        }
        .presentationDetents([.medium, .large])
        .presentationBackground(.clear)
    }

    @ViewBuilder private var content: some View {
        switch kind {
        case let .editPlaylist(id, name, description, isPublic, collaborative, image):
            EditPlaylistSheet(id: id, name: name, description: description, isPublic: isPublic, collaborative: collaborative, image: image)
        case let .newFolder(add):
            NameSheet(title: "Create folder", initial: "New Folder", note: "Folders are kept by Glass on this device.", button: "Create") { store.core.createFolder(name: $0, add: add) }
        case let .renameFolder(id, name):
            NameSheet(title: "Rename folder", initial: name, note: nil, button: "Save") { store.core.renameFolder(id: id, name: $0) }
        case let .newPlaylist(uris):
            NameSheet(title: "Create playlist", initial: "New Playlist", note: "New playlists are private.", button: "Create") { store.core.createPlaylist(name: $0, uris: uris) }
        case let .addToPlaylist(uris):
            AddToPlaylistSheet(uris: uris)
        case let .moveToFolder(playlist):
            MoveToFolderSheet(playlist: playlist)
        case .about:
            AboutSheet()
        case .shortcuts:
            ShortcutsSheet()
        case let .appleMusic(paths, playlist):
            AppleMusicSheet(paths: paths, playlist: playlist)
        }
    }
}

extension SheetKind {
    /// Sheets that carry on after they're closed say Close, not Cancel.
    var closes: Bool {
        if case .appleMusic = self { return true }
        return false
    }
}

struct NameSheet: View {
    @GlassAccent private var accent
    @Environment(\.dismiss) private var dismiss
    let title: String
    let initial: String
    let note: String?
    let button: String
    let done: (String) -> Void
    @State private var name = ""
    @FocusState private var focused: Bool

    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 14) {
            Text(title).gFont(20).foregroundStyle(look.foreground)
            TextField("", text: $name)
                .gFont(15).padding(12)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                .overlay(RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(accent.opacity(0.6)))
                .focused($focused)
                .submitLabel(.done)
                .onSubmit(submit)
            if let note { Text(note).gFont(12).foregroundStyle(look.sub) }
            BracketButton(title: button, primary: true, action: submit)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty)
            Spacer()
        }
        .onAppear { name = initial; focused = true }
    }

    private func submit() {
        let n = name.trimmingCharacters(in: .whitespaces)
        guard !n.isEmpty else { return }
        done(n)
        dismiss()
    }
}

/// Edit details (desktop `Modal::EditPlaylist`): name, description, public, collaborative.
struct EditPlaylistSheet: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let id: String
    let name: String
    let description: String
    let isPublic: Bool
    let collaborative: Bool
    let image: String?
    @State private var n = ""
    @State private var d = ""
    @State private var pub = false
    @State private var collab = false

    var body: some View {
        let look = Look.shared
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Edit details").gFont(20).foregroundStyle(look.foreground)
                HStack(alignment: .top, spacing: 14) {
                    Art(url: image, size: 110)
                    VStack(spacing: 10) {
                        TextField("", text: $n, prompt: Text("Add a name").foregroundColor(look.muted))
                            .gFont(15).padding(10)
                            .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                        TextField("", text: $d, prompt: Text("Add an optional description").foregroundColor(look.muted), axis: .vertical)
                            .lineLimit(3...6)
                            .gFont(13).padding(10)
                            .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                    }
                }
                Toggle(isOn: $pub) { Text("Show on your profile (public)").gFont(13.5) }
                    .disabled(collab)
                if collab { Text("Collaborative playlists can't be public").gFont(11.5).foregroundStyle(look.sub) }
                Toggle(isOn: $collab) { Text("Collaborative (others can add songs)").gFont(13.5) }
                BracketButton(title: "Save", primary: true) {
                    let name = n.trimmingCharacters(in: .whitespaces)
                    guard !name.isEmpty else { return }
                    store.core.updatePlaylist(id: id, name: name, description: d, public: pub && !collab, collaborative: collab)
                    dismiss()
                }
                .disabled(n.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .tint(accent)
        .onAppear { n = name; d = description; pub = isPublic; collab = collaborative }
    }
}

/// Add to playlist (for a selection): ＋ New playlist, then the playlists you can edit.
struct AddToPlaylistSheet: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let uris: [String]
    var body: some View {
        let look = Look.shared
        let lists = (store.library?.items ?? []).filter { $0.kind == .playlist && ($0.editable || $0.collaborative) }
        VStack(alignment: .leading, spacing: 8) {
            Text(uris.count == 1 ? "Add to playlist" : "Add \(uris.count) songs to playlist").gFont(20).foregroundStyle(look.foreground)
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    Button { store.sheet = .newPlaylist(uris: uris) } label: {
                        Text("＋  New playlist").gFont(14).foregroundStyle(accent).frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    ForEach(lists, id: \.uri) { p in
                        Button {
                            store.core.addToPlaylist(playlist: String(p.uri.split(separator: ":").last ?? ""), uris: uris)
                            dismiss()
                        } label: {
                            HStack(spacing: 10) {
                                Art(url: p.image, size: 40)
                                Text(p.title).gFont(14).foregroundStyle(look.foreground)
                                Spacer()
                            }
                            .frame(minHeight: 52)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }
}

/// Move to folder ▸ ＋ Create folder, the folders, Remove from folders.
struct MoveToFolderSheet: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    let playlist: String
    var body: some View {
        let look = Look.shared
        let folders = (store.library?.items ?? []).filter { $0.kind == .folder }
        VStack(alignment: .leading, spacing: 8) {
            Text("Move to folder").gFont(20).foregroundStyle(look.foreground)
            Button { store.sheet = .newFolder(add: playlist) } label: {
                Text("＋  Create folder").gFont(14).foregroundStyle(accent).frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            }
            .buttonStyle(.plain)
            ForEach(folders, id: \.uri) { f in
                Button { store.core.moveToFolder(playlist: playlist, folder: folderId(f)); dismiss() } label: {
                    HStack { Image(systemName: "folder").foregroundStyle(look.sub); Text(f.title).gFont(14).foregroundStyle(look.foreground); Spacer() }
                        .frame(minHeight: 48).contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            Button { store.core.moveToFolder(playlist: playlist, folder: nil); dismiss() } label: {
                Text("Remove from folders").gFont(14).foregroundStyle(look.sub).frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            }
            .buttonStyle(.plain)
            Spacer()
        }
    }
}

/// About (desktop `Modal::About`).
struct AboutSheet: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 12) {
            Text("Glass").gFont(24).foregroundStyle(look.foreground)
            Text("Version \(store.core.version())").gFont(13).foregroundStyle(look.sub)
            Text("A quiet, transparent Spotify player, written in Rust.").gFont(14).foregroundStyle(look.foreground)
            Text("Glass is an unofficial Spotify player and isn't affiliated with Spotify. Playing music needs Spotify Premium.").gFont(12.5).foregroundStyle(look.sub)
            BracketButton(title: "What Glass stores and sends") { store.sheet = nil; store.tab = .settings; store.paths[.settings] = [.activity] }
            NavigationLink { LicensesView() } label: {
                Text("Open-source licenses ›").gFont(13.5).foregroundStyle(look.foreground)
            }
            Spacer()
        }
    }
}

/// The open-source projects Glass is built on (Resources/THIRD-PARTY.txt, made by
/// scripts/licenses.py from the core's dependencies).
struct LicensesView: View {
    @State private var text = ""
    var body: some View {
        let look = Look.shared
        ScrollView {
            // In pieces: one huge Text is slow to lay out.
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Array(text.components(separatedBy: "\n\n").enumerated()), id: \.offset) { _, part in
                    Text(part).font(.system(size: 11, design: .monospaced)).foregroundStyle(look.sub).padding(.bottom, 10).textSelection(.enabled)
                }
            }
            .padding(16)
        }
        .background(look.background.ignoresSafeArea())
        .navigationTitle("Open-source licenses")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            if let u = Bundle.main.url(forResource: "THIRD-PARTY", withExtension: "txt"), let t = try? String(contentsOf: u, encoding: .utf8) { text = t }
        }
    }
}
