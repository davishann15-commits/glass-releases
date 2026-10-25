<!-- Read-only inventory of the desktop source (glass-player b403e2a, 1.0.0-beta.22), made for the iOS port. Paths are relative to the repository root. -->

GLASS DESKTOP: INVENTORY OF PERSISTED SETTINGS (read-only survey of the desktop source)

All paths below are relative to the repository root. References use file:line.

Things an iOS port should not copy blindly:
- The EQ, mono and normalization only run on the librespot engine path (Spotify streams, plus a playlist's local songs that librespot resolves). The Local Files page plays through its own rodio player, `LocalPlayer` in src/local.rs:547, which never touches DspParams. So the EQ note "Applies instantly to music played in Glass" is too broad.
- The explicit filter is enforced only in the UI (greyed rows plus an automatic skip). The engine itself does not filter.
- Theme/config live reload is a 500 ms poll of the file's mtime and size, not a file-system watcher.

====================================================================
1. SETTINGS STRUCT (settings.json): src/config.rs
====================================================================
Path: `config_dir()/settings.json` (config.rs:384-386).
How it is read and written:
- `#[serde(default)]` on the struct (config.rs:259): a missing key takes its value from `Default` (config.rs:336-381), and unknown keys are ignored.
- `version` has its own `#[serde(default)]`, so a file without it reads as 0.
- Enums are stored as their variant names ("High", "VeryHigh", "Normal", "Quiet", ...). Paths are strings. `Option` fields are stored as null.

Load (config.rs:388-410):
- File missing: defaults. Read error: a warning, then defaults.
- File that does not parse: `set_aside` renames it to `settings.json.bad`, then defaults are used.
- After a good parse: `upgrade()` then `sanitize()`.

Save (config.rs:437-442): `serde_json::to_string_pretty`, written with `write_atomic`. The file is not written 0600, but its folder is 0700.

When saves happen:
- `mark_settings_dirty` (app.rs:1077-1080) marks the change and requests a repaint 900 ms later.
- The tick saves once the change is more than 800 ms old (app.rs:3255-3260).
- `save_now` runs at exit (`on_exit`, app.rs:4114).
- The Settings page clones the settings before drawing, only on input frames, and diffs them afterwards (settings.rs:108-123). An `autostart` change calls `apply_autostart()` straight away.

Constants: `SETTINGS_VERSION = 1` (config.rs:15); `DEFAULT_CLIENT_ID = "65b708073fc0480ea92a077233ca87bd"` (config.rs:19).

Field-by-field ("instant" = takes effect immediately; "engine restart" = when the playback engine restarts):

1. `version`: u32. Default 1, or 0 when the key is missing. Records the file's schema version.

2. `client_id`: String. Default `DEFAULT_CLIENT_ID`. Not validated.
   - Used by: OAuth sign-in and resume (app.rs:1048, 3140) and the engine's `SessionConfig.client_id` (engine.rs:111).
   - UI note: "Changing it requires logging in again."

3. `device_name`: String. Default "Glass". Not validated.
   - Used by: Connect name (engine.rs:181), the device list (right.rs:297, 319) and the engine status text (app.rs:3595).
   - Applies on engine restart.

4. `streaming_quality`: Quality { Normal, High, VeryHigh }. Default High. The serde alias "Low" maps to Normal (config.rs:188-207).
   - Labels: "Normal (96 kbps)", "High (160 kbps)", "Very high (320 kbps)".
   - Engine bitrate (engine.rs:119-123): Bitrate96 / Bitrate160 / Bitrate320.
   - Downloads use the quality given to `Downloads::run` (app.rs:3598); the file-format preference order is in downloads.rs:200-208. A changed quality means re-downloading.
   - Applies on engine restart.

5. `normalize`: bool, default true. Sets `PlayerConfig.normalisation` (engine.rs:125). Applies on engine restart.

6. `loudness`: Loudness { Quiet, Normal, Loud }, default Normal.
   - `pregain_db` (config.rs:216-225): Quiet -9.0, Normal 0.0, Loud +3.0.
   - Sets `PlayerConfig.normalisation_pregain_db` (engine.rs:126).
   - Applies on engine restart. The row only shows while `normalize` is on.

7. `gapless`: bool, default true. Sets `PlayerConfig.gapless` (engine.rs:124). Applies on engine restart.

8. `autoplay`: bool, default true. Sets `SessionConfig.autoplay = Some(..)` (engine.rs:111). Applies on engine restart.

9. `mono`: bool, default false. Instant: the DspState is pushed to the audio thread whenever it changes (app.rs:3245-3249).

10. `allow_explicit`: bool, default true. Instant, UI-level only (see section 7).

11. `eq_enabled`: bool, default true. Instant (DspState).

12. `eq_preset`: String, default "Flat". Not validated on load: any string is shown in the combo. "Manual" is set when a band is dragged.

13. `eq_gains`: [f32; 6], default all 0.0.
   - Sanitized to -12..=12; NaN or infinity becomes 0 (config.rs:424-426).
   - The UI rounds drags to 0.1 dB. Instant.

14. `library_view`: LibraryView { List, Compact, Grid }, default List.
   - The library page offers "list", "compact", "grid" (library.rs:511-514).
   - The Settings toggle switches between Compact and List only. Instant.

15. `library_sort`: LibrarySort { Recents, RecentlyAdded, Alphabetical, Creator }, default Recents. Labels "recents", "recently added", "alphabetical", "creator" (library.rs:481-506). Instant.

16. `show_local_files`: bool, default true. Instant.

17. `local_dirs`: Vec<PathBuf>.
   - Default: [UserDirs audio_dir, download_dir], i.e. ~/Music and ~/Downloads (config.rs:338-339, 357).
   - Sanitize drops temporary folders (`local::is_temporary`: "TemporaryItems", or `$TMPDIR/com.apple.*`; local.rs:178).
   - `App::new` runs `local::tidy_sources`, which removes duplicates and folders nested inside another source (app.rs:1008-1012; local.rs:77).
   - Also handed to `PlayerConfig.local_file_directories` (engine.rs:129) and to `librespot local_file::set_directories` on rescan (app.rs:2621-2622).

18. `share_local_files`: bool, default false.
   - Instant: `keep_sharing` starts or stops LAN sharing each tick (app.rs:1580-1590), once signed in with the engine connected.

19. `local_view`: LocalView { Albums, Songs, Artists, Genres, Recent, Playlists }, default Recent.
   - Chips: "Albums", "Songs", "Artists", "Genres", "Recently Added", "Playlists" (local_library.rs:347-358).

20. `save_format`: save_song::Format { M4a, Mp3 }, default M4a. Names "M4A" / "MP3" (save_song/mod.rs:35-59).

21. `save_mp3_kbps`: u16, default 320. Must be one of BITRATES [128, 160, 192, 256, 320] (save_song/mp3.rs:17); anything else becomes 320 (config.rs:430-432).

22. `save_dir`: Option<PathBuf>, default None, meaning `default_dir()` = UserDirs audio_dir (or ~/Music) + "Glass" (save_song/mod.rs:70-80).

23. `pins`: Vec<String> of URIs, default empty. A new pin is inserted at the front (app.rs:2553-2562).

24. `folders`: Vec<Folder { id: String, name: String, playlists: Vec<String> }> (config.rs:249-256).
   - New ids are `random_hex(16)`; a playlist moves out of every other folder (app.rs:2563-2589).
   - These are Glass's own folders; Spotify's rootlist folders are ignored.

25. `now_playing_on_play`: bool, default false. Instant: `start_play` navigates to Now Playing unless the play is quiet (app.rs:2095). This field replaced `show_npv_on_play`, so the old value does not carry over.

26. `track_notifications`: bool, default false. Instant: notifies on a track change, but not for the first track (app.rs:3948).

27. `album_colors`: bool, default true. Instant (ui/mod.rs:68-90; see section 3).

28. `visualizer`: bool, default true. Toggled with the V key (mod.rs:558-563), the Now Playing meter button (mod.rs:271-290) or the theme panel's "show  V". Instant.

29. `dj_voice`: bool, default true. Instant: stored into `dj::SHARED.voice_on` every tick (app.rs:3220).

30. `auto_update`: bool, default true. Release builds check 20 s after start, then every 6 h (app.rs:978, 3211-3219).

31. `autostart`: Autostart { Off, Normal, Minimized }, default Off.
   - Instant: writes or removes ~/Library/LaunchAgents/app.glass.player.plist on macOS, or ~/.config/autostart/glass.desktop elsewhere. Minimized adds `--minimized` (app.rs:4044-4091).

32. `close_minimizes`: bool, default false. Instant (ui/mod.rs:434-439).

33. `window_size`: Option<[f32; 2]>, default None.
   - Kept only if both values are finite and 900≤w≤10000, 600≤h≤10000; otherwise None (config.rs:429).
   - Linux only (main.rs:77-80; app.rs:3384-3393). The window minimum is 900×600 and the fallback size 1280×820.

34. `proxy`: String, default "".
   - Validated when used (`proxy_url`, config.rs:31-37). The error text is "the proxy address “…” isn't valid (Settings → Proxy)".
   - Glass's own HTTP goes through a OnceLock set at startup (config.rs:54-67; app.rs:837). It also reaches the engine session (engine.rs:110-111), the Api client (app.rs:3498) and the LRCLIB client.
   - Applies after restarting Glass.

35. `cache_limit_mb`: u64, default 2048, clamped 256..=50000 (config.rs:428).
   - The prune runs every 600 s and when the engine is ready, and never deletes downloads (app.rs:3261-3264, 3375-3382).

36. `right_open`: bool. Marked `#[serde(skip)]`, so it is never persisted. The Default is true, but `App::new` sets it to false (app.rs:1027).

37. `volume`: f32, default 0.7. Sanitized to 0..1; NaN becomes 0.7.
   - Becomes Connect `initial_volume` = volume × 65535 (engine.rs:181).
   - Saved by `set_volume` except while muted (app.rs:2165-2175).

38. `recent_searches`: Vec<String>. A committed search goes to the front, duplicates are removed, and the list is cut to 10 (pages.rs:400-409).

Settings::upgrade (config.rs:412-420): there is only one migration. If `version < 1` (beta.6 made sharing opt-in), `share_local_files` is forced to false. Then `version = 1`. Test at config.rs:480-491.

Settings::sanitize (config.rs:422-435): eq_gains, volume, cache_limit_mb, window_size, save_mp3_kbps and temporary local_dirs, as listed above. Nothing else is validated: client_id, device_name, eq_preset, proxy, pins, folders and recent_searches length.

EQ_BANDS (config.rs:448): (60, "60Hz"), (150, "150Hz"), (400, "400Hz"), (1000, "1KHz"), (2400, "2.4KHz"), (15000, "15KHz").

====================================================================
2. OTHER FILES, HELPERS AND PATHS
====================================================================
Folder derivation, using the directories 6.0 crate:
- `config::config_dir()` = `ProjectDirs::from("", "", "Glass").config_dir()` (config.rs:21-23).
  - macOS bundle id "Glass": ~/Library/Application Support/Glass.
  - Linux: lowercased "glass", so $XDG_CONFIG_HOME/glass or ~/.config/glass.
  - Fallback: relative ".glass".
- `config::cache_dir()` (config.rs:25-27): ~/Library/Caches/Glass on macOS, ~/.cache/glass on Linux; fallback ".glass-cache".
- `theme_config::config_dir()` (theme_config.rs:469-476): $XDG_CONFIG_HOME/glass, only if that path is absolute; else $HOME/.config/glass; else relative ".config/glass".
  - Holds theme.toml and config.toml.
  - On macOS this is a different folder from config_dir; on Linux it is the same one.
- `UserDirs` gives the audio and download folders (Settings default, save_song default_dir). `BaseDirs` is used for the autostart paths.

Helpers in src/config.rs:
- `make_private_dir()` (81-91): `create_dir_all(config_dir)` plus chmod 0700 on unix. Called at startup (main.rs:73), after the engine starts, and by `write_private`.
- `write_atomic(path, bytes)` (95-103): creates the parent, writes `<path>.tmp` (".tmp" appended to the full name), then renames it over the file.
- `write_private(path, bytes)` (107-125):
  - Calls `make_private_dir` if the path is inside config_dir.
  - Removes any stale .tmp, opens a new one `create_new` with mode 0600, writes it, renames it.
- `read_json<T: Default>(path)` (130-143): missing file gives `T::default()`. A parse error calls `set_aside` and returns the default. Any other read error logs a warning and returns the default.
- `set_aside(path, why)` (147-152): renames the file to `<path>.bad` and logs a warning.
- `test_dir(name)` (156-175, cfg(test) only): `$TMP/glass-tests-<pid>/<name>`, removed by an `atexit` hook.
- `proxy_url` (31), `use_proxy` (58, OnceLock), `http()` (73: reqwest builder with the proxy), `notices_file()` (41).

theme_config.rs has its own `write_atomic(path, &str)` (568-576), used for both TOML files:
- It canonicalizes first, so a symlinked file is written at its target.
- It writes `<name>.toml.tmp` (`with_extension("toml.tmp")`), then renames.
- `file_stamp()` (562) returns (mtime, length).

Files in config_dir (also listed in the app's "What Glass stores" card, pages.rs:1817-1834):

- `settings.json`: see section 1. Pretty JSON, written with `write_atomic`.

- `session.json` (auth.rs:44-86): `{"refresh_token":"…"}`, compact.
  - Written with `write_private` (0600).
  - Read ad hoc: an invalid file means no session. `has_saved_session` = the file exists. Log out deletes it.

- `credentials.json`: librespot's reusable login, written by librespot Cache (credentials location = config_dir; engine.rs:114). Glass chmods it 0600 after the engine starts (engine.rs:184-191).

- `volume`: librespot Cache's volume file (same Cache volume path, config_dir). Written on Connect volume changes (vendor/librespot-connect/src/spirc.rs:1837-1838). Glass does not read it; the start volume comes from settings.volume.

- `device_id` (engine.rs:90-102): 40 random hex characters, plain text with no newline, written with `std::fs::write`. Regenerated if empty. This is the Spotify Connect device id.

- `data_device_id` (sp/mod.rs:159-172): 40 hex characters. A stored id is accepted if it has 34 or more hex characters. Used by the library/pages connection.

- `queries.json` (sp/mod.rs:113-157): `{queryName: "<64-hex hash>"}`, compact, `std::fs::write`.
  - Merged over the built-in hashes.
  - Written only after rescanning Spotify.app's xpui JS, so macOS only.

- `downloads.json` (downloads.rs:48-70, 196-232):
  - Shape: `{"collections": {uri: {"name","items":[uris]}}, "files": {uri: {"file_id":"<40 hex>","bytes":u64,"quality":"High"|null}}}`. Maps are BTreeMaps (sorted).
  - Compact JSON, written to `downloads.json.tmp` then renamed. Not private.
  - Loaded with `read_json`. Written after 10 completions or when idle.

- `local_playlists.json` (playlists.rs:8-70): pretty array of LocalPlaylist objects:
  - `id`: "lp_" + 12 hex characters.
  - `name`: at most 120 characters. `description`: at most 500, default "".
  - `tracks`: [paths]. `created` / `updated`: unix seconds. `from_draft`: Option.
  - Loaded with `read_json`; written with `config::write_atomic` before the in-memory list changes. Undo keeps the last 50 steps, in memory only.

- `lyrics_choices.json` (lyrics/choices.rs:16-147): compact array of entries:
  - `key`: track key.
  - `song`: base title, U+001F, normalized main artist, plus [U+001F + debug-formatted version markers] when the title has any.
  - `secs`: duration in seconds.
  - `pick` (optional): `{"kind":"lyrics","record":{id, track_name, artist_name, album_name, duration_ms, instrumental, has_word_sync, plain, synced, lyricsfile}}` or `{"kind":"none"}`.
  - `offset_ms` (optional, left out when 0).
  - Lookup: by key first, then by song with |secs difference| ≤ 2. An entry is removed when it has no pick and offset 0.
  - Written on a background thread that writes only the newest contents, to `.json.tmp` then renamed. `flush()` at quit.

- `share_key` (share.rs:725-743): text, line 1 = private key in hex, line 2 = public key in hex (32 bytes each), a NOISE "Noise_XX_25519_ChaChaPoly_BLAKE2s" keypair (share.rs:55). Written 0600 with `write_private`. Regenerated if it does not parse.

- `share_paired.json` (share.rs:626-688): pretty array of `{"key": "<hex Noise public key>", "name": "<computer name>"}`. Loaded with `read_json`, written with `write_private` (0600).

- `assistant/host.json` (ai/account.rs:41-70): `{"ext_agent_host_id":"urn:uuid:…"}`, pretty, 0600. Regenerated unless it starts with "urn:uuid:".

- `assistant/accounts.json` (ai/account.rs:33-79): `{"registrations":[{client_id, subject?, email?, plan_enabled}], "active": client_id|null}`, pretty, 0600. Tokens and API keys are kept in the OS keyring, not here.

- `app.ron`: exists on disk, but no current code writes it (the eframe persistence feature is not enabled in Cargo.toml). Probably left over from an earlier build.

Files in cache_dir:
- `audio/`: librespot audio cache, holding both downloads and the streaming cache.
- `lyrics/<fnv64:016x>.json`: lyrics cache entries `{v:1, key, query, stored, kind, records}`. Found results are kept 30 days, misses and ambiguous results 1 day (lyrics/cache.rs:20-22).
- `lyrics/translations/<fnv64:016x>.json`: `[key, Translation]` (lyrics/translate.rs:380-410).
- `artwork/`, `updates/`, and `glass.sock` (the single-instance socket, instance.rs:42).

TOML files in theme_config::config_dir(): `theme.toml` and `config.toml` (sections 3 and 4).

====================================================================
3. THEME.TOML: src/theme_config.rs, theme.example.toml
====================================================================
Colors are "#RGB", "#RRGGBB" or "#RRGGBBAA", case-insensitive, with the "#" optional (`Rgba::parse`, theme_config.rs:30-45). Written back as uppercase "#RRGGBB", or "#RRGGBBAA" when alpha < 255.

`with_alpha(a)` sets alpha = round(clamp(a, 0, 1) × 255).

Key table, KEYS at theme_config.rs:175-221. Defaults are at :284-406.

[theme]
| key | type / range | default | what it controls |
|---|---|---|---|
| background | Color | #0F1115 | window tint |
| foreground | Color | #E9E4DC | text and icons |
| accent | Color | #4F83A3 | selection, active tab, play button, progress |
| border | Color | #090A0B | outlines and dividers |
| opacity | Number 0.0–1.0 | 0.45 | background solidity |
| blur | Int 0–100 | 28 | blur radius; fractions are rejected |

[colors] (derived from [theme] when absent; the example-file value at default settings follows each formula):
| key | default formula | example value |
|---|---|---|
| surface | foreground at alpha 0.04 | #E9E4DC0A |
| surface_raised | foreground at 0.08 | #E9E4DC14 |
| overlay | background at 0.94 | #0F1115F0 |
| text_secondary | foreground at 0.80 | #E9E4DCCC |
| text_muted | foreground at 0.62 | #E9E4DC9E |
| on_accent | foreground | |
| selection | accent at 0.16 | #4F83A329 |
| hover | foreground at 0.06 | #E9E4DC0F |
| focus | accent | |
| player_background | background at min(opacity + 0.12, 1) | #0F111591 |
| player_progress | accent | |
| player_track | foreground at 0.25 | #E9E4DC40 |

[font]
| key | type / range | default | notes |
|---|---|---|---|
| family | Text, non-empty after trim | "SF Mono" | UI font |
| size | Number 8–32 | 13.0 | `px(s) = s × size/13`, so every UI size scales with it (ui/theme.rs:14, 195-202) |
| lyrics | Text, non-empty | "SF Mono" | "system" means the platform UI sans |

[layout]
| key | range | default | used as |
|---|---|---|---|
| radius | Number 0–32 | 10 | `radius()` rounds and clamps; `small_radius` = radius × 0.6 |
| spacing | Number 2–24 | 8 | `sp(n)` = spacing × n |

[motion]
- reduce: true, false or "system". Default: follow the system setting.
- The system value is read once, when ThemeStore loads (theme_config.rs:598-600), so a system change applies after a restart.
- Effects: egui animation_time becomes 0 (instead of 0.12); album color fades become instant; lyric line motion becomes a cross-fade unless "none"; visualizer smoothing is raised to at least 0.85.

[palette]
- colors: array of Color.
- Default: [background, border, foreground, accent, #5DA9D6, #A87C9F], built from the current base colors.
- A bad entry gives the error "palette.colors[i]: …".
- Shown as accent shortcuts in the theme panel. The panel cannot set it: `set` returns "edit the palette in theme.toml".

[lyrics]
| key | type / range | default | what it does (nowplaying.rs:571-844) |
|---|---|---|---|
| current | Color | #6D3A78 | the line being sung |
| other | Color | #80D0D8 | the other lines |
| highlight | Color | accent | the word being sung; also the "pop" flash |
| size | Number 0.5–2 | 0.8 | font size = clamp(width × 0.05 × size, px 14, px 80), in 2 pt steps |
| align | left, center, right | center | line alignment |
| position | top, upper, middle | upper | current-line anchor at 0.2 / 0.38 / 0.5 of the body height |
| spacing | Number 0.5–2.5 | 1.5 | gap = size × 0.55 × spacing |
| fade | Number 0–1 | 0.59 | other-line alpha × (1 − dist × fade); dist is normalized over 0.6 of the body height |
| animation | glide, fade, pop, none | glide | how a new line arrives |
| speed | Number 0.25–3 | 0.75 | tween length = 0.55 / speed s; line emphasis takes 0.8 of that |
| bold | Bool | true | current line in bold |

[visualizer]
| key | type / range | default | what it does (ui/visualizer.rs:52-135, nowplaying.rs:41-42) |
|---|---|---|---|
| style | bars, mirror, wave | mirror | drawing style |
| color | Color | accent | gradient bottom (quiet) |
| color2 | Color | foreground at 0.9 (#E9E4DCE6) | gradient top (loud) |
| opacity | Number 0.1–1 | 0.91 | multiplies both colors |
| bars | Int 12–128 | 54 | fractions rejected |
| height | Number 0.15–0.6 | 0.26 | share of the right column, minimum px 64 |
| sensitivity | Number 0.25–4 | 1.1 | analyzer gain |
| smoothing | Number 0–0.95 | 0.65 | how slowly bars fall |
| glow | Bool | true | soft glow behind the bars |

Validation rules:
- `from_doc` collects every error and returns them joined by "\n" (theme_config.rs:239-440).
- Numbers outside their range give "key: v is out of range (lo–hi)".
- Int keys given a float give "expected a whole number between lo and hi".
- Choice keys are trimmed but case-sensitive.
- Unknown keys are only warnings: "unknown key X (ignored)" (theme_config.rs:443-467). Top-level non-table items count as unknown keys.

Live reload and the last good value (ThemeStore, theme_config.rs:578-850):
- `poll()` runs from the tick (app.rs:3227) at most every 500 ms. It compares (mtime, length).
- An empty file (length 0) gets one more check: editors truncate first, so the defaults apply only if the file is still empty with the same stamp at the next poll.
- `reload()`:
  - Missing file: defaults.
  - IO error: the error is set and the current theme is kept.
  - Parse error: the error is set, the last good theme stays, and the generation does not change.
  - Success: the error is cleared, warnings are stored, the doc is cached for previews, and the generation goes up if the theme changed.
- The window blur is re-applied whenever `theme.blur` changes (app.rs:3231-3234).
- `ui::theme::sync` reinstalls fonts only when family or lyrics changes, then the style; on the first install it discards the frame (mod.rs:51-55; theme.rs:426-449).

How unknown keys and comments survive (toml_edit 0.25):
- `set(key, raw)` (732-776) validates the kind and range. It refuses unknown keys ("unknown theme key …") and palette.colors.
- It then calls `edit()` (713-727):
  - Reads the file; if missing, starts from DEFAULT_FILE, so the first edit writes the commented example.
  - Refuses an unparsable file: "fix theme.toml first — it isn't valid TOML: …".
  - Applies the change, copying the old value's decor so inline comments survive.
  - Re-validates the whole result with `parse()` before writing.
  - Writes atomically, then reloads.
- `preview(key, raw)` (780-809) applies only to Color, Number and Int keys, in memory, without writing. Invalid values are ignored.
- `reset_table(t)` (812-822) removes every key in [t].
- `reset_panel_values()` (825-849) rewrites only the six [theme] keys to their defaults (opacity rounded to 3 decimals), keeping their decor.
- `ensure_file()` writes DEFAULT_FILE if there is no file.
- Tests: theme_config.rs:1037-1077.

Fonts (theme_config.rs:478-559; ui/theme.rs:237-359):
- Installed fonts are scanned once (OnceLock) from:
  - ~/Library/Fonts, /Library/Fonts
  - /System/Library/Fonts (recursion depth ≤ 3, so Supplemental is included)
  - /System/Applications/Utilities/Terminal.app/Contents/Resources/Fonts (where SF Mono lives)
  - ~/.local/share/fonts, ~/.fonts, /usr/share/fonts, /usr/local/share/fonts
- Only .ttf, .otf and .ttc files, skipping names that contain "italic" or "oblique".
- Matching: names are reduced to lowercase alphanumerics, and a file matches if its name starts with the family's. The rest of the name is the weight suffix.
- A bare .ttc gives regular = index 0, medium = 0, bold = 1.
- Otherwise:
  - regular: first of "regular", "", "book", "text".
  - medium: first of "medium", "semibold", "regular", "".
  - bold: first of "bold", "semibold", "heavy", "medium".
  - medium and bold fall back to regular.
- If the UI family has no regular file, the first installed of Menlo, JetBrains Mono, DejaVu Sans Mono, Noto Sans Mono, Liberation Mono, Hack is used.
- CJK fallbacks are appended: Hiragino Kaku Gothic W3, Hiragino Sans GB, AppleSDGothicNeo, Arial Unicode, NotoSansCJK (two Linux paths). egui's built-ins come last.
- Families: Proportional and Monospace both use the UI regular chain; "medium" and "bold" are named families.
- Lyrics "system": /System/Library/Fonts/SFNS.ttf variable font at wght 520 (regular) and 720 (bold); otherwise Inter, Noto Sans, Cantarell, DejaVu Sans. If none is found, the UI regular and bold are used.
- Font files are memory-mapped.

Window effects (window_fx.rs):
- `effective_opacity`:
  - 1.0 when the system's Reduce Transparency is on (checked at most once a second).
  - On Linux without GLASS_LINUX_BLUR=1: max(opacity, 0.88).
- `blur_status` strings: see section 5, Appearance row 3.

Album colors (when settings.album_colors is on; album_colors.rs:159-182):
- Blends these tokens toward the album palette: accent, focus, selection, on_accent, player_progress, player_track, player_background, lyrics.current, lyrics.highlight, lyrics.other, viz.color, viz.color2.
- Fades over 0.45 s, or instantly with reduce motion.
- The whole page gets the wash only on Now Playing (mod.rs:68-99).

Fixed colors that are not themable: `col::error` (240,140,120). `divider` = border at 45% of its alpha.

What the theme panel exposes (src/ui/themepanel.rs), opened with ⌘T or Settings > "Theme panel", closed with Esc:
- Layout: width px(350), anchored bottom-right.
- Header: "theme.toml", then the home-relative folder plus "/". A status chip reads "live preview" or "file has errors" (hover: "Glass watches theme.toml and applies changes when you save it — from here or from any editor.").
- Tabs: "theme", "lyrics", "visualizer".
- "theme" tab, under the "[theme]" header: text-field rows labelled "background", "foreground", "accent", "border", "opacity", "blur" (FIELDS, :13-14).
  - Color fields have a swatch that opens egui's color picker. The picker previews while dragging and writes once the pointer is released.
  - Fields commit on focus loss; Esc cancels.
  - An invalid value outlines the field in the accent color and shows the error message.
  - Below: "palette" swatches (hover "Use #… as the accent") that set theme.accent; a "reset" button (hover "Restore the six [theme] values to the defaults"); a link "edit theme.toml ›" (hover "Fonts, sizes, spacing and every color are in the file").
- "lyrics" tab (:313-329), under "[lyrics]":
  - Color fields: current, other, highlight.
  - Sliders: size (0.5–2), spacing (0.5–2.5), fade (0–1), speed (0.25–3). They show 2 decimals, preview while dragging, and write on drag stop or a non-drag change.
  - Chips: align, position, animation.
  - Checkbox: "bold line".
  - Text field: "font" (font.lyrics).
  - Footer: "saved as you go" and "reset" (hover "Back to the defaults for [lyrics]").
- "visualizer" tab (:331-352), under "[visualizer]":
  - A "show  V" checkbox, which writes settings.visualizer in settings.json, not theme.toml.
  - style chips; color and color2 fields.
  - Sliders: opacity (0.1–1), bars (12–128, integer), height (0.15–0.6), sensitivity (0.25–4), smoothing (0–0.95).
  - glow checkbox; footer with reset.
- Not in the panel: all of [colors], font.family, font.size, [layout], [motion], [palette] (the palette is display-only).

====================================================================
4. CONFIG.TOML: src/user_config.rs, config.example.toml
====================================================================
Path: `theme_config::config_dir()/config.toml` (user_config.rs:167-169).

The KEYS whitelist is at :65-78 and the checks at :80-109. Defaults (:31-57) match config.example.toml (tested at :289-294).

[lyrics]
- provider: "auto" | "lrclib" | "spotify". Default "auto". Parsed trimmed and case-insensitive (lyrics/service.rs:30-33).
  - Plan (service.rs:51-53): auto with fallback = [LRCLIB, Spotify]; auto without fallback = [LRCLIB]; lrclib = [LRCLIB]; spotify = [Spotify].
- fallback: bool, default true. Strict boolean.
- offset_ms: integer with |n| ≤ OFFSET_LIMIT 10000. Default 0. Floats are rejected.
  - Positive shows lyrics earlier.
  - Total offset = offset_ms + the per-song offset from lyrics_choices (app.rs:1949-1951).
  - Actions clamp to ±10000 (app.rs:2746-2749).
- contact: string, at most 200 characters, no control characters. Default "". Trimmed. Sent in the LRCLIB User-Agent.
- translation: "off" | "below" | "only". Default "off". Aliases "original", "both", "translated". Trimmed, case-sensitive (translate.rs:39-46).
- translate_to: "system", or 2–3 ASCII letters (trimmed). Default "system". Lowercased when used (`target()`, translate.rs:128-134). The system language is read once.
- translator: "auto" | "apple" | "assistant". Default "auto". Trimmed, case-sensitive.

[assistant]
- enabled: bool, default true.
- billing: exactly "chatgpt" or "api_key". Default "chatgpt".
- model: at most 100 bytes of [A-Za-z0-9._-:]. "" is allowed. Default "".
- welcomed: bool, default false. Set once after the welcome modal (assistant_app.rs:1349-1352).
- spotify: bool, default true.

No other sections exist. Unknown keys or tables give the warning "unknown key X (ignored)". All errors are collected and joined by "\n".

ConfigStore (user_config.rs:152-275) behaves the same way as ThemeStore:
- 500 ms poll, the empty-file double check, the last good config kept on error (test :327-342).
- `set()` whitelists the key, checks the value, starts from DEFAULT_FILE if the file is missing (the first edit writes the commented example), refuses an unparsable file ("fix config.toml first — it isn't valid TOML: …"), preserves decor, re-validates, writes atomically and reloads.
- In-app callers go through `App::set_config`, which shows a toast on error (app.rs:2047-2051).
- A changed config triggers `refresh_lyrics_client` (app.rs:3235-3238).

====================================================================
5. SETTINGS PAGE, IN ORDER: src/ui/settings.rs
====================================================================
Page layout:
- Title "Settings" at 32 pt. Search field: hint "Search in Settings", width 220. Content width at most 900; margins 32/32/24/48.
- Each row (:57-74): label at 14 pt, note at 12 pt in the secondary color, control on the right, minimum height 36.
- Toggles are 40×22 pills with an 8 pt knob.
- Constants: RESTART = "Applies when the playback engine restarts."; LIVE = "" (no note).
- Controls in a right-to-left layout appear in reverse of code order. Button orders below are as shown, left to right.

Account
1. "Signed in as". Only shown once /me has loaded.
   - Note: display_name · email · country (empty parts dropped).
   - Control: a label in the accent color: "Premium", "Free" (for free or open), or the raw product value.
2. "Edit login methods". Note: "Opens your Spotify account page in the browser."
   - Button "Edit" opens https://www.spotify.com/account/overview/
3. "Log out". Note: "Forgets the session stored on this computer." Button "Log out".

Appearance
1. "Theme file". Note: the full theme.toml path.
   - Buttons: [Theme panel] [Edit in editor] [Reload]. Reload's hover: "Glass also reloads it by itself when the file changes".
   - Edit in editor creates the commented file first if needed.
2. "Theme status". The note is one of:
   - "Not applied: Glass keeps the last good theme until this is fixed.\n{error}"
   - "Applied. Edits show up as soon as the file is saved."
   - the warnings, joined by line breaks.
3. "Background effect". The note is one of:
   - "Native background blur (macOS WindowServer)"
   - "macOS vibrancy (fixed blur; the blur radius has no effect)"
   - "Transparency; blur is left to your compositor"
   - "Transparency only (no blur available)"
   - "Not started"
4. "Font". Note: "{family} · {size} pt (set in theme.toml)".
5. "Reduce motion". Note: "On|Off. Follows the system setting; [motion] reduce in theme.toml overrides it."

Assistant
The whole block appears if the search matches "Assistant ChatGPT OpenAI API key model" (settings.rs:183). Its contents are in ui/assistant.rs:408-572:
- Checkbox "Turn on the assistant" (assistant.enabled). When it is off, only the data note shows (`ai::data_note`, ai/mod.rs:62-68).
- Checkbox "Let it use my Spotify library and playback" (assistant.spotify), followed by explanatory text and the data note.
- "Pay for requests with" selectable labels: "my ChatGPT plan" / "my OpenAI API key" (assistant.billing).
- ChatGPT plan path:
  - Note "Works with ChatGPT Plus and Pro. …".
  - "Continue with ChatGPT", or "Waiting for the browser…" with [Cancel].
  - Saved accounts with "Continue with this account".
  - When signed in: the account label plus "· ChatGPT plan usage enabled" or "· plan usage not enabled"; buttons [Enable ChatGPT plan use] [Manage usage] [Sign out] [Add another account]; "Also saved: …" rows with [Switch].
- API key path:
  - Accent note "API usage is billed by OpenAI…".
  - A password field (hint "sk-…") with [Save key], or "A key is saved in {store}." with [Remove key].
- "Model" combo (assistant.model). It can also show "loading…", an error, "available once a key is saved" or "available after signing in".
  - Note: "GPT-6 Luna is chosen when your account offers it. Glass keeps your choice and never switches to a pricier model by itself."

Lyrics
1. "Lyrics source". The note depends on the setting:
   - Auto with fallback: "Auto: LRCLIB first, then Spotify's lyrics for Spotify songs LRCLIB doesn't have."
   - Auto without fallback: "Auto: LRCLIB only, since fallback is off."
   - "LRCLIB only."
   - "Spotify only: Spotify's lyrics for songs played from Spotify."
   - Chips: [Auto] [LRCLIB] [Spotify]. The last reads "Spotify · not connected" when there is no Spotify connection.
2. "Spotify lyrics". Only shown when the provider is spotify. Note: "Spotify's own lyrics, read through your Spotify connection. Only for songs played from Spotify."
3. "Translation". Note: "Show the original, the original with a translation under each line, or the translation alone. T in the lyrics view switches too."
   - Chips: [original] [original + translation] [translated].
4. "Translate into". Only when translation ≠ off.
   - Note: "{Language name}. "system" follows your system language; or type a code such as en, es or ja."
   - Text field, width 90, committed on focus loss.
5. "Translator". Only when translation ≠ off. Chips: [Auto] [Apple on-device] [Assistant]. The Apple chip is hidden when Apple translation isn't available. The note is one of:
   - "Auto: Spotify's own translation when it has one, then Apple's on-device translation (free, private), then the assistant if it's on."
   - "Auto: Spotify's own translation when it has one, then the assistant if it's on."
   - "Apple's on-device translation: free and private; nothing leaves this Mac. Languages are downloaded in System Settings → General → Language & Region → Translation Languages."
   - "Apple's on-device translation isn't available here (it needs macOS 26 and a release build): choose auto or assistant."
   - "The assistant's model: works on any system and with any language; sends the lyrics to OpenAI and counts toward your plan or key."
6. "Timing offset (all songs)".
   - Note: "{0.0 s | ±x.x s}. Positive shows lyrics earlier, negative later (Bluetooth speakers often need a little negative). To fix one song, use [ and ] on the now-playing screen; Glass keeps that song's timing."
   - Buttons: [−0.1 s] [+0.1 s] [Reset]. Reset only shows when the offset ≠ 0.
7. "Settings file". Note: the config.toml path. Buttons: [Edit in editor] [Reload].
8. "Settings file status". Only shown when there is an error or warnings.
   - Error: "Not applied: Glass keeps the last good settings until this is fixed.\n{error}". Otherwise the warnings.
9. "Saved lyrics".
   - Note: "{N song(s)} · {KB} KB. Matches are kept 30 days and misses a day. Clearing also removes saved translations; lyrics you picked by hand stay."
   - Button [Clear], only when the cache is not empty. The usage figure is refreshed every 5 s.
10. "About LRCLIB". Long informational note; no control.

Downloads
1. "Downloaded". Note: "{n} downloaded · {MB} MB[ · {left} still downloading]".
   - Button [Remove all downloads], only when n > 0. It opens a confirm dialog: title "Remove all downloads?", body "Downloaded songs will stream again when you play them.", button "Remove".
2. "Couldn't download". Only when failed > 0.
   - Note: "1 song couldn't be downloaded" or "{n} songs couldn't be downloaded", followed by ". Glass tries again the next time it starts."
3. "How downloads work". Long note: encrypted cache, quality, and so on.

Save songs
1. "Format". Long note about JioSaavn/YouTube Music sources and M4A vs MP3. Combo: M4A / MP3.
2. "MP3 bitrate". Only when the format is MP3. Note: "Higher keeps more of the source's quality." Combo: 128 / 160 / 192 / 256 / 320 kbps.
3. "Folder". Note: "Songs are saved to {dir}." Buttons: [Change…] [Reset]. Reset only shows when a custom folder is set.
4. "Show saved songs in Local Files". Only when show_local_files is on. Two forms:
   - Read-only note "Already in Local Files (it's inside {parent})." when an existing source already contains the folder.
   - Otherwise a toggle with note "Adds the folder above to Local Files."

Your Library
1. "Use compact library layout". Toggle between Compact and List.
2. "Show Local Files". Toggle.
3. "Share with Glass on your other computers". Toggle.
   - Note: "Send songs to Glass on your other computers on this network (signed in to this account), and get playlists' missing local songs from them. Received songs go to {inbox}. The first time two computers connect, both show a code to check before anything is shared."
4. "Paired with". One row per paired computer, only while sharing is running. Note: the computer's name. Button [Forget] (hover: "It has to be paired again (with a code) to share").
5. "Show songs from". One row per local folder, only when show_local_files is on. Note: the path. Button [Remove].
6. "Add a source". Only when show_local_files is on.
   - Note: "Pick a folder of music files (MP3, M4A, AAC, FLAC, Ogg, WAV or AIFF)."
   - Button [Add a source]. A redundant folder shows the toast "That folder is already in Local Files".

Display
1. "Open the now-playing screen when playback starts". Toggle.
2. "Show desktop notifications when the song changes". Note: "Uses your system's notifications." Toggle.
3. "Match Now Playing to the album's colors". Note: "Now Playing, its lyrics and visualizer, and the player bar take their colors from the cover of what's playing." Toggle.

Playback
1. "Gapless playback". RESTART note. Toggle.
2. "Autoplay — keep playing similar content when your music ends". RESTART note. Toggle.
3. "DJ voice — X talks between the DJ's songs". Toggle.
4. "Mono audio — left and right speakers play the same audio". Note: "Applies instantly." Toggle.
5. "Normalize volume — same volume level for all songs and podcasts". RESTART note. Toggle.
6. "Volume level". Only when normalize is on. Note: "Loud may reduce audio quality. Applies when the playback engine restarts." Combo: Quiet / Normal / Loud.
7. Equalizer card. Shown when the search matches "Equalizer" (settings.rs:632-709).
   - Header: "Equalizer" with a toggle (eq_enabled); on the right, "Presets", the preset combo, and [Reset].
   - Reset sets all gains to 0 and the preset to "Flat"; it does not change eq_enabled.
   - The combo lists the 22 presets and then "Manual" (display only). Choosing a preset sets its gains and turns the EQ on.
   - Note: "Applies instantly to music played in Glass."
   - Graph: 180 pt tall, lines at -12 / 0 / +12 dB, a Catmull-Rom curve and 6 draggable handles. A drag sets the gain rounded to 0.1 dB, the preset to "Manual" and turns the EQ on. A tooltip shows "+x.xdB". Band labels sit under the handles.
   - The curve is muted when the EQ is disabled.

Audio quality
1. "Streaming quality". Note: "Also the quality of downloads. Applies when the playback engine restarts." Combo with the 3 quality labels.

Explicit content
1. "Allow explicit content". Note: "When off, explicit songs are greyed out and skipped." Toggle.

Storage
1. "Cache". Note: "Audio kept for faster playback; downloads aren't touched. {cache/audio path}".
   - Button [Clear cache]. Toasts: "Cleared the cache (downloads kept)", or "The playback engine is still starting; try again in a moment".
2. "Cache size limit (MB)". Note: "Glass trims the cache to this every 10 minutes." DragValue, range 256..=50000, speed 32.

Startup and window behavior
1. "Open Glass automatically after you log into the computer".
   - Note on macOS: "Writes ~/Library/LaunchAgents/app.glass.player.plist." Otherwise: "Writes ~/.config/autostart/glass.desktop."
   - Combo: No / Yes / Minimized.
2. "Close button should minimize the window". Toggle.

Proxy
1. "HTTP proxy". Note: "For example http://host:8080. Applies after restarting Glass." Text field, hint "No proxy", width 240.

Spotify Connect
1. "Device name". Note: "How this computer appears in other Spotify apps. Applies when the playback engine restarts." Text field, width 200.
2. "Playback engine". Button [Restart engine]. The note is the engine status, one of:
   - "Not started"
   - "Connecting…"
   - "Ready · this computer appears as “{name}”"
   - "Playback engine failed: {error}. Trying again in {n} s."
   - "Disconnected; reconnecting…"
3. "Client ID". Note: "The OAuth client Glass signs in with. Changing it requires logging in again." Monospace text field, width 260.

Updates
1. "Version". Note: "Glass {VERSION}", plus " (a development build: it doesn't update itself)" on non-release builds.
2. "Update automatically". Toggle.
   - Note on macOS: "Download new versions in the background and install them when Glass quits." Otherwise: "Check for new versions in the background (to update, build the new package)."
3. "Check for updates". The note is the status, one of:
   - "Checks shortly after Glass starts, then every 6 hours."
   - "Not checked yet."
   - "Checking…"
   - "Glass is up to date."
   - "Glass {v} is downloaded: it installs when you quit, or restart now."
   - "Glass {v} is out: build the new package to update." (Linux)
   - "Glass {v} is out: get it from the releases page (this copy can't install it itself)."
   - "Updated: the new version opens next time."
   - "Couldn't check: {error}"
   - Button: [Restart to update] when an update is ready; otherwise [Check now], disabled while checking.
4. "What's new in {version}". Only when there are release notes. Note: the notes.

Disabled or conditional states: the page never greys out a control; it hides it instead. Hidden cases: Volume level, MP3 bitrate, Translate into/Translator, Spotify lyrics, local-folder rows, the Show-saved-songs row, Reset buttons, Remove all, Clear, Paired rows, Couldn't download, What's new, the Apple chip, the assistant details when it is off, and the "Spotify · not connected" label. Read-only explanation rows: Theme status, Background effect, Font, Reduce motion, How downloads work, About LRCLIB, Version, and "Already in Local Files (it's inside …)". The only disabled control is "Check now" while a check is running.

Search behaviour (settings.rs:11-54):
- The query is trimmed and lowercased. A row shows if its label or its note contains the query as a substring (case-insensitive), so dynamic notes such as paths and status text are searchable too.
- If a section title matches, the whole section shows. Otherwise the section heading appears just above its first matching row.
- The assistant block and the Equalizer card match on the keyword strings given above.
- There is no "no results" message.
- ⌘F or "/" (when not typing in a field) focuses the search field (mod.rs:653-659).
- Other screens deep-link by setting the query to "assistant" (nowplaying.rs:452; assistant.rs:88).
- The query is not cleared on navigation; it lasts for the session.

====================================================================
6. EQ PRESETS AND THE DSP: src/config.rs:445-474, src/dsp.rs
====================================================================
Gains in dB for [60, 150, 400, 1k, 2.4k, 15k]. The names match Spotify's; the curves are Glass's own.

| Preset | 60 | 150 | 400 | 1k | 2.4k | 15k |
|---|---|---|---|---|---|---|
| Flat | 0 | 0 | 0 | 0 | 0 | 0 |
| Acoustic | 4.5 | 3.5 | 1.0 | 1.5 | 3.0 | 3.5 |
| Bass booster | 5.5 | 4.0 | 1.5 | 0 | 0 | 0 |
| Bass reducer | -5.5 | -4.0 | -1.5 | 0 | 0 | 0 |
| Classical | 4.5 | 3.0 | -1.5 | -1.5 | 2.5 | 4.0 |
| Dance | 4.0 | 6.0 | 2.0 | 0 | 3.5 | 2.5 |
| Deep | 4.5 | 3.0 | 1.5 | 0.5 | -2.5 | -4.5 |
| Electronic | 4.0 | 3.5 | 0.5 | -1.5 | 2.0 | 4.5 |
| HipHop | 5.0 | 4.0 | 1.0 | -1.0 | 1.5 | 3.0 |
| Jazz | 4.0 | 2.5 | -1.5 | 1.5 | 2.5 | 3.5 |
| Latin | 4.5 | 3.0 | 0 | 0 | 2.0 | 4.5 |
| Loudness | 6.0 | 4.0 | 0 | -2.0 | 1.5 | 4.5 |
| Lounge | -3.0 | -1.5 | 2.0 | 1.5 | 0 | -1.5 |
| Piano | 3.0 | 2.0 | 0 | 2.5 | 3.0 | 3.5 |
| Pop | -1.5 | 1.0 | 4.0 | 4.5 | 2.0 | -1.5 |
| RnB | 3.0 | 7.0 | 3.0 | -1.5 | 2.5 | 3.0 |
| Rock | 5.0 | 4.0 | -0.5 | 0 | 2.5 | 4.0 |
| Small speakers | 5.5 | 4.0 | 2.5 | 1.5 | -1.0 | -4.0 |
| Spoken word | -3.5 | -0.5 | 2.5 | 3.5 | 3.0 | 1.0 |
| Treble booster | 0 | 0 | 0 | 1.5 | 3.5 | 5.5 |
| Treble reducer | 0 | 0 | 0 | -1.5 | -3.5 | -5.5 |
| Vocal booster | -1.5 | -3.0 | 2.5 | 4.0 | 3.0 | 1.0 |

Filters (dsp.rs:17-20, 79-87):
- Sample rate fixed at 44,100 Hz (local files are resampled to it), stereo, f64.
- biquad 0.6 DirectForm2Transposed, one filter per band per channel. The formulas are the standard RBJ cookbook ones, with A = 10^(dB/40).
- Band 0 (60 Hz) is a LowShelf. Bands 1–4 (150, 400, 1000, 2400 Hz) are PeakingEQ. Band 5 (15 kHz) is a HighShelf.
- Q = 1.0 for every band, shelves included.
- If coefficient creation fails, a PeakingEQ of 0 dB at 1 kHz is used instead.

Headroom and limiter:
- Preamp (dsp.rs:131-133): when the EQ is enabled, preamp = 10^(-(max(0, largest gain)) × 0.25 / 20). That is -0.25 dB per dB of the largest boost, so +12 dB gives -3 dB. Otherwise 1.0.
- Soft limiter (dsp.rs:167-172), applied per sample only while the EQ runs: |x| ≤ 0.8 passes unchanged; above that, y = sign(x) × (0.8 + 0.2 × tanh((|x| − 0.8) / 0.2)).

Processing (dsp.rs:109-165, 252-271):
- The EQ runs only if it is enabled and some |gain| > 0.01.
- When the EQ starts running again, the filter state is reset. A gain change only updates coefficients, keeping state, so dragging a band does not click.
- Mono = (L + R)/2, applied before the EQ.
- With the EQ disabled and mono off, the signal passes bit-exact (test at :346-351).
- Parameters reach the audio thread through a Mutex plus an AtomicU64 version, sent only when they change (app.rs:3245-3249).

====================================================================
7. NORMALIZATION, QUALITY, GAPLESS, AUTOPLAY, MONO, EXPLICIT: WHERE APPLIED
====================================================================
Signal path on the engine:
1. librespot decodes.
2. Normalization, when on.
3. Software volume (librespot mixer, VolumeCtrl::Log with a 60 dB range).
4. DspSink: mono, then preamp, then the 6 biquads, then the soft limiter.
5. rodio output.
The order is in vendor/librespot-playback/src/player.rs:1766-1800 and dsp.rs:1-2.

Engine::start_with (engine.rs:109-193) reads at engine start:
- proxy (an invalid one is a start error), client_id, device_id, autoplay
- streaming_quality as the bitrate, gapless, normalize, the loudness pregain, local_dirs
- device_name, volume
- position_update_interval = None

Normalization:
- Vendored defaults (vendor/librespot-playback/src/config.rs:129-148): type Auto, method Dynamic, threshold -2 dBFS, attack 5 ms, release 100 ms, knee 5 dB, triangular dither.
- Glass sets only `normalisation` and `normalisation_pregain_db`.

Applies on engine restart: quality, gapless, autoplay, normalize, loudness and device name. That happens through "Restart engine" (Action::RestartEngine, app.rs:2616-2620) or an automatic reconnect.

Applies instantly:
- mono and the EQ, through DspParams.
- dj_voice.
- allow_explicit (UI only):
  - Track rows become unplayable or greyed when `!allow_explicit && explicit` (tracklist.rs:252).
  - `maybe_skip_explicit` (app.rs:3955-3966): if the playing item is explicit and its position is past 300 ms, it skips once per URI, shows the toast "Skipped explicit content" and runs Action::Next.
  - Nothing is configured in librespot, and the account's own explicit filter is never read.

Downloads follow streaming_quality: `Downloads::run(session, quality)` (app.rs:3598), with the format order at downloads.rs:200-208.

Not affected by the EQ, mono or normalization: the Local Files page player (rodio, local.rs:547) and the DJ voice player.
