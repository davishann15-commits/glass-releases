import SwiftUI

/// Settings (desktop ui/settings.rs): the same sections, rows, wording and notes,
/// with a search that filters rows by label and note (a matching section title
/// shows the whole section). Rows for desktop-only settings (updates, open at login,
/// the close button) are left out; they're listed in ios/docs/PARITY.md.
struct SettingsScreen: View {
    @Environment(Store.self) private var store

    var body: some View {
        @Bindable var store = store
        let look = Look.shared
        let q = store.settingsQuery.trimmingCharacters(in: .whitespaces).lowercased()
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 6) {
                BlinkTitle(text: "settings")
                FilterField(text: $store.settingsQuery, prompt: "Search in Settings").padding(.bottom, 6)
                if store.settings == nil {
                    LoadingNote()
                } else {
                    ForEach(sections(), id: \.title) { section in
                        let titleHit = !q.isEmpty && section.title.lowercased().contains(q)
                        let rows = section.rows.filter { q.isEmpty || titleHit || $0.matches(q) }
                        if !rows.isEmpty {
                            SectionTitle(title: section.title)
                            ForEach(Array(rows.enumerated()), id: \.offset) { _, r in r.view }
                        }
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 60)
        }
        .scrollIndicators(.hidden)
        .scrollDismissesKeyboard(.interactively)
        .glassNav("settings")
        .onAppear { store.core.settingsOpen(open: true) }
        .onDisappear { store.core.settingsOpen(open: false) }
        .foregroundStyle(look.foreground)
    }

    struct Section {
        let title: String
        let rows: [SRow]
    }

    struct SRow {
        let label: String
        let note: String
        let view: AnyView
        func matches(_ q: String) -> Bool { label.lowercased().contains(q) || note.lowercased().contains(q) }
    }

    private func row<V: View>(_ label: String, _ note: String = "", @ViewBuilder _ control: () -> V) -> SRow {
        SRow(label: label, note: note, view: AnyView(SettingRow(label: label, note: note, control: control())))
    }

    private func toggle(_ label: String, _ note: String = "", _ key: WritableKeyPath<SettingsModel, Bool>) -> SRow {
        let on = store.settings?[keyPath: key] ?? false
        return row(label, note) {
            Toggle(label, isOn: Binding(get: { on }, set: { v in store.editSettings { $0[keyPath: key] = v } })).labelsHidden()
        }
    }

    private static let restart = ""

