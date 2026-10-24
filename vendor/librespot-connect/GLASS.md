# librespot-connect 0.8.0, patched for Glass

This is librespot-connect 0.8.0 from crates.io (MIT licensed, © the
librespot contributors, https://github.com/librespot-org/librespot) with
support for Spotify's DJ. `Cargo.toml` in the app uses it via
`[patch.crates-io]`; everything else is unchanged.

Why: resolving the DJ (`spotify:playlist:37i9dQZF1EYkqdzj48dyYq`) through
`context-resolve` returns no songs, only metadata with a
`lexicon_context_url`
(`hm://lexicon-session-provider/context-resolve/v2/session?contextUri=…`).
That session returns the mix a page at a time:
songs, each page linking the next with `next_page_url`, and the DJ's lines
as `narration.intro.*` / `narration.outro.*` metadata on songs. Upstream
plays the empty context (nothing happens) and only follows `page_url`.

Changes (all marked `Glass` in the source):

- `src/lib.rs` — `glass` (the hook below) is public.

- `src/context_resolver.rs` — a context that resolves to no songs but has a
  `lexicon_context_url` is fetched from that session instead; an `hm://`
  address resolves directly (the next page). Later pages come back as a
  single page, which is wrapped into a context. `ResolveContext::append_context`
  is `pub(crate)` so `spirc.rs` can ask for the next page.
- `src/state/context.rs` — a context remembers its `next_page_url`;
  `ConnectState::next_page_url()`.
- `src/spirc.rs` — near the end of a context that has a next page, the next
  page is appended instead of starting autoplay; `load_track` calls the
  hook below and may load the song paused.
- `src/glass.rs` (new) — `set_track_hook`: the app hears about each song
  as it loads (with its metadata and the next song's, and whether it's to
  play or was loaded paused), and can hold a playing one's start (Glass lets
  the DJ speak first). The hook can be set again (each new player sets its
  own, so a reconnect never leaves an older one in place). `TrackStart` also
  says whether the song before played to its end (`after_end`, set around
  `handle_next` for `EndOfTrack` in `src/spirc.rs`): a skip from another app
  shouldn't get the DJ's closing line for the song skipped.
- `src/state.rs` — the device reports `supports_dj`, so other Spotify apps
  can start the DJ on Glass.
- `Cargo.toml` — `http` and `protobuf-json-mapping`.

The DJ's voice itself is in the app (`src/dj.rs`): each line's SSML is
spoken by Spotify's text-to-speech service (`/client-tts/v1/fulfill`, the
`ResolveRequest` from tts-resolve.proto), as public projects documented.

Test (plays the DJ at volume 0 on a test device):
`cargo test --release live_dj -- --ignored --nocapture` in the app.
