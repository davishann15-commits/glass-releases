//! The typed records that cross to Swift (UniFFI). Rust reads Spotify's
//! Web-API-shaped JSON (as the desktop does) and hands the app these instead,
//! so the Swift side never interprets Spotify's JSON itself.

/// A named link to another page (an artist, an album, a playlist's owner…).
#[derive(uniffi::Record, Clone, PartialEq, Debug, Default)]
pub struct Link {
    pub name: String,
    /// `spotify:artist:…`, `spotify:user:…`, `spotify:album:…`, or empty (plain text).
    pub uri: String,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    Track,
    Episode,
    Chapter,
    /// A Spotify playlist's `spotify:local:` entry, played from a file on this device when found.
    LocalLink,
    /// A file in Local Files.
    LocalFile,
}

/// A song, episode or chapter in a list.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct Row {
    pub uri: String,
    pub kind: RowKind,
    pub title: String,
    pub artists: Vec<Link>,
    pub album: Option<Link>,
    /// A cover near 64 px (Spotify URL), or `localcover://<path>` for a local file's own.
    pub image: Option<String>,
    pub duration_ms: u64,
    pub explicit: bool,
    /// Greyed out when false (unavailable, explicit while explicit content is off, or a missing local file).
    pub playable: bool,
    /// Why a row is greyed out, or which file a local song plays from.
    pub note: Option<String>,
    /// ISO date it was added to the playlist (empty when the list has none).
    pub added_at: String,
    pub track_number: u32,
    pub disc_number: u32,
    /// A local file's path (Local Files, or a playlist's local song found here).
    pub path: Option<String>,
    /// Episodes: release date, description, resume point and whether it was played through.
    pub release_date: String,
    pub description: String,
    pub resume_ms: u64,
    pub played: bool,
    /// The show (episodes) or audiobook (chapters).
    pub parent: Option<Link>,
}

/// An album, playlist, artist, show, audiobook, profile or category tile.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct Card {
    pub uri: String,
    /// "playlist", "album", "artist", "show", "episode", "audiobook", "track", "user", "collection", "category", "folder".
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub image: Option<String>,
    /// Artists and profiles are round.
    pub round: bool,
    /// Albums: "album", "single", "compilation", "appears_on" (discography chips filter on it).
    pub group: String,
    pub year: String,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArtStyle {
    Square,
    Round,
    /// An artist's wide banner.
    Banner,
    LikedSongs,
    YourEpisodes,
    LocalFiles,
    Folder,
    Dj,
}

/// What the page's save button does, if it has one.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveKind {
    None,
    /// Save to / Remove from Your Library (albums, others' playlists, audiobooks).
    Library,
    /// Follow / Following (artists, shows).
    Follow,
    /// The heart (songs, episodes).
    Like,
}

/// The top of an entity page.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct Header {
    /// The context to play (`spotify:playlist:…`), empty when the page isn't playable.
    pub uri: String,
    /// "Public Playlist", "Album", "Single", "artist", "Podcast", "Profile"…
    pub kind_label: String,
    pub title: String,
    /// Owner, artists, publisher, authors, the show…
    pub links: Vec<Link>,
    /// "12 songs, 48 min" and the like.
    pub meta: String,
    pub description: String,
    pub image: Option<String>,
    pub art: ArtStyle,
    pub save: SaveKind,
    pub can_play: bool,
    pub can_shuffle: bool,
    /// Offered on playlists, albums, shows, Liked Songs and Your Episodes.
    pub can_download: bool,
    /// Your own playlist: edit details, add/remove/reorder songs.
    pub editable: bool,
    /// Collaborative or your own: songs can be added.
    pub can_add: bool,
    pub collaborative: bool,
    pub public: bool,
    pub owner_id: String,
    /// Artists: "N monthly listeners" or followers.
    pub listeners: String,
    /// Playlists: its local songs that aren't on this device (your other devices may have them).
    pub missing_local: u32,
}