    private func assistantRows() -> [SRow] {
        guard let a = store.assistant else { return [] }
        let look = Look.shared
        var rows: [SRow] = [
            row("Turn on the assistant", a.enabled ? "" : a.dataNote) {
                Toggle("Turn on the assistant", isOn: Binding(get: { a.enabled }, set: { store.core.aiEnable(on: $0) })).labelsHidden()
            },
        ]
        guard a.enabled else { return rows }
        rows.append(row("Let it use my Spotify library and playback", "Liked Songs, your playlists, listening history and search, so it can answer questions, play and queue songs, like them and build playlists. Spotify's Developer Terms restrict sending Spotify data to AI services; with this off, the assistant only sees your local files.") {
            Toggle("Let it use my Spotify library and playback", isOn: Binding(get: { a.spotify }, set: { store.core.aiUseSpotify(on: $0) })).labelsHidden()
        })
        rows.append(row("What it sends", a.dataNote) { EmptyView() })
        rows.append(row("Pay for requests with", "") {
            ChipPicker(options: [("chatgpt", "my ChatGPT plan"), ("api_key", "my OpenAI API key")], selected: a.apiKeyBilling ? "api_key" : "chatgpt") { v in
                let key = v == "api_key"
                if key != a.apiKeyBilling { store.core.aiSetBilling(apiKey: key) }
            }
        })
        if !a.apiKeyBilling {
            rows.append(row("ChatGPT plan", "Works with ChatGPT Plus and Pro. Requests count toward your plan's limits (on Plus, the five-hour limit is shared with your other apps). Glass can't see your ChatGPT conversations.") { EmptyView() })
            if let r = a.active {
                rows.append(row(r.label, (r.planEnabled ? "ChatGPT plan usage enabled" : "plan usage not enabled") + ". Your sign-in is kept in \(a.store). To revoke access, sign out here (Glass revokes the session), or disconnect Glass in ChatGPT settings.") {
                    VStack(alignment: .trailing, spacing: 2) {
                        if !r.planEnabled { BracketButton(title: "Enable ChatGPT plan use") { store.core.aiSignIn(reconsent: true, newAccount: false) } }
                        BracketButton(title: "Manage usage") { openManageUsage() }
                        BracketButton(title: "Sign out") { store.core.aiSignOut() }
                        BracketButton(title: "Add another account") { store.core.aiSignIn(reconsent: false, newAccount: true) }
                    }
                })
                for o in a.savedAccounts {
                    rows.append(row("Also saved: \(o.label)", "") { BracketButton(title: "Switch") { store.core.aiSelectAccount(clientId: o.clientId) } })
                }
            } else {
                if a.signingIn {
                    rows.append(row("Waiting for the ChatGPT page…", "") { BracketButton(title: "Cancel") { store.core.aiCancelSignIn() } })
                } else {
                    rows.append(row("Sign in", "Opens ChatGPT's sign-in page here in Glass.") {
                        BracketButton(title: "Continue with ChatGPT", primary: true) { store.core.aiSignIn(reconsent: false, newAccount: true) }
                    })
                }
                for o in a.savedAccounts {
                    rows.append(row("Saved: \(o.label)", "") {
                        BracketButton(title: "Continue with this account") {
                            store.core.aiSelectAccount(clientId: o.clientId)
                            store.core.aiSignIn(reconsent: false, newAccount: false)
                        }
                    })
                }
            }
            if let n = a.accountNote {
                rows.append(SRow(label: "Account note", note: n, view: AnyView(Text(n).gFont(12.5).foregroundStyle(look.accent).padding(.vertical, 4))))
            }
        } else {
            rows.append(SRow(label: "API key billing", note: "", view: AnyView(
                Text("API usage is billed by OpenAI to the account that owns the key, separately from any ChatGPT plan. Glass never switches to it on its own.").gFont(12.5).foregroundStyle(look.accent).padding(.vertical, 4)
            )))
            if a.hasApiKey {
                rows.append(row("API key", "A key is saved in \(a.store).") { BracketButton(title: "Remove key") { store.core.aiForgetApiKey() } })
            } else {
                rows.append(row("API key", "Typed here only — the key goes straight into \(a.store) and is never shown, logged or written to a file.") { ApiKeyField() })
            }
        }
        rows.append(row("Model", "GPT-6 Luna is chosen when your account offers it. Glass keeps your choice and never switches to a pricier model by itself.") {
            if a.modelsError != nil { Text("Couldn't load the list. Check your connection.").gFont(12.5).foregroundStyle(look.sub) } else { ModelMenu(a: a) }
        })
        return rows
    }

