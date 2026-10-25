# Glass for iOS — architecture and feasibility

Baseline: glass-player `main` at b403e2a (1.0.0-beta.22), inspected 2026-10-23.
The port lives on the `ios` branch (worktree `~/glass-ios`); the desktop app keeps
building from the same tree.

## Shape

```
ios/
  core/        glass-core: a Rust staticlib (UniFFI) — the desktop's service code plus an iOS orchestrator
  bindgen/     host tool: UniFFI's Swift binding generator (pinned, own lockfile)
  Glass/       the SwiftUI app (App/, Platform/, Theme/, Views/, Resources/, Generated/ ← bindings)
  GlassUITests/ XCUITest UI tests (demo data + a local mock of OpenAI's API)
  project.yml  XcodeGen spec → Glass.xcodeproj (app, UI tests, the Glass scheme)
  scripts/     build-core.sh (Rust core + bindings, run by Xcode), licenses.py (third-party
               notices), mock-openai.py (assistant test server), make-ios-icon.swift
  docs/        this file, PARITY.md (the ledger), STATUS.md, VALIDATION.md, BUILD.md,
               screenshots/, audit/ (inventories of the desktop source)
```

### What is shared, and how

The desktop crate stays one package. `ios/core/src/lib.rs` includes the desktop's
service modules **by path** (`#[path = "../../../src/sp/mod.rs"] mod sp;` …), so both
apps build from one copy of: `api`, `auth`, `sp/` (Spotify's internal services),
`engine` (librespot session, player, Spotify Connect), `dsp` (EQ, mono, limiter),
`audio_out`, `viz`, `dj`, `downloads`, `local`, `playlists`, `share`, `save_song/`,
`lyrics/`, `ai/`, `config`, `model`, `theme_config`, `user_config`.

`ios/core/build.rs` sets `cfg(glass_mobile)`. The shared files differ on iOS in a
handful of places, each behind that cfg (the desktop declares it in `[lints.rust]`
check-cfg, so it builds without warnings):

| Shared file | iOS difference |
|---|---|
| `auth.rs` | Refresh token in the Keychain (via `platform::secrets()`), not `session.json`; `Auth::from_token` for the iOS sign-in |
| `config.rs` | Default music folder = the app's Documents |
| `save_song/mod.rs` | `default_dir()` = Documents ("On My iPhone › Glass") |
| `local.rs` | The egui cover loader is desktop-only; `read_cover` is `pub(crate)` for the app |
| `theme_config.rs` | `Color32` from `ecolor` (egui's color crate) instead of `eframe` |
| `audio_out.rs` | `changed(gone)` split out of the poller so iOS route changes feed it |
| `engine.rs` | No reusable login (`credentials.json`) on disk: librespot's cache gets no credentials path; the engine signs in with a fresh access token each start |
| `sp/mod.rs` | A rejected GraphQL hash is recovered from Spotify's web player (`web_player_queries`) instead of Spotify.app |
| `share.rs` | Discovery through DNS-SD (`core/src/bonjour.rs`) instead of mdns-sd; the device's name comes from the app (the Connect device name) |
| `ai/oauth.rs` | The loopback answers the sign-in sheet with `302 glass-ios://assistant-signed-in` (so it closes) instead of a "return to Glass" page |
| `lyrics/translate.rs` | Apple's translation through the app's Translation framework adapter instead of the macOS helper process |

Two pure-logic pieces moved out of desktop UI files so both apps share them:
`src/palette.rs` (album-color palette, from ui/album_colors.rs) and `src/local_index.rs`
(Local Files albums/sort keys, from ui/local_library.rs); the desktop re-exports them.

`window_fx` (window effects) is replaced by a three-function shim in `core/src/window_fx.rs`.

Vendored crates: the iOS core uses the same `[patch.crates-io]` copies (librespot
core/playback/connect with Glass's changes). One change was needed for iOS, in
`vendor/librespot-core` (documented in its GLASS.md): `sysinfo` doesn't build for iOS,
so the OS/kernel version strings come from `sysctlbyname`, and on iOS librespot
presents itself exactly as on macOS (`config::OS == "macos"`) — the identity Glass's
internal-service code is written and tested against — instead of switching to
Spotify's iOS client ID, hash-cash client tokens and mobile version strings.

### The iOS orchestrator (replacing the desktop `App`)

The desktop `App` (src/app.rs) is egui's immediate-mode state: every frame reads it
and asks for data. It isn't compiled for iOS. `core/src/app.rs` (`MobileApp`) ports its
non-UI logic function by function (same actions, fallbacks, retries and toast wording;
comments name the desktop function):

- **Flow**: Swift calls `GlassCore` methods → `Msg::Act(Action)` into an `mpsc` channel →
  the **actor thread** (`glass-actor`) owns all state and applies messages in order →
  async work runs on a 2-worker tokio runtime and answers with `Msg`s → after each batch,
  `publish()` builds what Swift shows and sends only what changed (`CoreEvent`s).
- **Data**: `want(path)` / `want_all(path, cap)` exactly as the desktop (5-min TTL with
  background refresh, 20 s retry after a failure, invalidation by substring, eviction of
  pages unused for 10 min except Your Library). Page builders (`views.rs`) read the same
  paths as the desktop pages and produce typed `PageModel`s (header + blocks).
- **Subscriptions**: a SwiftUI page calls `subscribe(route)` on appear and `unsubscribe` on
  disappear; the actor rebuilds subscribed pages after each batch and emits only changed ones.
- **Events** (`bridge.rs` `CoreEvent`): Account, Playback, Queue, Devices, Library, Page,
  Saved (heart/follow flags), Toast, Navigate, Left, ShowNowPlaying, Lyrics, Theme,
  AlbumColors, Settings, Local, TagsSaved, Saves, Share, AppleMusic, Assistant, OpenSignIn,
  OpenAssistantSignIn, SignedOut. Pull-style data (the Activity page, file export/import)
  is an async call answered by the actor through a oneshot channel.
- **Feature modules**, each the desktop's logic on `MobileApp` with its model for Swift:
  `lyrics_app` (lookups, picker, translation), `colors_app` (album colors), `theme_app`
  (theme.toml, export/import), `settings_app`, `local_app` (Local Files, tags, imports),
  `save_app` (Save as M4A/MP3), `share_app` (+ `bonjour`), `assistant_app` (the
  desktop's assistant_app.rs), `activity_app`.
- **iOS-only module** `apple_music/` (Add to Apple Music, ported from ByeTunes):
  `device.rs` reaches the phone's own device services through LocalDevVPN with the idevice
  crate (lockdown before iOS 26.4, a RemotePairing tunnel after), `library.rs` edits a
  copy of MediaLibrary.sqlitedb with rusqlite on the system's SQLite (`libsqlite3.tbd`),
  `song.rs` reads the files with lofty, `mod.rs` runs it on the core's runtime and
  publishes `AppleMusicModel`. The pairing file lives in the Keychain.
- **Position** isn't streamed: `PlaybackModel` carries `position_ms` and `position_at`
  (Unix ms); Swift extrapolates while playing. A local file's position is only re-sent
  when it jumps (seek, track change, pause), so playing produces no per-tick events.
- **Visualizer**: Swift polls `visualizer_bands(n, sensitivity, smoothing)` at display rate
  only while the visualizer is on screen; it returns the desktop analyser's bands from
  the real output tap (empty when nothing plays on this device).

### The bridge

UniFFI 0.32 (proc-macros, library-mode bindgen). Chosen over a hand-written C ABI
because the API is wide (≈80 calls, ≈30 records/enums) and UniFFI gives typed Swift
records and enums (Equatable/Hashable, usable directly as SwiftUI navigation values),
error enums, foreign traits for callbacks, and it catches Rust panics at every exported
call (they never unwind into Swift).

- Ownership: `GlassCore` is an `Arc` object held by the Swift `Store`. Records are copied
  across. Swift implements three foreign traits: `CoreListener` (events), `Keychain` and
  `Translator` (Apple's on-device translation).
- Threads: Swift → Rust calls only enqueue (never block on I/O). Rust → Swift events
  arrive on the actor thread; `Store.Listener` hops them to the main actor, and only the
  main actor mutates the store. The listener holds the store weakly.
- Shutdown: `shutdown()` sends `Msg::Shutdown`, joins the actor (settings are saved).
- Panics: caught by UniFFI on exported calls; the actor and tokio tasks run on their own
  threads (a panicking task answers its request through `Answer`'s drop guard, as on the desktop).

### Build

`ios/scripts/build-core.sh [sim|device|all] [debug|release]`: `cargo build --locked` for
`aarch64-apple-ios(-sim)` (iOS 26.0 deployment target), UniFFI bindings from the built
library, the C header + module map in `ios/build/headers` (`SWIFT_INCLUDE_PATHS`). Xcode's
"Build Rust core" phase runs it first for the platform and configuration being built and
it copies `libglass_core.a` into `$(BUILT_PRODUCTS_DIR)`, declared as the phase's output;
the target links it from there as a build product, so the linker re-runs when the core
changes and a build always gets that platform's library. (An XCFramework was used at first;
Xcode reads an XCFramework before any build phase runs, so after building for one platform
a build for the other found no library for it.) `xcodebuild`/⌘B is all that's needed.
macOS deployment variables are unset in the script so they can't leak into the iOS
cross-builds. `ios/core/Cargo.lock` pins every crate (seeded from the desktop's lock).

LAME (MP3 encoding) is built from `vendor/lame-3.100` by `core/build.rs` for the iOS
target and linked statically (see PARITY.md, "LAME on iOS", for the licensing difference
from the desktop's separate dylib).

## Platform adapters (Swift)

| Concern | Desktop | iOS |
|---|---|---|
| Audio output | rodio/cpal → CoreAudio default device | rodio/cpal → **RemoteIO** (cpal's iOS host); `AVAudioSession` `.playback`/`.longFormAudio`, background audio mode |
| Following the output | polls the default device name each second | `AVAudioSession.routeChangeNotification` → `audio_route_changed(gone)`; `.oldDeviceUnavailable` pauses |
| Interruptions | — | `interruptionNotification` → pause; resume only with `.shouldResume` and only if it was playing |
| Media keys / Now Playing | souvlaki | `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter`; Play/Pause idempotent; claimed only while this device plays |
| Secrets | session.json 0600 + keyring | Keychain, `AfterFirstUnlockThisDeviceOnly` (refresh while locked; not in backups) |
| Sign-in | browser + librespot-oauth loopback | `ASWebAuthenticationSession` (shared browser session) + the core's loopback listener on the registered redirect, `state` checked, PKCE; listener answers `302 glass-ios://…` to close the sheet |
| Window blur | private CGS call / NSVisualEffectView | none: the app draws its own depth (`GlassBackground`, materials on panels) |
| Fonts | SF Mono from disk | the system monospaced design (SF Mono, ships with iOS, Dynamic Type) |
| Files | folder pickers, ~/Music/Glass | Documents (shown in the Files app) as Glass's folder; document-picker imports copied into Documents/Imported; folders picked in Files kept as security-scoped bookmarks; drag and drop onto the window |
| mDNS | mdns-sd raw multicast sockets | Bonjour through `dns_sd` (DNSServiceRegister/Browse/Resolve/GetAddrInfo on a dispatch queue; no multicast entitlement needed); `NSBonjourServices` + local-network permission |
| Translation | Swift helper process | Translation framework in-app (`AppleTranslator`) |
| Notifications | notify-rust | `UNUserNotificationCenter` (`TrackNotifier`) |
| Keyboard | egui key handling | SwiftUI `keyboardShortcut`s (`KeyCommands`) for a hardware keyboard |
| Assistant sign-in | system browser + loopback on 127.0.0.1:1455 | `ASWebAuthenticationSession` + the same loopback in the core, sheet closed by a `glass-ios://` redirect |

## Feasibility findings (evidence so far)

| Question | Finding | Evidence |
|---|---|---|
| Does the shared Rust (librespot 0.8 + patches, reqwest/rustls, native-tls, symphonia, lofty, snow, mdns-sd, keyring, LAME) build for iOS? | Yes, after one librespot-core change (sysinfo) and the cfg gates above | `cargo check`/`build` for `aarch64-apple-ios` and `-sim` |
| Swift ↔ Rust calls and Rust → Swift events | Work | Simulator: `pong … from Rust thread glass-core` |
| Local audio through the shared player on iOS | Works: AAC M4A decoded by symphonia, played via rodio → cpal → RemoteIO, position advancing in real time | Simulator: position 3063 → 5143 ms over ~2 s |
| Whole app (core actor + SwiftUI store + pages) | Works in demo mode | Simulator screenshots (Library, playlist, artist, Home, Now Playing) |
| Spotify from the iOS build (TLS, access point, login5, spclient, GraphQL, Connect) | Works: premium account read, 52 playlists and 1,693 Liked Songs via spclient, GraphQL search and Browse answered (built-in hashes valid), Connect observer, and Spotify listed the iOS device in Connect | Simulator, debug probe (`-probeSpotify <desktop credentials.json> -probeEngine`), 2026-10-23; read-only, your desktop login reused without refreshing any token |
| Standalone Spotify playback (librespot → DSP → rodio → RemoteIO) | Works: a Liked Song decoded and played (volume 0, ~8 s, only because nothing else was playing), output tap live | Simulator probe `-probePlay` |
| Spotify sign-in on iOS | Implemented (PKCE, state checked, loopback + custom-scheme close). The full sign-in needs you to enter your credentials in the sheet | Unit tests: wrong `state` refused, stray requests ignored |
| Background / lock-screen playback, route changes, interruptions | Configured (audio background mode, session, remote commands); **needs a real device** | — |
| Bonjour discovery without the multicast entitlement | Works: DNS-SD registers `_glass-files._tcp` with the desktop's TXT and resolves peers | Simulator `-probeBonjour`: found a service registered on the Mac with `dns-sd -R`; the Mac's `dns-sd -B` saw the app's |
| GraphQL hash rotation on iOS | Handled: current hashes read from Spotify's web player | Simulator `-probeHashes`: all five found, equal to Spotify.app 1.3.6's |
| The assistant's loop on iOS (tools, previews, approvals, save, undo) | Works against a mock of OpenAI's API | UI test `testAssistantPreviewSaveAndUndo` + request log (no file paths sent) |
| ChatGPT sign-in sheet on iOS (sheet → loopback → back to the app → code exchange) | Works against the mock; the real flow needs your ChatGPT account | UI test `testAssistantSignInPlumbing` |

Notes: UniFFI's own `async_runtime = "tokio"` wrapper doesn't run librespot's futures
properly (timers never fired); async exports spawn onto the core's runtime and await the
join handle instead (`GlassCore::debug_probe_spotify`).

### Provider and distribution constraints (not technical blockers for a personal build)

- **Spotify**: Glass signs in with Spotify's desktop client ID (as the desktop app and
  librespot do), uses Spotify's internal services, and plays through librespot. Spotify's
  Developer Terms don't allow any of that for third-party apps; the README already says
  Glass is for personal use. The same holds on iOS. App Store distribution of a librespot
  player would also conflict with App Review (third-party service terms). The client ID is
  still configurable (`settings.json` → `client_id`), as on the desktop; a registered client
  of your own would need `http://127.0.0.1:8898/login` registered and may not be granted
  the `streaming` scope for librespot's login.
- **GraphQL query hashes**: the desktop recovers rotated hashes by reading an installed
  Spotify.app. An iOS app can't read another app's bundle, so iOS reads the same
  declarations from Spotify's public web player (open.spotify.com and its scripts on
  open.spotifycdn.com) when a hash is refused, and keeps them in `queries.json` as the
  desktop does. Verified 2026-10-23 (all five current hashes found). If Spotify changes how
  its web player declares queries, the pages that need them show errors, not silence.
- **Spotify's official iOS SDK (App Remote)** wasn't used: it plays through the Spotify app,
  which would lose Glass's own decoding (EQ, mono, visualizer tap, local files in playlists,
  encrypted downloads, the DJ continuation) and background ownership.
- **Save songs (JioSaavn / YouTube Music)**: App Review requires authorization from the
  source for media download features; Glass has none. It can work in a personal build; it
  is not distributable through the App Store as is.
- **ChatGPT plan sign-in**: the desktop registers itself dynamically with OpenAI's agent
  sign-in (`client_id=dynamic_agent_client`, loopback redirect on 127.0.0.1:1455, PKCE,
  JWKS-verified ID tokens, plan scope `chatgpt.tokens.use.direct`). iOS runs the same flow
  with the same loopback inside the app while the sign-in sheet is up. The app side is
  verified against a mock server; whether OpenAI's real sign-in accepts it needs your
  account and wasn't tried. The API-key path doesn't depend on it.
