//! Now Playing in the album's colors (Settings ▸ Display ▸ "Match Now Playing to the
//! album's colors").
//!
//! The cover is sampled (a 64 px thumbnail, median cut in OKLab), and two things come
//! out of it: an accent, the most vivid color that has some presence, brightened so it
//! reads on the dark glass; and a wash, the cover's main colors darkened into a soft
//! gradient behind Now Playing. While Now Playing (or the player bar) draws, the theme's
//! accent, progress, lyrics and visualizer colors are the album's (`apply`). A new song's
//! colors glide in over a moment.

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[cfg(test)]
use crate::palette::contrast;
pub use crate::palette::{Lab, Palette, palette_of};
#[cfg(test)]
use crate::theme_config::{Rgba, Theme};

/// The wash as a small image; scaled up with linear filtering it's a smooth gradient.
trait WashImage {
    fn wash_image(&self) -> ColorImage;
}

impl WashImage for Palette {
    fn wash_image(&self) -> ColorImage {
        const W: usize = 48;
        const H: usize = 30;
        // Where each color glows from (x, y in 0–1) and how far it reaches.
        let blobs = [(0.08, 0.12, 0.85), (0.98, 0.95, 0.9), (0.92, 0.04, 0.6)];
        let base = self.wash[0].lerp(self.wash[1], 0.5).with(((self.wash[0].l + self.wash[1].l) / 2.0) * 0.8, 0.06);
        let mut pixels = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                let (px, py) = (x as f32 / (W - 1) as f32, y as f32 / (H - 1) as f32);
                let (mut acc, mut total) = (Lab { l: base.l * 0.35, a: base.a * 0.35, b: base.b * 0.35 }, 0.35);
                for (i, &(bx, by, r)) in blobs.iter().enumerate() {
                    // Wider than tall: the window is.
                    let d2 = ((px - bx) * 1.4).powi(2) + (py - by).powi(2);
                    let w = (-d2 / (r * r) * 2.2).exp();
                    let c = self.wash[i];
                    acc = Lab { l: acc.l + c.l * w, a: acc.a + c.a * w, b: acc.b + c.b * w };
                    total += w;
                }
                let mut c = Lab { l: acc.l / total, a: acc.a / total, b: acc.b / total };
                // A little darker toward the bottom, where the controls sit.
                c.l *= 1.0 - 0.16 * py;
                let [r, g, b] = c.to_rgb();
                pixels.extend_from_slice(&[r, g, b, 255]);
            }
        }
        ColorImage::from_rgba_unmultiplied([W, H], &pixels)
    }
}

/// How long a new song's colors take to settle.
const GLIDE: f64 = 1.4;
/// How strong the wash is over the window's own tint.
const WASH: f32 = 0.72;
/// How soft the bloom's edge is, as a share of the way to the farthest corner.
const FEATHER: f32 = 0.55;

/// Palettes sampled off the UI thread, waiting to be picked up: (cover URI, palette).
type Sampled = Arc<Mutex<Vec<(String, Option<Palette>)>>>;

