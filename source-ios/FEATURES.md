# Feature parity with Spotify for macOS

This ledger compares Glass with **Spotify 1.3.4.258** (`/Applications/Spotify.app`),
reverse-engineered with REA.

**How the inventory was built**

| Source | What it gave us |
|---|---|
| `Contents/Resources/Apps/xpui.spa` (the React UI bundle) — REA `analyze_javascript_application`, Evidence `ev_d82012ed9a55760319d00b05050c7b4f305986a36a9c8616c85d7fa4762fdcb2` | 188 JS files, 1,987 modules, ~50 lazily loaded route chunks (`xpui-routes-*`, `dwp-*`). REA's semantic graph is *partial* (result budget), so features were confirmed from routes, strings and the code itself, not only the graph. |
| `xpui.spa/i18n/en.json` | 3,132 UI strings: every context-menu item, setting, keyboard-shortcut description, empty state, and so on. |
| `xpui-root-dialogs.js` | The shortcut dialog's sections (Basic / Playback / Navigation / Layout) and the actions in each. |
| `xpui-snapshot.css` | Layout grid areas `left-sidebar / main-view / right-sidebar / now-playing-bar`, an 8 px panel gap, 8 px panel radius, and a 232 px minimum library width. |
| `xpui-routes-desktop-settings.js` | Equalizer sliders: −12…+12 dB in 0.1 dB steps, labelled `Hz`/`KHz`. |
| `Contents/MacOS/Spotify` (native) strings | Six EQ filters: `equalizer_low_shelf_gain`, `low_peak`, `low_mid_peak`, `high_mid_peak`, `high_peak`, `high_shelf`. |

**Legend:** ✅ works · ◐ partial (see note) · ✖ not possible for a third-party app (reason given)

Glass reads your library through the internal services Spotify's own desktop app
uses (see README). The ✖ items are what it still can't do, or what needs DRM that
only Spotify's apps hold. Glass shows these entry points and says why they're
unavailable instead of hiding them.

---

## Layout and navigation

| Feature | Status | Notes |
|---|---|---|
| Side rail: Home, Library, Search, Local Files, Settings | ✅ | Back is in the title strip (⌘[ / ⌘]); drag the strip to move the window, double-click to zoom. |
| Main view in its own panel | ✅ | |
| Side panel: Queue / Devices / Lyrics / Assistant | ✅ | Lyrics from LRCLIB (see Lyrics below). Now Playing has its own screen. Friend Activity was removed to keep Glass light. |
| Now-playing bar | ✅ | |
| "Playing on ⟨device⟩" strip | ✅ | |
| Toasts ("Added to queue", …) | ✅ | |

## Your Library

| Feature | Status | Notes |
|---|---|---|
| Filters: Playlists, Podcasts, Albums, Artists, Audiobooks | ✅ | |
| Filter: Downloaded | ✅ | |
| Search in Your Library | ✅ | |
| Sort: Recently Added, Alphabetical, Creator | ✅ | |
| Sort: Recents | ◐ | Ordered by your last 50 plays; Spotify's per-item "last played" is private. |
| View as List / Compact / Grid | ✅ | |
| Liked Songs, Your Episodes, Local Files | ✅ | |
| Pin / unpin | ◐ | Stored locally by Glass; Spotify's pins are private. |
| Folders: create, rename, delete, drag playlists in | ◐ | Kept by Glass; Spotify's own folders aren't read or changed. |
| Create playlist | ✅ | |
| Upcoming releases, events, venues, tours, prompted playlists | ✖ | Private APIs. |

## Home and Search

| Feature | Status | Notes |
|---|---|---|
| Home chips: All / Music / Podcasts / Audiobooks | ✅ | |
| DJ card, shortcut grid, Jump back in, top artists, on repeat, your playlists, albums and shows | ◐ | Built from your recents and top items; Spotify's personalised home feed isn't used. |
| Search with chips: Songs, Artists, Albums, Playlists, Podcasts, Episodes, Audiobooks | ✅ | Profiles and genres can't be searched via the Web API. |
| Top result + Songs | ✅ | |
| Recent searches | ✅ | Stored locally. |
| Browse all (categories) | ✅ | |
| Category pages | ✅ | From Spotify's browse pages; a search is offered if one doesn't load. |
| Command bar (⌘K) | ✅ | Find something in your library, run a command, or press Enter to search Spotify; ⌘L focuses search. |

## Pages