    private func sections() -> [Section] {
        guard let s = store.settings else { return [] }
        let c = store.config
        let st = store.status
        var out: [Section] = []

        // Account
        var account: [SRow] = []
        if store.demo {
            account.append(row("Demo mode", "Sample data, no account and no network.") { EmptyView() })
        } else {
            let a = store.account
            account.append(row("Signed in as", [a.displayName, a.email, a.country].filter { !$0.isEmpty }.joined(separator: " · ")) {
                Text(a.product == "premium" ? "Premium" : a.product == "free" || a.product == "open" ? "Free" : a.product).gFont(13).foregroundStyle(Look.shared.accent)
            })
            account.append(row("Edit login methods", "Opens your Spotify account page in the browser.") {
                BracketButton(title: "Edit") { if let u = URL(string: "https://www.spotify.com/account/overview/") { UIApplication.shared.open(u) } }
            })
            account.append(row("Log out", "Signs you out of Spotify on this iPhone.") {
                BracketButton(title: "Log out") {
                    store.confirm = Confirm(title: "Log out?", body: "You'll need to sign in again to play music.", button: "Log out") { store.core.logOut() }
                }
            })
        }
        out.append(Section(title: "Account", rows: account))

        // Appearance
        let theme = store.theme
        var appearance: [SRow] = [
            row("Theme", "Colors, text, lyrics and the visualizer.") { BracketButton(title: "Customize") { store.push(.settings); store.sheet = nil; store.themePanel = true } },
            row("Share or replace the theme", "Export your theme to share it, or import one.") { FileTransferButtons(which: .theme) },
        ]
        if theme?.error != nil {
            appearance.append(row("Theme status", "Your theme has a mistake, so the last good one is used. Import a fixed one, or reset it in the theme panel.") { EmptyView() })
        }
        out.append(Section(title: "Appearance", rows: appearance))

        // Assistant (desktop ui/assistant.rs `settings`)
        out.append(Section(title: "Assistant", rows: assistantRows()))

        // Lyrics
        if let c {
            let provider = c.lyricsProvider
            let sourceNote: String = switch provider {
            case "lrclib": "LRCLIB only."
            case "spotify": "Spotify only: Spotify's lyrics for songs played from Spotify."
            default: c.lyricsFallback ? "Auto: LRCLIB first, then Spotify's lyrics for Spotify songs LRCLIB doesn't have." : "Auto: LRCLIB."
            }
            var lyrics: [SRow] = [
                row("Lyrics source", sourceNote) {
                    ChipPicker(options: [("auto", "Auto"), ("lrclib", "LRCLIB"), ("spotify", store.account.phase == .signedIn || store.demo ? "Spotify" : "Spotify · not connected")], selected: provider) { store.core.setLyricsProvider(provider: $0) }
                },
                row("Use Spotify's lyrics when LRCLIB has none") {
                    Toggle("Use Spotify's lyrics when LRCLIB has none", isOn: Binding(get: { c.lyricsFallback }, set: { store.core.setLyricsFallback(on: $0) })).labelsHidden()
                },
                row("Translation", "Show the original, the original with a translation under each line, or the translation alone. 文A in the lyrics view switches too.") {
                    ChipPicker(options: [("off", "original"), ("below", "original + translation"), ("only", "translated")], selected: c.translation) { store.core.setLyricsTranslation(mode: $0) }
                },
            ]
            if c.translation != "off" {
                lyrics.append(row("Translate into", "\(c.translateToName). Type a language code such as en, es or ja, or system for this iPhone's language.") {
                    CommitField(initial: c.translateTo, width: 90) { store.core.setTranslateTo(language: $0) }
                })
                let tnote: String = switch c.translator {
                case "apple": c.appleTranslationAvailable ? "Apple's on-device translation: free and private; nothing leaves this device. A language that isn't downloaded yet is offered in the lyrics view." : "Apple's on-device translation isn't available here: choose auto or assistant."
                case "assistant": "The assistant's model: works with any language; sends the lyrics to OpenAI and counts toward your plan or key."
                default: c.appleTranslationAvailable ? "Auto: Spotify's own translation when it has one, then Apple's on-device translation (free, private), then the assistant if it's on." : "Auto: Spotify's own translation when it has one, then the assistant if it's on."
                }
                var engines = [("auto", "Auto")]
                if c.appleTranslationAvailable { engines.append(("apple", "Apple on-device")) }
                engines.append(("assistant", "Assistant"))
                lyrics.append(row("Translator", tnote) { ChipPicker(options: engines, selected: c.translator) { store.core.setTranslator(engine: $0) } })
            }
            let off = Double(c.lyricsOffsetMs) / 1000
            lyrics.append(row("Timing offset (all songs)", "\(c.lyricsOffsetMs == 0 ? "0.0 s" : String(format: "%+.1f s", off)). Positive shows lyrics earlier. To fix one song, use − and + under its lyrics.") {
                HStack(spacing: 0) {
                    BracketButton(title: "−0.1 s") { store.core.setLyricsOffset(ms: c.lyricsOffsetMs - 100) }
                    BracketButton(title: "+0.1 s") { store.core.setLyricsOffset(ms: c.lyricsOffsetMs + 100) }
                    if c.lyricsOffsetMs != 0 { BracketButton(title: "Reset") { store.core.setLyricsOffset(ms: 0) } }
                }
            })
            let n = st?.lyricsCachedSongs ?? 0
            lyrics.append(row("Saved lyrics", "\(n == 1 ? "1 song" : "\(n) songs") · \((st?.lyricsCacheBytes ?? 0) / 1024) KB") {
                if n > 0 { BracketButton(title: "Clear") { store.core.clearLyricsCache() } }
            })
            lyrics.append(row("About LRCLIB", "Lyrics come from LRCLIB (lrclib.net), a free, community-made database. Glass keeps them on this iPhone and never shares them.") { EmptyView() })
            out.append(Section(title: "Lyrics", rows: lyrics))
        }

        // Downloads
        let dl = st?.downloads ?? 0
        var downloads: [SRow] = [
            row("Downloaded", "\(dl) downloaded · \(String(format: "%.0f", Double(st?.downloadBytes ?? 0) / 1_048_576)) MB\((st?.downloadsPending ?? 0) > 0 ? " · \(st!.downloadsPending) still downloading" : "")") {
                if dl > 0 {
                    BracketButton(title: "Remove all downloads") {
                        store.confirm = Confirm(title: "Remove all downloads?", body: "Downloaded songs will stream again when you play them.", button: "Remove") { store.core.removeAllDownloads() }
                    }
                }
            },
        ]
        if let f = st?.downloadsFailed, f > 0 {
            downloads.append(row("Couldn't download", "\(f == 1 ? "1 song couldn't be downloaded" : "\(f) songs couldn't be downloaded"). Glass tries again the next time it starts.") { EmptyView() })
        }
        downloads.append(row("How downloads work", "Downloaded songs start at once and use no data, but still need a connection to play.") { EmptyView() })
        out.append(Section(title: "Downloads", rows: downloads))

        // Your Library
        // Save songs (desktop settings.rs `save_song_settings`)
        var saving: [SRow] = [
            row("Format", "Share › Download as saves songs in this format. The audio is matched from JioSaavn or YouTube Music, not Spotify. M4A keeps it as it is; MP3 plays anywhere.") {
                ChipPicker(options: [("m4a", "M4A"), ("mp3", "MP3")], selected: s.saveFormat == .mp3 ? "mp3" : "m4a") { v in store.editSettings { $0.saveFormat = v == "mp3" ? .mp3 : .m4a } }
            },
        ]
        if s.saveFormat == .mp3 {
            saving.append(row("MP3 bitrate", "Higher keeps more of the source's quality.") {
                ChipPicker(options: [128, 160, 192, 256, 320].map { ("\($0)", "\($0)") }, selected: "\(s.saveMp3Kbps)") { v in store.editSettings { $0.saveMp3Kbps = UInt16(v) ?? 320 } }
            })
        }
        saving.append(row("Folder", "Songs are saved to \(LocalImport.display(st?.saveDirEffective ?? "")). Glass's own folder shows in the Files app under On My iPhone › Glass.") {
            SaveFolderButtons(custom: s.saveDir != nil)
        })
        saving.append(row("Show saved songs in Local Files", (st?.saveDirEffective ?? "").contains("/Documents") ? "Already in Local Files (it's inside Glass's folder)." : "Adds the folder above to Local Files.") {
            if !(st?.saveDirEffective ?? "").contains("/Documents"), let dir = st?.saveDirEffective {
                BracketButton(title: "Add") { store.core.filesAdded(dirs: [dir]) }
            }
        })
        out.append(Section(title: "Save songs", rows: saving))

        out.append(Section(title: "Your Library", rows: [
            row("Use compact library layout") {
                Toggle("Use compact library layout", isOn: Binding(get: { store.library?.view == .compact }, set: { store.setLibrary(view: $0 ? .compact : .list) })).labelsHidden()
            },
            row("Add songs to Apple Music", store.appleMusic?.pairing == nil ? "Put Local Files songs in the Music app. Needs a one-time setup." : "Put Local Files songs in the Music app. Set up.") {
                BracketButton(title: store.appleMusic?.pairing == nil ? "Set up" : "Open") { store.sheet = .appleMusic(paths: [], playlist: nil) }
            },
            toggle("Share with your other devices", "Glass on your other devices on the same Wi-Fi, signed in to this account, shows up in Local Files by itself: see its songs and get the ones you pick. Works while Glass is open.", \.shareLocalFiles),
        ] + (store.local?.sources ?? []).filter { !$0.hasSuffix("/Documents") }.map { dir in
            row("Show songs from", LocalImport.display(dir)) { BracketButton(title: "Remove") { store.core.removeSource(dir: dir) } }
        }))

        // Display
        out.append(Section(title: "Display", rows: [
            toggle("Open the now-playing screen when playback starts", "", \.nowPlayingOnPlay),
            row("Show notifications when the song changes", "Uses iOS notifications (Glass asks for permission the first time).") {
                Toggle("Show notifications when the song changes", isOn: Binding(get: { s.trackNotifications }, set: { v in
                    if v { TrackNotifier.shared.requestPermission() }
                    store.editSettings { $0.trackNotifications = v }
                })).labelsHidden()
            },
            toggle("Match Now Playing to the album's colors", "Now Playing, its lyrics and visualizer, and the player bar take their colors from the cover of what's playing.", \.albumColors),
            toggle("Show the visualizer under the lyrics", "Follows the audio played on this device. The waveform button on Now Playing switches it too.", \.visualizer),
        ]))

        // Playback
        var playback: [SRow] = [
            toggle("Gapless playback", Self.restart, \.gapless),
            toggle("Autoplay — keep playing similar content when your music ends", Self.restart, \.autoplay),
            toggle("DJ voice — X talks between the DJ's songs", "", \.djVoice),
            toggle("Mono audio — left and right speakers play the same audio", "For Spotify songs. The iPhone's own Mono Audio (Settings › Accessibility › Audio & Visual) covers everything.", \.mono),
            toggle("Normalize volume — same volume level for all songs and podcasts", Self.restart, \.normalize),
        ]
        if s.normalize {
            playback.append(row("Volume level", "Loud may reduce audio quality.") {
                ChipPicker(options: [("quiet", "Quiet"), ("normal", "Normal"), ("loud", "Loud")], selected: s.loudness == .quiet ? "quiet" : s.loudness == .loud ? "loud" : "normal") { v in
                    store.editSettings { $0.loudness = v == "quiet" ? .quiet : v == "loud" ? .loud : .normal }
                }
            })
        }
        playback.append(SRow(label: "Equalizer", note: "presets bass treble 60Hz 150Hz 400Hz 1KHz 2.4KHz 15KHz", view: AnyView(EqualizerCard())))
        out.append(Section(title: "Playback", rows: playback))

        // Audio quality
        out.append(Section(title: "Audio quality", rows: [
            row("Streaming quality", "Also the quality of downloads.") {
                Menu {
                    ForEach([StreamingQuality.normal, .high, .veryHigh], id: \.self) { q in
                        Button(qualityLabel(q: q)) { store.editSettings { $0.streamingQuality = q } }
                    }
                } label: { Text(qualityLabel(q: s.streamingQuality) + " ⌄").gFont(13).foregroundStyle(Look.shared.accent) }
            },
        ]))

        out.append(Section(title: "Explicit content", rows: [
            toggle("Allow explicit content", "When off, explicit songs are greyed out and skipped.", \.allowExplicit),
        ]))

        // Storage
        out.append(Section(title: "Storage", rows: [
            row("Cache", "Audio kept for faster playback; downloads aren't touched. \(st?.audioCacheBytes.map { String(format: "%.0f MB used", Double($0) / 1_048_576) } ?? "measuring…")") {
                BracketButton(title: "Clear cache") { store.core.clearAudioCache() }
            },
            row("Cache size limit (MB)", "Glass trims the cache to this every 10 minutes.") {
                Stepper(value: Binding(get: { Int(s.cacheLimitMb) }, set: { v in store.editSettings { $0.cacheLimitMb = UInt64(v) } }), in: 256...50000, step: 256) {
                    Text("\(s.cacheLimitMb)").gFont(13).monospacedDigit()
                }
                .fixedSize()
            },
        ]))

        out.append(Section(title: "Spotify Connect", rows: [
            row("Device name", "How this iPhone shows up in Spotify on your other devices.") {
                CommitField(initial: s.deviceName, width: 170) { v in store.editSettings { $0.deviceName = v } }
            },
            row("Connection", st?.engineStatus ?? "Connecting…") { BracketButton(title: "Reconnect") { store.core.restartEngine() } },
        ]))

        out.append(Section(title: "About", rows: [
            row("Version", "Glass \(st?.version ?? "") for iOS") { EmptyView() },
            row("Activity", "Everything Glass sends, receives and stores.") { BracketButton(title: "Open") { store.push(.activity) } },
            row("About Glass") { BracketButton(title: "About") { store.sheet = .about } },
        ]))
        return out
    }
}

