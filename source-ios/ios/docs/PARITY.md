# Glass for iOS — feature parity ledger

Desktop baseline: glass-player b403e2a (1.0.0-beta.22). Detailed inventories of the
desktop source (every route, action, menu item, setting, key and string, with file:line
references) are in `docs/audit/`; this ledger maps each user-visible capability to its iOS
place and records what's actually working.

**Status**
- ✅ works — the action and its persistence work end to end; *how it was verified* is in the Test column
- 🧪 built, unverified — implemented, but needs your account, a real device, or a peer to prove
- ◐ partial — what's missing is in Notes
- ⏳ not started — with the planned phase
- ✖ platform/provider limit — what iOS or the provider doesn't allow, and the closest faithful substitute

**Test** column: `sim-demo` = seen working in the simulator with demo data; `unit` = Rust/Swift
unit test; `device` = on an iPhone; `account` = against your real Spotify account; `—` = not yet.

Source references: `app` = src/app.rs, `pages` = src/ui/pages.rs, etc. iOS paths are under `ios/`.

## Shell and navigation

| Desktop feature | Desktop source | iOS place | Implementation | Test | Status | Notes / difference |
|---|---|---|---|---|---|---|
| Destinations Home, Library, Search, Local Files, Settings | ui/mod.rs:317 | `Glass/Views/RootView.swift` NavBar (compact) / Rail (regular) | one NavigationStack per destination, state kept when switching | sim-demo | ✅ | Bottom bar on phones, the desktop's rail on iPad/large landscape |
| Starts on Library | app:932 | `Store.tab = .library` | — | sim-demo | ✅ | |
| Back / Forward, page state kept | app:2228 | NavigationStack per destination (inside a `TabView` whose own bar is hidden) | each tab keeps its stack and place; swipe back from the left edge or anywhere on a page (iOS 26 content swipe) | UI test `testSwipeBack` (edge and mid-page, slow to 3000 pt/s) | ✅ | No Forward (iOS has none). Song rows on pages swipe left to queue, so a rightward swipe always goes back (in Now Playing, where there's nothing to go back to, they swipe right too, as in Spotify) |
| Breadcrumb "~ / music / playlist" | ui/mod.rs:180 | `Breadcrumb`, `.glassNav` | same paths as desktop | sim-demo | ✅ | |
| Toasts "> text", newest, 3 s | ui/mod.rs:422 | `ToastView` | core `CoreEvent::Toast`, same wording | sim-demo | ✅ | |
| Player bar | nowbar.rs | `PlayerBar` | compact: cover/title/artist or DJ line, heart, play, next, progress; wide: transport, seek, devices/queue/lyrics, volume. Gestures: tap or swipe up → Now Playing, swipe left/right → next/previous, hold → the song's menu (VoiceOver actions too) | UI test `testPlayerGestures` | ✅ | Key hints replaced by touch; "playback needs spotify premium" and "playing on {device}" kept |
| Command bar ⌘K (library lookup, commands, Enter = Spotify search, `?` = assistant) | command.rs | `CommandSheet` (navigation bar ›_ button, ⌘K) | the desktop's commands and words (minus mini player / full screen / open in editor), library items, "search spotify for …", "? …" asks the assistant in the background (answer as a toast) | UI test `testCommandBar` | ✅ | A sheet instead of a bar under the title; ↑/↓ with a keyboard, return runs the first |
| Keyboard shortcuts (space, ⌘←/→, L/Q/P/V/D, ⌘K…) | ui/mod.rs:506 | `KeyCommands` (hidden buttons with key equivalents) | ⌘K, ⌘J, space, ⌘→/←, ⌘S, ⌘R, ⌥⇧B/J/Q/D/H/0/S/R, ⌘L, ⌘,, ⌘N, ⌘T, ⌘/ | build | 🧪 | Needs a hardware keyboard (iPad / iPhone with keyboard) to try; holding ⌘ shows the system list. Unmodified keys other than space (L, V…) aren't bound: they'd fight text fields |
| Shortcuts sheet ⌘/ | modals:24 | `ShortcutsSheet` (command "keyboard shortcuts", ⌘/) | | build | ✅ | |
| About | modals:47 | `AboutSheet` + "Open-source licenses" (`LicensesView`) | `Resources/THIRD-PARTY.txt` from `scripts/licenses.py`: the 379 crates linked into the app with versions, licenses, repositories and the 242 distinct license texts, plus LAME and ByeTunes' MIT notice | build | ✅ | |
| Mini player (always on top) | mini.rs | the persistent player bar + lock screen / Control Center | | — | ✖ | iOS has no floating windows for audio apps; video PiP isn't used for music |
| Full-screen Now Playing | mini.rs:102 | Now Playing slides up over the app; pull it down to close (the app shows behind as it goes); artwork: swipe for next/previous, double-tap to like, tap for the album | | UI test `testPlayerGestures` (open by swipe, close by pull) | ✅ | |
| Window drag, zoom, close-to-minimize, open at login, single instance, window size | window_fx, instance, app:4044 | — | | — | ✖ | No iOS equivalents; scene restoration keeps tab/page state while the app lives |
| Drop files from Finder | ui/mod.rs:389 | whole window (`RootView` `.onDrop`) | `LocalImport.drop` copies into Documents/Imported → core `add_files` with `Store.dropTarget` (the playlist on screen when you can add to it, a local playlist, else Local Files) — desktop `file_target` | build | 🧪 | Drag and drop is iPad/visionOS-style (from Files or another app); needs a manual check on iPad |

## Your Library

| Desktop feature | Desktop source | iOS place | Implementation | Test | Status | Notes |
|---|---|---|---|---|---|---|
| Greeting + "library_" with blinking cursor | library.rs | `LibraryScreen` | `BlinkTitle` (steady with Reduce Motion) | sim-demo | ✅ | |
| Tabs music / podcasts / audiobooks and chips | library.rs:480 | `LibraryScreen` | same chip sets per tab | sim-demo | ✅ | |
| Sources (playlists, albums, artists, shows, audiobooks; caps 2000/500) | library.rs:59 | `publish.rs library_model` | same paths and caps, kept for good | sim-demo | ✅ | |
| Liked Songs / Your Episodes / Local Files specials | library.rs:133 | same | | sim-demo | ✅ | "files on this device" |
| Sorts recents / recently added / alphabetical / creator (persisted) | library.rs | `SortViewMenu` → `set_library_sort` | sorted in the core like the desktop | sim-demo | ✅ | Recents = order in the last 50 plays (Spotify's per-item history is private) |
| Views list / compact / grid (persisted) | library.rs | same | | sim-demo | ✅ | |
| Row order: specials, pins, folders (children when expanded), rest | library.rs `build_rows` | `LibraryScreen.rows` | ported to Swift | sim-demo | ✅ | |
| Library search (command bar text) | library.rs | `FilterField` "search your library" | every word in title or subtitle | sim-demo | ✅ | |
| Downloaded chip | library.rs | same | `downloads.collections()`; `download_progress` (done/total per collection) | sim-demo | ✅ | "d/t" next to the download toggle while going |
| Pins (Glass-local) | app:2553 | menus "Pin …" | `settings.pins` | sim-demo | ✅ | Glass's own, not Spotify's pins |
| Glass folders: create, rename, delete, move in/out | app:2563 | sheets + menus + Folder page; drag a playlist onto a folder (or onto a row in another folder / the top level) | `settings.folders` | sim-demo (menus); build (drag) | ✅ / 🧪 drag | Drag and drop is touch-and-hold then drag |
| Detail pane (art, kind, name, subtitle, play/shuffle/download/⋯, featured artists) | library.rs:761 | phones: the item's page; iPad: list + the full page side by side | "Featured artists" (4 most frequent among the first 100 / 50 songs) at the end of playlist and Liked Songs pages; "Fans also like" on artists | UI test `testFeaturedAndDiscography` | ✅ | The full page stands in for the compact pane, so featured artists sit at the end of the page |
| Empty / loading / no matches / nothing downloaded states | library.rs | same strings | | sim-demo | ✅ | |
| Create playlist / new folder from sort menu | library.rs:480 | `SortViewMenu` | | sim-demo | ✅ | |

## Home and Search

| Desktop feature | Desktop source | iOS place | Implementation | Test | Status | Notes |
|---|---|---|---|---|---|---|
| Chips all / music / podcasts / audiobooks | pages:265 | `HomeTop` + `BlocksView` filtering | block IDs say which chip shows them | sim-demo | ✅ | |
| DJ banner | pages:238 | `DjBanner` | caption while the DJ plays | sim-demo | ✅ | |
| Shortcut grid (8 recent contexts) | pages:365 | `ShortcutGrid` | `recent_contexts` ported | sim-demo | ✅ | 2 columns on phones |
| Jump back in, top artists, On repeat, Your playlists, Saved albums, shows, Your Episodes, audiobooks | pages:290 | `views.rs home` | same sources | sim-demo | ✅ | Built from recents and top items (Spotify's personalised feed isn't used), as on the desktop |
| Search field, 350 ms debounce, Enter commits | pages:412 | `SearchScreen` | | sim-demo | ✅ | |
| Recent searches (10, persisted), clear | pages:400 | `RecentSearches` → `remember_search`, "Clear recent searches" | kept when a result is tapped | sim-demo | ✅ | |
| Browse all categories → category page (fallback search) | pages:480, 652 | `CategoryGrid`, `genre` page, `SearchInstead` | | sim-demo | ✅ | |
| Filter chips, Top result, Songs (4), sections with "show all ›", single filter up to 50 | pages:520 | `views.rs search` | | sim-demo | ✅ | |
| No results message | pages:640 | `LoadState::Missing` | same wording | — | ✅ | |

## Pages

| Desktop feature | Desktop source | iOS place | Implementation | Test | Status | Notes |
|---|---|---|---|---|---|---|
| Playlist header (kind, name, description, owner, saves, songs/length) | pages:678 | `views.rs playlist`, `PageHeader` | | sim-demo | ✅ | |
| Play / pause, shuffle, save (others'), download, ⋯ | pages:87 | `ActionRow` | | sim-demo | ✅ | |
| Filter list, sort (custom/title/artist/album/added/duration, again = reverse) | tracklist.rs:464 | `TableControls` | same order logic (`visible_order`) | sim-demo | ✅ | List/Compact view toggle pending |
| Reorder (drag), remove, multi-select | tracklist.rs | drag a row onto another (your playlists, own order, unfiltered); remove via menu; `SelectionBar` for several | `move_playlist_item` (Spotify reorder with the desktop's `insert_before`) | build | 🧪 | Reordering changes a real playlist, so it wasn't tried against your account |
| Edit details (cover tap / menu), public/private, collaborative | modals:75 | `EditPlaylistSheet` | "Collaborative playlists can't be public" | sim-demo | ✅ | |
| Find more / "Let's find something for your playlist" | pages:760 | `FindMore` | search songs, [Add] | sim-demo | ✅ | |
| DJ page ("Next pick", what X says, voice-off note) | pages:214 | `DjBody` | | sim-demo | ✅ | |
| Album (kind, artists, year, songs/length, discs, ℗/©, More by) | pages:832 | `views.rs album` | | sim-demo | ✅ | |
| Artist (banner, listeners, play, shuffle, Follow, Popular 5/10, Discography, Fans also like, Appears On, About) | pages:885 | `views.rs artist` | Discography chips All releases / Albums / Singles and EPs / Compilations (`DiscographyChips`, shared with the Discography page as on the desktop) | UI test | ✅ | |
| Discography page | pages:1029 | `discography` + chips | | sim-demo | ✅ | |
| Show (Follow, ⋯, All Episodes, About) | pages:1117 | `show` | episode rows with resume | — | ✅ | |
| Episode (show link, date • length, play, save, ⋯, description, See all episodes) | pages:1172 | `episode` | | — | ◐ | "See all episodes" = the show link |
| Audiobook (authors, narrators, description, chapters) | pages:1208 | `audiobook` | | — | ✅ | Playing needs the account's audiobook entitlement |
| Track (artists, album, year, length, popularity, play in album, heart, ⋯, Artist rows, Popular songs, Releases) | pages:1244 | `track` | | sim-demo | ✅ | Artist avatars in the Artist rows pending |
| Profile (yours/others', top artists/songs "Only visible to you", public playlists, following) | pages:1321 | `profile` + `OnlyYou` | | build | ✅ | |
| Liked Songs (header, Add all to queue, table, empty state) | pages:1400 | `liked` | | sim-demo | ✅ | |
| Your Episodes | pages:1436 | `your_episodes` | | — | ✅ | |
| Recents (last 50, grouped by day) | pages:1685 | `recents` | | — | ✅ | |
| Folder page | pages:1707 | `folder` | | — | ✅ | |
| Common states: loading…, couldn't load this + try again, unavailable | pages:60 | `LoadingNote`, `ErrorBox`, `Unavailable` | try again invalidates | sim-demo | ✅ | Pull to refresh added |

## Menus

| Desktop feature | Desktop source | iOS place | Status | Notes |
|---|---|---|---|---|
| Track menu: Add to playlist ▸ (New playlist, writable playlists), Remove from this playlist, Like / Save to Your Episodes, Add to queue, Go to song / radio / artist(▸) / album, Go to episode / podcast, Share ▸ (Download as M4A/MP3), Get from your other devices, Send to ▸, Edit song info…, Add to local playlist ▸ | menus:251 | `TrackMenu` (long-press, ⋯) | ✅ sim-demo | Share adds the iOS share sheet |
| Playlist menu (queue, edit details, delete, save, download, create playlist/folder, invite collaborators / non-collaborative, add to/remove from profile, move to folder, pin, share, Add local files…, Get N missing local songs) | menus:339 | `PlaylistMenu` | ✅ sim-demo | "Add local files…" copies picked files into Glass's folder, then adds them (core `add_files`, desktop `files_read`) |
| Album / artist / show / audiobook / profile menus | menus:426–491 | `AlbumMenu` … | ✅ sim-demo | |
| Multi-select (⌘/⇧-click, ⌘A) | tracklist.rs | `SelectionBar` (select, all, actions) | ✅ sim-demo | Touch: [select], tap rows, [all]; no ⌘A |
| Library row menus (folders, specials) | library.rs:663 | `LibMenu` | ✅ sim-demo | |

## Playback, queue, devices, Connect

| Desktop feature | Desktop source | iOS place | Implementation | Test | Status | Notes |
|---|---|---|---|---|---|---|
| This device is a Connect device | engine.rs | core `Engine::start` | librespot Spirc, named "Glass iPhone" by default | — | 🧪 | needs account |
| Play context / list (500 around the tapped one) / radio | app:2242 | `Store.play`, menus | `start_play`/`play_now` ported (engine here, Connect command elsewhere, waits for the engine) | sim-demo | 🧪 | |
| Play/pause, next/prev, seek, seek-by, volume (250 ms throttle remote), mute | app:2281 | Now Playing, bar, lock screen | Playing on this iPhone the volume is the iPhone's own (`SystemVolume`: the rail, side buttons and Control Center move the same thing; Connect is told; another device's or the assistant's change moves it). The audio goes out at full scale (`NoOpVolume`; the soft mixer's log curve put the default 70 % at −18 dB). Elsewhere it's that device's Spotify volume | sim-demo (UI) | 🧪 | Setting the system volume needs a device (the simulator's is fixed) |
| Shuffle, repeat off → context → track (carried into new loads) | app:2408, engine `PlayMode` | Transport | ported | — | 🧪 | |
| Podcasts ±15 s, in order | nowbar.rs, engine.rs | Transport, remote skip commands | | — | 🧪 | |
| Queue (add up to 100, collection, view, clear = restart context) | app:2436 | `QueuePane`, menus | ported, same toasts | sim-demo | 🧪 | Spotify's queue can't be undone or have single items removed (no API), as on desktop |
| Queue (iOS additions): "Next in queue" / "Next from" sections, reorder by dragging a song's ≡ handle (VoiceOver: Move up / Move down), swipe a song from "Next from" (left or right) to queue it; Local Files songs swipe into Local Files' own queue while it plays | — | `QueuePane`, `SwipeToQueue`, core `move_in_queue` / `queue_local` | Playing on this iPhone: librespot's own list (vendored librespot-connect `glass_move_next` and the queue hook); Local Files: `LocalPlayer::move_upcoming` / `queue_songs` | UI test `QueueUITests` (demo), `queue_tests` | ✅ sim-demo / 🧪 device | Spotify's Web API can't reorder a queue, so the handles show only when this iPhone (or Local Files) plays |
| Reconnect after the engine's connection dies (background, network change) | app:3834 | core `engine_lost` | shared `engine.rs` reports the loss once, at once (vendored librespot-connect sent it only after network requests); iOS resumes what was playing and asks iOS for background time meanwhile (`PlaybackModel.reconnecting`) | — | 🧪 device | The desktop now restarts reliably too (it doesn't resume) |
| Devices: current, list, transfer, restricted | right.rs:284 | `DevicesPane` | polled every 5 s while shown | sim-demo | 🧪 | |
| Player state pushed by Spotify, polling fallback, back-in-front catch-up | app:3400 | core `poll`, `set_foreground` | | — | 🧪 | |
| Explicit filter (greyed + skipped once, toast) | app:3955 | rows + `maybe_skip_explicit`; Settings ▸ Explicit content | | sim-demo (setting) | 🧪 | skipping needs real playback |
| Now playing on play setting | app:2095 | `ShowNowPlaying` event | | — | ✅ | |
| Premium required messages | app:3645 | toasts, bar hint | | — | ✅ | |
| "playing from" the context (P) | nowplaying.rs:61 | `PlayingFromPane` | opens scrolled to the song that's playing and follows it; swipe a song right (or left) to queue it, as in Spotify | UI test `PolishUITests` | ✅ | |
| Encrypted downloads (4 workers, retry, prune, collections, quality) | downloads.rs | core `downloads.run` on engine ready; toggle + "d/t"; Settings ▸ Downloads and Storage | shared code | — | 🧪 | needs your account |

## DJ

| Desktop feature | Desktop source | iOS | Status | Notes |
|---|---|---|---|---|
| DJ context (lexicon session, pages), track hook, hold-before-song, X's voice (Spotify TTS), prefetch, cancellation on skip/play, pause during speech, voice off | dj.rs, vendor/librespot-connect | core `dj_break`/`dj_clips_ready`/`dj_resume`/`dj_cancel` ported; same vendored connect | 🧪 | Needs account; the DJ voice plays through its own rodio stream (RemoteIO) like the desktop |

## System integration

| Desktop feature | Desktop source | iOS place | Status | Notes |
|---|---|---|---|---|
| Media keys / Now Playing widget, Play/Pause never toggle | app:3669 | `SystemAudio` (MPNowPlayingInfoCenter, MPRemoteCommandCenter) | 🧪 | Claims the system's Now Playing only while this device plays |
| Follow the output; pause when it goes away | audio_out.rs | route-change notifications → core | 🧪 | needs device |
| Interruptions (calls, Siri) | — | interruption notifications → pause / resume if iOS says so | 🧪 | needs device |
| Background / locked playback | — | UIBackgroundModes audio, `.playback` session | 🧪 | needs device |
| Track-change notifications | app:3948 | Settings ▸ Display toggle; `TrackNotifier` (UNUserNotificationCenter, asks permission when turned on; not for the first song; one at a time) | build | 🧪 | Shown in front too, as the desktop's are; needs a song change to see |

## Audio settings and DSP

| Desktop feature | Desktop source | iOS | Status | Notes |
|---|---|---|---|---|
| 96/160/320 kbps, normalisation + Quiet/Normal/Loud, gapless, autoplay (a new engine takes a change at once, carrying on with the song) | engine.rs, config.rs | shared engine reads `Settings`; Settings ▸ Audio quality / Playback | 🧪 | needs real playback |
| 6-band EQ (60/150/400/1k/2.4k/15k, ±12 dB, shelves + peaking Q 1), 22 presets, custom, preamp + soft limiter, instant | dsp.rs, config.rs | shared `DspSink`; Settings ▸ Playback `EqualizerCard` (curve, presets, sliders) | 🧪 | the UI and settings round-trip in the demo; hearing it needs real playback. As on the desktop, EQ/mono apply to Spotify (and playlists' local songs), not to the Local Files player |
| Mono, instant | dsp.rs | shared | 🧪 | |

## Lyrics, translation, visualizer, album colors, theme

| Desktop feature | Desktop source | Status | Notes |
|---|---|---|---|
| Lyrics: LRCLIB/Spotify/auto + fallback, matching, cache, picker, none-fit, look again, per-song pick and timing, global offset, plain/line/word/syllable timing, follow/scroll, footer provenance | lyrics/, nowplaying.rs | ✅ sim-demo (views, picker, timing) / 🧪 live lookups | `lyrics_app.rs` drives the shared `lyrics/` service; `LyricsPane` (synced with word emphasis, plain, picker, footer). Live LRCLIB/Spotify lookups run the desktop's code and need a song playing |
| Translation (provider's, Apple on-device, assistant), modes, caching, stale-result rejection | lyrics/translate.rs, translate_app.rs | ✅ display (UI test `testLyricsTranslation`, with a stand-in translator) / 🧪 Apple's engine on a device | Apple's on-device engine through the Translation framework in-app (`AppleTranslator`: `LanguageAvailability`, `TranslationSession(installedSource:target:)`); the same calls translate correctly on macOS 26 (ja→en, en→es checked); the iOS Simulator has no on-device translation (every pair reports unsupported — `-probeTranslate`). First use: turning 文A on asks for the language download right away (the system's sheet), then translates; a [download …] button stays in the lyrics view. The assistant's model as on the desktop |
| Visualizer (bars/mirror/wave, gradient, opacity, bars, height, sensitivity, smoothing, glow) | viz.rs, ui/visualizer.rs | ✅ sim-demo | `VisualizerView` on the core's `visualizer_bands` (the real output tap) |
| Album colors (palette, wash bloom 1.4 s, crossfade, prefetch next) | ui/album_colors.rs | ✅ sim-demo | shared `palette.rs`; on/off and **where** in the theme manager: Now Playing and the player bar (the desktop's way) or **everywhere** (every page's background, the accent, selection and player colors), with the 1.4 s bloom from the artwork and accent crossfade; the current and next songs' palettes come from the same 300 px cover, so the prefetched one is used (UI test `testThemeManager`) |
| theme.toml tokens, [colors], [lyrics], [visualizer], palette, motion, validation, last-good, import/export | theme_config.rs | ✅ UI test `testThemeManager` | the theme manager (Settings ▸ Appearance, ⌘T, the command bar): a live preview; tabs colors (presets, the four base colors, palette, opacity, the derived [colors]), album, type (text size, corners, spacing, motion), lyrics, visualizer, file (export/import); sliders and the color picker show live (`preview_theme_value`) and are written when let go; at its smaller height the app stays usable behind it. [colors] from the file now apply on iOS |

## Local music

| Desktop feature | Desktop source | Status | Notes |
|---|---|---|---|
| Local Files views (Albums, Songs, Artists, Genres, Recently Added, Playlists), sort, filter, play | local_library.rs | ✅ sim-demo | `LocalFilesScreen`; the shared index (`src/local_index.rs`) |
| Sources | settings | ✅ sim-demo | Glass's folder (Documents, visible in the Files app) is the source; Files import copies into Documents/Imported; folders picked in Files are kept as security-scoped bookmarks |
| Tag editor (title, artist, album, year, genre, cover) | local.rs:374 | 🧪 | `TagEditor` (cover from Photos or Files), shared `write_tags`; needs a real file |
| Spotify playlists' local songs (matching ±3 s, missing explained) | local.rs:303, vendor local_file.rs | ◐ | rows show the file or why it's missing; playback via librespot's lookup over Documents 🧪 |
| Glass local playlists (create, add, remove, delete, undo, play, add files) | playlists.rs | ✅ | the assistant UI test saves one and undoes it; menus add to existing ones |
| Share ▸ Download as M4A/MP3 (JioSaavn, YouTube Music, LAME) | save_song/ | ✅ | A downloaded playlist becomes a local playlist (name, description, cover, order; a later download updates it), ready for Add to Apple Music as a playlist; the debug probe saved a real song as MP3 through LAME on iOS (tags read back); Settings ▸ Save songs: format, MP3 bitrate, folder (Glass's or one picked in Files) |
| Sharing with Glass on your other devices (Bonjour, Noise XX, browse and get, send, get missing) | share.rs, app:1580 | Local Files lists your other devices by itself (no pairing, no code); tap one for its songs (`DeviceLibraryScreen`): pick songs as for Apple Music, ones already here are marked, Get; Send to ▸ on local songs/albums; "Get from your other devices" on missing local songs; Settings ▸ Your Library toggle (on by default) | shared `share.rs` protocol 4: the account (a secret from its username and email, never sent) proves a device is yours, bound to each Noise handshake, instead of Spotify's device list plus a six-digit code; new `List`/`Get` requests. Discovery through `ios/core/src/bonjour.rs` (DNS-SD) under `cfg(glass_mobile)` | unit (`share::tests`, both directions, strangers turned away, list and get); UI test `PolishUITests` (demo device) | ✅ sim-demo / 🧪 two devices | Needs Glass open on both, the same Wi-Fi and the same account. The desktop answers List/Get and sends; browsing another device's songs is on iOS only |

## Assistant

| Desktop feature | Desktop source | Status |
|---|---|---|
| Continue with ChatGPT / API key, models, billing, usage, panel, approvals, drafts, undo, `?` in the command bar, Spotify-data opt-out | ai/, assistant_app.rs, ui/assistant.rs | ✅ against a mock / 🧪 real accounts | `ios/core/src/assistant_app.rs` is the desktop's `assistant_app.rs` on the iOS core (same guardrails, approvals, previews, undo, context note); `AssistantPanel` (sheet on iPhone, inspector column on iPad); Settings ▸ Assistant; ⌘J. UI tests against `scripts/mock-openai.py`: search → preview → approval → save → undo, and checks that requests carry no file paths; sign-in plumbing (sheet → loopback → back to the app). The real ChatGPT sign-in and API need your account or key. The API key goes straight from the field to the Keychain (never through the core's message queue) |

## Settings, Activity

| Desktop feature | Status | Notes |
|---|---|---|
| Account: signed in as, plan, log out (confirm), Edit login methods | ✅ sim-demo | |
| Spotify Connect: engine status, restart, device name, client ID | ✅ sim-demo | |
| Every other section (Appearance, Assistant, Lyrics, Downloads, Save songs, Your Library, Display, Playback + EQ, Audio quality, Explicit, Storage, Proxy) and settings search | ✅ sim-demo | Desktop-only rows (updates, open at login, close button, background effect) are left out or explained |
| Activity (hosts, stored files and sizes, request log) | ✅ UI test `testActivity` | measured off the core's thread, refreshed every 2 s while open; Keychain items listed instead of session.json/credentials.json (iOS keeps no credential files) |
| Updates | ✖ | App Store / TestFlight updates replace self-update |

## Spotify sign-in and data

| Desktop feature | Desktop source | iOS | Test | Status | Notes |
|---|---|---|---|---|---|
| Sign in (PKCE), resume, refresh, retry backoff, refused session → sign in again | auth.rs, app:3489 | `signin.rs` + `SignInSession`; `Auth::resume` shared | unit; Spotify's page loads in the sheet; the sheet → loopback → `glass-ios://` close pattern passes in the assistant's UI test | 🧪 | `state` is now checked (the desktop doesn't); completing it needs your account |
| Sign out | app:2924 | `log_out` | sim-demo | ✅ | |
| Internal services (spclient, GraphQL pathfinder, Connect state) | sp/ | shared | — | 🧪 | |
| GraphQL hash recovery from Spotify.app | sp/mod.rs:446 | transparent: a rejected hash is retried with the built-in one, then the current hashes are read from Spotify's web player | `sp::web_player_queries` (`cfg(glass_mobile)`): open.spotify.com → main script → lazy route chunks (search, browse, profile…), the same `"<name>","query","<sha256>"` declarations; saved to `queries.json` like the desktop | sim: `-probeHashes` found all 5 (searchDesktop, userTopContent, queryArtistOverview, browseAll, browsePage), matching Spotify.app 1.3.6 on the Mac | ✅ | Same mechanism as the desktop with a different source (iOS can't read another app's bundle). Reads only Spotify's public web player scripts |

## iOS only: Add to Apple Music

Not a desktop feature: a stripped-down port of ByeTunes (github.com/EduAlexxis/ByeTunes,
MIT) that puts Local Files songs into the iPhone's own music library.

| Feature | iOS | Test | Status | Notes |
|---|---|---|---|---|
| Pick songs in Local Files (songs, albums, "all shown"), or long-press a song, album, album page or local playlist ▸ Add to Apple Music | `LocalFilesScreen` picking mode + `AppleMusicPickBar`, `AppleMusicButton` | UI test `testPickSongsAndAddThem` | ✅ sim-demo | |
| The sheet: setup steps, pairing file (import / replace / remove / check connection), add (optionally into a playlist), progress, Stop, result, restore | `AppleMusicSheet`; Settings ▸ Your Library ▸ Add songs to Apple Music | UI tests (demo run) | ✅ sim-demo / 🧪 device | The demo walks the steps without a device |
| Pairing file (classic before iOS 26.4, RP from 26.4) checked on import and kept in the Keychain | `core/src/apple_music/mod.rs`, `device.rs` | — | 🧪 device | Handed over as bytes, never written to a file |
| Connection through LocalDevVPN (10.7.0.1): lockdown, or the RemotePairing tunnel; heartbeat; Bonjour for the RP port | `device.rs` (idevice crate) | — | 🧪 device | ByeTunes uses idevice's C wrapper; Glass uses the crate directly |
| The library: download, check, add songs (and a playlist), alphabetical repair, upload beside the old one and swap | `library.rs`, `schema.sql` | `cargo test apple_music` (10 tests on built libraries) | ✅ unit / 🧪 device | Differences from ByeTunes listed at the top of `library.rs` |
| Restore the library from before the last change | `mod.rs` `restore` | — | 🧪 device | ByeTunes' backups feature, reduced to the one copy |

Left out of ByeTunes on purpose: downloads and its backend, metadata and lyrics lookups,
the Apple Music catalog matching, ringtones, the library browser and editors, smart
playlists, the repair tools, snapshots, Picture in Picture, Shortcuts, and deleting
files in the music folder that the library doesn't list (ByeTunes' garbage collection).

## Known differences that are by design

- **LAME on iOS**: linked statically into the app (the desktop ships it as a separate,
  replaceable dylib to meet the LGPL's relinking requirement). A distributed iOS build
  would need LAME as an embedded dynamic framework, or the object files offered for
  relinking, before it could be shared. Personal builds aren't affected.
- **Window-level glass**: iOS apps can't blur other apps; the depth is drawn in the app.