| Page | Status | Notes |
|---|---|---|
| Playlist: header, play, shuffle, save, more menu | ✅ | |
| Playlist: search in list, sort (custom/title/artist/album/date added/duration), List/Compact | ✅ | Sorting by clicking column headers works too. |
| Playlist: add, remove, drag to reorder, edit details, public/private, collaborative | ✅ | Changing the cover image isn't supported. |
| Playlist: "Find more" search | ✅ | |
| Playlist: Recommended songs | ✖ | `/recommendations` was removed for third-party apps (Nov 2024). |
| Playlist: column picker | ◐ | Columns adapt to width but can't be chosen. |
| Album: discs, copyright/℗, more by artist | ✅ | |
| Artist: banner, follow, popular, discography (with filters), appears on | ✅ | |
| Artist: Fans also like | ◐ | Shown when Spotify still returns it. |
| Artist: biography, monthly listeners | ✅ | |
| Artist: artist pick, concerts, merch | ✖ | Private APIs. |
| Discography page | ✅ | |
| Podcast (show) page, episode page, episode rows with resume progress | ✅ | |
| Audiobook page with chapters | ✅ | Playing needs an audiobook entitlement on your account. |
| Song (track) page | ✅ | |
| Profile (yours and others'): top artists/tracks, public playlists, following, follow/unfollow | ✅ | Follower lists aren't available. |
| Recents (listening history) | ✅ | Last 50 plays. |
| Local Files | ✅ | MP3/M4A/AAC/FLAC/OGG/WAV/AIFF from folders you choose, with cover art and a tag editor (title, artist, album, year, genre, cover). |
| Local songs inside Spotify playlists | ✅ | Linked to your files by title, artist and length (±3 s); any sample rate. Missing ones are greyed out with an explanation. |
| Adding local songs to playlists | ✅ | Right-click (⌘/⇧-click or ⌘A for several) → Add to playlist: any Spotify playlist you can edit, as in Spotify's apps, or a Glass local playlist. Also "Add local files…" on a playlist, or drop files from Finder onto it; their folder joins Local Files so they play. |
| Concerts, live events, blend, taste profile, country hubs, label pages, Premium page | ✖ | Private APIs; Premium upsell removed on purpose. |

## Playback

| Feature | Status | Notes |
|---|---|---|
| Play/pause, previous/next, seek, volume, mute | ✅ | |
| Shuffle; repeat / repeat one | ✅ | |
| Skip back/forward 15 s (podcasts) | ✅ | |
| Add to queue, view queue | ✅ | |
| Clear queue | ◐ | The Web API has no clear command; Glass restarts the current context at the same spot. |
| Remove a single item from the queue | ✖ | No API. |
| Song radio / artist radio | ◐ | On this computer it uses librespot's autoplay context. On other devices it plays the seed and Spotify's autoplay continues. |
| Smart shuffle | ✖ | Private. |
| Playback speed (podcasts) | ✖ | librespot can't change speed. |
| Crossfade, Automix | ✖ | librespot plays tracks back to back (gapless is supported). |
| Gapless | ✅ | |
| Normalize volume + level (Quiet/Normal/Loud) | ✅ | librespot normalisation with pre-gain. |
| Mono audio | ✅ | Applies instantly. |
| Equalizer: 6 bands, ±12 dB, 22 presets, draggable curve | ✅ | Band layout matches Spotify's native filters. Preset curves are Glass's approximations, since Spotify's values aren't in readable strings. |
| Streaming quality | ✅ | 96 / 160 / 320 kbps. No lossless. |
| Autoplay | ✅ | |
| Explicit-content filter | ✅ | Client-side: greyed out and skipped. |
| Canvas, music videos, video podcasts | ✖ | Private API / video DRM. |
| Lyrics on the now-playing screen and side panel | ◐ | From LRCLIB, a community database: line-synced for most songs, word by word only where LRCLIB has word timing (rare). Animated line changes; colors, size, alignment, position, spacing, fade, animation and font in `[lyrics]` / ⌘T. Your pick and timing are kept per song. |
| Audio visualizer (V) | ✅ | Bars, mirror or wave under the lyrics, in the theme's colors; follows audio played on this computer. |
| Now playing: the playlist that's playing (P) | ✅ | Pick any song from the playlist, album, artist or Liked Songs that's playing (or the local list). |
| Spotify's own lyrics (Musixmatch) | ✅ | The fallback for Spotify songs LRCLIB doesn't have, through Glass's Spotify connection. |
| Lyrics translation | ✅ | Original, original + translation, or translated (T). Spotify's own translations when present; Apple on-device (macOS); the assistant on any system. |

## Spotify Connect

| Feature | Status | Notes |
|---|---|---|
| This computer is a Connect device ("Glass") | ✅ | librespot `Spirc`. |
| Device picker, transfer, remote control of other devices | ✅ | |
| Google Cast, Jam, local-network discovery | ✖ | Private protocols. |

## Menus, sharing, shortcuts

| Feature | Status | Notes |
|---|---|---|
| Track menu: add to playlist, remove from playlist, like, queue, radio, go to artist/album, credits, share | ✅ | Works on multi-selections. |
| Playlist / album / artist / show / audiobook / profile menus | ✅ | Same items as Spotify where an API exists. |
| Exclude from taste profile, mark as played/finished, buy/unlock audiobook | ✖ | Private APIs; not shown. |
| Share: copy link, copy Spotify URI, open in browser | ✅ | |
| Share to WhatsApp/X/Facebook/…, embed code | ◐ | Copy the link instead. |
| Credits | ✖ | Songwriter/producer credits are private; the artist and album links are on every row. |
| Keyboard shortcuts + ⌘/ dialog | ✅ | Spotify's key table is mangled in the bundle, so the bindings follow Spotify's documented macOS shortcuts. |
| Multi-select (⌘-click, ⇧-click, ⌘A) | ✅ | |

## Desktop integration and settings

| Feature | Status | Notes |
|---|---|---|
| Miniplayer (always on top) | ✅ | |
| Full screen now playing | ✅ | |
| Media keys + macOS Now Playing widget (with artwork) | ✅ | Play and Pause from headphones (AirPods ear detection) never toggle. |
| Follows the system audio output | ✅ | Moves to new headphones or speakers mid-song; pauses when the playing device goes away. |
| Notifications on track change | ✅ | |
| Open at login (No / Yes / Minimized) | ✅ | Writes a LaunchAgent (macOS) or an autostart entry (Linux). |
| Close button minimizes | ✅ | |
| Proxy | ◐ | HTTP proxies only (no SOCKS). |
| Cache location display, size limit, clear cache | ✅ | |
| Downloads | ✅ | Whole files from Spotify's CDN, four at a time, retried when they fail. Kept encrypted in Glass's own cache, at the streaming quality; each play still needs a connection to unlock. |
| Language | ✖ | English only for now. |
| Private session, listening activity, show follows/recent artists | ✖ | Private APIs. |
| Account: details, log out, edit login methods | ✅ | Edit opens spotify.com. |
| Settings search | ✅ | |
| Ads, upsells, "Install app" | — | Removed on purpose. Premium is required. |

## Added beyond Spotify

| Feature | Status | Notes |
|---|---|---|
| Optional AI assistant (⌘J, or `?` in ⌘K) | ✅ | Sign in with ChatGPT (eligible Plus/Pro plans) or your own API key. Uses your Spotify library, history and search (can be turned off) and your local files: queues and plays songs, likes them, controls playback in the background, answers questions, and builds Spotify or local playlists from previews you save with one click. Bulk changes need approval; undo is available. |
| Glass local playlists | ✅ | Playlists of your local files, kept on this computer. Add songs by menu, file picker, Finder drop or dragging from Local Files; remove them with undo. |
| Automatic updates | ✅ | The Mac app downloads new versions in the background and installs them on quit (or now, from the top bar); only updates signed with the release key are installed. |
| Spotify DJ (X) | ✅ | Plays the DJ's mix page after page (librespot couldn't: the DJ resolves to an empty context plus a session link). X's lines are spoken in X's voice before songs, as in Spotify's apps, and shown in the player bar; pause works on X too, and the voice can be turned off. DJ card on Home, `dj` in ⌘K; other Spotify apps can start the DJ on Glass. |
| Local Files tab | ✅ | Its own tab in the side rail, organized like iTunes: Albums (a grid of covers; an album lists its songs in track order, by disc), Songs (a table sorted by artist, album and track, or by any column header), Artists (beside their albums), Genres, Recently Added and Playlists. Each file is listed once, however the source folders overlap. |
| Sending local files between your Glass devices | ✅ | Glass on your other computers, on the same network and signed in to the same account, shows up in Local Files. Right-click songs → Send to; a playlist's missing local songs come from whichever device has them (right-click, or the playlist's ⋯ menu). Encrypted, and each device proves it's on your account; received songs go to Music/Glass, which joins Local Files. |

## Added for transparency

- **Activity page**: every request to Spotify and LRCLIB (time, method, path, status, latency), every host Glass talks to, and every file it stores, with sizes.
- **Lyrics say how they're timed** (`lyrics / line sync`), where they came from and how they were matched, and are never highlighted more precisely than the provider timed them.
- **Settings say when changes apply**: instantly or on engine restart, and why a setting is disabled.
- **Demo mode** (`GLASS_DEMO=1`): sample data, no network.
