import SwiftUI

/// The assistant (desktop ui/assistant.rs): billing up top, the conversation with
/// playlist previews and approval requests, then the model, usage and composer at the
/// bottom. A sheet on iPhone, the right-hand column on iPad (`.inspector`), as the
/// desktop's right panel. Everything it shows is the core's `AssistantModel`.
struct AssistantPanel: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var input = ""
    @FocusState private var focused: Bool

    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Caps(text: "assistant")
                Spacer()
                IconButton(icon: "xmark", size: 15, label: "Close") { store.assistantOpen = false }
            }
            .padding(.horizontal, 16).padding(.top, 14).padding(.bottom, 6)
            if let a = store.assistant {
                if !a.enabled {
                    NotReady(why: "The assistant is off. It's optional: turn it on in Settings › Assistant.", limit: false)
                } else {
                    BillingLine(a: a).padding(.horizontal, 16)
                    Hairline().padding(.vertical, 6)
                    if let why = a.notReady {
                        NotReady(why: why, limit: a.limitReached)
                    } else {
                        Transcript(a: a)
                        Hairline()
                        BottomBar(a: a).padding(.horizontal, 16).padding(.vertical, 6)
                        composer(a)
                    }
                }
            } else {
                LoadingNote()
            }
            Spacer(minLength: 0)
        }
        .background(look.background.ignoresSafeArea())
        .foregroundStyle(look.foreground)
        .onAppear { focused = store.assistant?.notReady == nil && store.assistant?.enabled == true }
    }

    private func composer(_ a: AssistantModel) -> some View {
        let look = Look.shared
        return HStack(alignment: .bottom, spacing: 8) {
            TextField("ask, or tell Glass what to play…", text: $input, axis: .vertical)
                .gFont(13)
                .lineLimit(1...5)
                .focused($focused)
                .accessibilityIdentifier("assistant-input")
                .submitLabel(.send)
                .onSubmit(send)
                .padding(10)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                .overlay(RoundedRectangle(cornerRadius: look.smallRadius).stroke(look.divider, lineWidth: 1))
            if a.running != nil {
                BracketButton(title: "stop") { store.core.aiCancel() }
            } else {
                BracketButton(title: "send", primary: true) { send() }.disabled(input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(.horizontal, 16).padding(.bottom, 12)
    }

    private func send() {
        let t = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        input = ""
        store.core.aiSend(text: t)
    }
}

/// "Using ChatGPT plan · Manage usage", or the API-key note.
private struct BillingLine: View {
    let a: AssistantModel
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 2) {
            if a.apiKeyBilling {
                Text("Your API key · billed by OpenAI").gFont(11.5).foregroundStyle(look.muted)
            } else {
                HStack(spacing: 4) {
                    Text("Using ChatGPT plan ·").gFont(11.5).foregroundStyle(look.muted)
                    Button("Manage usage") { openManageUsage() }.gFont(11.5).foregroundStyle(look.sub)
                }
            }
            if a.modelsError != nil { Text("Couldn't load the list of models. Check your connection.").gFont(11.5).foregroundStyle(look.sub) }
        }
    }
}

func openManageUsage() {
    if let u = URL(string: "https://chatgpt.com/settings/usage") { UIApplication.shared.open(u) }
}

