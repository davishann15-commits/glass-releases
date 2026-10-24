# librespot-playback 0.8.0, patched for Glass

This is librespot-playback 0.8.0 from crates.io (MIT licensed, © the
librespot contributors, https://github.com/librespot-org/librespot) with
changes to how local files are played, faster pause and seek, an output that
stops (no audio callbacks) while paused, and a tap for Glass's visualizer. `Cargo.toml` in the app
uses it via `[patch.crates-io]`; everything else in librespot is unchanged.

Why: a Spotify playlist stores a local song only as
`spotify:local:artist:album:title:seconds`. Upstream plays it only when a file
produces exactly the same string (same percent-encoding, same length to the
second), only finds MP3 and FLAC files (it lists .mp4 and .m4p but can't read
them), and refuses anything that isn't 44.1 kHz stereo. Songs copied from a
phone usually miss on at least one of those, so they never played.

Changes (all marked `Glass` in the source):

- `src/local_file.rs` — rewritten lookup:
  - matches on normalized title and artist (one credit holding the other as
    whole words, such as "A" in "A feat. B", counts), lengths within 3 s,
    album as a tie-breaker;
  - untagged files are known by their file name;
  - MP4/M4A, AAC, Ogg, WAV and AIFF as well as MP3/FLAC;
  - scans lazily on first use and again (at most every 10 s) when a song
    isn't found, so newly copied files play without restarting; a rescan
    reads only files that are new or changed since the last one;
  - `set_directories()` lets the app change the folders at any time.
- `src/decoder/symphonia_decoder.rs` — any sample rate is resampled to
  44.1 kHz (cubic interpolation, light smoothing when shrinking a lot) and
  any channel layout becomes stereo, instead of refusing the file.
- `src/player.rs` — uses the owned path from the new lookup; guards a divide
  by zero for a local URI with a zero length.
- `src/lib.rs` — `local_file` is public (for `set_directories`).
- `Cargo.toml` — Symphonia's `isomp4`, `aac`, `alac`, `wav`, `aiff` and `pcm`
  features.
- `src/audio_backend/{mod,rodio}.rs`, `src/player.rs` — responsive
  controls. Upstream's rodio sink only has `stop`, which waits for the ~0.5 s
  of queued audio to play out (blocking the player thread), so pause, seek
  and picking another song all lagged. `Sink` gains `pause` (silence now,
  keep the queue to resume from) and `flush` (drop queued audio); the player
  pauses with `pause` and flushes on seek, on a load that isn't the gapless
  move to the next track, and on a stop mid-song.
- `src/tap.rs` (new) and `src/audio_backend/rodio.rs` — each buffer handed
  to rodio is wrapped in `tap::Tapped`, which passes the samples to a tap the
  app registers (`tap::set`) at the moment they're played, for Glass's audio
  visualizer. Without a tap it only forwards samples.
- `src/player.rs` — the player thread's runtime has two worker threads
  instead of one per CPU core (it runs only the player and track loads).
- `src/audio_backend/rodio.rs` — the output stream is built around rodio's
  mixer (instead of `rodio::OutputStream`) so it can be paused: paused or
  stopped, the device makes no audio callbacks. The stream starts paused,
  opening the output can fail without panicking (`try_open`,
  `try_open_default`), `stop` waits at most 2 s for the queue to play out,
  and `write` returns an error instead of waiting forever when the device
  stops taking audio (it went away). The stream is stereo in f32 (or i16,
  i32, f64) when the device offers it: ALSA lists 8-bit formats first.
- `src/tap.rs` — each block's buffer is allocated on the player thread, so
  the audio thread doesn't allocate.

Tests: `cargo test -p librespot-playback --lib -- local_file glass` (and
`GLASS_TEST_AUDIO=/path/to/song.m4a` to decode a real file).
