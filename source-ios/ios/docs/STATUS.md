# Glass for iOS — work log

Running record of what's built and verified, newest last. The parity ledger
(PARITY.md) has the per-feature status; this file is the narrative and the next steps.

## 2026-10-23

- Audit: four inventories of the desktop source in `docs/audit/` (UI surface, settings/theme/config,
  services, lyrics/assistant).
- Worktree `~/glass-ios`, branch `ios` (from main b403e2a). Nothing committed yet.
- Shared-code changes for iOS (desktop still `cargo check --all-targets` clean):
  `cfg(glass_mobile)` gates in auth.rs, config.rs, local.rs, save_song/mod.rs, theme_config.rs,
  lyrics/translate.rs; `audio_out::changed`; `Sp::connect_cached` for the debug probe;
  `src/palette.rs` split out of ui/album_colors.rs (pure palette code shared by both apps);
  vendored librespot-core: sysinfo → sysctl on iOS, presents as macOS.
- Core (`ios/core`): bridge (UniFFI), actor (`app.rs`, port of desktop App logic), views
  (page builders), publish (diffed events), demo fixtures, iOS Spotify sign-in, lyrics +
  translation (`lyrics_app.rs`), theme model (`theme_app.rs`), album colors (`colors_app.rs`),
  settings/config/status models (`settings_app.rs`), platform traits (Keychain, Translator),
  debug probe (`probe.rs`).
- App (`ios/Glass`): Store, Look (tokens, album accent via environment), components, shell
  (bottom bar / rail), player bar, Library, pages (header + blocks), menus, Search, Now Playing
  (compact controls + panes; large-art mode), lyrics view (word/line/plain, picker, footer,
  translation note + download), visualizer, Settings (all applicable sections, EQ curve),
  theme panel, sheets, sign-in + ASWebAuthenticationSession, AudioSession/NowPlaying/remote
  commands, Keychain, Apple Translation adapter.
- Verified in the simulator: bridge proof; local M4A playback via RemoteIO; demo screens;
  Spotify probe (session, spclient, GraphQL, Connect device listed) and volume-0 Spotify
  playback through RemoteIO; Spotify's authorize page loads in the sign-in sheet.

- Phase 5 (local music): Local Files (Glass's folder in Documents, Files imports, folders
  from Files as bookmarks, the six views, tag editor), local playlists, "Add local files…",
  drag and drop, Save as M4A/MP3 (LAME on iOS), sharing with Glass on your other devices
  (Bonjour through DNS-SD + the shared Noise protocol; pairing dialog; Send to ▸; get missing).
- Phase 6: the assistant (desktop assistant_app.rs ported; panel as sheet/inspector;
  Settings; ChatGPT sign-in sheet; API key straight to the Keychain), lyrics translation by
  the assistant, Activity, the command sheet and keyboard shortcuts, featured artists,
  discography chips, "Only visible to you", clear recent searches, download progress,
  theme/config export and import, track-change notifications, open-source licenses,
  drag-to-reorder playlists and drag-onto-folder, GraphQL hash recovery from Spotify's web
  player, Save songs settings.
- Phase 7: XCUITest target (7 tests, demo data + a mock of OpenAI's API), the core's Rust
  tests on the host (LAME built for macOS too), docs (BUILD.md, VALIDATION.md, ledger,
  architecture), screenshots, icon generator fixed (it produced a black icon).
- Fixes found on the way: iOS no longer writes librespot's reusable login to a file (the
  engine signs in with a fresh token each start); a stray one left by the earlier probe in the
  simulator was deleted. Settings rows merged their controls into one VoiceOver element
  (chips and buttons couldn't be reached separately); switches now carry their labels.
  The web-player chunk-map parser dropped the map's last entry.

- Final checks (2026-10-23): 7/7 UI tests pass; core Rust tests 212/212; desktop
  `cargo check --all-targets` clean, `cargo test` 268/268, `cargo build` succeeds; iOS
  Release build for devices succeeds (unsigned).
- Built-in GraphQL hashes updated to Spotify.app 1.3.6's (`searchDesktop`, `userTopContent`),
  verified read-only against Spotify first; this fixed the one failing test in both suites.
- The audio session is now released 35 s after playback stops here (after the local player
  lets its output go), so other apps can resume and iOS can suspend Glass.

## 2026-10-23 (later): theme manager, album colors everywhere, gestures, covers

- Theme manager rebuilt: it didn't work (choice values were sent quoted, so the core
  refused every align/position/animation/style edit; sliders only applied on release; it
  covered the app; no way to change text size, corners, spacing or motion; [colors] from
  theme.toml were ignored on iOS). Now: live preview, presets, every theme.toml value Glass
  draws with, live preview while dragging (`preview_theme_value`), album colors tab.
- Album colors can apply everywhere (choice kept on the device): every page's background
  takes the wash and the accent follows the cover, with the desktop's 1.4 s bloom.
  Pages now sit on Glass's own glass (the navigation containers painted plain black).
- Gestures: player bar (tap/swipe up → Now Playing, left/right → next/previous, hold →
  menu), Now Playing as an overlay you pull down to close, artwork swipe/double-tap,
  song rows swipe left to queue. Swipe back already worked from anywhere (verified).