/// A settings row: label, note, control on the right (desktop `settings::row`).
struct SettingRow<Control: View>: View {
    let label: String
    let note: String
    let control: Control
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .center, spacing: 10) {
                Text(label).gFont(14).foregroundStyle(look.foreground)
                Spacer(minLength: 8)
                control
            }
            if !note.isEmpty {
                Text(note).gFont(12).foregroundStyle(look.sub).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
            }
        }
        .padding(.vertical, 8)
        .frame(minHeight: 36)
        .tint(look.accent)
        .overlay(alignment: .bottom) { Hairline().opacity(0.6) }
        // Each control stays its own element (a row can hold several chips or buttons);
        // switches carry the row's label themselves.
        .accessibilityElement(children: .contain)
    }
}

/// Chips for a choice (desktop settings chips).
struct ChipPicker: View {
    let options: [(String, String)]
    let selected: String
    let pick: (String) -> Void
    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(options, id: \.0) { value, name in Chip(title: name, on: value == selected) { pick(value) } }
            }
        }
        .scrollIndicators(.hidden)
        .frame(maxWidth: 260)
    }
}

/// A text field that applies on return or when focus leaves (desktop: committed on focus loss).
struct CommitField: View {
    let initial: String
    var width: CGFloat = 160
    var placeholder = ""
    let commit: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool
    var body: some View {
        let look = Look.shared
        TextField("", text: $text, prompt: Text(placeholder).foregroundColor(look.muted))
            .gFont(13).multilineTextAlignment(.trailing)
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .focused($focused)
            .frame(width: width)
            .padding(.horizontal, 8).frame(height: 36)
            .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
            .onAppear { text = initial }
            .onSubmit { commit(text) }
            .onChange(of: focused) { _, f in if !f, text != initial { commit(text) } }
    }
}