/// When the assistant can't be used yet: why, and what to do (desktop `not_ready`).
private struct NotReady: View {
    @Environment(Store.self) private var store
    let why: String
    let limit: Bool
    var body: some View {
        let look = Look.shared
        VStack(spacing: 12) {
            if limit {
                VStack(alignment: .leading, spacing: 6) {
                        Text("ChatGPT").gFont(11.5).foregroundStyle(look.muted)
                        Text("Usage limit reached").gFont(15)
                        Text("Review your plan or this app's limit in ChatGPT settings.").gFont(11.5).foregroundStyle(look.sub)
                        HStack {
                            BracketButton(title: "Manage usage", primary: true) { openManageUsage() }
                            BracketButton(title: "try again") { store.core.aiLimitAcknowledged() }
                        }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .panel()
            } else {
                Unavailable(title: "assistant", detail: why)
                BracketButton(title: "open settings", primary: true) {
                    store.assistantOpen = false
                    store.settingsQuery = "assistant"
                    store.push(.settings)
                }
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity)
    }
}

/// The conversation, approval cards, then the previews (newest first).
private struct Transcript: View {
    @Environment(Store.self) private var store
    let a: AssistantModel
    var body: some View {
        let look = Look.shared
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 10) {
                    if a.lines.isEmpty && a.drafts.isEmpty && a.picks.isEmpty {
                        Text(a.dataNote).gFont(11.5).foregroundStyle(look.muted).padding(.top, 8)
                    }
                    ForEach(Array(a.lines.enumerated()), id: \.offset) { _, l in
                        switch l {
                        case let .user(text):
                            VStack(alignment: .leading, spacing: 2) {
                                Text("you").gFont(10.5).foregroundStyle(look.muted)
                                Text(text).gFont(13).textSelection(.enabled)
                            }
                        case let .assistant(text, waiting):
                            VStack(alignment: .leading, spacing: 2) {
                                Text("glass").gFont(10.5).foregroundStyle(look.accent)
                                Text(waiting ? "…" : text).gFont(13).foregroundStyle(look.sub).textSelection(.enabled)
                            }
                        case let .note(text):
                            Text("· \(text)").gFont(11.5).foregroundStyle(look.muted)
                        case let .error(text, retry, manageUsage):
                            VStack(alignment: .leading, spacing: 4) {
                                Text(text).gFont(11.5).foregroundStyle(look.accent)
                                if retry {
                                    HStack(spacing: 12) {
                                        Button("retry") { store.core.aiRetry() }.gFont(12).foregroundStyle(look.sub)
                                        if manageUsage { Button("Manage usage") { openManageUsage() }.gFont(12).foregroundStyle(look.accent) }
                                    }
                                }
                            }
                        }
                    }
                    ForEach(a.approvals, id: \.id) { ap in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(ap.text).gFont(12.5)
                            HStack {
                                BracketButton(title: "approve", primary: true) { store.core.aiApprove(id: ap.id) }
                                BracketButton(title: "decline") { store.core.aiDecline(id: ap.id) }
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .overlay(RoundedRectangle(cornerRadius: look.smallRadius).stroke(look.accent, lineWidth: 1))
                    }
                    ForEach(a.drafts, id: \.id) { d in DraftCard(d: d, busy: a.running != nil) }
                    // Recommendations last, the newest nearest the composer.
                    ForEach(a.picks, id: \.id) { p in PicksCard(p: p) }
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
            }
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: a.lines.count) { withAnimation { proxy.scrollTo("end", anchor: .bottom) } }
            .onChange(of: a.picks.count) { withAnimation { proxy.scrollTo("end", anchor: .bottom) } }
            .onChange(of: a.lines.last) { proxy.scrollTo("end", anchor: .bottom) }
            .onAppear { proxy.scrollTo("end", anchor: .bottom) }
        }
    }
}

/// A playlist preview (desktop `draft_card`): editable name, rows with ↑ ↓ ×, and
/// the save / replace / add-to, save as new, decline or discard buttons.
private struct DraftCard: View {
    @Environment(Store.self) private var store
    let d: DraftModel
    let busy: Bool
    @State private var name = ""
    @FocusState private var naming: Bool

    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text(d.spotify ? "Spotify preview" : "preview").gFont(11).foregroundStyle(look.muted)
                if d.editable {
                    TextField("name", text: $name).gFont(14.5).focused($naming)
                        .onSubmit(rename)
                        .onChange(of: naming) { if !naming { rename() } }
                } else {
                    Text(d.name).gFont(14.5)
                }
            }
            Text(d.meta).gFont(11.5).foregroundStyle(look.sub)
            ScrollView {
                VStack(spacing: 0) {
                    ForEach(Array(d.rows.enumerated()), id: \.offset) { i, r in
                        HStack(spacing: 6) {
                            Text(String(format: "%02d", i + 1)).gFont(11).foregroundStyle(look.muted)
                            Text(r.label).gFont(12).foregroundStyle(r.known ? look.foreground : look.muted).lineLimit(1)
                            Spacer(minLength: 4)
                            if r.known { Text(Fmt.ms(r.durationMs)).gFont(11).foregroundStyle(look.sub) }
                            if d.editable {
                                Button("↑") { store.core.aiEditDraft(draft: d.id, edit: .move(index: UInt32(i), by: -1)) }.disabled(i == 0).accessibilityLabel("Move up")
                                Button("↓") { store.core.aiEditDraft(draft: d.id, edit: .move(index: UInt32(i), by: 1)) }.disabled(i + 1 == d.rows.count).accessibilityLabel("Move down")
                                Button("×") { store.core.aiEditDraft(draft: d.id, edit: .remove(index: UInt32(i))) }.accessibilityLabel("Remove from the preview")
                            }
                        }
                        .gFont(12)
                        .foregroundStyle(look.muted)
                        .frame(minHeight: 26)
                    }
                }
            }
            .frame(maxHeight: 220)
            if let r = d.saved {
                HStack {
                    Text(d.spotify ? "saved to Spotify ✓" : "saved ✓").gFont(12).foregroundStyle(look.accent)
                    Button("open") { store.assistantOpen = false; store.push(r) }.gFont(12).foregroundStyle(look.sub)
                }
            } else if d.saving {
                Text("saving to Spotify…").gFont(12).foregroundStyle(look.sub)
            } else {
                if let asked = d.asked { Text(asked.text).gFont(11.5).foregroundStyle(look.sub) }
                FlowRow(spacing: 4) {
                    BracketButton(title: d.saveLabel, primary: true) { store.core.aiSaveDraft(draft: d.id, replace: d.replaces) }
                    if d.canSaveAsNew { BracketButton(title: "save as new") { store.core.aiSaveDraft(draft: d.id, replace: nil) } }
                    if let asked = d.asked {
                        BracketButton(title: "decline") { store.core.aiDecline(id: asked.id) }
                    } else {
                        BracketButton(title: "discard") { store.core.aiDiscardDraft(draft: d.id) }
                    }
                }
                .disabled(busy)
            }
        }
        .padding(10)
        .overlay(RoundedRectangle(cornerRadius: look.smallRadius).stroke(look.divider, lineWidth: 1))
        .onAppear { name = d.name }
        .onChange(of: d.name) { if !naming { name = d.name } }
    }

    private func rename() {
        let n = name.trimmingCharacters(in: .whitespaces)
        if !n.isEmpty, n != d.name { store.core.aiEditDraft(draft: d.id, edit: .rename(name: n)) }
    }
}

