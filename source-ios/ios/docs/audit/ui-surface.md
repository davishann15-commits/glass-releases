<!-- Read-only inventory of the desktop source (glass-player b403e2a, 1.0.0-beta.22), made for the iOS port. Paths are relative to the repository root. -->

GLASS DESKTOP: USER-VISIBLE FEATURE LEDGER (read-only inventory)

Files read: src/app.rs, src/demo.rs, and in src/ui/: mod.rs, library.rs, local_library.rs, pages.rs, tracklist.rs, menus.rs, modals.rs, nowbar.rs, nowplaying.rs, right.rs, mini.rs, command.rs, assistant.rs, settings.rs, themepanel.rs, widgets.rs. I also looked at small parts of assistant_app.rs, playlists.rs, local.rs, config.rs, lyrics/*, translate_app.rs, save_song, main.rs and visualizer.rs to get exact strings.
Unless a path is given, "app:N" means src/app.rs, "pages:N" means src/ui/pages.rs, and so on.

======================================================================
0. CHROME (always on screen once signed in), mod.rs
======================================================================
- Layout (mod.rs:48-176): a title strip (44px), a left nav rail (88px), the content area, and a floating player bar (96px). On Now Playing the bar is replaced by a key-hint strip (54px). The side panel takes its width from the page. The theme panel floats over the page. Toasts appear above the bar.
- Title strip (mod.rs:180): empty space drags the window; double-click toggles maximize. A Back button "Back  ⌘ [" shows only when there is history. The breadcrumb reads "~ / <path>". The path for each route: Library=music, NowPlaying=music / now-playing, Home=home, Search and Genre=search, Settings=settings, Activity=settings / activity, Playlist=music / playlist, Album=music / album, Artist and Discography=music / artist, Track=music / song, Show and Episode=podcasts, Audiobook=audiobooks, Profile=profile, LikedSongs=music / liked, YourEpisodes=podcasts / saved, LocalFiles=music / local, Recents=music / recents, Folder=music / folder, LocalPlaylist=music / local / playlist, LocalAlbum=music / local / album.
- Update pill (mod.rs:222): "update to {v} · restart", shown when an update is Ready. Tooltip: "Installs the new version and reopens Glass (or it installs when you quit)". Runs RestartToUpdate.
- Now Playing view pills in the title strip (mod.rs:238): "playlist" (tooltip "The list that's playing  P"), "queue" ("Queue  Q"), "lyrics" ("Lyrics  L"), plus a visualizer level-meter toggle ("Show the visualizer  V" / "Hide the visualizer  V"). The visualizer toggle persists settings.visualizer.
- Nav rail (mod.rs:317), in order: Home, Library, "Search  ⌘ L" (also focuses the search field), "Local Files", "Settings  ⌘ ,". Highlighting: Search is lit for Search/Genre; Local Files for LocalFiles/LocalPlaylist/LocalAlbum; Settings for Settings/Activity; Library for everything else.
- Now Playing hint strip (mod.rs:296): "space play / pause" on the left; "L lyrics • Q queue • P playlist • V visualizer" on the right.
- File drop (mod.rs:389): dragging files in from Finder shows an overlay, "Drop to add to “{playlist}”" or "Drop to add to Local Files", with the subline "songs play from their folder, which joins Local Files". The drop runs AddFiles. The target is the open Spotify playlist if you can write to it, the open local playlist, or else Local Files (file_target, mod.rs:374).
- Drag ghost (mod.rs:411): a label follows the pointer while songs are dragged inside Glass (song title, or "N songs").
- Toasts (mod.rs:422): only the newest shows, as "> {text}", for 3 s (app:3265).
- Close button (mod.rs:434): if settings.close_minimizes is on, closing minimizes instead.
- Album colors (mod.rs:68-98): when settings.album_colors is on, the cover palette tints the now-playing views and the bar. On Now Playing the whole window gets a wash. The next queued song's colors are prefetched.

======================================================================
1. ROUTES (app:51-85)
======================================================================
The app starts on Library (app:932). Navigating pushes history, at most 100 entries (app:2201). Each page keeps its own filter, sort, cursor and scroll for Back/Forward. A fresh visit scrolls to the top, except Library and Settings. Navigating anywhere leaves fullscreen. NowPlaying is an overlay: going there and back keeps the page's state.

- Library (library.rs:299): split view, with the list on the left and a detail pane on the right. The command bar sits above the detail pane, or above the list when narrow (detail width under 300 = no detail pane). Full detail in section 10. States: "loading…"; empty "nothing here yet" / "Things you save on Spotify show up here."; "no matches" / "Nothing in your library matches the command bar text."; Downloaded empty "nothing downloaded yet" / "Choose download on a playlist, album or podcast to keep it on this computer."
- NowPlaying (nowplaying.rs:23): see section 5.
- Home (pages:265):
  - Chips: all / music / podcasts / audiobooks. Then the lowercased greeting and the title "home_".
  - all/music: a DJ banner, an 8-tile shortcut grid of recent contexts (4 columns if wider than 900, else 2), "Jump back in" (recents 9+, Liked Songs excluded), "Your top artists", "On repeat" (albums of your top tracks), "Your playlists" (its "show all ›" goes to Library > playlists), "Saved albums".
  - all/podcasts: "Your shows", then "Your Episodes" ("show all ›" goes to YourEpisodes; 3 rows, or 10 under the podcasts chip).
  - all/audiobooks: "Your audiobooks". Under the audiobooks chip, an empty state reads "no audiobooks yet" / "Audiobooks you save appear here."
  - DJ banner (pages:238): title "DJ"; subtitle "X: {caption}" while playing, else "Songs picked for you, with X talking between them" (or "Songs picked for you" when voice is off). It has a play button; clicking the banner opens the DJ page.
  - Shortcut tiles (pages:365): cover (Liked Songs gets a purple heart tile), name (accent when that context is playing), hover play/pause; click opens.
  - Cards everywhere: hover play button and a right-click menu (menus::any).
- Search (pages:450):
  - Field (pages:412): placeholder "search spotify — songs, artists, albums, podcasts". Debounced 350 ms; Enter commits at once.
  - Empty query: "Recent searches" with chips (last 10) and a "Clear recent searches" link, then "Browse all", a grid of categories (lowercased names, colored stripe, rotated icon) that opens Genre.
  - Filter chips: All, Songs, Artists, Albums, Playlists, Podcasts & Shows, Episodes, Audiobooks.
  - All: "Top result" card (an artist whose name exactly matches the query, else the first track, else the first artist) with a play button; "Songs" (4 rows); then the sections Artists, Albums, Playlists, Podcasts, Episodes, Audiobooks, each with "show all ›" that switches the chip.
  - Single filter: up to 50 results. Songs show as a table (album column, cover, sortable header), Episodes as episode rows, everything else as a card grid.
  - No results: "No results found for “q”" / "Check the spelling, or try fewer or different words."
  - Any click in the results saves the query to recent searches (settings, persisted). Loading uses "loading…"; errors use the error box (see "Common states" below).
- Genre {id,name} (pages:652): a big title, then playlist cards. Error state: "couldn't load this category" / "Spotify didn't return this page. Searching for its name may still find its playlists." with a button "Search for “{name}”".
- Playlist(id) (pages:678):
  - The DJ playlist id renders the DJ page instead (pages:214): kind "spotify", title "DJ", description "Your personal DJ, X: songs you love, ones you used to, and new picks, with X introducing them as they play.", a play button, "Next pick" while the DJ is playing, and the caption “line” — X, or "X's voice is off (Settings → Playback): the DJ plays its picks without talking."
  - Header: kind "Collaborative Playlist" / "Public Playlist" / "Playlist", name, description, owner link (opens Profile), "N saves", "N songs, duration". Clicking the cover of your own playlist opens the Edit details modal.
  - Action row (pages:87): [play]/[pause]; [shuffle]/[shuffle on] (toggles shuffle only); a +/✓ save button for others' playlists ("Save to Your Library" / "Remove from Your Library"); a download toggle; ⋯ "More options" opening the playlist menu.
  - Controls: the sort/view menu and a "filter list" field (tracklist.rs:464).
  - Table: album column, "Date added" column, cover, header. Drag to reorder when editable, sort is Custom and there is no filter.
  - Your own empty playlist: "Let's find something for your playlist" with a "Search for songs" field (2+ characters, debounced) whose results each have an "Add" button. A non-empty own playlist shows the same search under "Find more".
  - If the /tracks endpoint fails it falls back to /items.
- Album(id) (pages:832):
  - Header: kind Single / Compilation / Album, artist avatar, artist links, year, "N songs, duration".
  - Action row with save to library, then controls.
  - Table with track numbers and "Disc N" headings (only in Custom order).
  - Below: release date, copyright lines (℗/©), and "More by {artist}" (its "show all ›" opens Discography).
- Artist(id) (pages:885):
  - Banner image with a dark fade, the caps label "artist", the name, and "N monthly listeners" (or "N followers").
  - Buttons: play, a Shuffle toggle, "Follow"/"Following", ⋯ opening the artist menu.
  - "Popular": 5 songs, with a "See more"/"Show less" link for 10.
  - "Discography": its "show all ›" opens Discography; chips "All releases" / "Albums" / "Singles and EPs" / "Compilations"; newest first.
  - Then "Fans also like", "Appears On", and an "About" card (followers, genres joined by " · ", monthly listeners, biography).
- Discography(id) (pages:1029): the artist name as a link back to the artist, the same 4 chips, and the full grid.
- Show(id) (pages:1117):
  - Header kind "Podcast", then the publisher. "Follow"/"Following" button and ⋯ opening the show menu.
  - "All Episodes" as episode rows, plus an "About" card with the description and "N episodes". Side by side when wider than 860.
  - Episode row (pages:1044): cover, name (accent if playing), show name, two lines of description, a play/pause circle, meta ("date • Played" / "date • {x} left" / "date • duration"), a resume progress bar, and a +/✓ button ("Save to Your Episodes" / "Remove from Your Episodes"). Click opens the Episode; right-click opens the track menu.
- Episode(id) (pages:1172): kind "Podcast Episode" and a show link; "date • duration"; play; +/✓; ⋯ opening the track menu; "Episode Description"; a "See all episodes" button.
- Audiobook(id) (pages:1208): kind "Audiobook", authors, "Narrated by …"; the action row with save; ⋯ opening the audiobook menu; "Description"; "Chapters" (number, name, duration). Double-clicking a chapter plays it in context.
- Track(id) (pages:1244):
  - Header kind "Song": artist links • album link • year • duration • "Popularity N".
  - Buttons: play (plays in the album context), heart ("Save to Liked Songs" / "Remove from Liked Songs"), ⋯ opening the track menu.
  - One row per artist ("Artist" + name). Then "Popular songs by" {artist} (top 5) and "Releases by {artist}" (its "show all ›" opens Discography).
- Profile(id) (pages:1321): an empty id means yourself.
  - Header kind "Profile" with a round avatar: "N Public Playlists • N Followers" (plus "• N Following" for yourself). ⋯ opens the user menu.
  - Yourself only: "Top artists this month" and "Top songs this month" (4 rows), each marked "Only visible to you".
  - Then "Public Playlists" (for yourself, only your own public ones) and "Following" (yourself only).
- LikedSongs (pages:1400):
  - Header: a heart tile, caps "collection", "Liked Songs", your name • N songs.
  - Action row; its ⋯ menu has a single item, "Add all to queue". Then controls.
  - Table: album, date added, cover, header. Hearts are pre-marked as saved.
  - Empty: "songs you like will appear here" / "Click the heart on any song to save it here."
- YourEpisodes (pages:1436): a bell tile, "Your Episodes", "N episodes"; a play button that plays them all; episode rows. Empty: "add to Your Episodes" / "Click + on an episode to save it here."
- LocalFiles (local_library.rs:228): see section 11.
- Recents (pages:1685): title "Recents", subtitle "The last 50 songs you played, from Spotify's listening history.", grouped by day with a heading per day, tables (album, cover, no header, so no keyboard cursor).
- Folder(id) (pages:1707):
  - A folder tile, "Folder", the name, "N playlists". Buttons "Rename" and "Delete folder" (opens a confirm).
  - A grid of its playlists.
  - Empty: "this folder looks empty" / "Start adding playlists by dragging and dropping them onto the folder in Your Library."
  - Missing: "folder not found" / "It may have been deleted."
- LocalPlaylist(id) (pages:1579):
  - Just deleted: "playlist deleted" / "Undo brings it back, songs and order included." Otherwise missing: "playlist not found" / "It may have been deleted." Either way, [undo last playlist change] shows if an undo is available.
  - Header: caps "local playlist", name, description, and "N songs · duration · files on this computer · N missing".
  - Buttons: [play]; [add files] (tooltip "Add songs from this computer (or drop them here)", file picker); [delete] (confirm); [undo last change].
  - Rows: number, cover, title, artist, duration; a single click plays from that row. Missing files show as "NN  file missing: {filename}".
- LocalAlbum{artist,album} (local_library.rs:408):
  - Missing: "this album isn't in Local Files any more" / "Its files may have moved; try Rescan in Local Files."
  - Header kind "album", title, "artist · genre · year · N songs, duration"; a play button; ⋯ opening the local album menu.
  - Songs grouped by "Disc N"; an artist column appears on compilations; keyboard works only on single-disc albums.
- Settings (settings.rs:92): title "Settings" and a "Search in Settings" field. Searching filters rows; a matching section title shows the whole section. Sections and rows:
  - Account: Signed in as (name · email · country, plan Premium/Free); Edit login methods [Edit] (opens spotify.com/account/overview); Log out [Log out] (no confirm here).
  - Appearance: Theme file [Reload] [Edit in editor] [Theme panel]; Theme status; Background effect; Font ("family · N pt (set in theme.toml)"); Reduce motion (On/Off note).
  - Assistant: section 5f / assistant.rs:408.
  - Lyrics (settings.rs:494):
    - Lyrics source chips Auto / LRCLIB / "Spotify" (or "Spotify · not connected"), with a note per mode. A "Spotify lyrics" note row shows when Spotify is the source.
    - Translation chips: translated / original + translation / original.
    - Translate into (text field; "system" or a language code).
    - Translator chips: Auto / Apple on-device (only where available) / Assistant, with a note per mode.
    - Timing offset (all songs): [Reset] [+0.1 s] [−0.1 s].
    - Settings file (config.toml) [Reload] [Edit in editor]; Settings file status.
    - Saved lyrics "N songs · N KB…" [Clear].
    - About LRCLIB.
  - Downloads: Downloaded "N downloaded · N MB · N still downloading" [Remove all downloads] (confirm); Couldn't download (only if any failed); How downloads work.
  - Save songs (settings.rs:436): Format combo M4A/MP3; MP3 bitrate 128/160/192/256/320 kbps (MP3 only); Folder [Reset] [Change…]; Show saved songs in Local Files (a toggle, or a note when the folder is already inside a source).
  - Your Library:
    - Use compact library layout (toggle) and Show Local Files (toggle).
    - Share with Glass on your other computers (toggle).
    - One "Paired with" {name} row per paired computer, with [Forget].
    - One "Show songs from" {dir} row per source folder, with [Remove]; then "Add a source" [Add a source].
  - Display: Open the now-playing screen when playback starts; Show desktop notifications when the song changes; Match Now Playing to the album's colors.
  - Playback:
    - Toggles: Gapless playback; Autoplay; DJ voice; Mono audio; Normalize volume.
    - Volume level Quiet/Normal/Loud (only when normalize is on).
    - Equalizer card: on/off toggle, Presets combo (Flat, Acoustic, Bass booster, Bass reducer, Classical, Dance, Deep, Electronic, HipHop, Jazz, Latin, Loudness, Lounge, Piano, Pop, RnB, Rock, Small speakers, Spoken word, Treble booster, Treble reducer, Vocal booster, Manual), [Reset], and a 6-band draggable curve: 60Hz, 150Hz, 400Hz, 1KHz, 2.4KHz, 15KHz at ±12 dB.
  - Audio quality: Streaming quality Normal (96 kbps) / High (160 kbps) / Very high (320 kbps).
  - Explicit content: Allow explicit content.
  - Storage: Cache [Clear cache]; Cache size limit (MB) drag value 256–50000.
  - Startup and window behavior: Open Glass automatically after you log into the computer (No/Yes/Minimized); Close button should minimize the window.
  - Proxy: HTTP proxy (text field).
  - Spotify Connect: Device name; Playback engine status [Restart engine]; Client ID.
  - Updates: Version; Update automatically; Check for updates [Check now] / [Restart to update]; What's new in {v}.
- Activity (pages:1785):
  - Title "Activity", subtitle "Everything Glass sends, receives and stores."
  - Card "Who Glass talks to": 9 hosts with reasons, and "No analytics, telemetry or crash reporting."
  - Card "What Glass stores": 13 files with sizes or "not present" (queries.json is macOS-only), audio cache "of N MB", lyrics cache, assistant keychain note, a "Show in Finder" button per folder, and "Playback engine: …".
  - "requests (N)" with a [Clear] button, then one row per request: time, method, status, ms, path.
- Common states (pages:60-84, widgets.rs:465-492): "loading…"; the error box "couldn't load this" + error text + [try again] (which invalidates the request); empty notes via widgets::unavailable (title + body).
- Filter enums: LibFilter (app:119) = Playlists, Podcasts, Albums, Artists, Audiobooks, Downloaded. SearchFilter (app:129) = All, Songs, Artists, Albums, Playlists, Podcasts, Episodes, Audiobooks. TrackSort (app:141) = Custom, Title, Artist, Album, Added, Duration.

======================================================================
2. ACTIONS (declared app:204-429; handled in apply() app:2199-2952)
======================================================================
"Web API" below means Web-API-shaped paths served through Api/sp (Spotify spclient and api-partner). "Engine" means librespot spirc on this computer. Glass uses the engine when it is the active device, otherwise a Connect command over the Web API.

Navigation
- Navigate(Route) app:2201: pushes history (max 100), resets page state unless the target is NowPlaying, and exits fullscreen.
- Back / Forward app:2228/2235: pop history and restore that page's filter, sort and cursor.

Playback
- PlayContext{context,track} app:2242: plays a context with an optional offset uri. Engine load or PUT /me/player/play. May auto-open Now Playing (setting). Waits with "Starting playback…" if the engine is still starting.
- PlayUris{uris,index} app:2249: plays a list, sending at most 500 uris centered on the clicked one.
- PlayLocal{index} app:2257: plays Local Files from an index. Scans first if needed ("Reading your local files…"). Uses the local rodio player and pauses Spotify.
- Radio(uri) app:2269: engine load_radio when local (toast "Starting radio"); otherwise plays the seed and Spotify autoplay continues.
- TogglePlay app:2281: pauses/resumes, in this order of priority: the DJ voice; the local file player; resume here if the old device vanished; transfer to here if nothing is loaded; then engine or PUT /me/player/pause|play.
- Next / Prev app:2335/2351: cancel the DJ; local player next/prev, or engine/POST /me/player/next|previous.
- Seek(ms) app:2367: local player or engine seek / PUT /me/player/seek; resyncs the OS media widget.
- SeekBy(delta) app:2390: relative seek. Used by ⇧⌘←/→ (±5 s), episode buttons (±15 s) and media keys (±10 s).
- Volume(f32) / VolumeCommit app:2395/2396: sets volume (engine, or PUT /me/player/volume throttled to 250 ms, commit on release). Persists settings.volume.
- ToggleMute app:2400: remembers the previous volume (at least 0.05) and sets 0; toggling again restores it.
- SetShuffle(bool) app:2408: engine or PUT /me/player/shuffle.
- CycleRepeat app:2419: off → context → track → off; engine or PUT /me/player/repeat.
- Queue(Vec<uri>) app:2436: one POST /me/player/queue per uri, at most 100. Toast "Added to queue", "Added N songs to the queue" or "Added the first 100 of N songs to the queue".
- QueueCollection(uri) app:2445: fetches a playlist, album, show, Liked Songs or Your Episodes (local files skipped), then queues it. Error "There's nothing in it to add".
- ClearQueue app:2459: there is no Web API for this, so it restarts the current context at the current position. Toast "Queue cleared".
- Transfer(device_id) app:2474: PUT /me/player {device_ids, play}. Toast "Switched device".

Library
- Save{uri,save} / SaveAll{uris,save} app:2480 (fn save app:3064): like/unlike, save/remove or follow/unfollow by uri kind (track, episode, album, audiobook, playlist, show, artist), 50 per request with a /me/library fallback. The heart updates optimistically and reverts on failure. Toasts per kind (section 12).
- AddToPlaylist{playlist,uris} app:2482: POST /playlists/{id}/tracks in chunks of 100 (falls back to /items). Toasts "Added to playlist" / "Added N songs to playlist" / partial.
- RemoveFromPlaylist{playlist,uris} app:2501: DELETE /playlists/{id}/tracks (falls back to /items). Toast "Removed from playlist".
- MovePlaylistItem{playlist,from,to} app:2513: PUT /playlists/{id}/tracks with range_start / insert_before (drag reorder).
- CreatePlaylist{name,uris} app:2517: POST /me/playlists (falls back to /users/{me}/playlists), private, adds the uris, then navigates to the new playlist. Toast "Playlist created".
- UpdatePlaylist{id,body} app:2544: PUT /playlists/{id} (name, description, public, collaborative). Toast "Playlist updated".
- DeletePlaylist(id) app:2547: DELETE /playlists/{id}/followers (unfollow). Toast "Removed from Your Library". Leaves the page if it was open.
- Pin(uri) / Unpin(uri) app:2553/2559: local only, settings.pins (newest first), persisted.
- CreateFolder{name,add} app:2563: local Glass folder in settings.folders, optionally holding a playlist; opens expanded. Toast "Folder created".
- RenameFolder{id,name} / DeleteFolder(id) app:2570/2576: local settings only. Deleting keeps the playlists.
- MoveToFolder{playlist,folder:Option} app:2580: moves into a folder, or None removes it from all folders. Local settings.

UI
- Copy(text) app:2591: clipboard. Toast "Copied to clipboard".
- Toast(text) app:2595.
- SetRight(panel) app:2596: opens that side panel, or closes it if that panel is already open. Devices starts polling immediately.
- ToggleRight app:2607: shows/hides the side panel (settings.right_open; always forced closed at launch, app:1027).
- ToggleMini app:2610: the miniplayer window.
- ToggleFullscreen app:2611: OS fullscreen plus the fullscreen now-playing UI.
- Invalidate(key) app:2615: marks cached data stale; failed loads retry.
- RestartEngine app:2616: drops and restarts the librespot engine.
- RescanLocal app:2621: re-reads the local folders (queued if a scan is running; clears the list if there are no folders).

Sharing, tags and downloads
- SharePair{id,yes} app:2871: answers the pairing dialog (share module, LAN).
- ShareForget(key) app:2877: unpairs a computer.
- EditTags(path) app:2634: opens the tag editor modal. Toast "That file isn't there any more" if the file is gone.
- ChooseCover app:2642: native image picker (jpg/png) for the tag editor; shows a preview.
- SaveTags app:2657: validates, then writes tags on a thread → Msg::TagsSaved.
- Download{uri,name} app:2672 (fn app:3014): keeps the collection's items in the encrypted librespot cache (downloads.json). Toast "Downloading N items".
- RemoveDownload(uri) / RemoveAllDownloads app:2682/2694: toasts "Removed download" / "Removed all downloads".
- SaveSong(item) app:2673 (fn app:1131): Share ▸ "Download as M4A/MP3". Finds the audio on JioSaavn or YouTube Music and saves a file to the save folder, 3 at a time.
- SaveSet{uri,name,kind,items} app:2674 (fn app:1150): saves a playlist, album or artist's top songs into a folder named after it; writes a playlist file for playlists.
- StopSaveSet(uri) app:2675: stops after the current songs. Toast "Stopping “X” after the songs already downloading".
- ClearAudioCache app:2686: prunes the streaming cache and keeps downloads.

Assistant (handlers in assistant_app.rs)
- AiSend(text) app:2765: sends the request to OpenAI (agent with tools).
- AiCancel: stops the running request.
- AiRetry: resends the last request.
- AiNewChat: cancels and clears the transcript.
- AiSignIn{reconsent,new_account}: ChatGPT OAuth sign-in. AiCancelSignIn cancels it. AiSignOut revokes the session. AiSelectAccount(id) switches saved accounts.
- AiApprove(id) / AiDecline(id): answer an approval card (queue, like or save a draft).
- AiSaveDraft{draft,replace}: saves a preview as a Spotify playlist (new, or added to an existing one) or a local playlist (new, or replacing one).
- AiDiscardDraft(id). AiEditDraft{draft, Rename|Move|Remove}.
- AiUndo: undoes the assistant's last queue or playlist change.
- AiRefreshModels; AiSetModel(slug) (config.toml assistant.model); AiSetBilling(ChatGptPlan|ApiKey) (config.toml).
- AiEnable(bool) (config assistant.enabled); AiUseSpotify(bool) (config assistant.spotify).
- AiSaveApiKey / AiForgetApiKey: keychain.
- AiDismissWelcome: closes the welcome modal. AiLimitAcknowledged: clears the usage-limit notice.

Local playlists (playlists.rs, local_playlists.json)
- PlayLocalPlaylist{id,index} app:2807: plays the files still present. Toast "None of this playlist's files are in your local folders" if none are.
- DeleteLocalPlaylist(id) app:2825: undoable. Toast "Playlist deleted (undo is on this page)".
- AddToLocalPlaylist{id,paths} app:2830: toasts "Added to “X”", "Added N songs to “X”", "Already in “X”".
- CreateLocalPlaylist{name,paths} app:2839: toast "Local playlist created", then navigates to it.
- RemoveFromLocalPlaylist{id,index} app:2846: toast "Removed from playlist (undo is on its page)".
- PlayLocalTracks{indices,start} app:2850: plays a list of Local Files indices.
- UndoPlaylistChange app:2913: undoes the last local playlist change. Toast "Removed “X”" / "Restored “X”" / "Brought back “X”" / "Nothing to undo".

Updates, sharing and files
- CheckForUpdates app:2858: manual GitHub update check.
- RestartToUpdate app:2859: installs and relaunches.
- SendToDevice{peer,paths} app:2882: sends local songs to another Glass on the LAN.
- GetFromDevices(uris) app:2894: fetches a playlist's missing spotify:local: songs from your other computers.
- AddFiles{paths,target} app:2902: copies files out of temporary folders, reads them, adds their folders as Local Files sources, then adds them to the target playlist (Spotify, as spotify:local: uris, or local).

Lyrics
- LyricsRetry app:2698: forgets the cache entry and any manual pick, then looks again.
- LyricsPick(record) app:2708: a manual LRCLIB pick, persisted in lyrics_choices.json.
- LyricsNoneFit app:2719: records "no lyrics" for this song.
- LyricsOpenPicker app:2727: shows the ambiguous candidates, or searches LRCLIB for "title artist".
- LyricsSearch(q) app:2739; LyricsClosePicker app:2744.
- LyricsProvider(p) app:2745: config lyrics.provider.
- LyricsOffset(ms) app:2746: config lyrics.offset_ms, clamped.
- LyricsSongOffset(ms) app:2751: this song's offset, in lyrics_choices.
- LyricsTranslation(mode) app:2750: config lyrics.translation (off / below / only).
- ClearLyricsCache app:2758: toast "Cleared saved lyrics".

Account
- SignIn app:2918: Spotify OAuth PKCE in the browser.
- CancelSignIn app:2919.
- LogOut app:2924: forgets session.json and clears all state.

======================================================================
3. MSG VARIANTS (app:447-525; handle() app:3482-3779)
======================================================================
- FilesRead(target, paths, tracks) app:449/3655: adds new source folders and rescans, then files_read. Toasts "Added to Local Files" / "Nothing there Glass can play…".
- OtherCopy(Request) app:451/3668: another Glass launched. Focus brings this window forward; Quit closes so the new build takes over.
- DjClips(gen, result) app:454/3666: the DJ voice clip is ready (or failed), then the held song resumes.
- Update(result, manual) app:457/3667: update check result; sets the Update state and toasts.
- Share(Event) app:458/3665: share events Peers, Pair (opens the pairing dialog and asks for attention), PairDone, Transfer (toasts), Received (adds the inbox folder; toast "Got N songs from X").
- SignedIn(gen, result) app:460/3489: creates the Api, fetches /me, starts the engine. Errors auto-retry 2/5/15/30 s up to 8 times; the sign-in screen shows "Can't reach Spotify. Trying again…".
- Data{key,result,generation} app:461/3527: cached GET result. Stale answers are kept but refetched; a failed background refresh keeps the old data.
- Player(result) app:467/3565: /me/player state → apply_player_state.
- PlayerPushed app:469/3557: a Spotify push; triggers an immediate poll.
- Queue(result) app:470/3571: /me/player/queue. On error the panel shows the error and retries in 2 s.
- SavedFailed(uris) app:471/3580: back off saved-state checks for 10 s.
- Devices(Value) app:472/3586: /me/player/devices.
- Saved(pairs) app:473/3587: saved/followed flags (the hearts).
- EngineReady(result) app:474/3593: engine up (status "Ready · this computer appears as “X”", downloads start, pending play runs), or failed (retry after 5/15/30/60/120 s; first failure toasts "Can't play on this computer right now; other devices still work").
- Engine(EngineEvent) app:475/3831: SessionLost (reconnect), Unavailable (toast "This song isn't available"), Dj, Loading, Track, Playing, Paused, Position, EndOfTrack, Stopped, Volume, Shuffle, Repeat.
- Done{toast,invalidate,refresh_player,navigate} app:476/3630: a successful write; toast, invalidate, poll, navigate.
- Failed(err) app:482/3644: toast; 403 on player becomes the Premium message, 404 on player becomes the no-device message.
- Lyrics{generation,resolved} app:484/3755: lookup result.
- LyricsSearch{generation,result} app:488/3758: picker search results.
- Media(MediaControlEvent) app:492/3669: OS media keys. Play, Pause/Stop, Toggle, Next, Previous, Seek ±10 s, SeekBy, SetPosition, SetVolume. Pause only pauses (AirPods safety).
- LocalScanned(tracks) app:493/3744: scan done; runs any pending local play; rescans if one was requested meanwhile.
- LocalAdded{base,all} app:495/3736: incremental add (a full rescan if the library changed meanwhile).
- TagsSaved(path,result) app:499/3700: closes the editor, refreshes the track, toast "Saved song info". On failure: an error in the modal, or toast "Song info wasn't saved. {e}".
- SaveSong(key,status) app:501/3699: progress of a song-to-file save, then the final toasts.
- AiWrote(Wrote) app:503/3693: the result of a Spotify write the assistant asked for.
- LyricsTranslated{key,result} app:505/3694: translation arrived.
- Ai{generation,event} app:510/3692: streaming assistant events.
- AiRegistered(id) app:514; AiSignedIn(result) app:515; AiModels{key,result} app:516; AiSignedOut{confirmed} app:520 (ChatGPT account lifecycle, model list).
- Artwork(uri,path) app:524/3763: local cover for the OS Now Playing widget.

======================================================================
4. MODALS / DIALOGS (Modal enum app:151-179; drawn in modals.rs)
======================================================================
Esc or a backdrop click closes any modal.
- Shortcuts (modals:24): opened by ⌘/ or the command "keyboard shortcuts". Title "Keyboard shortcuts", the line "Press ⌘ / to show or hide this.", a scrollable list of sections (SHORTCUTS, mod.rs:445), and [Close].
- About (modals:47): opened only by the command "about glass". "Glass", "Version x", "A quiet, transparent Spotify player, written in Rust.", a credits paragraph ("…Glass isn't affiliated with Spotify. Playback requires Spotify Premium."). Buttons [What Glass stores and sends] (opens Activity), [Open-source licenses] (if a notices file exists), [Close].
- EditPlaylist{id,name,desc,public,collaborative} (modals:75): opened by clicking your own playlist's cover, or the playlist menu "Edit details".
  - Title "Edit details", with the cover (160px).
  - Fields: name ("Add a name"), multiline description ("Add an optional description").
  - Checkboxes: "Show on your profile (public)" (disabled while collaborative, hover "Collaborative playlists can't be public"); "Collaborative (others can add songs)".
  - [Save] does nothing if the name is blank, and forces public=false when collaborative. [Cancel].
- NewFolder{name,add} (modals:104): opened by ⇧⌘N, Library sort menu "new folder", playlist menu "Create folder", or Move to folder ▸ "＋  Create folder". Title "Create folder", name field (default "New Folder", autofocused), note "Folders are kept by Glass on this computer.", [Create] (or Enter; blank names are blocked), [Cancel].
- RenameFolder{id,name} (modals:126): opened by the folder right-click "Rename" or the Folder page "Rename". Title "Rename folder", field, [Save] / Enter (blank blocked), [Cancel].
- Confirm{title,body,confirm,action} (modals:216): title, body, a [{confirm}] button and [Cancel]. The instances:
  - Log out (⇧⌘W or command): "Log out?" / "Glass will forget your Spotify session on this computer." / "Log out".
  - Delete folder: "Delete folder?" / "This will delete “X”. Its playlists stay in Your Library." / "Delete".
  - Delete playlist: "Delete from Your Library?" / "This will delete X from Your Library." / "Delete".
  - Delete local playlist: "Delete “X”?" / "The playlist is removed from Glass (you can undo). Your music files aren't touched." / "Delete".
  - Clear queue: "Clear queue?" / "You can't undo this." / "Clear".
  - Remove all downloads: "Remove all downloads?" / "Downloaded songs will stream again when you play them." / "Remove".
- EditTags(TagEditor) (modals:145): opened by EditTags (E key on a song row, the hover edit icon "Edit song info  E", or a menu "Edit song info…").
  - Title "Edit song info", with the filename under it.
  - Cover (140px) with [Choose image…] and [Remove] ("Remove the cover from the file").
  - Fields: Title, Artist, Album, Year (hint "e.g. 2019"), Genre. Enter in a field saves.
  - Warning: "Playlists find local songs by title, artist and length, so changing the title or artist can stop a playlist from finding this file."
  - Errors show inline. Validation (local.rs:363-369): "A song needs a title." and "The year should be four digits, like 2019."
  - [Save] (reads "Saving…" while disabled), [Cancel].
- AiWelcome (modals:200): opened once, on the first ChatGPT sign-in with plan usage enabled (assistant_app.rs:1351). "You're using your ChatGPT plan" / "Eligible usage in this app uses your ChatGPT plan. Manage usage in your ChatGPT settings." [Got it] [Manage usage].
- Pairing dialog (modals:245): not a Modal variant. It comes from app.pairing (a share event) and takes priority over any other modal. "Pair with {name}?", then "{name} wants to share songs with this computer." (incoming) or "To share songs with {name}, the two computers pair first.", then "Pair them only if {name} shows this same code:", the big mono code, and "Paired computers share without asking again. Settings → Your Library lists them." [Pair] [Don't pair]; closing counts as no.
- Native pickers: the "Add local files" picker (audio extensions mp3, m4a, mp4, flac, ogg, oga, wav, aac, aif, aiff); folder pickers ("Add a source", Save songs "Change…"); the cover image picker.

======================================================================
5. RIGHT PANEL AND NOW PLAYING
======================================================================
5a. Side panel (RightPanel app:111 = Queue, Devices, Lyrics, Assistant; right.rs:12)
- Never shown on Now Playing. Width 340px (400 for the assistant), at most 45% of the content area.
- Header: the title (queue / devices / lyrics / assistant), then buttons: "Close  esc", "Assistant  ⌘ J", "Devices", "Lyrics", "Queue", "Now playing  ⌥ ⇧ J".
- Queue (right.rs:202):
  - States: loading; "couldn't load the queue" + error + [try again] (hidden while the error is "isn't ready"); empty "Add to your queue" / "Right-click a song and choose Add to queue."
  - "Now playing" with the current row, then "Next from: {context name}" or "Next up", with a "Clear queue" link that opens a confirm.
  - Rows: cover, title, artists. Clicking the current row toggles play; clicking a later row plays from it (PlayUris of the rest). Right-click opens the track menu.
  - Polled every 8 s while visible.
- Devices (right.rs:284):
  - A card "Current device" (icon by type; the active device's name, or "{device name} (this computer)", or "No active device"), plus the engine status.
  - "Select another device", with rows: icon, "name" or "name (this computer)", and the subline "{type} • volume N% • can't be controlled remotely". Click runs Transfer (not for restricted devices).
  - This computer is added to the list if missing. Empty: "No other devices found. Open Spotify on another device on your account and it'll appear here."
  - Polled every 5 s while visible.
- Lyrics (right.rs:62):
  - "nothing playing" / "Lyrics follow the song that's playing."; loading.
  - Non-found outcomes show a title and body ("instrumental"; "no lyrics found"; "several versions match" / "Choose one on the now-playing screen."; "{provider} lyrics aren't available"; "couldn't load lyrics"), each with [open now playing].
  - Found: the current line is bright with an accent bar and auto-scrolls; clicking a synced line seeks. The translation mode applies. Footer: "{provider} · {timing}".
- Assistant (assistant.rs:281): see 5f.

5b. Now Playing screen (Route::NowPlaying, nowplaying.rs:23). NpView (app:41) = Lyrics (default), Queue, Devices, Playlist.
- Left column (nowplaying.rs:175):
  - Blinking "now playing_", artwork (crossfade with album colors; click goes to the album, tooltip "Go to album").
  - Title, artist links (local tracks show plain artist text), ⋯ track menu and heart (Spotify items only). With nothing playing: "nothing playing" / "pick something from your library".
  - Seek rail with times.
  - Transport: Shuffle "Shuffle  ⌘ S", Previous "Previous  ⌘ ←", Play 72px, Next "Next  ⌘ →", Repeat "Repeat  ⌘ R" (repeat-one icon in track mode).
  - Mute/Unmute, volume rail, Devices button "Devices  D" (toggles the Devices view).
- Right column:
  - Queue view: "queue / up next" and the queue list. On Now Playing it also takes j/k/↑/↓ and Enter.
  - Devices view: "devices" and the devices list.
  - Playlist view (nowplaying.rs:61), described below.
  - Lyrics view (nowplaying.rs:486), described below.
- Playlist view (P):
  - A local file is playing: "playing from / local files" and its queue list; click plays from that row.
  - Otherwise "playing from {name}", where the name is a link to the page. Supported contexts: playlist, album, artist top tracks, Liked Songs. The tracklist ignores the page's filter/sort, takes keys, and scrolls to the playing song on first open.
  - Any other context: "not from a playlist" / "This music isn't playing from a playlist, album, artist or Liked Songs. The queue (Q) shows what's next." [show the queue].
- Lyrics view states:
  - Picker open (see below).
  - "nothing playing" / "Lyrics follow the song that's playing."
  - "finding lyrics" / "Looking on LRCLIB…" (or Spotify…).
  - Instrumental: header "lyrics / instrumental", "♪  instrumental", "LRCLIB lists this recording as instrumental." [it has lyrics — choose].
  - Not found: "no lyrics found", "{provider} has no lyrics that match this recording." plus the other providers tried. [search LRCLIB] [look again].
  - Ambiguous: header "lyrics / choose", "several versions match", "They have different lyrics. Pick the one that fits what's playing.", the candidate list, [search] [none of these].
  - Unavailable: header "lyrics / unavailable", "{provider} lyrics aren't available", the reason, and [use LRCLIB (auto)] (tooltip: sets lyrics.provider = "auto").
  - Failed: "couldn't load lyrics from {provider}", the error, [try again].
- Found lyrics (nowplaying.rs:570):
  - Header "lyrics / {unsynced|line sync|word sync|syllable sync}", with a tooltip explaining the timing.
  - Translation note at top left: "translating…", "from {lang} · {by}", "already in {lang}", an unavailable reason, or "translation failed: e". Action links: "open Language & Region", "assistant settings", "try again".
  - Plain lyrics scroll freely. Synced lyrics have karaoke word/syllable highlighting, animated line scroll (glide / fade / pop / none; Reduce Motion turns movement into a fade), and click a line to seek ("Play from this line").
  - Scrolling with the wheel pauses auto-follow and shows "↓ back to current line".
- Lyrics footer (nowplaying.rs:882):
  - Right side: a "文A original|+ translation|translated" toggle (T); "wrong lyrics?" (opens the picker); "−" ("Show lyrics 0.1 s later for this song  ["); "timing ±x.x s" (double-click resets); "+" ("Show lyrics 0.1 s earlier for this song  ]").
  - Left side: "{provider} · exact match|matched by search|your pick · {first provider} had none · {lang}". Tooltip: "Lyrics from {provider} (record id): “title” — artist · album · length.\nLyrics belong to their rights holders and are shown here for personal use."
- Lyrics picker (nowplaying.rs:984):
  - Header "lyrics / choose", "choose lyrics for this song", and "title — artists · length".
  - A "search LRCLIB" field; Enter searches.
  - Results: loading; "search failed"; "nothing found" / "Try fewer words, or just the title."; or candidate rows ("title — artist"; "album · length (same length / ±N s) · timing or instrumental"; “preview”).
  - [none of these] ("Show no lyrics for this song"), [cancel].
- Visualizer (visualizer.rs): below the right column when settings.visualizer is on (V, the pill, or the theme panel "show  V"). Styles bars / mirror / wave, optional glow. At rest it is a thin line; if music plays on another device it shows "the visualizer follows music played on this computer".
- ⌘K on Now Playing: the command bar appears centered at the top, only while in use.

5c. Fullscreen (mini.rs:102; ⌃⌘F on macOS, F11 elsewhere, or the "full screen" command): a dimmed cover filling the background, a large cover, title (36), artist, controls (Shuffle, Previous, Play 64, Next, Repeat) and a seek rail with times. Top-right button "Exit full screen  esc". Modals still draw on top.
5d. Miniplayer: see section 9.
5e. Theme panel (themepanel.rs, ⌘T or the command "theme panel"): a floating panel at the bottom-right of the content area.
- Title "theme.toml" with Close "Close  esc"; the folder path; a status pill "live preview" or "file has errors".
- Tabs:
  - theme: [theme] fields background, foreground, accent, border, opacity, blur. Color fields have a swatch that opens a picker.
  - lyrics: current, other, highlight; size slider 0.5–2; align left/center/right; position top/upper/middle; spacing 0.5–2.5; fade 0–1; animation glide/fade/pop/none; speed 0.25–3; bold line; font.
  - visualizer: a "show  V" checkbox; style bars/mirror/wave; color, color2; opacity 0.1–1; bars 12–128; height 0.15–0.6; sensitivity 0.25–4; smoothing 0–0.95; glow.
- Theme tab only: palette swatches ("Use #hex as the accent"), [reset] ("Restore the six [theme] values to the defaults"), and a link "edit theme.toml ›". Lyrics and visualizer tabs: "saved as you go" with [reset] ("Back to the defaults for [table]").
- Edits are written to theme.toml live. Esc in a field reverts it.

5f. Assistant panel (assistant.rs:281)
- Off: "assistant" / "The assistant is off. It's optional: turn it on in Settings → Assistant." [open settings].
- Header: "Using ChatGPT plan · Manage usage", or "Your API key · billed by OpenAI". Model listing errors show "Couldn't list models: e".
- Not ready (the reason from ai_ready, assistant_app.rs:110): "The assistant is off. Turn it on in Settings → Assistant." / "Sign in with ChatGPT in Settings → Assistant to use your ChatGPT plan." / "You're signed in, but permission to use your ChatGPT plan wasn't granted. Enable it in Settings → Assistant, or use your own API key." / "Usage limit reached for your ChatGPT plan or this app." / "Add your OpenAI API key in Settings → Assistant." Each comes with [open settings].
- Usage limit card: "ChatGPT", "Usage limit reached", "Review your plan or this app's limit in ChatGPT settings.", [Manage usage], [try again].
- Transcript:
  - Lines labelled "you" or "glass"; notes as "· text"; errors in accent with "retry" (and "Manage usage" for limit errors).
  - Approval cards: "The assistant asks to play N songs." / "…play N songs next." / "…add N songs to your Spotify queue." / "…add N songs to the end of the queue." / "…add N songs to your Liked Songs." Buttons [approve] [decline].
- Draft cards:
  - Label "Spotify preview" or "preview"; the name is editable in place.
  - Meta: "N songs · dur", plus " · would be added to “X”" or " · would replace “X”".
  - Rows with ↑ ("Move up"), ↓ ("Move down"), × ("Remove from the preview"). Rows that can't be resolved read "(song details unavailable)" or "(file no longer in the library)".
  - Status: "saved to Spotify ✓" / "saved ✓" with an "open" link; "saving to Spotify…"; or the asked-text.
  - Buttons: the save button ("save" / "save to Spotify" / "replace “X”" / "add to “X”", tooltip "Save this preview (your approval)"), [save as new], then [decline] or [discard].
- Status line: while running, the status ("thinking…") and "stop  esc". Otherwise the model combo (models, "refresh list"; placeholders "loading…" / "no models" / "choose a model"), "N / N tokens · ≈ $x (estimate)" (API key only), "new chat", and "undo" ("Undo the last change the assistant made").
- Composer: "ask, or tell Glass what to play…". Enter sends; ⇧Enter adds a new line; Esc stops.
- Settings section (assistant.rs:408):
  - "Turn on the assistant"; "Let it use my Spotify library and playback"; the data-use notes.
  - "Pay for requests with" my ChatGPT plan / my OpenAI API key.
  - Plan accounts: "Continue with ChatGPT"; "Waiting for the browser…" with [Cancel]; "Saved: X" with [Continue with this account]; for an active account, "· ChatGPT plan usage enabled" or "· plan usage not enabled", then [Enable ChatGPT plan use] [Manage usage] [Sign out] [Add another account], and "Also saved: X" with [Switch].
  - API key: a password field "sk-…" with [Save key], or "A key is saved in …" with [Remove key].
  - Model combo, and the note "GPT-6 Luna is chosen when your account offers it…".

======================================================================
6. CONTEXT MENUS (menus.rs, plus library.rs, local_library.rs, pages.rs)
======================================================================
A "▸" marks a submenu.

Track / episode / chapter (menus::track menus:251). Opened by right-click or ⋯ in tables; ⋯ on the Track, Episode and Now Playing pages; right-click on episode rows and queue rows. If a multi-selection contains the item, it acts on all selected uris. Items in order:
1. "Add to playlist" ▸ containing "＋  New playlist", a separator, then your writable playlists (owned or collaborative; scrolls at 360px).
2. "Remove from this playlist" (only in an editable playlist's table).
3. "Save to Liked Songs" / "Remove from Liked Songs". Episodes: "Save to Your Episodes" / "Remove from Your Episodes".
4. "Add to queue".
5. Separator. Then for songs:
   - "Go to song" (not spotify:local and has an id)
   - "Go to song radio" (not local)
   - "Go to artist" (one artist), or "Go to artist" ▸ listing the artist names (several)
   - "Go to album" (has an album id)
   For episodes instead: "Go to episode", then "Go to podcast" (if a show is known).
6. "Get from your other computers": a playlist's local song that is missing here, while peers are on the network.
7. A playlist's local song that is linked to a file here: separator, "Send to" ▸ peer names (only when peers exist), "Edit song info…", "Show in Finder" (Linux: "Show in folder").
8. Separator, then "Share" ▸ containing:
   - "Copy link to song" (or "…episode"), "Copy Spotify URI", "Open in browser"
   - For Spotify songs only (not episodes or local), a separator and then one of: "Download as M4A|MP3", "Finding it to download as M4A…" (disabled), "Downloading as M4A… N%" (disabled), "Saving as M4A…" (disabled), "Downloaded · Show in Finder", "Download as M4A (try again)".
   The Share submenu is omitted entirely when there is neither a web link nor a save option.

Playlist (menus::playlist menus:339). Opened from library rows, cards, and the playlist page ⋯:
1. "Add to queue"
2. "Get N missing local song(s) from your other computers" (peers present and something missing)
3. "Add local files…" (own or collaborative; opens the file picker)
4. Own: "Edit details", "Delete" (confirm). Others: "Save to Your Library" / "Remove from Your Library".
5. "Download" / "Remove download" / "Remove download (d/t done)"
6. Separator, then "Create playlist" and "Create folder" (the new folder contains this playlist)
7. Own only: "Invite collaborators" (makes it collaborative and private, copies the link, toast) or "Make non-collaborative"; then "Add to profile" / "Remove from profile"
8. "Move to folder" ▸ containing "＋  Create folder", the folder names, "Remove from folders"
9. "Pin playlist" / "Unpin playlist"
10. Separator, then "Share" ▸ containing "Copy link to playlist", "Copy Spotify URI", "Open in browser", a separator, and one of:
    - "Downloading playlist as M4A… d of n" (disabled) with "Stop downloading"
    - "Downloaded · Show in Finder"
    - "Download playlist as M4A" (toast "Still loading this playlist — try again in a moment" if the tracks haven't loaded)

Album (menus::album menus:426):
- "Save to Your Library" / "Remove from Your Library"; "Add to queue"; "Go to artist radio"; "Go to artist"
- "Add to playlist" ▸ (only when the album object includes its tracks)
- "Pin album" / "Unpin album"; Download item
- Separator, then "Share" ▸ containing "Copy link to album", "Copy Spotify URI", "Open in browser", and "Download album as M4A" or its progress states (the folder is named "Artist - Album").

Artist (menus::artist menus:455): "Follow" / "Unfollow"; "Go to artist radio"; "Pin artist" / "Unpin artist"; separator; "Share" ▸ containing "Copy link to artist", "Copy Spotify URI", "Open in browser", and "Download top songs as M4A" or progress (loading toast says "…this artist…").

Show / podcast (menus::show menus:469): "Follow" / "Unfollow"; "Pin podcast" / "Unpin podcast"; separator; "Share" ▸ containing "Copy link to podcast", "Copy Spotify URI", "Open in browser" (no download).

Audiobook (menus::audiobook menus:480): "Save to Your Library" / "Remove from Your Library"; "Pin audiobook" / "Unpin audiobook"; separator; "Share" ▸ containing "Copy link to audiobook", "Copy Spotify URI", "Open in browser".

Profile / user (menus::user menus:491): "Edit profile" (your own profile only; opens spotify.com/account/profile). "Share" ▸ containing "Copy link to profile", "Copy Spotify URI", "Open in browser". There is no Follow; the code says following people isn't supported.

Dispatch (menus:502): track, episode and chapter use the track menu. A Liked Songs tile on Home gets no menu (type "collection").

Library rows and cards (library.rs:663):
- Folder: "Rename" (modal), "Delete" (confirm).
- Specials (Liked Songs, Your Episodes, Local Files): "Open", plus "Download" / "Remove download" for Liked Songs and Your Episodes only. Local Files gets just "Open".
- Everything else: menus::any. The detail pane's ⋯ opens the same menu.

Local track (Local Files song rows; local_library.rs:809). Right-click on an unselected row selects it first; a multi-selection acts on all selected songs.
- "Play" (single song only)
- "Add to playlist" ▸, or "Add N songs to playlist" ▸, containing:
  - Signed in: "＋  New playlist", a separator, the writable Spotify playlists (as spotify:local: entries), a separator.
  - Always: the caps label "only on this computer", "＋  New local playlist" (named "Local Playlist #N"), then the local playlist names.
- "Send to" ▸ or "Send N songs to" ▸ (only with peers)
- Single song only: separator, "Go to album" (LocalAlbum), "Go to artist" (switches Local Files to the Artists view on that artist), "Edit song info…", "Show in Finder".

Local album card / page ⋯ (album_menu local_library.rs:394): "Play"; "Add N songs to playlist" ▸; "Send N songs to" ▸ (with peers); "Go to album".

Local playlist row (pages:1634):
- File present: "Play", "Add to playlist" ▸, "Send to" ▸ (with peers), "Remove from this playlist", "Show in Finder".
- File missing: only "Remove from this playlist".

Local playlists list rows (Playlists view, pages:1514): no menu. Each row has a play icon ("Play"), opens on click, and accepts dropped songs.

Queue item (right.rs:191): the track menu, with no playlist and no selection.

Device rows: no context menu; click to transfer.

Library sort/view dropdown (library.rs:480):
- "sort by": recents, recently added, alphabetical, creator.
- "view as": list, compact, grid.
- "new playlist", "new folder".

Tracklist sort/view menu (tracklist.rs:464): the button shows the current sort with "⌄" (tooltip "Sort and view").
- "Sort by": Custom order, Title, Artist, Album, Date added, Duration. Picking the current sort again flips the direction.
- "View as": List, Compact.

======================================================================
7. COMMAND BAR ⌘K (command.rs)
======================================================================
- The bar (command.rs:214): a "⌕" icon, a ">" prompt, and a "⌘ K" keycap. Placeholder "search your library" on Library, else "search, go to, or run a command". On Library the text also filters the library list; text starting with "?" doesn't filter.
- Placement: above the Library detail pane; at the top-right of other pages; as an overlay on Now Playing.
- Matching (command.rs:144):
  - "?…" gives a single Ask hit.
  - Empty text gives 6 starters: play / pause, queue, devices, lyrics, search spotify, liked songs.
  - Otherwise: commands where every word matches the name or its keywords (max 5); then up to 6 library items (Liked Songs, Your Episodes, Local Files, then playlists, albums, artists, shows, audiobooks), every word matching the name or subtitle; then always "search spotify for “q”". A leading ">" is stripped.
- Running a hit (command.rs:173):
  - A library item: on Library it selects the row; elsewhere it navigates.
  - Search: opens the Search page with the query committed.
  - Ask with a ready assistant: runs AiSend in the background (toast "Asking the assistant…" if the panel is hidden; the answer arrives as a toast truncated to 180 characters with "(⌘ J for more)").
  - Ask otherwise, or empty: opens the assistant panel (leaving Now Playing first) and sends the text or focuses the input.
- Result list (command.rs:257):
  - Glyphs: ">" command, "♪" item, "⌕" search, "?" ask.
  - Right-side hint: the command's shortcut, or "enter". An empty ask reads "ask the assistant, or tell it what to play" (hint ⌘ J); a non-empty one reads "ask the assistant: “q”".
  - Footer "↑↓ move   enter run   esc close". Clicking outside closes it.
- Commands in order, as name [shortcut hint]:
  - play / pause [space]; next song [⌘ →]; previous song [⌘ ←]; shuffle on/off [⌘ S]; repeat: off / all / one [⌘ R]; like current song [⌥ ⇧ B]
  - queue [⌥ ⇧ Q]; devices [⌥ ⇧ D] (both open the Now Playing view instead when on Now Playing); lyrics [L]; now playing [⌥ ⇧ J]
  - theme panel [⌘ T]; reload theme.toml (toast "Reloaded theme.toml" / "theme.toml wasn't applied: e"); open theme.toml in your editor
  - library [⌥ ⇧ 0]; home [⌥ ⇧ H]; search spotify [⌘ L]; liked songs [⌥ ⇧ S]; your episodes
  - albums [⌥ ⇧ 4]; artists [⌥ ⇧ 3]; podcasts [⌥ ⇧ 2]; downloads (Library > downloaded); recents [⌥ ⇧ R]; local files
  - dj (plays the DJ); profile; settings [⌘ ,]; activity log; new playlist [⌘ N]; download what's playing (Download of the context, or the track)
  - miniplayer [⌥ ⇧ M]; full screen [⌃ ⌘ F / F11]; keyboard shortcuts [⌘ /]; about glass; log out [⇧ ⌘ W] (opens a confirm)
  - There is no "audiobooks" command.

======================================================================
8. KEYBOARD SHORTCUTS (mod.rs:506-731 unless noted)
======================================================================
"No text focus" means the shortcut is ignored while a text field has focus.

Global
- Esc (mod.rs:520), skipped when a popup is open or a text field owns it. In order of priority: stop a running assistant request (assistant panel open); close the command bar; close a modal; close the theme panel; exit fullscreen; Back from Now Playing; close the side panel.
- Space: play/pause (no text focus).
- L / Q / P, bare keys with no text focus: open Now Playing on lyrics / queue / playlist. Pressing the same key again while there goes Back.
- V: toggles the visualizer and opens Now Playing when turning it on.
- On Now Playing with the Lyrics view only: T cycles original → original + translation → translated (toast "Lyrics: …"); [ makes this song's lyrics 0.1 s later; ] makes them 0.1 s earlier.
- Playback chords (no text focus): ⇧⌘→ / ⇧⌘← seek ±5 s; ⌘→ / ⌘← next / previous; ⌘↑ / ⌘↓ volume ±10%; ⌘S toggle shuffle; ⌘R cycle repeat.
- Dialog and create chords (no text focus): ⇧⌘N new folder dialog; ⇧⌘W log-out confirm; ⌘N create "New Playlist" immediately (then opens it).
- These work even while typing: ⌘J assistant panel (toggles; leaves Now Playing first and focuses the input); ⌘K command bar; ⌘L Search with field focus; ⌘T theme panel; ⌃⌘F fullscreen on macOS (F11 elsewhere, no text focus); ⌘[ / ⌘] Back / Forward; ⌘, Settings; ⌘/ shortcuts sheet.
- ⌘F (anywhere) or / (bare, no text focus): on Library, focuses the command bar; elsewhere, focuses the page's filter field (tracklist "filter list", Local Files "filter", the Search field, Settings search).
- ⌥⇧ chords (no text focus):
  - B like/unlike the current song
  - 0 Library; H Home; J Now Playing; S Liked Songs; R Recents
  - Q queue (side panel, or the Now Playing view)
  - D devices (side panel; on Now Playing it toggles the Devices/Lyrics view, and bare D does the same there)
  - M miniplayer; ] toggle side panel
  - 1 Library playlists; 2 podcasts (shows); 3 artists; 4 albums; 5 audiobooks
- ⌘A: select all in the current tracklist, or all songs in Local Files' Songs view.

Context keys
- Library list (library.rs:323; no text focus, no modal or popup): j/↓ and k/↑ move the selection; Enter opens; ⇧Enter plays (keeping the current shuffle); 1/2/3 switch the tab to music/podcasts/audiobooks.
- Tracklist (tracklist.rs:198; tables with a header, or with keys enabled): j/k/↓/↑ move the cursor; Enter plays; ⇧Enter queues (the selection if more than one).
- Mouse in tracklists: click selects and plays (or toggles if it's the current song); ⌘-click toggles selection; ⇧-click selects a range.
- Local song rows (local_library.rs:660): j/k/↓/↑; Enter plays; E opens Edit song info. Click plays; ⌘/⇧-click select.
- Queue on Now Playing (right.rs:223): j/k/↓/↑ and Enter (plays from that song).
- Command palette: ↑/↓ move, Enter run, Esc close.
- Assistant composer: Enter send, ⇧Enter new line, Esc stop.
- Modals: Enter in a field submits NewFolder, RenameFolder and EditTags.
- Lyrics picker: Enter in the field searches. Theme panel fields: Enter or blur applies, Esc reverts.
- OS media keys: see Msg::Media in section 3.

Discrepancies in the shortcuts sheet (SHORTCUTS, mod.rs:445): it lists "Now playing: lyrics / queue / playlist / devices  L / Q / P / D", "Lyrics timing for this song  [ / ]" and "Ask from the command bar  ⌘ K, then ?". It doesn't list the library 1/2/3 keys, ⇧Enter, E, or ⌥⇧] beyond "Toggle side panel".

======================================================================
9. PLAYER BAR (nowbar.rs) AND MINI PLAYER (mini.rs)
======================================================================
Player bar (nowbar.rs:38)
- Left side:
  - A local file: cover, title, artist (no links, no heart).
  - A Spotify item: a 64px cover (click opens Now Playing; tooltip "Now playing  ⌥ ⇧ J"); the title as a link (episode → Episode, else Album, else Track); either the DJ caption "DJ X · {line}" in accent or the artist links; and a heart ("Save to Liked Songs" / "Remove from Liked Songs", or the Your Episodes wording for episodes).
  - Nothing: an empty cover and "nothing playing".
- Center:
  - Episodes: "Back 15 seconds" and "Forward 15 seconds" replace shuffle and repeat.
  - Otherwise: Shuffle ("Shuffle is on  ⌘ S" / "Shuffle  ⌘ S") and Repeat ("Repeat all  ⌘ R" / "Repeat one  ⌘ R" / "Repeat  ⌘ R").
  - Always: Previous "Previous  ⌘ ←", a 46px Play/Pause, Next "Next  ⌘ →".
  - Elapsed time, a seek rail (hover shows the time at the pointer; seeks on release or click), total time.
- Right side:
  - Devices "Devices  ⌥ ⇧ D" (highlighted when playing on a remote device), Queue "Queue  ⌥ ⇧ Q", Lyrics "Lyrics  L" (opens Now Playing on lyrics), a volume rail, and a Mute/Unmute icon (mute / low / high).
  - Key hints (nowbar.rs:236), each group shown only if it fits:
    - On Library: "open ↵ move j/k |". On Playlist, Album or Liked: "play ↵ move j/k |". Elsewhere: "commands ⌘ K |".
    - Then "play/pause space".
    - For non-Premium accounts: "| playback needs spotify premium".
    - When remote: "| playing on {device}" (click opens the devices panel).
Mini player (mini.rs:61, ⌥⇧M or the command "miniplayer")
- A separate always-on-top, transparent, frameless window titled "Glass Miniplayer", 300×380 (min 240×300). Drag anywhere to move it.
- Close button "Close Miniplayer"; closing the OS window also clears it.
- Contents: a square cover, title, artist; controls Shuffle, Previous, Play/Pause (40), Next, Repeat (repeat-one icon in track mode); elapsed time, seek rail, duration.
- It shows the local file when one is active, else the Spotify item, else "nothing playing". No heart, volume or queue.
Sign-in screen (mini.rs:136; shown whenever not signed in)
- "Glass" / "A quiet, transparent Spotify player.", then the button "Sign in with Spotify" (while waiting: disabled "Waiting for your browser…" with a "Cancel" link). Auth errors show in red.
- Three bullets: "Your browser opens Spotify's own sign-in page. Glass never sees your password."; "Glass keeps your session in {dir}, readable only by you."; "Playing music needs Spotify Premium. The Activity page lists who Glass talks to and logs its Spotify and lyrics requests."

======================================================================
10. LIBRARY LIST AND DETAIL (library.rs)
======================================================================
- Data sources (library.rs:59): /me/playlists (cap 2000), /me/albums (2000), /me/following?type=artist (2000), /me/shows (500), /me/audiobooks (500); all kept in cache for good.
- Subtitles: "Playlist • {owner, or your display name}", "Album • {artists}", "Artist", "Podcast • {publisher}", "Audiobook • {authors}".
- Specials (library.rs:133):
  - "Liked Songs": "Playlist • N songs" (from /me/tracks?limit=1).
  - "Your Episodes": "Podcast • N episodes".
  - "Local Files": "Playlist • files on this computer", only if Show Local Files is on.
  - Folders: "Folder • N playlist(s)".
- Header: the greeting (lowercase), and the title "library" with a blinking "_" cursor (stops blinking after 10 s idle, or with reduce motion).
- Tabs: music / podcasts / audiobooks. Chips per tab:
  - music: playlists, albums, artists, downloaded
  - podcasts: shows, downloaded
  - audiobooks: audiobooks
- Sort (settings.library_sort, persisted): recents (by order in /me/player/recently-played; default), recently added (added_at), alphabetical, creator.
- Views (settings.library_view, persisted): list (74px rows with a 58px cover), compact (40px rows, no cover, title and subtitle inline), grid (cards; click selects, double-click opens, hover play button).
- Row order (build_rows library.rs:215): specials first, then pins (in pin order), then (Playlists only) folders with their children indented under them, then the remaining items that aren't pinned or in a folder. Children show when the folder is expanded or a filter is active. The command-bar text filters every word against name and subtitle.
- Downloaded chip: lists downloads.collections() for the tab; unknown items show the subtitle "Download".
- Row interactions:
  - The number "01" turns into a play icon on hover (pause if that context is playing), or equalizer bars while it plays. Clicking it plays or toggles; folders and Your Episodes aren't playable.
  - The title turns accent when that context is playing.
  - A chevron "Open" opens the item.
  - Click selects; on a folder it also toggles expand. Double-click opens.
  - Drag a playlist row onto a folder row to move it (MoveToFolder); the folder row highlights on hover.
- Play behavior (library.rs:650): Local Files runs PlayLocal(0); Your Episodes and folders open instead; otherwise SetShuffle (if it needs to change) then PlayContext.
- Pins: managed with "Pin X" / "Unpin X" in the menus, stored in settings.pins. There is no separate pin indicator; pins simply sort first.
- Folders: Glass-local only (settings.folders); Spotify's own folders aren't read. Expanded state is session-only.
- Detail pane (library.rs:761):
  - Large artwork (special icons: a filled heart for Liked Songs, a bell for Your Episodes, a download icon for Local Files, a folder icon for folders); clicking it opens.
  - Kind caps: folder / collection / playlist / album / artist / audiobook / podcast / download. Then the name (28pt).
  - Subtitle: for a playlist its description, or "by X · N songs"; for an album "artists · year"; for an artist "N monthly listeners" (or "artist"); otherwise the row's subtitle.
  - Buttons: [play]/[pause] (not for folders or Your Episodes); [shuffle] (not for folders, Your Episodes or Local Files); a download toggle (non-special non-artist items, plus Liked Songs and Your Episodes; tooltip "Download to this computer" / "Downloading d of t — click to remove" / "Downloaded — click to remove", with a "d/t" label while going); ⋯ "More options" opening the context menu.
  - "featured artists": the 4 artists appearing most on the first 100 playlist tracks (first 50 for Liked Songs), or an album's artists. For an artist the section is "fans also like" (related artists). Shown as round 84px avatars that open the artist.

======================================================================
11. LOCAL FILES (local_library.rs, pages.rs devices and lists)
======================================================================
- Header (local_library.rs:264):
  - "Local Files" and "N songs · N albums · {minutes|hours|days}".
  - A play button "Play the list shown" (toggles when the local player is active; hidden on the Playlists view unless something is active).
  - [Add a source] (folder picker; toasts "Now showing songs from {dir}" / "That folder is already in Local Files"), [Rescan].
  - "Showing songs from: {folders | no folders}", then the sharing device strip.
- Sharing device strip (pages:1480; only when Settings > Share is on):
  - No peers: "Glass on your other computers shows up here when it's on this network and signed in to your account, to send songs back and forth."
  - With peers: "On this network: A, B. Right-click songs to send them; playlists get their missing local songs from these computers."
  - Up to 3 recent transfers: "Sending a song to X" / "Receiving a song from X" / "Sending i of n to X" / "Receiving i of n from X" (with a progress bar) / "Sent N songs to X" / "Got N songs from X" / "Couldn't send to X: why" / "Couldn't receive from X: why".
- States: "local files are turned off" / "Turn on \"Show Local Files\" in Settings."; "loading…"; empty library "listen to local files" / "Add a folder of music with \"Add a source\", or drop songs onto this window."; "Nothing matches the filter."
- Tab chips (settings.local_view, persisted; default Recently Added): Albums, Songs, Artists, Genres, Recently Added, Playlists. A "filter" field on the right (titles, artists, albums, genres).
- Albums view: a card grid sorted by artist (case-insensitive, leading "The" skipped), then year, then title. Cards show cover, title, album artist, and a playing state. The card play button plays the album; click opens LocalAlbum; right-click opens the local album menu.
- Recently Added view: the same grid sorted by newest file added.
- Songs view (local_library.rs:460): sortable column headers, tooltip "Sort by x"; clicking again flips ▴/▾.
  - Columns: Title (with a small cover), Artist (default sort: artist, album, track), Album (shown when wider than 640), Genre and Year (wider than 900), Time.
  - Rows: alternating shading; a play icon on hover or equalizer bars while playing; an edit icon on hover ("Edit song info  E").
  - Click plays the visible order from that song. Drag rows onto a local playlist (dragging a selected row drags the whole selection).
- Artists view: a 240px scrolling list ("N albums · N songs"; click picks the artist). The right side shows the artist name and each album: 96px cover, title (both open LocalAlbum), "genre · year", and the album's songs.
- Genres view: chips "Genre  N" (with "Unknown Genre" last); picking one shows its albums as a grid.
- Playlists view: a "playlists" list of local playlists (play icon, name, "N songs"); click opens LocalPlaylist; rows accept dropped songs. There is no create button here; local playlists are created from "＋  New local playlist" in Add to playlist, or by the assistant.
- Local album page and local playlist page: see section 1.
- Shared rules:
  - Albums are grouped by album artist (else artist) plus album title, case-folded.
  - Unknowns show as "Unknown Artist", "Unknown Album" and "Unknown Genre".
  - Local Files plays in order (no shuffle button).
  - Playlists' spotify:local: entries link to local files by title, artist and length. Missing ones are dimmed, with tooltips: "Local file: {path}", or "This local song isn't on this computer. Right-click to get it from your other computers." (peers present), or "…Put the file in a folder listed under Local Files to play it here."

======================================================================
12. TOASTS AND "UNAVAILABLE" STRINGS
======================================================================
Toasts from app.rs, by line:
- Saving songs as files:
  - 1135 "Only Spotify songs can be downloaded"
  - 1144 "“T” is already saved as M4A"; 1145 "Saving “T” as M4A…"
  - 1178 "“X” has no songs that can be downloaded"
  - 1185 "Saving N song(s) from “X” as M4A…" (artists read "from X's top songs")
  - 1259 "Saved “T” as {file}"
  - 1262 "Couldn't save “T”: why". Reasons: "couldn't find it on JioSaavn or YouTube Music" / "YouTube wouldn't play it" / "YouTube refused the download; try again later" / "the download stalled" / "the server refused it (status)" / "network error: …"
  - 1302-1313 "Saved the song from “X”" / "Saved all N songs from “X”" / "Saved s of t songs from “X”", plus " · N failed", " · N stopped", " · 1 episode or local file skipped" / " · N episodes or local files skipped"
- Sharing:
  - 1644 "Couldn't send to X: why" / "Couldn't receive from X: why"
  - 1646 "Sent N song(s) to X"; 1648 "X already has those songs"
  - 1669 "Got a song from X" / "Got N songs from X"
- Updates: 1705 "Glass v is up to date"; 1709 "Glass v is ready: it installs when you quit, or restart from the top bar"; 1714 "Glass v is out"; 1721 "Couldn't check for updates: e"
- Adding files: 1857 "Nothing there Glass can play (it takes MP3, M4A, AAC, FLAC, Ogg, WAV and AIFF files)"; 1864 "Added to Local Files" / "Added N songs to Local Files"
- Playback start: 2101 "Starting playback…"; 2121 "Couldn't start playback: e"; 2262/2811 "Reading your local files…"; 2273 "Starting radio"
- Queue and devices: 2451 "There's nothing in it to add"; 2471 "Queue cleared"; 2476 "Switched device"
- Playlists:
  - 2490-2492 "Added to playlist" / "Added N songs to playlist" / "Added a of N songs; the rest couldn't be added (e)"
  - 2508 "Removed from playlist"
  - 2533-2534 "Playlist created" / "Playlist created, but not all the songs could be added (e)"
  - 2545 "Playlist updated"; 2548 "Removed from Your Library"
  - 2568 "Folder created"; 2593 "Copied to clipboard"
- Tags and downloads:
  - 2637 "That file isn't there any more"
  - 2679 "Stopping “X” after the songs already downloading"
  - 2684 "Removed download"; 2696 "Removed all downloads"
  - 2690 "Cleared the cache (downloads kept)"; 2692 "The playback engine is still starting; try again in a moment"
  - 3010 "This can't be downloaded"; 3024 "Downloading N item(s)"
- Lyrics: 2760 "Cleared saved lyrics" / 2761 "Couldn't clear saved lyrics: e"
- Local playlists:
  - 2819 "None of this playlist's files are in your local folders"
  - 2827 "Playlist deleted (undo is on this page)"
  - 2833-2835 "Already in “X”" / "Those songs are already in “X”" / "Added to “X”" / "Added N songs to “X”"
  - 2841 "Local playlist created"
  - 2847 "Removed from playlist (undo is on its page)"
  - 2914-2916 the undo text ("Removed “X”" / "Restored “X”" / "Brought back “X”") / "Nothing to undo". Errors: "A playlist needs a name." / "That playlist doesn't exist any more."
- Updating: 2868 "Couldn't update: e"
- Sending and fetching: 2884 "That computer isn't on this network any more"; 2892 "Sending to X…" / "Sending N songs to X…"; 2898 "Getting it from your other computers…" / "Getting N songs from your other computers…"; 2900 "None of your other computers with Glass are on this network"
- Queue add (2981-2984): "Added to queue" / "Added N songs to the queue" / "Added the first 100 of N songs to the queue"
- Save / follow (3074-3082): "Added to Liked Songs" / "Added N songs to Liked Songs" / "Removed from Liked Songs" / "Removed N songs from Liked Songs" / "Saved to Your Episodes" / "Saved N episodes to Your Episodes" / "Removed from Your Episodes" / "Removed N episodes from Your Episodes" / "Added to Your Library" / "Removed from Your Library" / "Followed" / "Unfollowed"
- Engine and errors:
  - 3326 "Couldn't open an audio output device"
  - 3626 "Can't play on this computer right now; other devices still work"
  - 3646 "Spotify refused the playback command (Premium is required to control playback)"
  - 3648 "No active device — pick one in the device panel"
  - 3725 "Saved song info"; 3732 "Song info wasn't saved. e"
  - 3847 "This song isn't available"; 3962 "Skipped explicit content"
  - 2049 config.toml write errors appear verbatim
Toasts from UI and assistant code:
- mod.rs:571 "Lyrics: original" / "Lyrics: original + translation" / "Lyrics: translated"
- command.rs:61-62 "theme.toml wasn't applied: e" / "Reloaded theme.toml"; command.rs:193 "Asking the assistant…"
- menus.rs:400 "Playlist is now collaborative — link copied"; menus.rs:196 "Still loading this playlist|album|artist — try again in a moment"
- local_library.rs:302/304 and settings.rs:272: "Now showing songs from {dir}" / "That folder is already in Local Files"
- assistant_app.rs:340/500 "Assistant: {why}"; :470 the assistant's answer text when the panel is hidden; :1411 "API key saved in the Keychain" (Linux: "system keyring"); :1413 "The key wasn't saved. e"; :1424 "API key removed"
Explanations and "not possible" strings in the UI:
- "Collaborative playlists can't be public" (modals:86)
- "playback needs spotify premium" (bar hint); "can't be controlled remotely" (device row)
- "the visualizer follows music played on this computer"
- "X's voice is off (Settings → Playback): the DJ plays its picks without talking."
- Local songs missing here (tracklist.rs:257-261); "This music isn't playing from a playlist, album, artist or Liked Songs. The queue (Q) shows what's next."
- Spotify lyrics: "Spotify has lyrics only for songs played from Spotify, not local files." and "Glass isn't connected to Spotify right now, so Spotify's lyrics can't be fetched." (lyrics/spotify.rs:12-13)
- Translation (translate_app.rs:113, lyrics/translate.rs): "On-device translation isn't part of this build of Glass." / "Apple's on-device translation is only on macOS." / "{pair} needs downloading for on-device translation: System Settings → General → Language & Region → Translation Languages." / "Apple's on-device translation doesn't support {pair}." / "Couldn't tell what language these lyrics are in."
- Settings: "Apple's on-device translation isn't available here (it needs macOS 26 and a release build): choose auto or assistant."; "Applies when the playback engine restarts." / "Applies after restarting Glass."; " (a development build: it doesn't update itself)"; "…get it from the releases page (this copy can't install it itself)."; on Linux "…build the new package to update."
- Assistant readiness strings (section 5f); "The assistant took too many steps without finishing, so Glass stopped it. Try a more specific request."; sign-in notes ("Signed in, but permission to use your ChatGPT plan wasn't granted…", "You declined in the browser, so nothing changed…", "Your ChatGPT sign-in ended (why). Sign in again in Settings → Assistant.")
- The sign-in screen error "Can't reach Spotify. Trying again…"

======================================================================
13. DEMO MODE AND ENV FIXTURES
======================================================================
- GLASS_DEMO (demo.rs:22, app:848/1013/1028):
  - Runs in its own HOME (a temp folder deleted on quit, or the given folder if the value contains "/") and clears the XDG_* variables.
  - The assistant uses an in-memory secret store and dead 127.0.0.1:9 endpoints. No local scan. Sharing is off. Update checks are off unless GLASS_UPDATE_FEED is set. Several instances may run (instance.rs:136).
  - demo::install (demo.rs:58) fills:
    - me: "Davis", premium, country US.
    - 6 artists: Aurora Lane, The Paper Kites, Nils Frahm, Phoebe Bridgers, Bonobo, Khruangbin.
    - 8 albums: Quiet Hours, Woodland, Says, Punisher, Migration, Mordechai, Glass Garden, Low Tide.
    - 15 tracks, "Golden Hour" … "Undertow"; track 4 ("Kerala") is explicit.
    - 8 playlists: Late Night Drive (description "Slow songs for empty roads."), Focus Flow, Sunday Morning, Discover Weekly (owned by Spotify), Running 170bpm, Rainy Day, Deep Work, Road Trip.
    - Liked count 1284, episodes 7; recently played; top artists and tracks; 8 browse categories (Pop, Hip-Hop, Rock, Jazz, Classical, Focus, Chill, Workout). Every third track is liked; all artists are followed.
    - Playing "Golden Hour" in p0 at 1:04 with shuffle on; queue tracks 2–8.
    - Devices "Davis's iPhone" (Smartphone, 60%) and "Kitchen" (Speaker, 35%).
    - Engine status "Demo mode — no network". Peer "Arch Desktop" with a finished transfer of 12 received songs.
    - Gradient cover art generated in memory.
- GLASS_ROUTE:
  - Demo (demo.rs:170): local | playlist (p0) | artist (a0) | search | settings | activity | liked | home | nowplaying; default library.
  - Non-demo with a saved session (app:1032): nowplaying | home | search | settings | local only.
- GLASS_RIGHT (demo.rs:183): queue | devices | lyrics | assistant (anything else means queue); opens the side panel.
- GLASS_THEME_PANEL (demo.rs:182): any value opens the theme panel.
- GLASS_LYRICS (demo.rs:359): word (default; word-timed via a lyricsfile) | line | plain | instrumental | none | ambiguous (two candidates). Uses an offline in-memory lyrics cache with 6 sample lines starting at 0:52.
- GLASS_SETTINGS_QUERY (demo.rs:194): pre-types the Settings search.
- GLASS_NP_VIEW (demo.rs:200): 1|queue, 2|devices, 3|playlist, anything else lyrics.
- GLASS_VIZ (demo.rs:209): visualizer on, fed a fake signal.
- GLASS_THEME_TAB (demo.rs:213): 0 theme, 1 lyrics, 2 visualizer.
- GLASS_CMD (demo.rs:217): opens the command bar with that text (e.g. "? …").
- GLASS_AI_DEMO (demo.rs:222):
  - Any value: an assistant signed in as "you@example.com" with plan usage enabled; models "GPT-6 Luna" and "GPT-6.1 Sol"; 12 city-pop local tracks; a sample transcript; a Spotify draft "Night Drive — City Pop" awaiting approval; usage 5840/412 tokens.
  - "empty": the same, but with no transcript, drafts, approvals or usage.
- GLASS_SCREENSHOT=/path.png (demo.rs:305): captures at frame GLASS_SHOT_FRAME (default 120), flattens onto a gradient wallpaper, saves, and quits.
- Other variables:
  - GLASS_WINDOW=WxH (main.rs:77): initial window size. The default is 1280×820, min 900×600.
  - GLASS_UPDATE_FEED (update.rs:78): an alternate update feed.
  - GLASS_RELEASE: compile-time release flag (update.rs:25).
  - GLASS_LINUX_BLUR=1 (window_fx.rs:56).
  - GLASS_ONE_INSTANCE (instance.rs:136).
  - GLASS_TEST_TRACK: used in downloads.rs tests only.
  - The CLI flag --minimized (main.rs:65) starts the window hidden.

Main gaps an iOS port should decide on explicitly:
- Spotify-side: radio via the engine, Spotify Connect device transfer, encrypted offline downloads, the DJ voice, desktop notifications.
- Glass-local state: pins and folders are local to Glass (settings.json), not Spotify's.
- Desktop-only features: Local Files with tag editing, LAN sharing and pairing, Save-song-as-file (JioSaavn/YouTube), the miniplayer window, theme.toml/config.toml live files, the Activity page, the updater, autostart, the EQ/mono DSP, and the ChatGPT-plan/API-key assistant.