/// Covers' palettes, and what's on screen.
#[derive(Default)]
pub struct AlbumColors {
    palettes: HashMap<String, Option<Palette>>,
    loading: HashSet<String>,
    done: Sampled,
    from: Option<Palette>,
    to: Option<Palette>,
    since: f64,
    /// How far the change from `from` to `to` has run (0–1).
    progress: f32,
    shown: Option<Palette>,
    /// Where a new song's colors bloom from (0–1 across the window): the artwork.
    origin: Option<egui::Pos2>,
    /// The wash when the change began (whatever was on screen), and the new song's.
    wash_from: Option<ColorImage>,
    wash_to: Option<ColorImage>,
    /// What the texture holds: the target palette and progress it was drawn at.
    drawn: Option<(Palette, f32)>,
    last: Option<ColorImage>,
    texture: Option<TextureHandle>,
    /// The song playing has colors (false without a cover, or with one that couldn't be read).
    has_colors: bool,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl AlbumColors {
    /// Call once a frame with the cover of what's playing; returns the colors to show
    /// (gliding between songs). A song whose cover is still loading keeps the last colors.
    pub fn frame(&mut self, ctx: &egui::Context, cover: Option<&str>) -> Option<Palette> {
        for (uri, palette) in self.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default() {
            self.loading.remove(&uri);
            self.palettes.insert(uri, palette);
        }
        let now = ctx.input(|i| i.time);
        if let Some(uri) = cover {
            match self.palettes.get(uri) {
                Some(Some(p)) if self.to != Some(*p) => {
                    // Start from what's on screen, even mid-change.
                    self.from = self.shown;
                    self.wash_from = self.last.clone().filter(|_| self.shown.is_some());
                    self.wash_to = Some(p.wash_image());
                    self.to = Some(*p);
                    self.since = now;
                }
                Some(_) => {}
                None => self.load(ctx, uri),
            }
            // Still loading: the last colors stay until the new ones are ready.
            if let Some(p) = self.palettes.get(uri) {
                self.has_colors = p.is_some();
            }
        } else {
            self.has_colors = false;
        }
        let t = if super::theme::reduce_motion() { 1.0 } else { ((now - self.since) / GLIDE).clamp(0.0, 1.0) as f32 };
        self.progress = t;
        // Ease in and out, so colors neither jump off nor snap in.
        let k = smoothstep(0.0, 1.0, t);
        self.shown = match (self.from, self.to) {
            (Some(f), Some(to)) => Some(f.lerp(&to, k)),
            (_, to) => to,
        };
        if t < 1.0 {
            ctx.request_repaint();
        }
        self.shown
    }

    /// False once the playing song turns out to have no usable cover. While a new cover
    /// loads it keeps its last answer, so the last colors stay up instead of fading out
    /// and back in.
    pub fn has_colors(&self) -> bool {
        self.has_colors
    }

    /// Samples a cover ahead of time (the next song's), so its colors are ready.
    pub fn prefetch(&mut self, ctx: &egui::Context, uri: &str) {
        if !self.palettes.contains_key(uri) {
            self.load(ctx, uri);
        }
    }

    /// Where the artwork is, so the next song's colors bloom out of it.
    pub fn set_origin(&mut self, window: Rect, at: egui::Pos2) {
        let p = (at - window.min) / window.size();
        self.origin = Some(egui::pos2(p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0)));
    }

    /// The colors on screen now.
    #[cfg(test)]
    pub fn shown(&self) -> Option<Palette> {
        self.shown
    }

    fn load(&mut self, ctx: &egui::Context, uri: &str) {
        if self.loading.contains(uri) {
            return;
        }
        if self.palettes.len() > 300 {
            self.palettes.clear();
        }
        // Counts as in use, so the downloaded bytes are freed in time if the song never plays.
        super::widgets::mark_drawn(ctx, uri);
        match ctx.try_load_bytes(uri) {
            Ok(egui::load::BytesPoll::Ready { bytes, .. }) => {
                self.loading.insert(uri.to_string());
                let bytes: Arc<[u8]> = match bytes {
                    egui::load::Bytes::Static(b) => Arc::from(b),
                    egui::load::Bytes::Shared(b) => b,
                };
                let (done, ctx, uri) = (self.done.clone(), ctx.clone(), uri.to_string());
                std::thread::spawn(move || {
                    let palette = palette_of(&bytes);
                    if let Ok(mut d) = done.lock() {
                        d.push((uri, palette));
                    }
                    ctx.request_repaint();
                });
            }
            // The loader asks for a repaint when it has the bytes; we'll be back.
            Ok(egui::load::BytesPoll::Pending { .. }) => {}
            Err(_) => {
                self.palettes.insert(uri.to_string(), None);
            }
        }
    }

    /// Paints the wash over `rect` at `strength` (0–1). While a song changes, the new
    /// song's wash blooms out from the artwork over the old one.
    pub fn paint_wash(&mut self, ui: &egui::Ui, rect: Rect, radius: egui::CornerRadius, strength: f32) {
        let (Some(to), Some(target)) = (self.to, self.wash_to.as_ref()) else { return };
        if strength <= 0.0 {
            return;
        }
        // Redrawn only while something changes: during a bloom, or once for a new wash.
        let t = if self.wash_from.is_some() { self.progress } else { 1.0 };
        if self.drawn != Some((to, t)) {
            let image = match &self.wash_from {
                Some(from) if t < 1.0 => bloom(from, target, t, self.origin.unwrap_or(egui::pos2(0.2, 0.35)), rect.aspect_ratio()),
                _ => target.clone(),
            };
            match &mut self.texture {
                Some(tex) => tex.set(image.clone(), TextureOptions::LINEAR),
                None => self.texture = Some(ui.ctx().load_texture("album-wash", image.clone(), TextureOptions::LINEAR)),
            }
            self.last = Some(image);
            self.drawn = Some((to, t));
            if t >= 1.0 {
                self.wash_from = None;
            }
        }
        let Some(tex) = &self.texture else { return };
        let tint = Color32::from_white_alpha((strength * WASH * 255.0).round() as u8);
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().add(egui::epaint::RectShape::filled(rect, radius, tint).with_texture(tex.id(), uv));
    }
}