/// Recommendations (desktop `picks_card`): ▶ plays a song's 30-second preview (what's
/// playing pauses for it), + adds it to the playlist they're for, or to one chosen from
/// a menu; "add all" adds the rest.
private struct PicksCard: View {
    @Environment(Store.self) private var store
    let p: PicksModel

    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Text("recommendations").gFont(11).foregroundStyle(look.muted)
                Text(p.title).gFont(14.5).lineLimit(1)
            }
            Text(meta).gFont(11.5).foregroundStyle(look.sub)
            ForEach(p.songs, id: \.uri) { s in row(s) }
            FlowRow(spacing: 4) {
                if let id = p.forPlaylist, let name = p.forPlaylistName, p.songs.filter({ !$0.addedTo.contains(id) }).count > 1 {
                    BracketButton(title: "add all to “\(name.prefix(18))”", primary: true) { store.core.aiAddPicks(card: p.id, uri: nil, playlist: id) }
                }
                BracketButton(title: "dismiss") { store.core.aiDismissPicks(card: p.id) }
            }
        }
        .padding(10)
        .overlay(RoundedRectangle(cornerRadius: look.smallRadius).stroke(look.divider, lineWidth: 1))
    }

    private var meta: String {
        let n = p.songs.count == 1 ? "1 song" : "\(p.songs.count) songs"
        if let name = p.forPlaylistName { return "\(n) for “\(name)”, none already in it · ▶ plays 30 seconds" }
        return "\(n) · ▶ plays 30 seconds, + adds to a playlist"
    }

    private func row(_ s: PickRow) -> some View {
        let look = Look.shared
        let playing: UInt32? = if case let .playing(left) = s.preview { left } else { nil }
        return HStack(alignment: .top, spacing: 8) {
            Button { store.core.aiPreview(uri: s.uri) } label: {
                ZStack {
                    switch s.preview {
                    case .idle: Image(systemName: "play.fill")
                    case .loading: ProgressView().controlSize(.mini)
                    case .playing: Image(systemName: "stop.fill")
                    }
                }
                .font(.system(size: 12))
                .foregroundStyle(s.preview == .idle ? look.sub : look.accent)
                .frame(width: 28, height: 28)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(s.preview == .idle ? "Play a 30-second preview of \(s.title)" : "Stop the preview of \(s.title)")
            VStack(alignment: .leading, spacing: 2) {
                Text("\(s.title) — \(s.artist)").gFont(12.5).lineLimit(1)
                if !s.why.isEmpty { Text(s.why).gFont(11).foregroundStyle(look.muted).fixedSize(horizontal: false, vertical: true) }
            }
            .padding(.top, 5)
            Spacer(minLength: 4)
            Text(playing.map { String(format: "0:%02d", $0) } ?? Fmt.ms(s.durationMs)).gFont(11).foregroundStyle(playing == nil ? look.sub : look.accent).padding(.top, 6)
            add(s).padding(.top, 5)
        }
    }

    @ViewBuilder private func add(_ s: PickRow) -> some View {
        let look = Look.shared
        if let id = p.forPlaylist {
            if s.addedTo.contains(id) {
                Text("added ✓").gFont(11.5).foregroundStyle(look.accent)
            } else {
                Button("+ add") { store.core.aiAddPicks(card: p.id, uri: s.uri, playlist: id) }
                    .gFont(11.5).foregroundStyle(look.sub)
                    .accessibilityLabel("Add \(s.title) to \(p.forPlaylistName ?? "the playlist")")
            }
        } else {
            Menu {
                ForEach(writable, id: \.uri) { pl in
                    let pid = String(pl.uri.split(separator: ":").last ?? "")
                    Button(s.addedTo.contains(pid) ? "\(pl.title)  ✓" : pl.title) { store.core.aiAddPicks(card: p.id, uri: s.uri, playlist: pid) }
                        .disabled(s.addedTo.contains(pid))
                }
            } label: {
                Text(s.addedTo.isEmpty ? "+ add" : "added ✓").gFont(11.5).foregroundStyle(s.addedTo.isEmpty ? look.sub : look.accent)
            }
            .accessibilityLabel("Add \(s.title) to a playlist")
        }
    }

    /// Playlists songs can be added to (owned or collaborative), as in Add to playlist.
    private var writable: [LibItem] {
        (store.library?.items ?? []).filter { $0.kind == .playlist && ($0.editable || $0.collaborative) }
    }
}