/// The equalizer card (desktop settings.rs 632–709): on/off, presets (choosing one
/// turns it on), Reset, and the six-band curve with draggable handles (0.1 dB steps;
/// dragging sets the preset to Manual and turns it on). Applies instantly.
struct EqualizerCard: View {
    @Environment(Store.self) private var store
    @State private var drag: [Float]?

    var body: some View {
        let look = Look.shared
        let s = store.settings
        let gains = drag ?? s?.eqGains ?? Array(repeating: 0, count: 6)
        let on = s?.eqEnabled ?? false
        let bands = eqBands()
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Equalizer").gFont(15)
                Toggle("Equalizer", isOn: Binding(get: { on }, set: { v in store.editSettings { $0.eqEnabled = v } })).labelsHidden()
                Spacer()
                Menu {
                    ForEach(eqPresets(), id: \.name) { p in
                        Button(p.name) { store.editSettings { $0.eqPreset = p.name; $0.eqGains = p.gains; $0.eqEnabled = true } }
                    }
                } label: { Text((s?.eqPreset ?? "Flat") + " ⌄").gFont(13).foregroundStyle(look.accent) }
                .accessibilityLabel("Presets")
                BracketButton(title: "Reset") { store.editSettings { $0.eqGains = Array(repeating: 0, count: 6); $0.eqPreset = "Flat" } }
            }
            GeometryReader { g in
                let w = g.size.width, h = g.size.height
                let x = { (i: Int) -> CGFloat in w * (CGFloat(i) + 0.5) / 6 }
                let y = { (db: Float) -> CGFloat in h / 2 - CGFloat(db) / 12 * (h / 2 - 10) }
                ZStack {
                    ForEach([-12, 0, 12], id: \.self) { db in
                        Path { p in p.move(to: CGPoint(x: 0, y: y(Float(db)))); p.addLine(to: CGPoint(x: w, y: y(Float(db)))) }
                            .stroke(look.divider, style: StrokeStyle(lineWidth: 1, dash: db == 0 ? [] : [3, 4]))
                    }
                    Path { p in
                        let pts = (0..<6).map { CGPoint(x: x($0), y: y(gains[$0])) }
                        p.move(to: CGPoint(x: 0, y: pts[0].y))
                        p.addLine(to: pts[0])
                        for i in 1..<6 {
                            let p0 = pts[max(i - 2, 0)], p1 = pts[i - 1], p2 = pts[i], p3 = pts[min(i + 1, 5)]
                            p.addCurve(to: p2, control1: CGPoint(x: p1.x + (p2.x - p0.x) / 6, y: p1.y + (p2.y - p0.y) / 6), control2: CGPoint(x: p2.x - (p3.x - p1.x) / 6, y: p2.y - (p3.y - p1.y) / 6))
                        }
                        p.addLine(to: CGPoint(x: w, y: pts[5].y))
                    }
                    .stroke(on ? look.accent : look.muted, lineWidth: 2)
                    ForEach(0..<6, id: \.self) { i in
                        Circle().fill(on ? look.accent : look.muted).frame(width: 14, height: 14).position(x: x(i), y: y(gains[i]))
                    }
                }
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { v in
                        let i = min(max(Int(v.startLocation.x / w * 6), 0), 5)
                        let db = Float(min(max((h / 2 - v.location.y) / (h / 2 - 10) * 12, -12), 12))
                        var gs = drag ?? gains
                        gs[i] = (db * 10).rounded() / 10
                        drag = gs
                    }
                    .onEnded { _ in
                        if let gs = drag { store.editSettings { $0.eqGains = gs; $0.eqPreset = "Manual"; $0.eqEnabled = true } }
                        drag = nil
                    })
                .accessibilityRepresentation {
                    VStack {
                        ForEach(0..<6, id: \.self) { i in
                            Slider(value: Binding(get: { Double(gains[i]) }, set: { v in store.editSettings { $0.eqGains[i] = Float((v * 10).rounded() / 10); $0.eqPreset = "Manual"; $0.eqEnabled = true } }), in: -12...12, step: 0.1) {
                                Text(bands[i])
                            }
                        }
                    }
                }
            }
            .frame(height: 170)
            HStack {
                ForEach(Array(bands.enumerated()), id: \.offset) { i, b in
                    VStack(spacing: 2) {
                        Text(b).gFont(10.5).foregroundStyle(look.sub)
                        Text(String(format: "%+.1f", gains[i])).gFont(10).foregroundStyle(look.muted).monospacedDigit()
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            Text("Applies instantly to Spotify songs (and playlists' local songs) played on this device.").gFont(12).foregroundStyle(look.sub)
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: look.radius).fill(look.surface))
        .overlay(RoundedRectangle(cornerRadius: look.radius).strokeBorder(look.divider))
        .padding(.vertical, 6)
    }
}


