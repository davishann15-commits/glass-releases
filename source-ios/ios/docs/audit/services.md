<!-- Read-only inventory of the desktop source (glass-player b403e2a, 1.0.0-beta.22), made for the iOS port. Paths are relative to the repository root. -->

GLASS DESKTOP SERVICE LAYER: INVENTORY FOR THE iOS PORT
(All paths are under . "R/" means ~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/. Line numbers are file:line.)

Five things that shape iOS feasibility:
- Sign-in uses a loopback HTTP listener. The OAuth state parameter is generated but never checked on the callback.
- The fallback that recovers GraphQL hashes needs a local Spotify.app, so it cannot run on iOS.
- Device-change following polls the device name once a second. There are no CoreAudio listeners.
- Sharing uses mdns-sd, which opens its own UDP multicast sockets on port 5353.
- LAME's build script has no iOS branch, so an iOS target won't build as-is.

==================================================================
1. AUTH
==================================================================
- Client ID: config.rs:19 DEFAULT_CLIENT_ID = "65b708073fc0480ea92a077233ca87bd" (Spotify's desktop ID, librespot's default). It can be overridden through Settings.client_id (config.rs:267, 342). The same ID is passed to the engine session (engine.rs:111). The data session (Sp) uses SessionConfig::default(), i.e. librespot's default client ID (sp/mod.rs:253).
- Redirect: auth.rs:15 REDIRECT_URI = "http://127.0.0.1:8898/login". librespot-oauth 0.8 binds a one-shot TCP listener on that socket address (R/librespot-oauth-0.8.0/src/lib.rs ~170-210). It opens the browser with open::that_in_background, and the success page is DONE_PAGE (auth.rs:39-41).
- Scopes: auth.rs:18-37. streaming, user-read-playback-state, user-modify-playback-state, user-read-currently-playing, user-read-recently-played, user-read-playback-position, user-top-read, user-library-read, user-library-modify, user-follow-read, user-follow-modify, playlist-read-private, playlist-read-collaborative, playlist-modify-private, playlist-modify-public, ugc-image-upload, user-read-private, user-read-email.
- PKCE and state: both handled by librespot-oauth (oauth2 crate).
  - PkceCodeChallenge::new_random_sha256 + CsrfToken::new_random (oauth lib.rs ~237-245, 455-465).
  - Endpoints: accounts.spotify.com/authorize and /api/token.
  - The callback extracts only `code`. The CSRF state is NOT verified.
- Cancel: auth.rs:58-65 cancel_sign_in connects to 127.0.0.1:8898 and writes "GET /login?error=cancelled" to free the port. app.rs:2919-2923 bumps signin_gen so late results are ignored.
- Token storage:
  - Refresh token: config_dir/session.json {refresh_token}, written with write_private (0600, atomic) (auth.rs:43-84; config.rs:107-125).
  - Access token: memory only (auth.rs:87-90).
  - config_dir = directories ProjectDirs("", "", "Glass") (config.rs:21-23), made 0700 (config.rs:82-91).
- Refresh:
  - Auth::bearer refreshes when the token expires within 60 s (auth.rs:113-119).
  - refresh_locked: if the refresh fails, it retries with a different refresh token found in session.json (another instance may have rotated it). It keeps the old refresh token when the response omits one (auth.rs:121-137).
  - Startup: Auth::resume (auth.rs:102-110), run from app.rs:1047-1054.
  - Resume retry backoff: [2, 5, 15, 30] s, up to 8 tries, unless the error is "invalid_grant", "invalid_client", etc. (app.rs:3511-3526).
- librespot credentials:
  - Both sessions use Credentials::with_access_token(OAuth bearer): Engine at engine.rs:106 (token from app.rs:3159); Sp at sp/mod.rs:236-238.
  - The engine Session has a Cache: credentials and volume in config_dir, audio in cache_dir/audio, no size limit (engine.rs:114). librespot therefore writes reusable credentials.json into config_dir, which Glass chmods 0600 (engine.rs:184-191).
  - The app never reads credentials.json back. Only the ignored tests do (cache.credentials()).
- Premium check:
  - /me "product" = session.user_data().attributes["type"], defaulting to "premium" (sp/mod.rs:641-642).
  - App::is_premium (app.rs:1086-1088). The now-bar shows "playback needs spotify premium" (ui/nowbar.rs:246).
  - A 403 on a player command becomes a "Premium is required" toast (app.rs:3645-3646).
  - There is no hard gate.
- Sign-out: Action::LogOut (app.rs:2924-2951).
  - auth::forget_session deletes session.json (auth.rs:71-73).
  - Drops share, engine (Drop → spirc.shutdown, engine.rs:251-255) and api (Sp Drop → session.shutdown, sp/mod.rs:227-231), then clears state.
  - NOT removed: credentials.json, device_id, data_device_id, downloads (gap to note).
- iOS needs:
  - An ASWebAuthenticationSession flow. The loopback listener only works while the app is in the foreground (needs verification), and a custom-scheme redirect would need a client ID that registers it.
  - Keychain for the refresh token instead of a 0600 file.
  - A decision on credentials.json (cache it in the Keychain or drop it).

