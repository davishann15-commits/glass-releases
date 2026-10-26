# Glass

A terminal-inspired, transparent music player for Spotify on macOS and Linux, written
in Rust (egui). It works like Spotify — library, search, playlists, queue,
Spotify Connect, lyrics, downloads — and looks like a quiet pane of smoked
glass with monospaced type.

Spotify.app 1.3.4 was reverse-engineered with REA to find the services its
own desktop app uses. See [FEATURES.md](FEATURES.md) for a feature-by-feature
comparison, including what can't be done and why.

## Requirements

- macOS 12+ (Apple silicon or Intel), or Arch Linux (x86_64 or aarch64;
  X11 or Wayland)
- A Spotify account. Playing songs from Spotify needs **Premium** (Spotify streams
  only to Premium accounts); with a free account Glass browses, plays Local Files
  and saves songs (Share ▸ Download as M4A/MP3).
- To build: Rust (`rustup`, stable)

## Install

**macOS** — `./scripts/package-macos.sh` builds `dist/Glass-<version>.dmg`: a
universal app (Apple silicon and Intel) with a "Read Me First" for whoever
you share it with. It's ad-hoc signed, not notarized, so the first open needs
System Settings → Privacy & Security → *Open Anyway* (or
`xattr -dr com.apple.quarantine /Applications/Glass.app`).

**Arch Linux** — `./scripts/package-arch.sh` (on any machine) writes
`dist/arch/glass-<version>.tar.gz` and a `PKGBUILD` for it. Copy `dist/arch/`
to the Arch machine and run:

```sh
makepkg -si
```

That builds Glass with Cargo and installs `/usr/bin/glass`, the desktop entry
and icons. Sound goes through ALSA (with `pipewire-alsa` on PipeWire), media
keys through MPRIS, the assistant's sign-in through the Secret Service
(GNOME Keyring or KWallet). On Linux, lyrics are translated by the provider
(Spotify, when it sends a translation) or the assistant; Apple's on-device
translation is macOS-only.

## Build and run

```sh
./scripts/bundle.sh          # optimized build -> target/Glass.app (macOS)
open target/Glass.app
```

The first build takes a few minutes; rebuilds take about half a minute. If
Glass is running, the script opens the new build, which takes over from the
old one (Glass runs one copy at a time: starting a newer build replaces the
running one, starting the same one again brings its window forward).
`./scripts/bundle.sh release` makes the slower, fully optimized build that
the shared packages use.

Or for development: `cargo run` (macOS or Linux).

Click **Sign in with Spotify**. Your browser opens Spotify's own login page,
and you return to Glass when it's done.

## Using it

- **Library** (start screen): your library as a numbered list; the selection
  on the right with `[ play ]`, `[ shuffle ]`, download and featured artists.
- **Now playing** (`L`, or click the cover): artwork and controls on the
  left; lyrics on the right, following the song, with the audio visualizer
  under them (`V`). `Q` shows the queue; `P` shows the playlist, album or
  Liked Songs that's playing, so you can pick any song from it.
- **Album colors** (Settings → Display, on by default): Now Playing takes its
  colors from the cover, a soft wash behind everything and the cover's
  most vivid color, brightened, as the accent. Songs change with a crossfade, and the new
  colors bloom out from the artwork.
- **Command bar** (`⌘K`): search your library, run commands ("queue",
  "theme panel", "liked songs"…), or press Enter to search Spotify.
- Keys (`space`, letter keys, `/` and `[ ]` don't fire while you're typing in a
  field; the `⌘` keys work anywhere):
  - `space` play/pause · `⌘ ←/→` previous/next · `⇧⌘ ←/→` seek · `⌘ ↑/↓` volume
  - Library: `j`/`k` or `↑`/`↓` move, `Enter` open, `⇧Enter` play, `1` `2` `3` tabs
  - Song lists (playlists, albums, Liked Songs, search): `j`/`k` move,
    `Enter` play, `⇧Enter` add to queue
  - `L` lyrics, `Q` queue, `P` the playlist that's playing (again to go back,
    to the same spot in the page you came from); on Now Playing, `D` devices
    (again for the lyrics). There, `j`/`k` and `Enter` work in the list. `V` shows or
    hides the visualizer; `[` / `]` shift this song's lyrics timing
  - `/` or `⌘F` filter the page · `⌘K` command bar · `Esc` closes the top-most
    thing (menu first) · `⌘[`/`⌘]` back/forward · `⌘T` theme · `⌘/` all shortcuts