/// The API key field: a secure field whose text goes straight to the core's
/// Keychain write and is cleared at once.
struct ApiKeyField: View {
    @Environment(Store.self) private var store
    @State private var key = ""
    var body: some View {
        HStack(spacing: 6) {
            SecureField("sk-…", text: $key)
                .gFont(13)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .frame(width: 170)
                .onSubmit(save)
            BracketButton(title: "Save key") { save() }
        }
    }
    private func save() {
        let k = key
        key = ""
        store.core.aiSaveApiKey(key: k)
    }
}


/// The save folder: Glass's own (Documents/Glass), or one picked in Files, kept as a
/// security-scoped bookmark and opened again at each launch.
struct SaveFolderButtons: View {
    @Environment(Store.self) private var store
    let custom: Bool
    @State private var picking = false
    static let key = "saveFolderBookmark"

    var body: some View {
        HStack(spacing: 0) {
            if custom {
                BracketButton(title: "Reset") {
                    UserDefaults.standard.removeObject(forKey: Self.key)
                    store.editSettings { $0.saveDir = nil }
                }
            }
            BracketButton(title: "Change…") { picking = true }
        }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.folder]) { r in
            guard case let .success(url) = r, url.startAccessingSecurityScopedResource() else { return }
            if let data = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
                UserDefaults.standard.set(data, forKey: Self.key)
            }
            store.editSettings { $0.saveDir = url.path }
        }
    }

    /// At launch: open the picked folder again (or go back to Glass's own if it's gone).
    @MainActor static func restore(store: Store) {
        guard let data = UserDefaults.standard.data(forKey: key) else { return }
        var stale = false
        guard let url = try? URL(resolvingBookmarkData: data, options: [], relativeTo: nil, bookmarkDataIsStale: &stale), url.startAccessingSecurityScopedResource() else {
            UserDefaults.standard.removeObject(forKey: key)
            return
        }
        if stale, let fresh = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) {
            UserDefaults.standard.set(fresh, forKey: key)
        }
    }
}