/// `to` spreading over `from` from `origin` (0–1), a soft-edged circle that has grown
/// `t` (0–1) of the way past the farthest corner. `aspect` is the window's width / height.
fn bloom(from: &ColorImage, to: &ColorImage, t: f32, origin: egui::Pos2, aspect: f32) -> ColorImage {
    let [w, h] = to.size;
    let far = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().map(|&(x, y): &(f32, f32)| ((x - origin.x) * aspect).hypot(y - origin.y)).fold(0.0, f32::max).max(1e-3);
    // Eased at both ends, like the accent's glide (`frame`), so the two land together.
    let reach = smoothstep(0.0, 1.0, t) * (1.0 + FEATHER);
    let pixels = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 / (w - 1) as f32, (i / w) as f32 / (h - 1) as f32);
            let d = ((x - origin.x) * aspect).hypot(y - origin.y) / far;
            let m = 1.0 - smoothstep(reach - FEATHER, reach, d);
            let (a, b) = (from.pixels[i], to.pixels[i]);
            let mix = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * m).round() as u8;
            Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), 255)
        })
        .collect();
    ColorImage { size: [w, h], pixels, source_size: to.source_size }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips() {
        for rgb in [[0, 0, 0], [255, 255, 255], [216, 155, 122], [12, 200, 90], [255, 0, 0]] {
            let back = Lab::from_rgb(rgb).to_rgb();
            for i in 0..3 {
                assert!((back[i] as i32 - rgb[i] as i32).abs() <= 1, "{rgb:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn a_red_cover_gets_a_red_accent_and_a_dark_red_wash() {
        // Mostly deep red, some black, a little white text.
        let mut px = vec![[170, 20, 30]; 600];
        px.extend(vec![[10, 10, 10]; 300]);
        px.extend(vec![[240, 240, 240]; 40]);
        let p = Palette::from_pixels(&px).unwrap();
        let [r, g, b] = p.accent.to_rgb();
        assert!(r > g + 60 && r > b + 50, "red accent: {:?}", [r, g, b]);
        assert!((0.72..=0.84).contains(&p.accent.l), "light enough to read: {}", p.accent.l);
        for w in p.wash {
            assert!(w.l <= 0.44, "the wash stays dark under white text: {}", w.l);
        }
        let [r, g, _] = p.wash[0].to_rgb();
        assert!(r > g, "the wash leads with the cover's red");
    }

    #[test]
    fn a_black_and_white_cover_stays_neutral() {
        let mut px = vec![[20, 20, 20]; 500];
        px.extend(vec![[200, 200, 200]; 500]);
        let p = Palette::from_pixels(&px).unwrap();
        assert!(p.accent.chroma() < 0.03 && p.accent.l > 0.8, "{:?}", p.accent);
    }

    #[test]
    fn a_new_wash_blooms_out_from_the_artwork() {
        let solid = |c: Color32| ColorImage { size: [48, 30], pixels: vec![c; 48 * 30], source_size: egui::vec2(48.0, 30.0) };
        let (old, new) = (solid(Color32::from_rgb(200, 0, 0)), solid(Color32::from_rgb(0, 0, 200)));
        let origin = egui::pos2(0.2, 0.35);
        assert_eq!(bloom(&old, &new, 0.0, origin, 1.6).pixels, old.pixels, "it starts as the old wash");
        assert_eq!(bloom(&old, &new, 1.0, origin, 1.6).pixels, new.pixels, "and ends as the new one");
        let mid = bloom(&old, &new, 0.45, origin, 1.6);
        let at = |x: usize, y: usize| mid.pixels[y * 48 + x];
        let near = at((0.2 * 47.0) as usize, (0.35 * 29.0) as usize);
        let far = at(47, 29);
        assert!(near.b() > 150 && near.r() < 50, "the artwork's corner has turned: {near:?}");
        assert!(far.r() > far.b(), "the far corner hasn't yet: {far:?}");
    }

    #[test]
    fn apply_moves_the_theme_by_amount() {
        let p = Palette::from_pixels(&vec![[30, 90, 200]; 100]).unwrap();
        let base = Theme::default();
        let mut none = base.clone();
        p.apply(&mut none, 0.0);
        assert_eq!(none.accent, base.accent, "nothing at 0");
        let mut full = base.clone();
        p.apply(&mut full, 1.0);
        assert_eq!(full.accent, p.accent.rgba(1.0));
        assert_eq!(full.player_progress, full.accent);
        assert!(contrast(full.accent, full.on_accent) >= 4.5, "text on the accent reads");
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// A preview sheet for tuning: `ALBUM_COVERS=<dir of jpgs> cargo test album_preview -- --ignored`
    /// writes `<dir>/preview.png`: each cover, its wash over the default background (#0F1115),
    /// the accent, and the sung-line and sung-word colors.
    #[test]
    #[ignore]
    fn album_preview() {
        let dir = std::path::PathBuf::from(std::env::var("ALBUM_COVERS").unwrap());
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "jpg")).collect();
        files.sort();
        let (cw, ch, ww) = (150u32, 150u32, 330u32);
        let mut sheet = image::RgbaImage::new(cw + ww + 10, ch * files.len() as u32);
        for (row, path) in files.iter().enumerate() {
            let bytes = std::fs::read(path).unwrap();
            let p = palette_of(&bytes).unwrap();
            let y0 = row as u32 * ch;
            let cover = image::load_from_memory(&bytes).unwrap().resize_exact(cw, ch, image::imageops::FilterType::Triangle).to_rgba8();
            image::imageops::overlay(&mut sheet, &cover, 0, y0 as i64);
            // The wash, scaled up smoothly, over the default background (#0F1115) at its strength.
            let wash = p.wash_image();
            let small = image::RgbaImage::from_raw(wash.size[0] as u32, wash.size[1] as u32, wash.pixels.iter().flat_map(|c| c.to_array()).collect()).unwrap();
            let big = image::imageops::resize(&small, ww, ch, image::imageops::FilterType::Triangle);
            for (x, y, px) in big.enumerate_pixels() {
                let bg = [0x0F as f32, 0x11 as f32, 0x15 as f32];
                let a = WASH;
                let c: Vec<u8> = (0..3).map(|i| (bg[i] * (1.0 - a) + px.0[i] as f32 * a).round() as u8).collect();
                sheet.put_pixel(cw + x, y0 + y, image::Rgba([c[0], c[1], c[2], 255]));
            }
            // Accent, sung line, sung word, text on the accent; then a progress bar.
            let mut theme = Theme::default();
            p.apply(&mut theme, 1.0);
            for (i, color) in [theme.accent, theme.lyrics.current, theme.lyrics.highlight, theme.on_accent].iter().enumerate() {
                for y in 0..22 {
                    for x in 0..60 {
                        sheet.put_pixel(cw + 20 + i as u32 * 72 + x, y0 + 100 + y, image::Rgba(color.0));
                    }
                }
            }
            for x in 0..280 {
                for y in 0..4 {
                    let c = if x < 170 { theme.player_progress } else { Rgba([255, 255, 255, 60]) };
                    sheet.put_pixel(cw + 20 + x, y0 + 80 + y, image::Rgba([c.0[0], c.0[1], c.0[2], 255]));
                }
            }
        }
        sheet.save(dir.join("preview.png")).unwrap();
    }

    /// The bloom from the first cover to the second, in frames: `bloom.png` next to them.
    #[test]
    #[ignore]
    fn bloom_preview() {
        let dir = std::path::PathBuf::from(std::env::var("ALBUM_COVERS").unwrap());
        let wash = |name: &str| palette_of(&std::fs::read(dir.join(name)).unwrap()).unwrap().wash_image();
        let (from, to) = (wash("01.jpg"), wash("02.jpg"));
        let (fw, fh) = (240u32, 150u32);
        let times = [0.0, 0.2, 0.35, 0.5, 0.65, 0.8, 1.0];
        let mut sheet = image::RgbaImage::new(fw * times.len() as u32, fh);
        for (i, t) in times.iter().enumerate() {
            let frame = bloom(&from, &to, *t, egui::pos2(0.2, 0.35), fw as f32 / fh as f32);
            let small = image::RgbaImage::from_raw(48, 30, frame.pixels.iter().flat_map(|c| c.to_array()).collect()).unwrap();
            let big = image::imageops::resize(&small, fw, fh, image::imageops::FilterType::Triangle);
            image::imageops::overlay(&mut sheet, &big, (i as u32 * fw) as i64, 0);
        }
        sheet.save(dir.join("bloom.png")).unwrap();
    }
}
