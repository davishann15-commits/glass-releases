# Glass for iOS — validation

What was checked, how, and what still needs a real device or your accounts.
Simulator: iPhone 17 Pro (iOS 26), Xcode 26, on this Mac, 2026-10-23.
Nothing here used your Spotify or ChatGPT accounts for writes; see "Your data" below.

## Automated

| Suite | Result | Notes |
|---|---|---|
| UI tests (`GlassUITests`, XCUITest, demo data) | 12 tests, all pass | Below. Screenshots in `docs/screenshots/` |
| UI tests (`AppleMusicUITests`, demo data) | 2 tests, all pass | Picking songs, the sheet, a demo run's progress and result; Settings opens the setup |
| UI tests (`QueueUITests`, demo data) | 1 test, passes | Now Playing's queue: sections, dragging Kyoto's handle onto Bloom queues it, swiping a "Next from" song queues it |
| UI tests (`PolishUITests`, demo data) | 3 tests, pass | Now Playing ▸ playlist opens on the playing song (the 14th of 15) and a song swiped right is queued; Local Files lists another device, its songs list (ones already here marked), two are picked and fetched; the bottom bar under the rose preset |
| Add to Apple Music core tests (`cargo test --lib apple_music`) | 10 pass | New and existing libraries: rows and triggers, covers (and iOS 26.4's local-source rows), alphabetical repair, duplicates left alone, albums by album artist, playlists, removed files' entries, a damaged library refused, lyrics cleanup, cover color |
| Core Rust tests on the Mac (`ios/core`, `cargo test --lib`) | 212 pass, 0 fail, 13 ignored | The shared desktop modules' tests run in the iOS configuration (`cfg(glass_mobile)`), plus the core's own (sign-in redirect, DNS-SD TXT records, sharing transfer wording, web-player chunk map) |
| Desktop (`cargo check --all-targets`, `cargo test`, `cargo build`) | see "Builds" below | |

UI tests:

| Test | What it proves |
|---|---|
| `testAssistantPreviewSaveAndUndo` | Against `scripts/mock-openai.py`: a message goes out, the model's tool calls run against the local library (search → propose a playlist → ask to save), the preview and the approval request show, **save** creates a real local playlist, **undo** removes it. The mock's request log shows the API key only as the bearer header, `store: false`, and no file paths in any request |
| `testAssistantSignInPlumbing` | ChatGPT sign-in's iOS part: the web sign-in sheet opens the (mock) sign-in page, which sends it to the core's loopback on 127.0.0.1:1455; the core answers with `glass-ios://…` so the sheet closes, then exchanges the code (PKCE verifier, client ID, redirect URI sent; the mock refuses it and the refusal shows in Settings). Spotify's sign-in uses the same sheet → loopback → redirect pattern |
| `testAssistantSettings` | Settings › Assistant renders off and on |
| `testActivity` | Settings › Activity: stored files measured, caches, the request log |
| `testCommandBar` | The command sheet finds a command by its words ("hearts" → liked songs) and runs it |
| `testFeaturedAndDiscography` | Featured artists on a playlist; the discography chips filter releases |
| `testThemeManager` | The theme manager: a preset changes the colors, a lyrics choice (align left) is saved, album colors switch to everywhere; then back to the defaults |
| `testPlayerGestures` | Player bar: swipe left skips, swipe right goes back, swipe up opens Now Playing; pulling Now Playing down closes it |
| `testRowSwipeToQueue` | A song swiped left is queued ("Added to queue") and doesn't start playing; the list still scrolls |
| `testSwipeBack` | Swipe back from the left edge and from the middle of a page (header and rows; playlist, album, Liked Songs), slow to 3000 pt/s; fails if any doesn't go back |
| `testTour` | The main screens, one launch each (screenshots) |
| `testLyricsTranslation` | Translation turned on in Settings (into Spanish): translated lines show under each lyric line, then in place of it. The translator is a stand-in (`-mockTranslate`) because the simulator has no on-device translation; everything else is the real path |

## Probes in the simulator (debug-only launch flags)

| Probe | Result |
|---|---|
| `-probeSpotify <desktop credentials.json> -probeEngine -probePlay` (earlier, read-only) | Session, spclient, GraphQL, Connect device listed; a Liked Song played at volume 0 through RemoteIO |
| `-probeSave` | A real song saved as MP3 through LAME on iOS; tags read back |
| `-probeBonjour` | DNS-SD: the app's `_glass-files._tcp` service was seen by the Mac's `dns-sd -B`, and the app resolved one registered on the Mac with the desktop's TXT (v=3, acct, name) |
| `-probeHashes` | All five GraphQL hashes read from Spotify's web player |

The built-in GraphQL hashes were updated to Spotify.app 1.3.6's (`searchDesktop`,
`userTopContent`); before that, a desktop test that compares them with the installed app
failed. The new ones were checked first with a read-only live test: Spotify accepted them
with Glass's variables, and search (tracks, artists, albums, playlists) and top artists /
songs parsed as the pages read them.

## Seen working in the simulator with demo data

