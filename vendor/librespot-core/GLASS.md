# librespot-core 0.8.0, patched for Glass

This is librespot-core 0.8.0 from crates.io (MIT licensed, © the librespot
contributors, https://github.com/librespot-org/librespot) with one change to
its HTTP client, and one lint attribute. `Cargo.toml` in the app uses it via
`[patch.crates-io]`; everything else is unchanged.

Why: librespot builds its hyper client without a pool timer, and hyper only
closes idle connections when it has one. Each burst of requests (a page's
metadata, a playlist's songs) opened connections that then stayed open for as
long as the app ran: an idle Glass held over 50 connections to Spotify's API.

Changes (marked `Glass` in the source):

- `src/http_client.rs` — the client gets hyper-util's Tokio timer, closes
  connections idle for 30 s and keeps at most 4 per host.
- `Cargo.toml`, `Cargo.toml.orig` — hyper-util's `tokio` feature, for the timer.
- `src/authentication.rs` — an `#[expect(deprecated)]` that nothing matches with
  the dependency versions Glass builds against is an `#[allow]`, so builds stay
  free of warnings.