/// Status and stop while running; otherwise the model, usage, new chat and undo.
private struct BottomBar: View {
    @Environment(Store.self) private var store
    let a: AssistantModel
    var body: some View {
        let look = Look.shared
        HStack(spacing: 10) {
            if let status = a.running {
                Text(status).gFont(11.5).foregroundStyle(look.sub)
                Spacer()
                Button("stop") { store.core.aiCancel() }.gFont(11.5).foregroundStyle(look.accent)
            } else {
                ModelMenu(a: a, compact: true)
                if !a.usage.isEmpty { Text(a.usage).gFont(10.5).foregroundStyle(look.muted).minimumScaleFactor(0.7).accessibilityHint("This session: tokens in / out") }
                Spacer()
                if a.canUndo { Button("undo") { store.core.aiUndo() }.gFont(11.5).foregroundStyle(look.sub).accessibilityHint("Undo the last change the assistant made") }
                Button("new chat") { store.core.aiNewChat() }.gFont(11.5).foregroundStyle(look.sub)
            }
        }
        .lineLimit(1)
    }
}

/// The model, as a small menu (with "refresh list").
struct ModelMenu: View {
    @Environment(Store.self) private var store
    let a: AssistantModel
    var compact = false
    var body: some View {
        let look = Look.shared
        let shown = a.model.flatMap { m in a.models.first { $0.slug == m }?.name } ?? {
            switch a.modelsState {
            case .loading: "loading…"
            case .failed: "no models"
            case .unknown: a.apiKeyBilling ? "available once a key is saved" : "available after signing in"
            case .ready: "choose a model"
            }
        }()
        Menu {
            ForEach(a.models, id: \.slug) { m in
                Button { store.core.aiSetModel(slug: m.slug) } label: {
                    if a.model == m.slug { Label(m.name, systemImage: "checkmark") } else { Text(m.name) }
                }
            }
            Divider()
            Button("refresh list") { store.core.aiRefreshModels() }
        } label: {
            HStack(spacing: 3) {
                Text(shown).gFont(compact ? 11 : 13).foregroundStyle(look.sub).lineLimit(1)
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 9)).foregroundStyle(look.muted)
            }
        }
    }
}

/// The ChatGPT sign-in page in the system's web sign-in sheet; the core's loopback
/// listener gets the code (as on the desktop) and sends the sheet back to the app.
struct AssistantSignInSession: View {
    @Environment(Store.self) private var store
    @Environment(\.webAuthenticationSession) private var session

    var body: some View {
        Color.clear
            .task(id: store.assistantSignIn?.url) {
                guard let req = store.assistantSignIn, let url = URL(string: req.url) else { return }
                do {
                    _ = try await session.authenticate(using: url, callback: .customScheme(req.callbackScheme), preferredBrowserSession: .shared, additionalHeaderFields: [:])
                } catch {
                    // Closed before finishing: stop waiting (unless the code already arrived).
                    if store.assistant?.signingIn == true { store.core.aiCancelSignIn() }
                }
                store.assistantSignIn = nil
            }
    }
}

/// The navigation bar's way in (when the assistant is on).
struct AssistantButton: View {
    @Environment(Store.self) private var store
    @Environment(\.horizontalSizeClass) private var hSize
    var body: some View {
        // iPad has it on the rail.
        if store.assistant?.enabled == true, hSize != .regular {
            IconButton(icon: "sparkle", size: 16, on: store.assistantOpen, label: "Assistant") { store.assistantOpen.toggle() }
                .accessibilityIdentifier("assistant-button")
        }
    }
}