/// Where added files go (desktop `FileTarget`).
#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum FileTarget {
    LocalFiles,
    /// A Spotify playlist you can edit (as `spotify:local:` entries).
    Spotify { id: String, name: String },
    /// One of Glass's local playlists.
    Local { id: String, name: String },
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowsStyle {
    /// Playlists, Liked Songs, search: number, cover, title/artists, album, date added, length.
    Table,
    /// Albums: track numbers grouped by disc, no covers.
    Album,
    /// Artist "Popular", a track page's top songs: numbered with covers, no album column.
    Popular,
    Episodes,
    Chapters,
    /// Recents: covers, no numbers.
    Plain,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardsStyle {
    /// A wrapping grid.
    Grid,
    /// One horizontal row ("shelf").
    Shelf,
}

/// One part of a page, in order from the top.
#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum Block {
    Rows {
        id: String,
        title: String,
        rows: Vec<Row>,
        style: RowsStyle,
        /// Play rows inside this context (`spotify:playlist:…`); empty: play the list's URIs.
        context: String,
        /// Your playlist's ID: rows can be removed and reordered.
        playlist: Option<String>,
        /// "show all ›" goes there.
        more: Option<Route>,
        /// Only the first rows show until "See more" (artist Popular: 5 of 10).
        collapsed_count: u32,
    },
    Cards {
        id: String,
        title: String,
        cards: Vec<Card>,
        style: CardsStyle,
        more: Option<Route>,
    },
    /// A titled paragraph (About, Episode Description).
    Text {
        title: String,
        body: String,
    },
    /// Small print (release date, copyrights).
    Lines {
        lines: Vec<String>,
    },
    /// Links in a list (a song's artists).
    Links {
        title: String,
        links: Vec<Link>,
    },
    /// A note in place of content ("songs you like will appear here").
    Empty {
        title: String,
        body: String,
    },
    /// The DJ's banner (Home) or page body.
    Dj {
        caption: Option<String>,
        voice_on: bool,
        playing_here: bool,
    },
    /// Home's grid of recent contexts.
    Shortcuts {
        cards: Vec<Card>,
    },
    /// Search: the top result.
    TopResult {
        card: Card,
    },
    /// Search with no query: recent searches.
    RecentSearches {
        queries: Vec<String>,
    },
    /// Search with no query: Browse all.
    Categories {
        categories: Vec<Category>,
    },
    /// Your own playlist: "Let's find something for your playlist" / "Find more".
    FindMore {
        playlist: String,
        empty: bool,
    },
    /// A category page that didn't load: offer a search for its name.
    SearchInstead {
        query: String,
    },
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    /// A stable color for the tile's stripe, `#RRGGBB`.
    pub color: String,
}

#[derive(uniffi::Enum, Clone, PartialEq, Debug)]
pub enum LoadState {
    Loading,
    Ready,
    /// "couldn't load this" with the reason and [try again].
    Failed { message: String },
    /// A title and body in place of the page ("folder not found").
    Missing { title: String, body: String },
}

/// A page, built from whatever has loaded so far (parts that are still loading are left out).
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct PageModel {
    pub route: Route,
    pub state: LoadState,
    pub header: Option<Header>,
    pub blocks: Vec<Block>,
}

/// Every page Glass has. Now Playing and the side panels are views of their own.
#[derive(uniffi::Enum, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Library,
    Home,
    Search { query: String, filter: SearchFilter },
    Genre { id: String, name: String },
    Playlist { id: String },
    Album { id: String },
    Artist { id: String },
    Discography { id: String },
    Show { id: String },
    Episode { id: String },
    Audiobook { id: String },
    Track { id: String },
    /// Empty ID: your own profile.
    Profile { id: String },
    LikedSongs,
    YourEpisodes,
    LocalFiles,
    Recents,
    Folder { id: String },
    LocalPlaylist { id: String },
    LocalAlbum { artist: String, album: String },
    /// Another device's Local Files (local files sharing), by its peer ID.
    Device { id: String, name: String },
    Settings,
    Activity,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SearchFilter {
    All,
    Songs,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Episodes,
    Audiobooks,
}

// ------------------------------------------------------------------ library

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibKind {
    Playlist,
    Album,
    Artist,
    Show,
    Audiobook,
    LikedSongs,
    YourEpisodes,
    LocalFiles,
    Folder,
}

