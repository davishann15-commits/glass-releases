//! The album's colors from its cover (Settings ▸ Display ▸ "Match Now Playing to the
//! album's colors"), independent of any UI: the cover is sampled (a 64 px thumbnail,
//! median cut in OKLab) into an accent — the most vivid color with some presence,
//! brightened so it reads on the dark glass — and a wash of the cover's main colors,
//! darkened. The desktop paints it with egui (src/ui/album_colors.rs); the iOS app
//! with SwiftUI.

use crate::theme_config::{Rgba, Theme};

/// A color in OKLab: lightness 0–1, then the green–red and blue–yellow axes.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Lab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
}

impl Lab {
    pub fn chroma(self) -> f32 {
        self.a.hypot(self.b)
    }

    pub fn lerp(self, o: Lab, t: f32) -> Lab {
        Lab { l: self.l + (o.l - self.l) * t, a: self.a + (o.a - self.a) * t, b: self.b + (o.b - self.b) * t }
    }

    pub fn dist(self, o: Lab) -> f32 {
        ((self.l - o.l).powi(2) + (self.a - o.a).powi(2) + (self.b - o.b).powi(2)).sqrt()
    }

    /// Same hue, with this lightness and chroma, pulled into sRGB if it falls outside.
    pub fn with(self, l: f32, chroma: f32) -> Lab {
        let c = self.chroma();
        let (ua, ub) = if c > 1e-4 { (self.a / c, self.b / c) } else { (0.0, 0.0) };
        let at = |c: f32| Lab { l, a: ua * c, b: ub * c };
        if in_gamut(at(chroma)) {
            return at(chroma);
        }
        let (mut lo, mut hi) = (0.0, chroma);
        for _ in 0..14 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(at(mid)) { lo = mid } else { hi = mid }
        }
        at(lo)
    }

    pub fn from_rgb([r, g, b]: [u8; 3]) -> Lab {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let (r, g, b) = (lin(r), lin(g), lin(b));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        Lab { l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s, a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s, b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s }
    }

    /// Linear sRGB (may fall outside 0–1).
    fn linear(self) -> [f32; 3] {
        let l = (self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b).powi(3);
        let m = (self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b).powi(3);
        let s = (self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b).powi(3);
        [4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s, -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s, -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s]
    }

    pub fn to_rgb(self) -> [u8; 3] {
        let gamma = |c: f32| {
            let c = c.clamp(0.0, 1.0);
            let v = if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
            (v * 255.0).round() as u8
        };
        let [r, g, b] = self.linear();
        [gamma(r), gamma(g), gamma(b)]
    }

    pub fn rgba(self, alpha: f32) -> Rgba {
        let [r, g, b] = self.to_rgb();
        Rgba([r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8])
    }
}

fn in_gamut(c: Lab) -> bool {
    c.linear().iter().all(|v| (-0.0005..=1.0005).contains(v))
}

/// What a cover contributes to the look.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette {
    /// Bright enough for text and progress on the dark glass.
    pub accent: Lab,
    /// Dark versions of the cover's main colors (and the accent's hue), for the wash.
    pub wash: [Lab; 3],
}

impl Palette {
    /// The colors' bits: two palettes with the same key look the same.
    pub fn key(&self) -> [u32; 12] {
        let lab = |c: &Lab| [c.l.to_bits(), c.a.to_bits(), c.b.to_bits()];
        let (a, w) = (lab(&self.accent), self.wash.each_ref().map(lab));
        [a[0], a[1], a[2], w[0][0], w[0][1], w[0][2], w[1][0], w[1][1], w[1][2], w[2][0], w[2][1], w[2][2]]
    }

    pub fn lerp(&self, o: &Palette, t: f32) -> Palette {
        Palette { accent: self.accent.lerp(o.accent, t), wash: [self.wash[0].lerp(o.wash[0], t), self.wash[1].lerp(o.wash[1], t), self.wash[2].lerp(o.wash[2], t)] }
    }

    /// The palette from cover pixels (RGB).
    pub fn from_pixels(pixels: &[[u8; 3]]) -> Option<Palette> {
        if pixels.is_empty() {
            return None;
        }
        let swatches = median_cut(pixels.iter().map(|&p| Lab::from_rgb(p)).collect(), 8);
        let score = |s: &(Lab, f32)| {
            let (lab, share) = *s;
            let mut v = (lab.chroma() / 0.2).min(1.2) * 0.75 + share.sqrt() * 0.6;
            if lab.l < 0.22 || lab.l > 0.97 {
                v *= 0.4; // near black or white: no use as a color
            }
            if share < 0.02 {
                v *= 0.3; // a speck
            }
            v
        };
        let vivid = swatches.iter().max_by(|a, b| score(a).total_cmp(&score(b)))?.0;
        let accent = if vivid.chroma() < 0.035 {
            // A black-and-white cover: a light, nearly neutral accent.
            vivid.with(0.86, vivid.chroma().min(0.02))
        } else {
            vivid.with(vivid.l.clamp(0.72, 0.84), (vivid.chroma() * 1.12).clamp(0.08, 0.19))
        };

        // The wash: the two biggest distinct colors that have some color to them (a cover's
        // biggest area is often near-black), and the accent's hue.
        let presence = |s: &(Lab, f32)| s.1 * (0.35 + (s.0.chroma() / 0.1).min(1.0)) * if s.0.l < 0.14 { 0.3 } else { 1.0 };
        let mut ranked = swatches.clone();
        ranked.sort_by(|a, b| presence(b).total_cmp(&presence(a)));
        let mut main: Vec<Lab> = Vec::new();
        for (lab, _) in &ranked {
            if main.len() < 2 && main.iter().all(|m| m.dist(*lab) > 0.08) {
                main.push(*lab);
            }
        }
        while main.len() < 2 {
            main.push(main.first().copied().unwrap_or(vivid));
        }
        // Dark enough for white text, light and colorful enough to read as the album.
        let darken = |c: Lab| c.with(0.27 + 0.16 * c.l.clamp(0.0, 1.0), (c.chroma() * 1.15).min(0.14));
        let wash = [darken(main[0]), darken(main[1]), accent.with(0.38, accent.chroma().min(0.14))];
        Some(Palette { accent, wash })
    }