==================================================================
2. REMOTE ENDPOINTS
==================================================================
There is no public Web API (api.spotify.com) traffic. api.rs:19 BASE is only stripped from paths (api.rs:179; sp/mod.rs:212). Every Web-API-shaped path is answered in-process by Sp::handle / handle_shared (sp/mod.rs:493-632).
- Api layer: CONNECT_TIMEOUT 25 s, REQUEST_TIMEOUT 60 s (4x for /glass/*) (api.rs:21-23, 184). NetLog keeps the last 500 requests (api.rs:34-69).

Transport base:
- spclient: host from apresolve.spotify.com/?type=accesspoint&type=dealer&type=spclient (vendor/librespot-core/src/apresolve.rs:88). Auth via login5.spotify.com/v3/login (login5.rs:67). Client token from clienttoken.spotify.com/v1/clienttoken (spclient.rs:151).
- AP TCP + dealer websocket come from librespot.
- Sp has its own reqwest client: 20 s timeout, proxy aware (sp/mod.rs:255-258).

A) spclient REST (session.spclient()):
- /me → GET /user-profile-view/v3/profile/{user}?playlist_limit=0&artist_limit=0 plus AP user_data and country (sp/mod.rs:636-658; vendor/librespot-core/src/spclient.rs:675-698).
- /users/{id} → same profile call (660-667).
- /users/{id}/playlists → profile ?playlist_limit=500 (~300 actually returned) (669-690).
- /me/playlists → GET /playlist/v2/user/{user}/rootlist?decorate=revision,attributes,length,owner,capabilities,status_code&from=0&length=1000 (sp/library.rs:233-237).
  - Capped at 1000 items, no paging. Folders are skipped (Glass keeps its own folders, config.rs:249-256).
  - Mosaics: GET /playlist/v2/playlist/{id}?from=0&length=16, 8 concurrent (library.rs:242-269), producing mosaic.scdn.co/640/{4 ids} or i.scdn.co/image/{id} (505-519).
- /playlists/{id} and /playlists/{id}/tracks|items → GET /playlist/v2/playlist/{id}?from=N&length=1000, paged (library.rs:285-314, 371-384). Cached for 30 s (LIST_TTL, library.rs:22).
- Playlist writes: POST /playlist/v2/playlist/{id}/changes (ListChanges, base_revision taken from ?from=0&length=1) (library.rs:388-409).
  - POST /playlists/{id}/tracks → ADD add_last (422-425).
  - DELETE → REM items_as_key (427-429).
  - PUT {range_start, insert_before} → MOV length 1 (431-440).
  - PUT /playlists/{id} → UPDATE_LIST_ATTRIBUTES name/description/collaborative (442-461).
- Create playlist: POST /playlist/v2/playlist (ListUpdateRequest → CreateListReply), then a rootlist ADD first (464-475).
- Follow/unfollow (also used to delete own playlists): POST /playlist/v2/user/{user}/rootlist/changes (411-420). Routes: /playlists/{id}/followers PUT/DELETE and /me/library?uris=spotify:playlist:…
- Not available (errors out): /playlists/{id}/images and following users (sp/mod.rs:589, 608).
- Collection sets: POST /collection/v2/paging and /collection/v2/write. Content-type/accept are application/vnd.collection-v2.spotify.proto, hand-rolled protobuf (sp/pb.rs).
  - paging: 1 user, 2 set, 3 token, 4 limit 300 (library.rs:69-106).
  - write: 1 user, 2 set, 3 items{1 uri, 2 secs, 3 removed}, 4 hex update id (117-143).
  - Sets: "collection" (tracks + albums + local), "artist", "show", "listenlater". Cache TTL 90 s.
  - Routes:
    - GET: /me/tracks, /me/albums, /me/episodes, /me/following, /me/shows, /me/audiobooks, /me/{k}/contains, /me/library/contains, /playlists/{id}/followers/contains (sp/mod.rs:521-541, 694-729).
    - Writes: PUT/DELETE /me/tracks|albums|shows|audiobooks|episodes, /me/following?type=artist, /me/library?uris= (584-601).
- Metadata: POST /extended-metadata/v0/extended-metadata (BatchedEntityRequest, header country + catalogue "premium", TRACK_V4/EPISODE_V4/ALBUM_V4/ARTIST_V4/SHOW_V4). 200 URIs per batch, 4 concurrent (sp/mod.rs:310-351; spclient.rs:580-587).
  - Routes: /tracks/{id}, /episodes/{id}, /albums/{id}[/tracks], /shows|audiobooks/{id}, /shows/{id}/episodes, /audiobooks/{id}/chapters, /artists/{id}/albums (all releases incl. appears_on, newest first) (549-558, 825-864).
  - Metadata cache: META_LIMIT 4000, idle eviction 600 s (67-71, 433-444).
- /me/player/recently-played → GET /recently-played/v3/user/{user}/recently-played?format=json&offset=0&limit=50&filter=default,collection-new-episodes (sp/mod.rs:731-740).
- /glass/lyrics/{id} → GET /color-lyrics/v2/track/{id} (sp/mod.rs:563-570; spclient.rs:647-648).
- /glass/library-songs (assistant): rootlist + every non-Spotify-owned playlist (max 400, 6 concurrent) + Liked Songs, compact rows, max 40,000 (library.rs:321-369).

B) Connect state (sp/player.rs):
- Dealer subscriptions: hm://pusher/v1/connections/ (Spotify-Connection-Id) and hm://connect-state/v1/cluster (ClusterUpdate) (player.rs:45-84).
- Observer registration: PUT /connect-state/v1/devices/hobs_{first 34 hex of data_device_id}. PutStateRequest CONNECT_STATE / NEW_DEVICE, name "Glass", COMPUTER, hidden + is_observable, header X-Spotify-Connection-Id. 3 tries with 2 s / 4 s backoff (player.rs:56-65, 104-123).
- GETs read from the cluster:
  - /me/player → null when no active device; position extrapolated (143-169).
  - /me/player/queue → next_tracks without delimiter/meta entries, max 40, 503 if no cluster yet (171-181).
  - /me/player/devices → excludes the observer and hidden devices (183-188).
- Commands: POST /connect-state/v1/player/command/from/{observer}/to/{device}, JSON {command:{endpoint,…, logging_params}} (190-198).
  - Endpoints: resume, pause, skip_next, skip_prev, seek_to, set_shuffling_context, set_options (repeat), add_to_queue (provider "queue"), play.
  - play_command maps context_uri / uris / offset / position_ms to context/pages/skip_to/seek_to, play_origin feature_identifier "glass" (262-285).
- Transfer: PUT /me/player → POST /connect-state/v1/connect/transfer/from/{from}/to/{dest} {transfer_options:{restore_paused}} (212-219).
- Volume: PUT /me/player/volume → PUT /connect-state/v1/connect/volume/from/{observer}/to/{dev} {volume: pct*65535/100} (242-249).

C) Pathfinder GraphQL: POST https://api-partner.spotify.com/pathfinder/v2/query (sp/mod.rs:59, 474-486).
- Body: {variables, operationName, extensions.persistedQuery{version:1, sha256Hash}}.
- Headers: Bearer login5 token, client-token, "app-platform: OSX".
- Operations and the routes that use them:
  - searchDesktop → /search?q&type (types mapped to tracksV2, artists, albumsV2, playlists, podcasts, episodes, audiobooks) (866-895).
  - userTopContent → /me/top/{artists|tracks} (time_range ignored, always SHORT_TERM) (742-748).
  - queryArtistOverview → /artists/{id}, /artists/{id}/top-tracks, /artists/{id}/related-artists (trimmed, OVERVIEW_TTL 30 s) (62, 767-821).
  - browseAll → /browse/categories (897-917).
  - browsePage → /browse/categories/{id}/playlists (919-934).
- Hash sources, in order:
  1. Built-in QUERIES from Spotify.app 1.3.4 (sp/mod.rs:81-87).
  2. Overridden by config_dir/queries.json (113-124).
- Recovery when an error contains "PersistedQuery" (446-472):
  1. Retry with the built-in hash if the current one differs.
  2. Otherwise rescan_queries: spawns `unzip -p /Applications/Spotify.app/Contents/Resources/Apps/xpui.spa *.js` (XPUI const at 75; 144-147). Searches for `"<name>","query|mutation","<64 hex>"` (127-141), writes queries.json (150-156), retries.
  3. Non-macOS error text: "a Glass update will follow it" (461-465). ui/pages.rs:1839 hides queries.json off macOS.
- iOS: no Spotify.app and no process spawning, so only built-in or server-pushed hashes are possible (needs a remotely updatable hash list).

D) DJ: lexicon session hm://lexicon-session-provider/context-resolve/v2/session?contextUri=… (vendor/librespot-connect/GLASS.md:8-29). TTS: POST {spclient base}/client-tts/v1/fulfill, then GET of the returned clip URL (dj.rs:185-209).

E) librespot internals:
- /context-resolve/v1/{uri} (spclient.rs:880), /radio-apollo/v3/… for autoplay/radio (743), audio key via AP.
- CdnUrl::resolve_audio storage-resolve, then CDN GET (downloads.rs:525-535).
- Images: i.scdn.co, mosaic.scdn.co, lexicon-assets.spotifycdn.com (dj.rs:20).

F) Non-Spotify hosts: github.com (update feed), jiosaavn.com, music.youtube.com / www.youtube.com plus their CDNs, lrclib.net, OpenAI (list at ui/pages.rs:1794-1804).

==================================================================
3. ENGINE (engine.rs)
==================================================================
- Session:
  - SessionConfig{client_id, device_id, proxy, autoplay: Some(settings.autoplay) (default true)} (111).
  - device_id: config_dir/device_id, 40 random hex, persisted (90-102).
  - Cache: credentials and volume in config_dir, files in cache_dir/audio, no limit (112-114, 86-88).
- PlayerConfig (118-133):
  - Bitrate: Normal 96 / High 160 (default) / VeryHigh 320 (config.rs:189-207, 344).
  - gapless (default true); normalisation (default true).
  - normalisation_pregain_db: Quiet -9 / Normal 0 / Loud +3 (config.rs:216-225).
  - local_file_directories = settings.local_dirs (default Music + Downloads, config.rs:338-357).
  - position_update_interval None. Everything else is librespot default.
- Mixer: mixer::find(None) (softvol) + MixerConfig::default. The player gets mixer.get_soft_volume(). The sink factory builds DspSink(try_open_default), so the output is opened lazily (135-138).
- Spirc:
  - ConnectConfig{name: settings.device_name (default "Glass"), DeviceType::Computer, initial_volume = settings.volume (default 0.7) * u16::MAX} (180-182).
  - Spirc::new(…, credentials, player, mixer); the task is spawned (182-183).
  - The vendored connect reports supports_dj (GLASS.md:39-40).
- Event stream:
  - player.get_player_event_channel() is mapped to EngineEvent: Playing, Paused, Position (PositionChanged/Correction/Seeked), Loading, Track(item_json), Stopped, EndOfTrack, Unavailable, Volume, Shuffle, Repeat, SessionLost (154-178).
  - The DJ hook emits EngineEvent::Dj (142-153).
  - on_event sends Msg::Engine plus ctx.request_repaint (app.rs:3160-3163). Handled in app.rs:3831-3925 (most events apply only when is_local()).
- Routing:
  - is_local() = engine device == active Connect device, or no active device (app.rs:1554-1560).
  - Local: Engine::load / spirc.* directly (app.rs:2114-2138, 2150-2158). Remote: Connect commands through Sp.
  - Engine::load: LoadRequest::from_context_uri or from_tracks; PlayingTrack Uri/Index; seek_to; LoadContextOptions::Options{shuffle, repeat, repeat_track}; spirc.activate() then load (195-214).
  - load_radio: LoadContextOptions::Autoplay (216-221).
  - PlayUris is limited to 500 URIs around the clicked one (app.rs:2249-2256).
- PlayMode (60-78): carries shuffle/repeat into each new load (librespot resets them). Forced in order for the DJ, spotify:show:* and episodes.
- Volume (app.rs:2165-2188):
  - Local: spirc.set_volume(v*65535).
  - Remote: PUT /me/player/volume, throttled to 250 ms unless committed.
  - LocalPlayer and DJ Voice use a v² curve.
  - EngineEvent::Volume updates the UI unless the slider is being dragged (app.rs:3907-3911). Mute keeps the previous value (2400-2407).
- Queue:
  - Add: one POST /me/player/queue?uri=…&device_id= per song, max 100. This goes through Connect add_to_queue even for the local device (app.rs:2973-2987).
  - Read: polled every 8 s while visible (app.rs:3452-3460).
  - Clear (app.rs:2459-2473): no clear-queue API exists. It re-plays the current context (with offset at the current track) or the single URI at the current position_ms via start_play(quiet), which drops queued items.
- Local files:
  - (a) spotify:local: entries inside Spotify contexts are played by librespot itself, through the vendored local_file lookup (tolerant match, rescan ≤ every 10 s, set_directories at app.rs:2622) and symphonia resampled to 44.1 kHz stereo (vendor GLASS.md:18-35).
  - (b) Local Files pages play through the app's own rodio LocalPlayer, after pausing Spotify (app.rs:3305-3330; local.rs:546-752).
- Reconnect:
  - SessionLost: drop the engine, then start_engine() (app.rs:3834-3845).
  - EngineReady Err: retry after [5, 15, 30, 60, 120] s (app.rs:3609-3627).
  - On ready: downloads.run + prune (app.rs:3593-3608).
  - Data connection: Api::sp() reconnects when the session is invalid, or the observer failed and the connection is more than 60 s old. A failure is remembered for 5 s (api.rs:126-157; sp/mod.rs:280-289).

==================================================================
4. DSP / AUDIO OUT / RODIO BACKEND
==================================================================
- Sink chain:
  1. librespot decoder (+ normalisation, soft volume).
  2. DspSink.write (dsp.rs:252-270): optional mono downmix, then a 6-band biquad EQ (DF2T f64) with preamp 10^(-maxboost*0.25/20) and a tanh soft limiter above a knee of 0.8 (136-172).
     - Bands: 60/150/400/1k/2.4k/15k Hz (config.rs:448). Band 0 low-shelf, band 5 high-shelf, the rest peaking, Q=1.0. Coefficients update live without resetting state (79-134).
     - When off or flat the signal passes bit-exact.
  3. RodioSink.write (vendor rodio.rs:355-384): Converter f64→f32, SamplesBuffer(2ch, 44.1k), wrapped in tap::Tapped, appended to a rodio::Sink, which feeds a rodio mixer at the device's channels/rate (rodio resamples), which feeds a cpal output stream.
- Format assumptions: SAMPLE_RATE 44.1k and stereo are hard-coded in the DSP (dsp.rs:17-19). The vendored decoder resamples everything to 44.1k stereo.
- Device config (rodio.rs:206-228): stereo, format preference f32 > i16 > i32 > f64; 44.1k if the device offers it, else its default rate; else the default config.
- Lazy open:
  - DspSink opens on the first start or write (dsp.rs:174-188).
  - The cpal stream is built paused (rodio.rs:264-270).
  - try_open_default never panics (rodio.rs:273-289).
  - On a write/start error the sink is dropped and reopened next time (dsp.rs:221-270).
- Pause / flush / stop:
  - pause: sink.pause() + stream.pause(), so no callbacks run; the queue is kept (rodio.rs:337-341).
  - flush: a new rodio::Sink on the same mixer (346-353).
  - stop: drains for at most 2 s, then flushes and halts (320-333).
  - write backpressure: waits while more than 26 buffers are queued (~0.5 s); errors if there is no progress for 2 s ("device went away") (373-382).
  - The player thread runtime is reduced to 2 workers (vendor player.rs:517).
- Device-change following (audio_out.rs):
  - OutputWatch.poll is called every UI frame and checks once per second: it compares cpal default_output_device().name() and enumerates devices to see whether the old one is "gone" (37-77). Skipped when headless.
  - A change bumps the GENERATION atomic (and GONE if the old device vanished). DspSink sees the new generation, reopens on the new default, and if the device is "gone" keeps the new output paused for up to 2 s while sleeping in real time to pace the player (dsp.rs:177-216).
  - The app pauses when the device is gone (app.rs:1876-1891). LocalPlayer reopens and re-seeks (local.rs:619-640).
- macOS-specific code: none explicit. CoreAudio is only reached through cpal's default host. There are no AudioObject property listeners anywhere; it is name polling.
- There are three independent outputs: engine RodioSink, LocalPlayer rodio::OutputStream (local.rs:748-752), and DJ Voice rodio::OutputStream on its own thread (dj.rs:248-276).
- iOS replacements:
  - cpal 0.16 has an iOS host (RemoteIO, R/cpal-0.16.0/src/host/coreaudio/ios). It does not configure AVAudioSession, so the app must set category .playback, activate it, handle interruptions, and add UIBackgroundModes=audio.
  - Replace OutputWatch name polling with AVAudioSession.routeChangeNotification (reason oldDeviceUnavailable → pause).
  - Consider one output and mixer for engine, local playback and DJ.
  - 44.1k fixed DSP is fine (rodio resamples), but check the session sample rate.

==================================================================
5. VISUALIZER (viz.rs)
==================================================================
- Tap: librespot_playback::tap::set(push) installed once at startup (viz.rs:23-26; main.rs:64).
  - Tapped wraps every rodio source played: engine buffers (rodio.rs:366), LocalPlayer files (local.rs:605), DJ clips (dj.rs:269).
  - It hands blocks of 1024 interleaved f32 samples (~12 ms stereo at 44.1k) with (channels, rate) as rodio pulls them on the audio thread. The block Vec is allocated on the player thread (vendor/librespot-playback/src/tap.rs:23-59).
- Ring: static Mutex<Ring> of N=2048 mono f32 (channel average). push uses try_lock and skips the block if the lock is busy. Rate and last-time are tracked (viz.rs:12-21, 54-65). latest_into returns None if nothing was pushed in the last 200 ms (70-79).
- Analysis, on demand:
  - Hann window, in-place radix-2 FFT N=2048, magnitudes over N/2 bins (81-117, 196-205).
  - n log-spaced bands from 40 Hz to min(16 kHz, Nyquist). Wide bands take the peak; narrow bands average with neighbours. Tilt ×(0.8+2.7·b/n) (207-229).
  - Auto-gain follows the loudest band with slow decay. Output = clamp(log10(m·sens/gainref)·0.55+1, 0..1) (230-236).
  - Smoothing: fast rise, configurable fall (smoothing 0-0.95, bass slower). Peaks hold, then fall 0.6/s; frame-rate independent (172-185).
- Output: Analyzer.bands / peaks Vec<f32> of length n, 0..1, plus `live`.
  - Driven per UI frame by ui/visualizer.rs:51-57 with n = theme bars clamped 12..128, sensitivity, smoothing (≥0.85 under reduce-motion).
  - The rate is therefore the UI repaint rate; the band count is set by the theme.
- Demo: synthetic signal thread (viz.rs:29-52).
- iOS: portable. It just needs the tap on whatever output path replaces rodio.

==================================================================
6. DJ (dj.rs + vendor/librespot-connect)
==================================================================
- Constants: URI = spotify:playlist:37i9dQZF1EYkqdzj48dyYq; COVER = lexicon-assets.spotifycdn.com/Your-DJ-Cover-Art-300.png (dj.rs:13-20).
- Context (vendored connect, GLASS.md:8-41):
  - context-resolve returns no tracks but a lexicon_context_url; the session is fetched from it.
  - Pages are chained by next_page_url and appended near the end of a page instead of autoplay.
  - Lines live in each track's metadata as narration.{intro|outro}.{ssml,voice,tts_provider,sample_rate}.
- Track hook: vendor/librespot-connect/src/glass.rs. set_track_hook replaces any earlier hook. TrackStart{uri, playing, metadata, next:(uri, metadata), after_end}. Returning true loads the song paused. ENDED is set around EndOfTrack → handle_next.
- Engine hook (engine.rs:142-153):
  - (say, ready) = dj::on_track_start(t).
  - hold = playing && say non-empty.
  - Emits EngineEvent::Dj when playing and anything is to say or prepare.
- on_track_start (dj.rs:41-52):
  - intro and outro are parsed.
  - The previous song's pending outro is swapped out and said only if after_end (skips from other apps get no outro).
  - If voice_on is off, nothing.
  - say = [prev outro, intro]; ready = [this outro, next intro].
- Line parsing (63-80):
  - VOICEn → n (default 1).
  - Provider: CLOUD_TTS 1, READSPEAKER 2, POLLY 3, WELL_SAID 4, SONANTIC_DEPRECATED 5, else 6 (SONANTIC_FAST).
  - sample_rate default 44100.
  - Caption = SSML stripped of tags and unescaped (89-120).
- TTS (134-209): POST {spclient base_url}/client-tts/v1/fulfill.
  - Headers: Authorization "{token_type} {login5 token}", content-type application/x-protobuf, client-token.
  - Body: hand-encoded ResolveRequest {2 ssml, 3 audio_format=5 (MP3), 5 voice, 6 provider, 7 sample_rate}.
  - Response: either audio (ID3 / MPEG sync / RIFF / OggS) or ResolveResponse{1 url}. The URL is then fetched with plain reqwest (redirect followed without the access token).
- Clip cache: keyed by SSML, cleared once it exceeds 24 entries (212-235).
- App flow, dj_break (app.rs:1742-1784):
  - Sets the caption.
  - Prefetches `ready` clips in the background.
  - If hold: dj_gen++, dj_waiting=true, pb.is_playing=true. Fetches `say` clips with a 6 s timeout, then Voice::play on spawn_blocking, then Msg::DjClips(gen).
- dj_clips_ready (1786-1799): a stale generation is ignored. None or error → dj_resume.
- Voice (dj.rs:238-293): a dedicated "dj-voice" thread owns a rodio OutputStream (open_default_stream). A Sink plays the clips decoded by rodio::Decoder and tapped. Volume v². Supports toggle and set_volume.
- Hold-before-song: the song is loaded paused by Spirc. tick() calls dj_resume() when voice.done(), which runs spirc.play() (app.rs:3221-3223, 1802-1809). Paused events during DJ speech keep is_playing true (3892-3897).
- Cancellation:
  - dj_cancel on start_play / Next / Prev: gen++, drop voice, forget_outro (app.rs:1813-1820, 2092, 2336, 2352).
  - Seek during the DJ → dj_resume (2370-2372).
- Voice setting: settings.dj_voice (default true) is copied into SHARED.voice_on every tick (app.rs:3220; config.rs:312).
- Shuffle is never applied to the DJ (engine.rs:75).
- iOS: portable except the separate rodio output, which should be merged into the shared AVAudioSession output.

==================================================================
7. DOWNLOADS (downloads.rs)
==================================================================
- Storage:
  - Spotify's encrypted files (Ogg Vorbis / MP3 as served) go into librespot's Cache files dir (cache_dir/audio/<xx>/<rest>, Cache::file_path).
  - Playback still fetches the audio key, so it is online-only (doc 1-20).
  - Index: config_dir/downloads.json {collections:{uri:{name, items}}, files:{uri:{file_id hex, bytes, quality}}}, written atomically via a .tmp rename (49-71, 196-198, 227-235). Saved after every 10 finished songs or when idle (179-180).
- Workers:
  - WORKERS=4; STARTS_PER_SECOND=2 with a shared pacer (36-41, 384-392).
  - Workers wake through a Notify, nap 300 s (5 s if retries are pending), and stop when the session is invalid or a newer run() generation starts (336-419).
- Per song (501-522):
  1. AudioItem::get_file, falling back to `alternatives` for relinked songs (482-499).
  2. Pick the format in the player's preference order for the current quality (201-208).
  3. If already cached, done.
  4. whole_file: CdnUrl::resolve_audio, try each URL with GET Range bytes=0-, 30 s stall timeout, write to `<dest>.<rand6>.part`, sync, rename (525-573). ATTEMPTS=3 with 750ms·2^n backoff.
  5. Fallback: librespot AudioFile::open read to the end (577-597).
  - CDN URLs are stripped from errors.
- Retry: ROUNDS=3 per session; after a failure, retry after 15·2^n s (30 s, 60 s), then count as failed (164-178).
- Pruning, prune(cache, audio_dir, limit) (423-458):
  - LRU by atime (else mtime). Never deletes downloaded file_ids. Skips .part files, deleting those older than 1 h.
  - Runs at engine ready, every 600 s with cache_limit_mb (default 2048, clamped 256..50000), and with limit 0 for "Clear cache" (app.rs:3261-3264, 3376-3382, 2686-2693; config.rs:375, 428).
- Collections:
  - add(uri, name, items) keeps tracks and episodes only. Removed songs are deleted unless another collection holds them (drop_unwanted) (245-260, 128-136).
  - remove(Some or None) (264-280).
  - App: Action::Download → collection_items → add (app.rs:3014-3031). sync_download re-adds when a downloaded playlist or Liked Songs changes (app.rs:3033+).
- Quality: follows the streaming quality. forget_stale drops entries of another quality so they download again (111-116).
- iOS: portable (librespot-audio plus file cache). The cache dir must be in Application Support with isExcludedFromBackup, not Caches, or iOS may purge it. Background downloads would need a BGProcessingTask, because a tokio worker dies when suspended.

==================================================================
8. LOCAL FILES (local.rs)
==================================================================
- Formats: EXTENSIONS mp3, m4a, mp4, flac, ogg, oga, wav, aac, aif, aiff (16). The vendored lookup also lists m4p (vendor local_file.rs:26).
- Scan:
  - tidy_sources: canonicalize, drop folders covered by an outer source within DEPTH=5, processed outer-first (64-89, 104-115).
  - walk: depth ≤5, skips dotfiles, stops at 20,000 files (117-135).
  - Dedup by canonical path; sorted by mtime, newest first (69-72).
  - add_source and missing_sources (92-102, 220-231).
  - Runs on a std::thread (app.rs:3361-3373).
- Tags: lofty Probe::open → guess_file_type → read (233-235).
  - Fields: title (falls back to the file stem), artist, album, genre, year, has_cover, AlbumArtist, track, disc, duration (143-154, 237-249).
- Tag editing, safe write (374-439):
  1. Validate: title required, year 1000-2999 (361-372).
  2. Canonicalize (follows symlinks), copy to `.{name}.glass-edit`.
  3. Edit the primary tag, inserting one if missing. Cover Keep / Remove / Set (JPEG CoverFront).
  4. save_to_path, restore mtime, rename over the original. The temp file is removed on error.
  - prepare_cover: ≤1000 px, JPEG q90 (442-448).
- Covers: the `localcover://<path>` egui BytesLoader. Reads 2 at a time via spawn_blocking; MIME sniffed from magic bytes (452-544).
- Temporary handover (macOS-specific): files under TemporaryItems or $TMPDIR/com.apple.* are copied into the inbox (175-215).
- LocalPlayer (546-752):
  - rodio OutputStream default + Sink; Decoder::try_from(File); undecodable files are skipped.
  - Shared Arc queue; enqueue/dequeue; prev restarts the song if more than 3 s in; seek via try_seek.
  - Releases the output after 30 s paused and resumes at the saved position (734-745). follow_output (622-640).
- Spotify-playlist URI matching:
  - spotify_uri() writes spotify:local:artist:album:title:secs, form-encoded with + for spaces, seconds rounded (271-284).
  - find(): normalized title must be equal; artist matches by whole-word containment either way; length within ±3 s; same album breaks ties (303-334). Uses crate::lyrics::matching::normalize.
  - Mirrored in vendor/librespot-playback/src/local_file.rs:66-130 for engine playback. App cache: local_link (app.rs:3334-3341).
- reveal: macOS `open -R`, else open::that (287-295).
- iOS:
  - There are no user folders to scan. Use the app's Documents directory (UIFileSharingEnabled) plus UIDocumentPicker security-scoped bookmarks, or MPMediaLibrary (DRM'd tracks are unplayable).
  - Drop the temporary-folder logic and reveal.

==================================================================
9. LOCAL PLAYLISTS (playlists.rs)
==================================================================
- Model: LocalPlaylist{id "lp_"+12 hex uuid, name ≤120 chars, description ≤500, tracks Vec<PathBuf>, created/updated unix seconds, from_draft Option} (8-20).
- Stored in config_dir/local_playlists.json via write_atomic, pretty JSON. Each commit writes first, then swaps in memory (40-70).
- Operations:
  - create, idempotent on from_draft (78-102).
  - replace, with optional rename (105-119).
  - append, skipping duplicates (123-137).
  - remove_at (140-148), delete (150-155).
- Undo stack (max 50): Created(id) / Replaced(old) / Deleted(p, index). undo() returns a message (22-27, 157-192). Wired at app.rs:2913-2917.
- iOS: portable. Absolute PathBufs break across container UUID changes, so store paths relative to Documents or as bookmarks.

==================================================================
10. LOCAL-FILE SHARING (share.rs)
==================================================================
- mDNS: SERVICE "_glass-files._tcp.local." (51).
  - Instance = 12 random hex per run (app.rs:1597). Host "glass-{instance}.local.". enable_addr_auto. ip_check_interval 30 s.
  - TXT: v=3, acct=<tag>, name=<computer name> (1037-1044).
  - Browse: ServiceResolved peers are kept only if the acct tag matches this account and v==3 and the instance is not our own; ServiceRemoved drops them (1045-1073).
  - Unregister and shutdown on Drop (1133-1146).
  - All of this is mdns-sd's ServiceDaemon, which runs its own raw UDP multicast sockets on 5353 (Cargo.toml:53). That is the ONLY multicast use.
- Port: TCP bind 0.0.0.0:47321, else an ephemeral port (54, 999-1005).
  - Accept loop with an INCOMING=8 semaphore; extra connections are dropped (348, 1007-1031).
  - Dial: IPv4 first, 5 s timeout, nodelay (932-948).
- Protocol version: VERSION=3 (58).
- Noise: "Noise_XX_25519_ChaChaPoly_BLAKE2s" via snow, 3-message XX. Framing is a u16 BE length + payload (55, 359-397).
  - Transport plaintext = 1 kind byte (0 JSON last, 2 JSON part, 1 DATA) + data. CHUNK = 60 KiB (60, 338-342, 399-416).
- Account tag: "{salt}.{hex(HMAC-SHA256(key=username, "glass-lan-v2"‖salt)[..8])}", salt = instance (186-196).
- Account proof:
  - Hello{v, name, account, proof}.
  - proof = HMAC-SHA256(key=own Connect device_id, "glass-lan-v1"‖initiator byte‖handshake hash) (246-252).
  - Verified against the account's device IDs from GET /me/player/devices, filtered to type=="Computer" (app.rs:1599-1609). Cached; refetched at most every 5 s (600-623).
  - The initiator sends Hello first; the responder verifies before replying (472-496).
- Pairing (498-598):
  - Exchange Known(knows_key, open). If both sides know each other's key, skip.
  - Otherwise commit-reveal: initiator sends Commit(SHA256(hash‖r_i)), responder sends Random(r_r), initiator sends Random(r_i), commitment checked.
  - code = SHA256(hash‖dialing_r‖answering_r)[0..4] as u32 BE % 1e6, shown as "ddd ddd" (203-213, 552-568).
  - Both users confirm (Event::Pair; Share::answer), then Paired(bool) both ways. PAIR_WAIT 120 s. One pairing at a time; declined keys are remembered for the run (637-724).
  - Trusted = the remote Noise static key, in hex.
- Messages (316-336): Send(Vec<FileMeta>), Want(Vec<spotify:local URIs>), Offer, Accept(Vec<bool>), Saved(n), Refused(s), Known(bool, bool), Commit(hex), Random(hex), Paired(bool).
  - FileMeta{name, size, title, artist, album, duration_ms} (292-313).
  - serve handles Send and Want (886-913). fetch() asks peers in turn for missing URIs (1110-1130).
- Limits: MAX_FILE 4 GiB, MAX_FILES 20,000, MAX_JSON 32 MiB, MAX_HELLO 4 KiB, QUIET 30 s (61-66, 345-346).
- Filename sanitisation and placement, destination() (749-767):
  - The name must equal Path::file_name of itself (no path parts), must not start with '.', must have an audio extension, and size must be 1..=4 GiB.
  - Skipped if local::find already matches, or a same-size file exists.
  - Collisions become "stem (n).ext" for n=2..99.
  - Received into `.{name}.glass-part`, then renamed (801-845).
  - inbox = save_song::default_dir() = ~/Music/Glass (239-242). Received paths go out as Event::Received, which feeds Local Files.
- Persistence: config_dir/share_key (Noise keypair as 2 hex lines, write_private 0600) (726-743); config_dir/share_paired.json [{key, name}] (649-682).
- Computer name: macOS `scutil --get ComputerName`, else /etc/hostname or /proc (216-232).
- Started only when settings.share_local_files is on and the engine is up (app.rs:1580-1621).
- iOS:
  - Replace mdns-sd with Network.framework NWListener(service:) + NWBrowser (or NetService). Info.plist needs NSLocalNetworkUsageDescription and NSBonjourServices ["_glass-files._tcp"]. Raw multicast would need the com.apple.developer.networking.multicast entitlement.
  - The listener works only in the foreground.
  - No scutil; UIDevice.name is generic since iOS 16 without an entitlement.
  - Inbox → Documents.
  - Noise, HMAC and the protocol are portable.

==================================================================
11. SAVE SONG (src/save_song/)
==================================================================
- Pipeline, save() (mod.rs:238-306):
  1. dest = `<dir>/<first artist> - <title>.<m4a|mp3>`. file_name() replaces control characters and /\:*?"<>| with _, trims dots, caps at 200 bytes, falls back to "Untitled" (122-139).
  2. Work files `.{name}.{uuid8}.glass-part|glass-plain|glass-tmp`, removed by the Cleanup Drop guard (141-146, 243-245, 308-317).
  3. Status sequence: Finding → Downloading(%) → Converting → Done / Failed / Stopped (206-216).
  4. JioSaavn first, then YouTube Music.
  5. Cover fetched from the Spotify image URL.
  6. Finish on spawn_blocking:
     - M4A: mp4::rewrite(part → tmp, tags).
     - MP3: mp4::rewrite(part → plain), then mp3::encode, then tag_mp3 (lofty).
     - Then rename into place.
- JioSaavn (saavn.rs), base www.jiosaavn.com/api.php?_format=json&_marker=0&cc=in:
  - __call=autocomplete.get&includeMetaTags=1&query=<title>, ranked by rank_saavn.
  - __call=song.getDetails&pids=<id>, then encrypted_media_url decrypted with DES-ECB, key "38346591", PKCS7, base64 (43-49).
  - "_96.mp4" is replaced by _320 when the 320kbps flag is set, else _160; forced https (10-41).
- YouTube (youtube.rs):
  - Search: POST music.youtube.com/youtubei/v1/search, client WEB_REMIX 1.20250101.01.00. Songs filter "EgWKAQIIAWoMEA4QChADEAQQCRAF" and Videos filter "EgWKAQIQAWoMEA4QChADEAQQCRAF" run in parallel. musicResponsiveListItemRenderer rows are scanned; visitorData extracted (16-109).
  - Player: POST www.youtube.com/youtubei/v1/player as the VISIONOS client (version 1.02, X-YouTube-Client-Name 101, deviceModel RealityDevice17,1, osVersion 26.5.23O471, Safari visionOS UA) with visitorData / X-Goog-Visitor-Id. itag 140 (AAC-LC 128k) or the best audio/mp4 (21-28, 148-175).
  - Download: 10 MB `&range=a-b` chunks; on a short read, ask for the rest (≤2 tries); 4xx aborts (178-221).
  - On 403: fresh visitor from POST youtubei/v1/visitor_id (WEB 2.20260708.05.00), up to 4 attempts (115-140).
- Matching (matching.rs):
  - words(): lowercase, accents folded, hyphens/apostrophes inside a word joined.
  - Title is taken without brackets or " - …" suffixes. Must contain most title words: ≥60% if more than 2 words, else all. close() allows edit distance 1 for 5-7 letters, 2 for 8+.
  - Must credit ≥1 artist (all words, or a ≥4-char run-together name like "TheWeekndVEVO").
  - Version penalty ×0.5 for cover / remix / live / karaoke / instrumental / acoustic / sped / slowed / reverb / nightcore / 8d / lofi / mashup / tribute / parody, unless the song itself is one.
  - YouTube score = 0.45 artist + 0.35 length (1/(1+((Δs)/6)²)) + 0.2 title + 0.05 song bonus.
  - Saavn score = 0.65 artist + 0.35 title (28-124).
- MP4 remux (mp4.rs): reads plain stbl (stsz/stts/stco|co64/stsc) or fragmented moof/traf/trun with trex defaults; bounds-checked (16-237).
  - Writes ftyp "M4A " + one mdat (64-bit if large) + moov: mvhd/tkhd/mdhd/hdlr/stbl, original stsd copied, single chunk (239-356).
  - udta/meta/ilst tags: ©nam ©ART ©alb aART ©day trkn ©cmt (Spotify link) covr (JPEG 13 / PNG 14) (358-392).
- MP3: symphonia decodes AAC per packet (damaged frames skipped), LAME encodes. Bitrates 128/160/192/256/320, CBR, quality 2, no Xing tag. Output under 1 KB counts as an error (mp3.rs:17-79; lame.rs:12-100).
- LAME FFI: lame_init / set_num_channels / set_in_samplerate / set_brate / set_quality / set_bWriteVbrTag / init_params / encode_buffer_ieee_float / encode_flush / close (lame.rs:12-24).
  - build.rs builds vendor/lame-3.100 with autotools as a shared dylib only when TARGET contains "apple-darwin" (LGPL relinkability; @rpath in Frameworks). Otherwise it links the system dylib mp3lame (build.rs:6-54).
  - An iOS target ("aarch64-apple-ios") falls into the system-lib branch and fails to link.
- Concurrency: SAVES_AT_ONCE=3 tokio Semaphore (app.rs:580, 942, 1209-1233). Single saves are deduplicated while busy (1138-1140).
- Cancellation:
  - Only collection "Stop": an AtomicBool checked while waiting for a slot (polled every 300 ms) and just before starting (app.rs:1210-1224, 2675-2681).
  - In-flight saves are NOT aborted. There is no cancel for single songs.
- Collections, save_set (app.rs:1150-1188):
  - Subfolder = file_name(collection name). Deduped per song URI and per output path. Keys are "{set}|{song}".
  - Playlists only, when the set finishes: `<folder>/<name>.m3u8` = "#EXTM3U\n" + bare filenames of the saved songs, in order (mod.rs:193-204; app.rs:1284-1299).
- Startup: sweep_work_files deletes work files older than 1 h in the save dir (one level of subfolders) and in the share inbox (mod.rs:150-170; app.rs:1016-1024).
- iOS: the remux and symphonia code are portable.
  - The LAME build needs an iOS branch, a static link (LGPL concerns), or dropping MP3 (iOS has no system MP3 encoder).
  - The YouTube/JioSaavn downloading carries App Store review and legal risk.
  - ~/Music/Glass → Documents.

==================================================================
12. PLATFORM-SPECIFIC CODE AND iOS IMPACT
==================================================================
- cfg(target_os = "macos") in scope:
  - sp/mod.rs:461 (hash error text).
  - local.rs:288-295 (`open -R`, label).
  - share.rs:217 (scutil).
  - update.rs:27, 90, 218-250 (TARGET, renamex_np RENAME_SWAP).
  - heap.rs:34-48 (mi_collect).
  - main.rs:57-59 (mimalloc global allocator), 96 (icon).
  - window_fx.rs:74-226.
  - Elsewhere: app.rs:4045 autostart LaunchAgents, 4159.
- objc2 / objc2-app-kit / objc2-foundation (Cargo.toml:77-79), window_fx.rs:136-226:
  - NSView → NSWindow windowNumber, setFrameUsingName / setFrameAutosaveName.
  - Private CGSMainConnectionID / CGSSetWindowBackgroundBlurRadius via dlsym.
  - NSVisualEffectView fallback; NSWorkspace accessibility reduce-motion / reduce-transparency.
  - iOS: UIAccessibility.isReduceMotionEnabled / isReduceTransparencyEnabled and UIVisualEffectView. Drop the private CGS calls (App Store rejects them).
- CoreAudio: used only implicitly through cpal (macOS host). No property listeners. iOS: cpal's RemoteIO host + AVAudioSession setup, route-change and interruption observers.
- keyring (Cargo.toml:45, "apple-native"): only used in ai/secrets.rs:51-79 (assistant API key), not for Spotify tokens. keyring 3.6 supports aarch64-apple-ios via security-framework, so it can also hold the Spotify refresh token on iOS.
- directories:
  - config_dir / cache_dir (config.rs:21-27).
  - UserDirs audio_dir / download_dir for default local_dirs and ~/Music/Glass (config.rs:338-339; save_song/mod.rs:71-81).
  - directories 6 shares one mac/ios module, so it resolves inside the sandbox ($HOME/Library/Application Support|Caches/Glass). UserDirs audio/download dirs are meaningless on iOS → use Documents.
- open: local.rs:291, the oauth browser launch inside librespot-oauth, and various UI links. iOS: UIApplication.open / ASWebAuthenticationSession; no "reveal in folder".
- rfd: file and folder pickers (app.rs:2644; ui/settings.rs:267, 466; ui/local_library.rs:297; ui/menus.rs:95). rfd has no iOS backend → UIDocumentPickerViewController.
- souvlaki: media keys / Now Playing (app.rs:1056-1074, 3670, 3998-4017). souvlaki 0.8.3 compiles one module for macos+ios (R/souvlaki-0.8.3/src/platform/mod.rs:8). The MPNowPlayingInfoCenter / MPRemoteCommandCenter path is likely reusable; verify on device.
- memmap2: ui/theme.rs:254 only (UI). POSIX mmap works on iOS.
- mimalloc: macOS-only global allocator plus mi_collect on tokio thread park (main.rs:57-59; heap.rs:32-49). Under iOS cfg it falls back to the system allocator, and heap::on_idle becomes a no-op. Harmless.
- Process spawning (not allowed on iOS):
  - `unzip` (sp/mod.rs:145).
  - `scutil` (share.rs:218).
  - `open` (local.rs:289; update.rs:254).
  - `ditto` / `plutil` (update.rs:186, 210).
- Unix socket single-instance: instance.rs (cache_dir/glass.sock, 0600, "glass1 <identity>" → focus|quit). Not applicable on iOS; drop.
- update.rs: GitHub feed (FEED, update.rs:20), ed25519 signature check over "glass-update v1\n{ver}\n{target}\n{sha}" (PUBLIC_KEY at 22), zip, ditto, swap .app. Not applicable on iOS (App Store / TestFlight); drop.
- File permissions: 0600 / 0700 via cfg(unix) PermissionsExt (config.rs:85-89, 119-123; engine.rs:187-191). Harmless on iOS (sandbox); prefer Data Protection classes plus Keychain.
- Background: everything runs on a 2-worker tokio runtime (app.rs:829). On iOS the Connect dealer websocket, Spirc and observer drop when the app is suspended unless audio is playing (background audio mode). Expect SessionLost / reconnect on resume; that path already exists (app.rs:3834-3845; api.rs:126-157).