/// One row of Your Library.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LibItem {
    pub uri: String,
    pub kind: LibKind,
    pub title: String,
    /// "Playlist • Davis", "Album • Aurora Lane"…
    pub subtitle: String,
    pub image: Option<String>,
    pub owner_id: String,
    pub added_at: String,
    /// Place in the last 50 plays (lower is more recent); `u32::MAX` when not played lately.
    pub recent_rank: u32,
    /// Position in the pins (0 first); `None` when not pinned.
    pub pin: Option<u32>,
    /// The Glass folder it's in.
    pub folder: Option<String>,
    /// Folders: how many playlists are in it.
    pub children: u32,
    pub editable: bool,
    pub collaborative: bool,
    pub downloaded: bool,
    /// For a playlist: its description; for an album: artists · year.
    pub detail: String,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibrarySort {
    Recents,
    RecentlyAdded,
    Alphabetical,
    Creator,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibraryView {
    List,
    Compact,
    Grid,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LibraryModel {
    pub state: LoadState,
    /// Specials (Liked Songs, Your Episodes, Local Files), then everything else, sorted as `sort` says.
    pub items: Vec<LibItem>,
    pub sort: LibrarySort,
    pub view: LibraryView,
    /// Downloaded collections (the Downloaded chip): URI and name.
    pub downloads: Vec<Link>,
    /// How far each kept collection has got (desktop's "d/t" while downloading).
    pub download_progress: Vec<DownloadProgress>,
    pub greeting: String,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct DownloadProgress {
    pub uri: String,
    pub done: u32,
    pub total: u32,
}

// ---------------------------------------------------------------- playback

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Repeat {
    Off,
    Context,
    Track,
}

/// What's playing (or loaded): a song, an episode or a local file.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct NowItem {
    pub uri: String,
    pub kind: RowKind,
    pub title: String,
    pub artists: Vec<Link>,
    pub album: Option<Link>,
    /// Episodes: the show.
    pub show: Option<Link>,
    /// The largest cover (Now Playing), and one near 64 px (the player bar).
    pub image: Option<String>,
    pub image_small: Option<String>,
    pub duration_ms: u64,
    pub explicit: bool,
    pub path: Option<String>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct DeviceModel {
    pub id: String,
    pub name: String,
    /// Spotify's type: "Smartphone", "Computer", "Speaker"…
    pub kind: String,
    pub volume_percent: Option<u32>,
    pub is_active: bool,
    /// This iPhone (Glass's own Connect device).
    pub is_this: bool,
    /// "can't be controlled remotely"
    pub restricted: bool,
}

/// The player. Position is `position_ms` at `position_at` (Unix ms) and moves on
/// by itself while `is_playing`: Swift extrapolates it, so this is only sent
/// when something actually changes.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct PlaybackModel {
    pub item: Option<NowItem>,
    pub context_uri: String,
    /// The context's name when known ("Late Night Drive"), for "playing from".
    pub context_name: String,
    pub is_playing: bool,
    pub position_ms: u64,
    pub position_at: u64,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub volume: f32,
    pub muted: bool,
    /// The device playing (`None`: nothing is active anywhere).
    pub device: Option<DeviceModel>,
    /// Playing on this iPhone (its own Connect device), or from Local Files.
    pub here: bool,
    /// A file from Local Files is playing (Glass's own player).
    pub local_file: bool,
    /// What the DJ (X) is saying.
    pub dj_caption: Option<String>,
    pub premium: bool,
    /// "Ready · this iPhone appears as “Glass”" and the like.
    pub engine_status: String,
    /// The engine is up: this device can play.
    pub engine_ready: bool,
    /// Playback here waits for a new engine (after its connection died): Swift asks
    /// iOS for background time meanwhile, or it would suspend Glass before it resumes.
    pub reconnecting: bool,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct QueueModel {
    pub current: Option<Row>,
    pub next: Vec<Row>,
    /// "Next from: Late Night Drive", or "Next up".
    pub next_from: String,
    pub loading: bool,
    pub error: Option<String>,
    /// Local Files' own queue (no Spotify).
    pub local: bool,
    /// The first `queued` of `next` were queued by you ("Next in queue"); the rest come
    /// from what's playing ("Next from …").
    pub queued: u32,
    /// `next` can be reordered (playing on this iPhone): `positions[i]` is row i's place
    /// in the player's own list, for `move_in_queue`.
    pub positions: Vec<u32>,
    /// Covers to fetch ahead: the next songs' large and small ones.
    pub prefetch: Vec<String>,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct DevicesModel {
    pub devices: Vec<DeviceModel>,
    pub loaded: bool,
    pub engine_status: String,
}

// -------------------------------------------------------------------- auth

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthPhase {
    SignedOut,
    SigningIn,
    SignedIn,
}

#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct AccountModel {
    pub phase: AuthPhase,
    pub error: Option<String>,
    pub user_id: String,
    pub display_name: String,
    pub email: String,
    pub country: String,
    /// "premium", "free", "open"…
    pub product: String,
    pub image: Option<String>,
    pub demo: bool,
}

/// Swift opens this in ASWebAuthenticationSession; the sign-in finishes when
/// Spotify's redirect reaches the core and the session sees `callback_scheme`.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct SignInRequest {
    pub url: String,
    pub callback_scheme: String,
}

// ------------------------------------------------------------------- misc

/// A request in the Activity log.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct LogEntryModel {
    /// "14:03:22"
    pub time: String,
    pub method: String,
    pub path: String,
    pub status: String,
    pub millis: u64,
}

/// A file Glass keeps, for the Activity page.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct StoredFile {
    pub label: String,
    pub path: String,
    /// "12 KB", or "not present".
    pub size: String,
    pub note: String,
}