    /// Theme changes, at `amount` (0: the theme's own, 1: the album's).
    pub fn apply(&self, t: &mut Theme, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        let mix = |from: Rgba, to: Rgba| lerp_rgba(from, to, amount);
        let accent = self.accent.rgba(1.0);
        // Text on an accent fill: the theme's, unless dark ink reads better.
        let ink = self.accent.with(0.2, 0.04).rgba(1.0);
        let on_accent = if contrast(accent, ink) > contrast(accent, t.on_accent) { ink } else { t.on_accent };
        t.accent = mix(t.accent, accent);
        t.focus = mix(t.focus, accent);
        t.selection = mix(t.selection, self.accent.rgba(0.18));
        t.on_accent = mix(t.on_accent, on_accent);
        t.player_progress = mix(t.player_progress, accent);
        t.player_track = mix(t.player_track, self.accent.with(0.9, 0.03).rgba(0.22));
        t.player_background = mix(t.player_background, self.wash[0].with(self.wash[0].l * 0.8, self.wash[0].chroma()).rgba(t.player_background.0[3] as f32 / 255.0));
        // Lyrics: the sung line nearly white with a hint of the hue, the sung word in the accent.
        t.lyrics.current = mix(t.lyrics.current, self.accent.with(0.96, 0.025).rgba(t.lyrics.current.0[3] as f32 / 255.0));
        t.lyrics.highlight = mix(t.lyrics.highlight, self.accent.with(self.accent.l.max(0.82), self.accent.chroma()).rgba(1.0));
        t.lyrics.other = mix(t.lyrics.other, self.accent.with(0.9, 0.02).rgba(t.lyrics.other.0[3] as f32 / 255.0));
        t.viz.color = mix(t.viz.color, accent);
        t.viz.color2 = mix(t.viz.color2, self.accent.with(0.92, 0.05).rgba(t.viz.color2.0[3] as f32 / 255.0));
    }
}

pub(crate) fn lerp_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba(std::array::from_fn(|i| (a.0[i] as f32 + (b.0[i] as f32 - a.0[i] as f32) * t).round() as u8))
}

/// WCAG contrast ratio of two opaque colors.
pub(crate) fn contrast(a: Rgba, b: Rgba) -> f32 {
    let lum = |c: Rgba| {
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.0[0]) + 0.7152 * lin(c.0[1]) + 0.0722 * lin(c.0[2])
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Median cut: splits the colors into up to `n` boxes; each box's mean and share.
pub(crate) fn median_cut(colors: Vec<Lab>, n: usize) -> Vec<(Lab, f32)> {
    let total = colors.len() as f32;
    // Hue differences count double: they matter more than lightness for a palette.
    let axis = |c: &Lab, i: usize| match i {
        0 => c.l,
        1 => c.a * 2.0,
        _ => c.b * 2.0,
    };
    let range = |b: &[Lab], i: usize| {
        let (lo, hi) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), c| (lo.min(axis(c, i)), hi.max(axis(c, i))));
        hi - lo
    };
    let mut boxes = vec![colors];
    while boxes.len() < n {
        // Split the box with the most spread (weighted by size).
        let Some((at, i)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(at, b)| {
                let (i, r) = (0..3).map(|i| (i, range(b, i))).max_by(|x, y| x.1.total_cmp(&y.1)).unwrap();
                (at, i, r * (b.len() as f32).sqrt())
            })
            .max_by(|x, y| x.2.total_cmp(&y.2))
            .filter(|&(_, _, spread)| spread > 0.01)
            .map(|(at, i, _)| (at, i))
        else {
            break;
        };
        let mut b = boxes.swap_remove(at);
        b.sort_by(|x, y| axis(x, i).total_cmp(&axis(y, i)));
        let upper = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(upper);
    }
    boxes
        .into_iter()
        .map(|b| {
            let k = b.len() as f32;
            let sum = b.iter().fold(Lab::default(), |s, c| Lab { l: s.l + c.l, a: s.a + c.a, b: s.b + c.b });
            (Lab { l: sum.l / k, a: sum.a / k, b: sum.b / k }, k / total)
        })
        .collect()
}

/// Decodes a cover and samples it.
pub fn palette_of(bytes: &[u8]) -> Option<Palette> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(64, 64).to_rgba8();
    let pixels: Vec<[u8; 3]> = img.pixels().filter(|p| p.0[3] >= 128).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    Palette::from_pixels(&pixels)
}
