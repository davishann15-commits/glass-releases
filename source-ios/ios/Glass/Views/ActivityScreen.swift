import SwiftUI

/// Activity (desktop pages.rs `activity`): who Glass talks to, what it stores, and
/// every request to Spotify and LRCLIB. Phase 6 fills in the stored files and the log.
struct ActivityScreen: View {
    @Environment(Store.self) private var store
    @State private var data: ActivityModel?

    var body: some View {
        let look = Look.shared
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 10) {
                Text("Activity").gFont(28)
                Text("Everything Glass sends, receives and stores.").gFont(13).foregroundStyle(look.sub)
                SectionTitle(title: "Who Glass talks to")
                ForEach(Self.hosts, id: \.0) { host, why in
                    VStack(alignment: .leading, spacing: 2) {
                        Text(host).gFont(13.5, .medium)
                        Text(why).gFont(12).foregroundStyle(look.sub)
                    }
                }
                Text("No analytics, telemetry or crash reporting.").gFont(12.5).foregroundStyle(look.accent).padding(.top, 6)
                SectionTitle(title: "What Glass stores")
                if let d = data {
                    ForEach(d.files, id: \.label) { f in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(f.label).gFont(13.5, .medium)
                            Text("\(f.note) · \(f.size)").gFont(12).foregroundStyle(look.sub)
                        }
                    }
                    item("audio cache", d.audioCache)
                    item("lyrics cache", d.lyricsCache)
                    item("Keychain", "Your Spotify sign-in, the assistant's ChatGPT sign-in and any OpenAI API key: on this device only, never in files")
                    ForEach(d.folders, id: \.self) { f in Text(f).gFont(11).foregroundStyle(look.muted).textSelection(.enabled) }
                    Text("Spotify: \(d.engineStatus)").gFont(12).foregroundStyle(look.sub).padding(.top, 4)
                    HStack {
                        Text("requests (\(d.log.count))").gFont(18)
                        Spacer()
                        BracketButton(title: "Clear") { store.core.clearRequestLog(); Task { await load() } }
                    }
                    .padding(.top, 12)
                    ForEach(Array(d.log.reversed().enumerated()), id: \.offset) { _, e in
                        HStack(spacing: 8) {
                            Text(e.time).foregroundStyle(look.sub)
                            Text(e.method).foregroundStyle(e.method == "GET" ? look.sub : e.method == "DELETE" ? look.error : look.accent).frame(width: 52, alignment: .leading)
                            Text(e.status).foregroundStyle(e.status.hasPrefix("2") ? look.foreground : look.error).frame(width: 36, alignment: .leading)
                            Text("\(e.millis) ms").foregroundStyle(look.sub).frame(width: 60, alignment: .trailing)
                            Text(e.path).lineLimit(1).truncationMode(.middle)
                        }
                        .gFont(11.5)
                    }
                } else {
                    LoadingNote()
                }
            }
            .padding(16)
        }
        .glassNav("settings / activity")
        .task {
            // Live while open, as the desktop's page.
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private func item(_ label: String, _ note: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).gFont(13.5, .medium)
            Text(note).gFont(12).foregroundStyle(Look.shared.sub)
        }
    }

    private func load() async {
        if let d = await store.core.activity() { data = d }
    }

    static let hosts: [(String, String)] = [
        ("accounts.spotify.com", "Sign-in (OAuth with PKCE) and token refresh"),
        ("Spotify spclient + api-partner.spotify.com", "Library, search, pages and playback commands (logged below)"),
        ("open.spotify.com, open.spotifycdn.com", "Only when Spotify rotates its page queries: reading the current ones from its web player"),
        ("lrclib.net", "Lyrics: the title, artist, album and length of the song whose lyrics are on screen (logged below)"),
        ("Spotify access points + audio CDN", "Encrypted audio streams and Spotify Connect, via librespot"),
        ("i.scdn.co / mosaic.scdn.co / lexicon-assets.spotifycdn.com", "Cover art, artist images and the DJ's cover"),
        ("jiosaavn.com, youtube.com and their CDNs", "Only when you save songs as files: finding and downloading them"),
        ("api.openai.com, auth.openai.com", "Only with the assistant on: sign-in, your requests, and lyrics it translates"),
        ("your other Glass devices", "Only with sharing on: songs you send, over an encrypted connection on this network"),
    ]
}