- Covers: one download per image (300 MB disk cache across launches), any decoded size
  stands in at once, the playing and next songs' covers prefetched, crossfade on change,
  the player bar's cover is the 300 px one (was 64 px, soft); album colors use the cover
  that was prefetched for them (they missed it before).
- Tabs are a `TabView` with its bar hidden: inactive tabs were still read by VoiceOver.
- Checks: 11/11 UI tests; core Rust tests 214/214; desktop `cargo check --all-targets` clean.

- Add to Apple Music (2026-10-23, iOS only): a stripped-down port of ByeTunes
  (EduAlexxis/ByeTunes, MIT) in `core/src/apple_music/` + `AppleMusicSheet.swift`. Local
  Files gets a picking mode and "Add to Apple Music" on songs, albums and local playlists;
  the sheet takes a pairing file (Keychain), shows progress, Stop and the result, and can
  put back the library from before the last change. idevice and rusqlite added to the
  core (libsqlite3.tbd linked; `_remotepairing._tcp` declared for Bonjour). Verified:
  iOS and simulator builds, 10 core tests, 2 UI tests on demo data. Not verified: anything
  on a real iPhone (see VALIDATION.md item 14).
- 2026-10-24, first try on a real iPhone: Check connection worked; Add crashed the app.
  Cause (from the Debug build's stack frames, no crash log): unoptimized async frames on the
  add path (`run` 297 KB, `connect` 402 KB, `Device::connect` 213 KB, `tunnel` 171 KB, then
  idevice's RSD/XPC handshake ~400 KB) overflowed the core runtime's 2 MB worker stack;
  Check uses ~300 KB less. Apple Music jobs now run on their own thread (16 MB stack,
  current-thread runtime). Measure frames with an arm64 disassembly of the prologues
  (`sub x9, sp, #N, lsl #12` before a probe loop).

## 2026-10-24: lyrics translation on iOS

- Why it seemed not to work: translation is off until 文A is tapped, and Apple's engine needs
  the two languages downloaded first; that was a small link at the top of the lyrics. The
  iOS Simulator can't translate at all (`-probeTranslate`: 47 languages listed, every pair
  "unsupported"), while the same calls translate on macOS 26, so the code path is sound.
- Now: 文A asks for the download at once when it's needed, then translates; a clear
  [download Japanese → English] button stays in the lyrics view; the note has its own line.
- Fixed: the lyrics footer pushed Now Playing off the screen in "+ translation" mode (two
  rows when one doesn't fit).
- UI test with a stand-in translator (`-mockTranslate`) checks both display modes.
- Tests now run on their own simulator ("Glass Translate Test", `ios/build/sim-id-tr`) with
  `ios/build/DerivedData-tr`: another session tests on the first one at the same time, and
  two runs on one simulator kill each other.

- 2026-10-24: "after a while in the background playback stops: Internal error { channel
  closed }" (all versions). Cause: librespot-connect reported a dead Connect task
  (`SessionDisconnected`) only after two requests to Spotify succeeded, so when the
  network or session was what died the app never heard and kept a dead engine. Fixed in the
  vendored librespot-connect (reported at once; Shutdown always ends the task) and the
  shared engine (one report per engine from either sign; `Engine::running`). iOS replaces a
  dead engine before commands and on return to the foreground, resumes what was playing,
  and asks iOS for background time while reconnecting. Desktop: restarts reliably now.
- Queue (iOS): sections, ≡-handle reordering (this iPhone and Local Files), swipe to queue in
  Now Playing and on Local Files songs while they play (`QueueUITests`). Desktop
  `cargo check --all-targets` clean and `cargo test` 268 pass; core 225 pass.

## 2026-10-24: "Operation not permitted" on a device

- theme.toml and config.toml were kept in ~/.config/glass. On iOS ~ is the app container's
  root, which an app may not write (only Documents, Library, tmp); the simulator doesn't
  enforce it. So on an iPhone every change saved to those files failed: 文A, the lyrics
  settings, the theme manager, the assistant's settings. They're in Library/Application
  Support/Glass now (with the rest of Glass's data); files from the old place move over at
  launch. Checked by making the simulator container's root read-only (as on a device) and
  running the tests that save settings (`testLyricsTranslation`, `testThemeManager`): both pass.

## 2026-10-25: loudness, polish, sharing without pairing

- Quiet audio: Glass's own volume (default 70 %) went through librespot's soft mixer, whose
  log curve put 70 % at −18 dB, under the iPhone's volume. On iOS the audio now goes out at
  full scale (`mixer::NoOpVolume`; Local Files and the DJ's voice too) and the volume is the
  iPhone's own (`Platform/SystemVolume.swift`: the rail and mute set it through a system
  volume view; the side buttons move the rail; Connect is told; another device's change moves
  it). Normalization stays as Spotify's default (Normal).
- The bottom bar was a flat 92 % of the theme's base color; it's the pages' glass now (theme
  and album colors).
- Now Playing ▸ playlist opens on the song that's playing and follows it; songs swipe right to
  join the queue (and left), as in Spotify (`queueSwipe` environment: pages keep left only,
  since a rightward swipe goes back there).
- Sharing with your other devices (shared `share.rs`, protocol 4): no pairing or code. Devices
  signed in to the same account (a secret from its username and email, proved per Noise
  handshake, never sent) find each other by themselves; Local Files lists them; a device's songs
  can be browsed and the ones picked fetched (`List`/`Get`, `DeviceLibraryScreen`). On by
  default (settings version 2 turns it on once). The desktop answers and sends; its pairing
  dialog and paired list are gone.
- No raw errors on screen: `core/src/errors.rs` turns Web API, network and player errors into a
  few plain sentences, or nothing when Glass recovers by itself (a refused load or queue move
  now replaces the engine instead of showing "Internal error { channel closed }"; queue polling
  errors aren't shown). Page loads, sign-in, lyrics, translation, downloads, the tag editor,
  theme imports, the assistant and Add to Apple Music say what happened in plain words; the
  details go to the log.
- Add to Apple Music's text rewritten (setup collapses once done; "Undo last add").
- From the audit: Edit song info works on local album pages; engine settings (quality,
  gapless, normalize, volume level, autoplay, device name) apply at once (a new engine carries
  on with the song); this iPhone shows as a phone in Spotify; lock-screen info, the audio
  session and volume follow playback from the store (views don't update in the background);
  song-change notifications don't banner while Glass is open; Settings lost the desktop-only
  rows (theme file paths, background effect, font, proxy, client ID, config.toml) and long
  notes; theme panel sections in words, dead "border"/"progress" rows gone; keyboard hints
  only with a keyboard; the assistant knows it's on a phone.
- Swiping back from a page's song rows (playlist, Liked Songs) had stopped working: the rows'
  swipe-to-queue drag (SwiftUI) took every sideways drag. It's a UIKit pan now that begins only
  the allowed way (`QueuePan`); `testSwipeBack` now fails if any case doesn't go back.
- A playlist downloaded as files (Share › Download as M4A/MP3) becomes a local playlist: its
  name, description, cover (`cover.jpg` beside the songs) and order; downloading it again updates
  the same one (`Playlists::from_download`, keyed by the Spotify URI; undoable). Its menu's Add to
  Apple Music fills in the playlist. Saved songs also carry their disc number now. Desktop too
  (name and order).
- "File missing" for every song of a downloaded playlist on the iPhone: the playlist kept
  `/var/mobile/…/Documents/…` while Local Files scanned the same folder as `/private/var/…`
  (an earlier import had added it in that spelling), and Local Files' settings still held the
  Documents folders of three earlier installs (iOS moves the app's container on each install).
  Paths inside the app's Documents are now rebased to this run's folder, in one spelling, at
  every launch (`paths::rebase`: Local Files' folders via `tidy_sources`, the save folder, local
  playlists via `Playlists::map_paths`, and imported files). Seen on the device's own files.
- Add to Apple Music put Apple Music's copy of a song in the playlist instead of the file: any
  library song with the same title, artist and album counted as "already there". Only songs Glass
  (or ByeTunes, or a sync) put in the music folder count now: Apple Music's (no file, a real store
  ID, or a subscription song; Glass's own carry placeholder IDs from 9 000 000 000) don't, so the
  file goes in beside it, and in a playlist the file takes the copy's place, in order.
- Tests: core 224 pass (new `errors`, `share` list/get and account tests); desktop
  `cargo check --all-targets` clean; UI tests `PolishUITests` (playlist pane, swipe right,
  browse a device and get songs) and the rest of the suite.

## Not done / needs you

- Everything in VALIDATION.md "Needs a real iPhone" (no device attached here).
- Real Spotify writes (reorder, save to Spotify, likes by the assistant) and the real
  ChatGPT/OpenAI flows weren't run against your accounts.
- A signed archive: signing needs your team; nothing about signing is stored.
- Distribution: LAME is linked statically (LGPL relinking), and the provider constraints in
  ARCHITECTURE.md apply. Personal builds only.

## Build notes

- Disk on the dev Mac is tight (a few GB free). `ios/core` builds with `incremental = false`
  and line-tables debug info. Desktop checks use a separate target dir that's deleted after.
- `xcodegen` must run after adding Swift files (Glass.xcodeproj lists files explicitly).
- Simulator: "Glass iPhone 17 Pro" (`ios/build/sim-id`). Launch flags: `-demo`,
  `-route playlist|artist|album|home|search|liked|local|settings|nowplaying`,
  `-pane lyrics|queue|playlist|devices`, `-lyrics word|line|plain|instrumental|none|ambiguous`,
  `-viz`, `-autoSignIn`, `-ephemeralSignIn`; debug builds only: `-probeSpotify <credentials.json>
  [-probeEngine] [-probePlay]`, `-probeSave [-mp3]`, `-probeBonjour`, `-probeHashes`,
  `-assistantMock <base URL>` (the assistant against scripts/mock-openai.py), `-probeTranslate`
  (what the Translation framework offers here, to Documents/probe.log), `-mockTranslate`
  (a stand-in translator for UI tests).