- One click on a song (or its ▶) plays it; ⌘-click and ⇧-click select several.
  A click on a queued song plays from there.

## Save songs

Share ▸ **Download as M4A** (or MP3) on a song, playlist, album or artist (its
top songs) saves it as a tagged audio file in `~/Music/Glass`, which Local
Files already reads. The audio isn't Spotify's: each song is matched on
JioSaavn (AAC up to 320 kbps), then YouTube Music, and fetched natively (no
yt-dlp or ffmpeg). M4A keeps that audio as it is; MP3 is encoded by LAME.
Collections go into their own folder, three songs at a time, with progress
and Stop in the same menu, and playlists get an `.m3u8`. Settings → Save
songs picks the format, MP3 bitrate and folder. See `src/save_song/`.

## Updates

Glass updates itself (the Mac app from the DMG): it checks for a new version
shortly after it starts and every few hours, downloads it in the background,
and installs it when Glass quits; *update to … · restart* at the top installs
it straight away. Settings → Updates turns this off or checks now.

Releases are published to the public
[glass-releases](https://github.com/davishann15-commits/glass-releases)
repository (the built app only; this repository stays private). Each update
is signed with an ed25519 key that lives in the developer's Keychain, and
Glass installs only updates signed with it, for this platform and the version
they claim (see `src/update.rs`). To publish one, raise the version in
`Cargo.toml`, commit, and run:

```sh
./scripts/release-macos.sh "What's new"
```

(`tools/glass-release keygen` made the key once; `pubkey` prints the public
half that's in `src/update.rs`.) Builds from `scripts/bundle.sh` are
development builds and don't update themselves. On Linux, Glass says when a
new version is out; building the new package (`./scripts/package-arch.sh`, then
`makepkg -si`) updates it.

## The DJ

Spotify's DJ plays in Glass: the DJ card at the top of Home, its page, or
`dj` in ⌘K (and other Spotify apps can start it on Glass). X introduces songs
in X's own voice, as in Spotify's apps (the song starts when X is done;
Space pauses X too), and what X says shows in the player bar. Settings →
Playback → *DJ voice* turns the talking off.

How: Spotify answers the DJ with an empty context and a link to its session,
which hands out the mix a page at a time with X's lines as SSML; Glass's
copy of librespot-connect follows it (vendor/librespot-connect/GLASS.md),
and `src/dj.rs` has each line spoken by Spotify's text-to-speech service.

## Local files

Local Files has its own tab in the side rail, organized the way iTunes is:
**Albums** (covers; an album lists its songs in track order, disc by disc),
**Songs** (a table sorted artist → album → track, or by any column),
**Artists** (each beside their albums), **Genres**, **Recently Added** and
**Playlists**. Add the folders your music is in with **Add a source**; a
folder that's already covered (the same one, or one inside another) isn't
added twice, and each file is listed once.

- **Local songs in your Spotify playlists play** when the file is in one of
  those folders — for example songs copied over from your phone. Spotify only
  remembers a local song's title, artist, album and length, so Glass finds the
  file by those: same title and artist, lengths within 3 seconds. A playlist's
  local song that isn't found is greyed out; hover it to see why, or to see
  which file it's linked to. MP3, M4A/MP4 (AAC, ALAC), FLAC, Ogg, WAV and
  AIFF play at any sample rate, mono or stereo. Newly copied files are picked
  up without restarting.
- **Edit a song's info**: the ✎ on a Local Files row, `E` (with `j`/`k` to
  choose), or *Edit song info…* in its menu (also on a playlist's local song).
  Title, artist, album, year, genre and cover art are written into the file
  itself (ID3 for MP3, the native tags for other formats). The edit is made on
  a copy that then replaces the file, so a failure leaves it untouched, and
  the file keeps its date. Changing a title or artist can stop a playlist from
  finding that file, since that's how it's recognised.

### Between your devices

Glass on your other devices (a Mac, a Linux PC, an iPhone) shows up at the top
of Local Files by itself when it's open on the same network and signed in to
the same Spotify account. There's nothing to set up and no code to type
(Settings → Your Library → *Share with Glass on your other devices* turns it
off).

- **Get songs from another device**: click it in Local Files to see its songs,
  pick the ones you want (or *pick all*) and *get* them. Songs already here are
  marked and left out.
- **Send songs**: right-click songs (⌘/⇧-click for several) → *Send to* → the
  device.
- **Get a playlist's missing local songs**: right-click a greyed-out local
  song → *Get from your other devices*, or *Get N missing local songs from
  your other devices* in the playlist's ⋯ menu. Whichever device has the
  file sends it.

Received songs are saved in `Music/Glass`, which joins Local Files, so they
play straight away. Devices find each other with Bonjour/mDNS and are listed
once they answer (a Glass that quit without saying so, or a computer that's
asleep, isn't). A Glass of your account that speaks an older or newer version
of sharing is listed with which one needs updating. Each transfer is
encrypted (Noise), and before anything is sent both sides prove they're
signed in to your account: an HMAC of that connection's handshake, keyed with
a secret made from your username and email address, which Spotify shows only
to your account (the secret itself is never sent). Nothing goes through a
server, so both devices need to be on the same network. On macOS and iOS,
allow Glass when asked about your local network; on Linux with a firewall,
allow mDNS (UDP 5353) and TCP 47321.

Glass plays these files with a patched copy of librespot's player, kept in
`vendor/librespot-playback` (see GLASS.md there for exactly what changed).

## Theme: `~/.config/glass/theme.toml`

Every color, size and font comes from this file (`$XDG_CONFIG_HOME/glass/theme.toml`
when `XDG_CONFIG_HOME` is set). It's optional: anything missing uses the
default. [`theme.example.toml`](theme.example.toml)
is the commented default; copy it there to start, or open the theme panel
(`⌘T`) and change a value — Glass then writes the file for you.

```toml
[theme]
background = "#0F1115"   # window tint
foreground = "#E9E4DC"   # text
accent     = "#4F83A3"
border     = "#090A0B"
opacity    = 0.45        # 0 clear … 1 solid
blur       = 28          # background blur radius, 0–100

[font]
family = "SF Mono"       # any installed font
size   = 13.0            # everything scales from this
lyrics = "SF Mono"       # "system" is SF Pro on macOS
```

`[colors]` overrides individual tokens (surfaces, secondary/muted text,
selection, hover, focus, player bar…); `[layout]` sets `radius` and `spacing`;
`[motion] reduce = true|false|"system"`; `[palette] colors` are the accent
shortcuts in the theme panel. Colors are `#RGB`, `#RRGGBB` or `#RRGGBBAA`.

- **Live reload**: save the file and the window updates within a second.
- **Mistakes are safe**: a bad value is reported (theme panel and Settings →
  Appearance, with the key and the reason) and the last good theme stays.
  The panel validates before writing and never overwrites a file it can't
  parse. Comments and unknown keys are kept.
- Preferences (volume, EQ, library layout…) stay in `settings.json` below,
  where they've always been; the theme file only holds the look.

## Lyrics: `~/.config/glass/config.toml`

Lyrics come from [LRCLIB](https://lrclib.net), a free lyrics database
maintained by its community, and — for Spotify songs LRCLIB doesn't have —
from Spotify's own lyrics, read through Glass's Spotify connection
(librespot's lyrics request). Glass looks them up only while they're on
screen, once per song, and keeps LRCLIB's answers on this computer.

```toml
[lyrics]
provider  = "auto"   # "auto" (LRCLIB, then Spotify), "lrclib" or "spotify"
fallback  = true     # with "auto": try Spotify when LRCLIB has no match
translation  = "off"     # "off", "below" (original + translation) or "only"
translate_to = "system"  # "system" or a language code like "en", "es", "ja"
translator   = "auto"    # "auto", "apple" (macOS on-device) or "assistant"
offset_ms = 0        # positive shows lyrics earlier (-10000…10000)
contact   = ""       # optional homepage/email for LRCLIB's User-Agent
```

[`config.example.toml`](config.example.toml) is the commented default. The
file works like theme.toml: optional, `$XDG_CONFIG_HOME` respected, applied
when saved, bad values reported in Settings → Lyrics while the last good
settings stay, and in-app changes (Settings → Lyrics) edit it in place
without touching comments or other keys.

**Per song, kept for good** (in Glass's data folder, `lyrics_choices.json` —
not the cache, so clearing the cache doesn't lose them):

- **Your pick.** Lyrics you choose with "wrong lyrics?" (or "none of these")
  become that song's lyrics every time it plays — also when it plays from
  another album, playlist or release (matched by title, artist and length).
  "look again" goes back to automatic matching.
- **Timing.** `[` / `]` and the `− timing +` control under the lyrics shift
  only the song that's playing, and Glass remembers it for that song.
  `offset_ms` in config.toml is a general shift for every song (e.g. a
  speaker's delay), added on top.

**Look:** `[lyrics]` in theme.toml, or the theme panel (`⌘T` → lyrics) with a
live preview: colors of the current line, the other lines and the word being
sung; size; alignment (left, center, right); where the current line sits
(top, upper, middle); line spacing; how much far lines fade; the animation
when a line changes (glide, fade, pop or none) and its speed; bold current
line; and the typeface (`font.lyrics`). Lines glide into place and cross-fade;
with word timing, each word brightens as it's sung. With Reduce Motion on,
lines cross-fade instead.

**Visualizer** (`V`): bars, mirrored bars or a wave under the lyrics, in a
gradient of your accent (or `[visualizer]` colors). It shows the audio played
on this computer — Spotify through Glass's player, and local files — taken as
it reaches the speakers, so it's in step. It works only while it's on screen.
Style, colors, opacity, number of bars, height, sensitivity, smoothing and
glow are in `[visualizer]` or the theme panel (`⌘T` → visualizer). When music
plays on another device, it rests and says so.

- **Spotify's lyrics** are the fallback: with `auto` and `fallback`, a
  Spotify song LRCLIB has no lyrics for gets Spotify's (line-synced, usually
  from Musixmatch). Choosing `spotify` uses only Spotify's. Local files have
  no Spotify lyrics. The footer says which source you're seeing.
- **Translation** (`T`, or the `文A` switch under the lyrics, or Settings →
  Lyrics): the original, the original with the translation under each line,
  or the translation alone. Translations come from, in order: the provider's
  own (Spotify's, when it sends one), Apple's on-device translation on macOS 26
  or later (free and private — nothing leaves the Mac; a language not yet downloaded
  is offered in System Settings → General → Language & Region → Translation
  Languages), and the assistant's model when the assistant is on (any
  system, Linux included; sends the lyrics to OpenAI with your plan or key).
  A song is translated once per language and kept in the cache folder.
- **Animation and Reduce Motion:** with macOS Reduce Motion on (or `[motion]
  reduce = true`), lines change with a cross-fade instead of gliding or
  popping, as Apple recommends. Set `reduce = false` in theme.toml for full
  motion regardless of the system. Only a change of line animates; resizing
  or a translation appearing just moves the text.
- **Only real timing is shown.** Word-synced lyrics (LRCLIB's Lyricsfile
  `words`, or enhanced LRC) light up word by word; syllable timing, when a
  record splits words, syllable by syllable. Line-synced lyrics highlight the
  whole current line. Plain lyrics don't move. The header says which
  (`lyrics / line sync`); the footer says where they came from and how they
  were matched. In practice almost every LRCLIB record is line-synced: none
  of ~600 records sampled in October 2026 had word timing.
- **Matching** uses title, artist, album and duration. A remaster or a
  "feat." credit counts as the same recording; a live version, radio edit,
  remix or acoustic take must match on both sides, and the length must be
  within 3 s. LRCLIB's own exact match comes first; if it has none, Glass
  searches and checks the results itself. When several records fit but their
  words differ, you choose. "wrong lyrics?" opens the same picker at any time,
  and your pick is kept for that song (see above).
- **Being a good LRCLIB client** (per lrclib.net/docs): requests name Glass
  and its version in the User-Agent, go out one at a time with a pause
  between them, time out after 10 s, retry at most twice, and honor `429`/`503`
  `Retry-After`. Answers are cached: matches 30 days, misses a day (LRCLIB adds
  lyrics in the background). Your picks are kept apart, until you choose
  *look again*.
- Lyrics are copyrighted by their rights holders. LRCLIB's free API isn't a
  license to redistribute them; Glass only displays them, for you, and never
  exports or shares them.

## Assistant

An AI assistant that works inside Glass: ask about your music or tell it what
to play. "Queue me up a couple of Anri songs I haven't heard" finds them and
queues them, quietly, without anything moving on screen. It answers questions
about your listening, plays albums, playlists and Liked Songs, likes songs,
skips, pauses, shuffles and sets the volume, and builds playlists from your
Spotify library or your local files. It's on by default and does nothing until
you sign in (Settings → Assistant, or ⌘J); turn it off there.

**Set up**

1. Settings → Assistant (it's on unless you turned it off).
2. *Continue with ChatGPT*. Your browser opens OpenAI's sign-in. Choose the
   account and workspace and allow Glass to use your ChatGPT plan. Eligible
   **ChatGPT Plus and Pro** plans can be used. Signing in alone doesn't allow
   requests: Glass checks that the plan permission
   (`chatgpt.tokens.use.direct`) was actually granted.
3. Pick a model. GPT-6 Luna is chosen when your account offers it. Your choice
   is kept, and Glass never switches to a pricier model on its own.
4. ⌘J opens the panel. Or type `?` and your request in the ⌘K bar (it works on
   Now Playing too): it runs in the background, and the answer appears as a
   toast.

Or choose *my OpenAI API key* and paste a key into that field in Settings
(never anywhere else). API usage is billed by OpenAI to the key's account,
separately from ChatGPT. Glass never switches between the two on its own, not
even after an error.

**How it works**

- The model only proposes. Rust checks every ID against what the tools
  actually returned (your library, Spotify search, your playlists), so
  made-up songs never get played, queued or saved. Glass carries out each
  action and tells the model what really happened.
- Everyday actions just happen: queueing a few songs, playing, liking a
  song, skip, pause, shuffle, repeat and volume. Queueing or liking more than
  20 songs at once needs your approval.
- It knows what you listen to from your playlists: Glass reads every song in
  your playlists (yours and ones you follow, not Spotify's editorial ones) and
  Liked Songs once, keeps a small index on this computer (refreshed every 20
  minutes and after the assistant changes your library), and recommendations
  start from the artists you save and play most. Only a summary (top artists
  and counts) or the songs a question needs are sent, never the whole list.
- "Songs by X I haven't heard" is one step: songs by X that aren't in any of
  your playlists or Liked Songs (any release of the same song counts) and
  weren't played lately. Spotify doesn't share full play history, so the
  assistant says "probably new to you".
- It answers in a sentence or two. If a request takes more than 16 tool
  steps, Glass asks the model for an answer with what it found instead of
  stopping with an error.
- Playlists appear as an editable preview with real songs and their total
  length. Nothing is saved until you click save (or approve when the
  assistant asks). Spotify previews become a new **private** Spotify
  playlist, or are added to one you can edit. Spotify playlists are never
  replaced and songs are never removed from them. Local-file previews become
  Glass **local playlists**, and replacing one needs your approval. *Undo*
  reverts the last playlist, like or local-queue change (Spotify's queue
  can't be undone from Glass). Saving the same preview twice can't create a
  second copy.
- With each request Glass adds one line about what's playing and which page
  is open, so "this song" and "this album" work.
- Requests stream; *stop* (Esc) cancels and late results are discarded. Each
  turn is limited to 16 model steps, 12 tool calls per step, recent history,
  capped tool results and a reply-length cap.
- Usage is shown per session in tokens. With an API key, a dollar figure
  appears only when the model's price is known, and it's labelled an estimate.
  *Manage usage* opens ChatGPT settings → Usage. On Plus, the five-hour limit is
  shared with other apps. When you reach a limit, the assistant pauses; Glass
  doesn't retry or fall back to your API key.

**What's sent to OpenAI:** your message, the one-line note of what's playing
and which page is open, and only what the assistant looks up for it:
matching songs, albums, artists and playlists from your Spotify library,
listening history and Spotify search (names, artists, lengths, IDs), plus
the tags of matching local files. Liked Songs and playlists are searched
inside Glass, and only the matches are sent, never the whole library. Glass
never sends audio, file paths or your Spotify login, and sends a song's lyrics
only when the assistant translates them.

*Let it use my Spotify library and playback* (Settings → Assistant, or
`assistant.spotify` in `config.toml`) is on by default once the assistant is
on. Turn it off to keep the assistant to your local files, with nothing from
Spotify sent. Note that Spotify's Developer Terms restrict sending Spotify
content to AI services.

Requests are sent with `store: false`. Names and tags are passed as data, and
the model is told never to follow instructions found in them.

**Credentials:** sign-in tokens and API keys are kept in the macOS Keychain
(the Secret Service — GNOME Keyring or KWallet — on Linux). They never go in
config files, logs or the UI. Non-secret registration details are kept in
`assistant/accounts.json` and `assistant/host.json` (the installation's host
ID): the issued client ID, the verified account ID and email, and whether plan
use was granted.

**Revoke access:** *Sign out* in Settings → Assistant revokes the session with
OpenAI and deletes the tokens. You can also disconnect Glass in ChatGPT
settings. Glass can't see your ChatGPT conversations.

## Glass and blur

- **macOS**: the window is transparent and WindowServer blurs what's behind
  it at `blur` points (`CGSSetWindowBackgroundBlurRadius`, the private but
  long-stable call Ghostty, kitty, Alacritty and WezTerm use). If a future
  macOS drops it, Glass falls back to AppKit vibrancy, where `blur` has no
  effect. Settings → Appearance shows which one is active.
- **Reduce Transparency** (System Settings → Accessibility) makes the window
  opaque; **Reduce Motion** turns off animations unless the theme says otherwise.
- **Linux** (Wayland): Glass asks the compositor to blur what's behind it
  (ext-background-effect-v1: Hyprland, KWin and others that support it).
  How strongly it blurs is the compositor's setting — on Hyprland,
  `decoration:blur` size 7 with 2 passes looks like the Mac's 28 pt — so
  `blur` only turns it on or off. Without compositor blur (X11, or a
  compositor without the protocol), Glass keeps the background at least 88%
  opaque for legibility; set `GLASS_LINUX_BLUR=1` if your compositor blurs
  Glass anyway (picom rules), or `0` if it doesn't. The Mac's window buttons
  (close, minimize, zoom) are drawn in the title strip.

## How it works

```
auth.rs          OAuth (PKCE) in the browser; stores only a refresh token
api.rs           data client the UI calls with Web-API-shaped paths; logs every request
sp/              Spotify's internal services (what the desktop app itself uses):
                 playlists, collection, batched metadata, recently played,
                 profiles, GraphQL search/artist/browse, Connect player state
engine.rs        librespot session + player + Spotify Connect device
dsp.rs           6-band EQ, mono and soft limiter; opens the audio output when
                 something plays (and lets it rest when nothing does)
downloads.rs     whole tracks from the CDN, in parallel, kept in the encrypted audio cache
audio_out.rs     follows the system audio output (headphones coming and going)
share.rs         local files between your Glass devices on the network
dj.rs            the DJ's voice (Spotify's text-to-speech for X's lines)
instance.rs      one Glass at a time (a newer build takes over)
lyrics/          one lyrics model; LRCLIB client, Lyricsfile/LRC parsing,
                 recording matching, cache, provider choice and fallback
theme_config.rs  theme.toml: parsing, validation, live reload, safe edits
user_config.rs   config.toml (lyrics and assistant settings), same rules as theme.toml
window_fx.rs     transparency, blur, accessibility settings
app.rs           state, Actions (what the UI asks for), Msgs (what comes back)
ai/              the optional assistant: Sign in with ChatGPT, keyring storage,
                 Responses API streaming, music tools, bounded agent turns
playlists.rs     Glass local playlists (with undo)
save_song/       Share ▸ Download as M4A/MP3 (JioSaavn, YouTube Music, MP4 remux, LAME)
ui/              egui interface (ui/images.rs: cover loading; one copy of each, on the GPU)
```

Spotify's public Web API rate-limits the client ID every librespot player
shares, so Glass talks to the internal services instead and converts their
answers to the Web API's JSON shapes. The GraphQL query hashes come from
Spotify.app. When Spotify rotates them, Glass on macOS reads the current ones
from Spotify.app, if it's installed, and retries; otherwise search, artist
pages and Browse fail until Glass is updated.

## What's stored

| Path | Contents |
|---|---|
| `~/.config/glass/theme.toml` | Your theme (only once you create or edit it) |
| `~/.config/glass/config.toml` | Lyrics and assistant settings (only once you create or change one) |
| `~/Library/Application Support/Glass/settings.json` | Preferences, local folders, pins |
| `~/Library/Application Support/Glass/session.json` | Spotify refresh token (mode 0600) |
| `~/Library/Application Support/Glass/credentials.json` | librespot's reusable login (mode 0600) |
| `~/Library/Application Support/Glass/downloads.json` | What's downloaded |
| `~/Library/Application Support/Glass/device_id`, `data_device_id` | This computer's Connect IDs |
| `~/Library/Caches/Glass/audio/` | Encrypted audio: streaming cache (size limit in Settings) and downloads |
| `~/Library/Application Support/Glass/lyrics_choices.json` | Your lyrics picks and per-song timing |
| `~/Library/Application Support/Glass/local_playlists.json` | Glass local playlists |
| `~/Library/Application Support/Glass/assistant/` | Assistant registration details (no tokens: those are in the Keychain) |
| `~/Library/Application Support/Glass/queries.json` | Spotify's current page queries, when Glass had to read them from Spotify.app |
| `~/Library/Caches/Glass/lyrics/` | LRCLIB answers and saved translations, one small file per song (clear in Settings → Lyrics) |
| `~/Library/Caches/Glass/artwork/`, `updates/`, `glass.sock` | The playing song's cover (for the media controls), a downloaded update, and the socket that keeps Glass to one copy |

On Linux the same files are in `~/.config/glass` (with theme.toml and
config.toml) and `~/.cache/glass`, and the assistant's secrets are in the
system keyring.

No analytics, telemetry or crash reporting.

## Development

```sh
cargo test                     # unit tests + headless UI tests (clicks, keys, theme panel)
cargo clippy --all-targets && cargo fmt --check
GLASS_DEMO=1 cargo run         # sample data, no sign-in, no network
./scripts/check-linux.sh       # from a Mac: the Linux build type-checks and lints
./scripts/test-linux.sh        # in a Debian container (Docker): build, test, start the demo
```

The demo runs in a new home folder of its own, removed when it quits, so it never
touches your settings, library or accounts; `GLASS_DEMO=/some/folder` uses (and
keeps) that folder instead.

`GLASS_ROUTE` (`library`, `nowplaying`, `home`, `playlist`, `artist`,
`search`, `liked`, `local`, `device`, `settings`, `activity`), `GLASS_RIGHT` (`queue`,
`devices`, `lyrics`, `assistant`), `GLASS_LYRICS` (`word`, `line`, `plain`,
`instrumental`, `none`, `ambiguous`), `GLASS_THEME_PANEL=1` and
`GLASS_WINDOW=900x600` set up the demo for checking layouts. Demo lyrics are
sample data answered from memory; the demo never contacts LRCLIB.
`GLASS_DEMO_SHARE=/a/folder/of/songs` is the one exception to "no network": Local
Files reads that folder and sharing runs for real, signed in as the demo's account
(which only other demos have), to test sharing between two demos or with the iPhone
app (`ios/scripts/live-share.sh`).

Live tests are opt-in: `cargo test --release live_ -- --ignored --nocapture`.
`live_lrclib` looks up real songs on lrclib.net (line-synced, plain-only,
instrumental, a live version, a missing song) and searches for word-synced
records. Against your own account: `live_routes` and `live_player_state` only
read; `live_playback` and `live_contexts` play at volume 0 (they take over
your Spotify playback while they run); `live_dj` plays the DJ on a test device
at volume 0 (it skips if something is playing); `live_download` fetches one
song into a throwaway cache; `live_playlist_edits` creates and deletes a test
playlist. `live_save` and `live_save_many` save songs from JioSaavn and YouTube
Music, and `live_get_a_song_from_running_glass` needs Glass running with
sharing on.

## Notes

- Downloads stay encrypted exactly as Spotify serves them and only play in
  Glass while you're signed in; each play still needs a connection to unlock.
  Spotify's audio is never written out as plain files: *Download as M4A/MP3*
  saves a copy found on JioSaavn or YouTube Music instead.
- librespot is an unofficial re-implementation of Spotify's protocol, and
  Glass uses Spotify's internal services. Neither is allowed by Spotify's
  Developer Terms; Glass is meant for personal use.
- Not affiliated with Spotify.

## License

Glass is under the MIT License (`LICENSE`). The open-source software it's built
on keeps its own licenses, listed in `packaging/THIRD-PARTY-NOTICES.txt` (About
→ Open-source licenses in the app). LAME, the MP3 encoder, is LGPL: it's a
separate library (`Contents/Frameworks/libmp3lame.0.dylib` on macOS, the
system's on Linux) that can be replaced with another build, and its source is
published with the releases.