Library (tabs, chips, sorts, views, folders), Home, Search, playlist / album / artist /
discography / track / profile pages, menus, Now Playing (compact and large-art), lyrics
(word / line / plain, picker, timing), translation note, visualizer, album colors, theme
panel, Settings (every section and its search), Local Files (views, add menus), the
assistant panel and settings, Activity, the command sheet, About and licenses.

## Needs a real iPhone (not done: no device attached)

Install with your own signing team (see BUILD.md), then:

1. Sign in to Spotify in the sheet; Library loads; restart the app → still signed in (Keychain).
2. Play a song here: lock the phone → keeps playing; lock-screen controls and Control
   Center work; Play and Pause never toggle the wrong way.
3. Unplug headphones / disconnect AirPods while playing → pauses (doesn't move to the speaker).
4. A phone call or Siri interrupts → pauses; resumes after only if iOS says so.
5. Spotify Connect: "Glass iPhone" appears in Spotify's device list; transfer to and from it.
6. Downloads: download a playlist, airplane mode, it plays.
7. EQ presets and mono are audible; visualizer follows the music.
8. Save a song as M4A and MP3 (Share ▸ Download as …); it shows in Local Files and Files.
9. Import songs from Files; edit a song's tags and cover; local playlists; Spotify playlists'
   local songs play from Documents.
10. Sharing: with the desktop Glass (or another iPhone) open on the same Wi-Fi and account,
    Local Files lists it by itself (no code); tap it, its songs list, ones already here are
    marked; pick a few, Get → they arrive in Local Files. Send songs the other way; a
    playlist's missing local songs come from the Mac. iOS asks for local-network access the
    first time. A Glass signed in to another account doesn't show up.
11. Lyrics translation with Apple's on-device engine: on a song in another language, tap 文A
    in the lyrics view; iOS asks to download the two languages (once); the translation then
    appears under each line (tap 文A again for the translation alone).
12. The assistant with your ChatGPT plan or an API key: sign in, ask, approve, undo.
13. Track-change notifications (Settings › Display), with permission.
14. Add to Apple Music: install LocalDevVPN and connect it; make an RP pairing file with
    idevice_pair (a classic one before iOS 26.4) and import it in the sheet; Check
    connection; add a few songs (one with a cover, one without, one into a playlist);
    restart the iPhone; they play in the Music app with their covers, the playlist is
    there; add the same songs again → "already in Apple Music"; Stop during copying →
    nothing changes; Restore puts the library back. First try it on a phone whose
    library you can lose: this writes to the Music app's database directly.
15. Background playback: play on this iPhone, lock it and leave it playing for an hour or
    more (or switch Wi-Fi ↔ cellular): playback carries on, or picks up again within a few
    seconds where it was (queued songs aren't kept across a reconnect). Pause, leave Glass
    in the background for an hour, come back, press play: it plays (no "Internal error
    { channel closed }").
16. Queue: in Now Playing ▸ queue, drag a song's ≡ handle to reorder; drop a "Next from"
    song among the queued ones (it joins the queue); swipe a "Next from" song left (it's
    queued); check the order then plays as shown. Local Files: while one plays, swipe other
    songs left, then reorder them in the queue. Now Playing ▸ playlist opens on the song
    that's playing; swipe a song right → it's queued.
17. Volume: playing on this iPhone, Glass is as loud as the Spotify app at the same iPhone
    volume (normalization Normal in both); the volume rail moves with the side buttons and
    moves them; mute and unmute; another Spotify app changing "Glass iPhone"'s volume moves
    the iPhone's. Playing on another device, the rail is that device's volume.
18. iPad: rail layout, the library list + page side by side, the assistant as a side
    column, drag files in from Files, drag playlist rows to reorder, drag playlists onto
    folders, a hardware keyboard's shortcuts (⌘K, ⌘J, space, ⌘→…).

## Your data

- Your desktop Glass files were read once by the read-only Spotify probe and never written.
  The probe's engine left a reusable librespot login in the **simulator's** app container;
  that file was deleted, and the iOS engine no longer writes one at all.
- No playlist, like, follow or queue change was made on your Spotify account. Reordering,
  saving to Spotify and the assistant's Spotify actions are built but weren't run against it.
- The assistant was never pointed at OpenAI; all assistant tests used the local mock.
- One Spotify playback (a Liked Song at volume 0, while nothing else was playing) was used to
  prove the audio path, as noted above.

## Builds

| Build | Result |
|---|---|
| iOS app, simulator, Debug (`build-core.sh sim debug` + `xcodebuild`) | Succeeds; the UI tests above run on it |
| iOS app, device (`generic/platform=iOS`), Release, unsigned | Succeeds (Glass.app, arm64, 33.5 MB binary; debug probes compiled out). Not installed: no device and no signing team here |
| Desktop `cargo check --all-targets` | Clean (only the pre-existing future-incompatibility note for the `block` crate) |
| Desktop `cargo test` | 268 pass, 0 fail, 15 ignored |
| Desktop `cargo build` | Succeeds (`target/debug/glass`) |
