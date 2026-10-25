//! Album colors on iOS: the desktop's palette (src/palette.rs) from the playing
//! song's cover, sampled off the actor thread, cached per cover, with the next
//! song's prefetched near the end of this one (the queue is read then; see `poll`).
//! The app crossfades to them (the desktop's 1.4 s bloom).

use crate::app::{MobileApp, Msg};
use crate::bridge::CoreEvent;
use crate::palette::Palette;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct AlbumState {
    palettes: HashMap<String, Option<Palette>>,
    loading: HashSet<String>,
    published: Option<Option<AlbumColorsModel>>,
}

/// The album's colors applied to the theme, as `#RRGGBB[AA]`.
#[derive(uniffi::Record, Clone, PartialEq, Debug)]
pub struct AlbumColorsModel {
    /// Which cover these are from.
    pub cover: String,
    pub accent: String,
    pub on_accent: String,
    pub selection: String,
    pub player_progress: String,
    pub player_track: String,
    pub player_background: String,
    pub lyrics_current: String,
    pub lyrics_highlight: String,
    pub lyrics_other: String,
    pub viz_color: String,
    pub viz_color2: String,
    /// The wash: three dark colors (top-left, bottom-right, top-right) and the base between them.
    pub wash: Vec<String>,
    pub wash_base: String,
}

fn hex(l: crate::palette::Lab) -> String {
    let [r, g, b] = l.to_rgb();
    format!("#{r:02X}{g:02X}{b:02X}")
}

impl MobileApp {
    /// The cover album colors come from: a local file's own, or the album's at about
    /// 300 px — the same size the next song's is prefetched at, so a song that starts
    /// finds its colors ready (and a small cover is quicker to fetch and sample).
    fn color_source(&mut self) -> Option<String> {
        let item = self.now_item()?;
        item.path.as_ref().filter(|_| item.image.as_deref().is_some_and(|i| i.starts_with("localcover://"))).map(|p| format!("localcover://{p}")).or(item.image_small).or(item.image)
    }

    /// Each tick: sample the playing cover (and the next one's, once known) if not done.
    pub fn sync_album_colors(&mut self) {
        if !self.settings.album_colors {
            return;
        }
        let mut wanted: Vec<String> = self.color_source().into_iter().collect();
        if let Some(next) = self.queue.as_ref().and_then(|q| q["queue"].as_array().and_then(|a| a.first()).and_then(|t| crate::model::image(t, 300))) {
            wanted.push(next);
        }
        for url in wanted {
            if self.album.palettes.contains_key(&url) || self.album.loading.contains(&url) {
                continue;
            }
            self.album.loading.insert(url.clone());
            let tx = self.tx.clone();
            let demo = self.demo;
            self.rt.spawn(async move {
                let bytes: Option<Vec<u8>> = if let Some(path) = url.strip_prefix("localcover://") {
                    let p = std::path::PathBuf::from(path);
                    tokio::task::spawn_blocking(move || crate::local::read_cover(&p)).await.ok().flatten()
                } else if url.starts_with("glass-demo:") || demo {
                    None
                } else {
                    match crate::config::http().timeout(std::time::Duration::from_secs(20)).build() {
                        Ok(c) => match c.get(&url).send().await {
                            Ok(r) if r.status().is_success() => r.bytes().await.ok().map(|b| b.to_vec()),
                            _ => None,
                        },
                        Err(_) => None,
                    }
                };
                let palette = match bytes {
                    Some(b) => tokio::task::spawn_blocking(move || crate::palette::palette_of(&b)).await.ok().flatten(),
                    None => demo_palette(&url),
                };
                tx.send(Msg::Palette(url, palette));
            });
        }
        // At most a few dozen covers are kept.
        if self.album.palettes.len() > 48 {
            self.album.palettes.clear();
        }
    }

    pub fn palette_arrived(&mut self, url: String, p: Option<Palette>) {
        self.album.loading.remove(&url);
        self.album.palettes.insert(url, p);
    }

    pub fn publish_album_colors(&mut self, events: &mut Vec<CoreEvent>) {
        let model = if self.settings.album_colors { self.color_source().and_then(|url| self.album.palettes.get(&url).copied().flatten().map(|p| (url, p))) } else { None };
        let model = model.map(|(url, p)| {
            let mut t = self.theme.theme().clone();
            p.apply(&mut t, 1.0);
            let base = p.wash[0].lerp(p.wash[1], 0.5).with(((p.wash[0].l + p.wash[1].l) / 2.0) * 0.8, 0.06);
            AlbumColorsModel {
                cover: url,
                accent: t.accent.hex(),
                on_accent: t.on_accent.hex(),
                selection: t.selection.hex(),
                player_progress: t.player_progress.hex(),
                player_track: t.player_track.hex(),
                player_background: t.player_background.hex(),
                lyrics_current: t.lyrics.current.hex(),
                lyrics_highlight: t.lyrics.highlight.hex(),
                lyrics_other: t.lyrics.other.hex(),
                viz_color: t.viz.color.hex(),
                viz_color2: t.viz.color2.hex(),
                wash: p.wash.iter().map(|c| hex(*c)).collect(),
                wash_base: hex(base),
            }
        });
        if self.album.published.as_ref() != Some(&model) {
            self.album.published = Some(model.clone());
            events.push(CoreEvent::AlbumColors { colors: model });
        }
    }
}

/// The demo's covers are gradients the app paints (`glass-demo:<n>`): their palette
/// comes from the same two colors, so the demo shows album colors too.
fn demo_palette(url: &str) -> Option<Palette> {
    let n: usize = url.strip_prefix("glass-demo:")?.parse().ok()?;
    let palette = [(236u8, 72u8, 153u8), (99, 102, 241), (16, 185, 129), (245, 158, 11), (14, 165, 233), (168, 85, 247), (239, 68, 68), (34, 197, 94)];
    let (a, b) = (palette[n % palette.len()], palette[(n * 3 + 1) % palette.len()]);
    let mut pixels = Vec::with_capacity(64 * 64);
    for y in 0..64u32 {
        for x in 0..64u32 {
            let t = (x + y) as f32 / 126.0;
            let mix = |p: u8, q: u8| (p as f32 * (1.0 - t) + q as f32 * t) as u8;
            pixels.push([mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2)]);
        }
    }
    Palette::from_pixels(&pixels)
}
