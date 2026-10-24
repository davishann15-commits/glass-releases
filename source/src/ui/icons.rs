//! Line icons drawn with egui's painter on a 24×24 grid.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Play,
    Pause,
    Next,
    Prev,
    Shuffle,
    Repeat,
    RepeatOne,
    Heart,
    HeartFilled,
    Plus,
    Check,
    Search,
    Home,
    Queue,
    Devices,
    Lyrics,
    NowPlaying,
    VolumeMute,
    VolumeLow,
    VolumeHigh,
    Mini,
    ExitFullscreen,
    Close,
    Back,
    More,
    Clock,
    Download,
    Folder,
    Skip15Back,
    Skip15Fwd,
    Music,
    Bell,
    Settings,
    Chevron,
    ChevronDown,
    Edit,
    Assistant,
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0 * s, 12.0 * s);
    let pt = |x: f32, y: f32| -> Pos2 { o + vec2(x * s, y * s) };
    let w = (1.7 * s).max(1.2);
    let st = Stroke::new(w, color);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| pt(x, y)).collect(), color, Stroke::NONE));
    };
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = 24;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                pt(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        p.add(Shape::line(pts, st));
    };
    match icon {
        Icon::Play => fill(&[(7.0, 4.5), (19.5, 12.0), (7.0, 19.5)]),
        Icon::Pause => {
            p.rect_filled(Rect::from_min_max(pt(6.5, 5.0), pt(10.0, 19.0)), 1.0, color);
            p.rect_filled(Rect::from_min_max(pt(14.0, 5.0), pt(17.5, 19.0)), 1.0, color);
        }
        Icon::Next => {
            fill(&[(5.0, 5.0), (16.0, 12.0), (5.0, 19.0)]);
            p.rect_filled(Rect::from_min_max(pt(16.5, 5.0), pt(19.0, 19.0)), 1.0, color);
        }
        Icon::Prev => {
            fill(&[(19.0, 5.0), (19.0, 19.0), (8.0, 12.0)]);
            p.rect_filled(Rect::from_min_max(pt(5.0, 5.0), pt(7.5, 19.0)), 1.0, color);
        }
        Icon::Shuffle => {
            line(&[(3.0, 7.0), (7.0, 7.0), (15.0, 17.0), (20.0, 17.0)]);
            line(&[(3.0, 17.0), (7.0, 17.0), (15.0, 7.0), (20.0, 7.0)]);
            line(&[(17.5, 4.5), (20.5, 7.0), (17.5, 9.5)]);
            line(&[(17.5, 14.5), (20.5, 17.0), (17.5, 19.5)]);
        }
        Icon::Repeat | Icon::RepeatOne => {
            line(&[(4.0, 13.0), (4.0, 8.0), (6.0, 6.0), (19.0, 6.0)]);
            line(&[(16.5, 3.5), (19.0, 6.0), (16.5, 8.5)]);
            line(&[(20.0, 11.0), (20.0, 16.0), (18.0, 18.0), (5.0, 18.0)]);
            line(&[(7.5, 15.5), (5.0, 18.0), (7.5, 20.5)]);
            if icon == Icon::RepeatOne {
                p.circle_filled(pt(12.0, 12.0), 3.6 * s, color);
                p.text(pt(12.0, 12.2), eframe::egui::Align2::CENTER_CENTER, "1", eframe::egui::FontId::proportional(6.5 * s), Color32::BLACK);
            }
        }
        Icon::Heart | Icon::HeartFilled => {
            let mut pts = Vec::new();
            for i in 0..=40 {
                let t = i as f32 / 40.0 * TAU;
                let x = 16.0 * t.sin().powi(3);
                let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                pts.push(pt(12.0 + x * 0.52, 12.3 - y * 0.52));
            }
            if icon == Icon::HeartFilled {
                p.add(Shape::Path(eframe::egui::epaint::PathShape::closed_line(pts.clone(), Stroke::new(w, color))));
                // Fill via triangle fan from center (heart is star-shaped around it).
                let c = pt(12.0, 12.5);
                let mut mesh = eframe::egui::Mesh::default();
                mesh.colored_vertex(c, color);
                for q in &pts {
                    mesh.colored_vertex(*q, color);
                }
                for i in 1..pts.len() as u32 {
                    mesh.add_triangle(0, i, i + 1);
                }
                p.add(Shape::mesh(mesh));
            } else {
                p.add(Shape::Path(eframe::egui::epaint::PathShape::closed_line(pts, Stroke::new(w, color))));
            }
        }
        Icon::Plus => {
            line(&[(12.0, 5.0), (12.0, 19.0)]);
            line(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Icon::Check => line(&[(5.0, 12.5), (10.0, 17.0), (19.0, 7.0)]),
        Icon::Search => {
            p.circle_stroke(pt(10.5, 10.5), 6.0 * s, st);
            line(&[(15.0, 15.0), (20.0, 20.0)]);
        }
        Icon::Home => line(&[(4.0, 20.0), (4.0, 11.0), (12.0, 4.0), (20.0, 11.0), (20.0, 20.0), (4.0, 20.0)]),
        Icon::Queue => {
            line(&[(4.0, 6.0), (20.0, 6.0)]);
            line(&[(4.0, 11.0), (20.0, 11.0)]);
            line(&[(4.0, 16.0), (12.0, 16.0)]);
            fill(&[(15.0, 13.5), (20.0, 16.5), (15.0, 19.5)]);
        }
        Icon::Devices => {
            p.rect_stroke(Rect::from_min_max(pt(3.0, 5.0), pt(15.0, 15.0)), 1.5, st, eframe::egui::StrokeKind::Middle);
            line(&[(6.0, 19.0), (12.0, 19.0)]);
            p.rect_stroke(Rect::from_min_max(pt(17.0, 8.0), pt(21.0, 19.0)), 1.5, st, eframe::egui::StrokeKind::Middle);
        }
        Icon::Lyrics => {
            p.circle_stroke(pt(15.0, 8.0), 4.5 * s, st);
            line(&[(12.0, 11.0), (4.0, 19.0), (5.5, 20.5), (13.5, 12.5)]);
        }
        Icon::NowPlaying => {
            p.rect_stroke(Rect::from_min_max(pt(3.0, 4.0), pt(21.0, 20.0)), 2.0 * s, st, eframe::egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(pt(13.0, 7.0), pt(18.0, 17.0)), 1.0, color);
        }
        Icon::VolumeMute | Icon::VolumeLow | Icon::VolumeHigh => {
            fill(&[(3.0, 9.0), (7.0, 9.0), (12.0, 5.0), (12.0, 19.0), (7.0, 15.0), (3.0, 15.0)]);
            match icon {
                Icon::VolumeMute => {
                    line(&[(15.5, 9.0), (21.0, 15.0)]);
                    line(&[(21.0, 9.0), (15.5, 15.0)]);
                }
                Icon::VolumeLow => arc(12.0, 12.0, 4.5, -PI / 3.5, PI / 3.5),
                _ => {
                    arc(12.0, 12.0, 4.5, -PI / 3.5, PI / 3.5);
                    arc(12.0, 12.0, 8.5, -PI / 3.5, PI / 3.5);
                }
            }
        }
        Icon::Mini => {
            p.rect_stroke(Rect::from_min_max(pt(3.0, 4.0), pt(21.0, 20.0)), 2.0 * s, st, eframe::egui::StrokeKind::Middle);
            p.rect_filled(Rect::from_min_max(pt(12.0, 12.0), pt(19.0, 18.0)), 1.0, color);
        }
        Icon::ExitFullscreen => {
            line(&[(4.0, 9.0), (9.0, 9.0), (9.0, 4.0)]);
            line(&[(15.0, 4.0), (15.0, 9.0), (20.0, 9.0)]);
            line(&[(20.0, 15.0), (15.0, 15.0), (15.0, 20.0)]);
            line(&[(9.0, 20.0), (9.0, 15.0), (4.0, 15.0)]);
        }
        Icon::Close => {
            line(&[(6.0, 6.0), (18.0, 18.0)]);
            line(&[(18.0, 6.0), (6.0, 18.0)]);
        }
        Icon::Back => line(&[(15.0, 5.0), (8.0, 12.0), (15.0, 19.0)]),
        Icon::More => {
            for x in [5.0, 12.0, 19.0] {
                p.circle_filled(pt(x, 12.0), 1.7 * s, color);
            }
        }
        Icon::Clock => {
            p.circle_stroke(pt(12.0, 12.0), 8.5 * s, st);
            line(&[(12.0, 7.0), (12.0, 12.0), (15.5, 14.0)]);
        }
        Icon::Download => {
            p.circle_stroke(pt(12.0, 12.0), 9.0 * s, st);
            line(&[(12.0, 7.0), (12.0, 16.0)]);
            line(&[(8.5, 12.5), (12.0, 16.0), (15.5, 12.5)]);
        }
        Icon::Assistant => {
            // A four-point spark.
            line(&[(12.0, 3.0), (13.6, 10.4), (21.0, 12.0), (13.6, 13.6), (12.0, 21.0), (10.4, 13.6), (3.0, 12.0), (10.4, 10.4), (12.0, 3.0)]);
        }
        Icon::Edit => {
            line(&[(15.0, 5.0), (19.0, 9.0), (8.5, 19.5), (4.5, 19.5), (4.5, 15.5), (15.0, 5.0)]);
            line(&[(13.0, 7.0), (17.0, 11.0)]);
        }
        Icon::Folder => line(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0), (3.0, 6.0)]),
        Icon::Skip15Back | Icon::Skip15Fwd => {
            let back = icon == Icon::Skip15Back;
            if back {
                arc(12.0, 13.0, 8.0, -PI * 0.5, PI * 1.25);
                line(&[(12.0, 2.0), (9.0, 5.0), (12.0, 8.0)]);
            } else {
                arc(12.0, 13.0, 8.0, -PI * 0.25, PI * 1.5);
                line(&[(12.0, 2.0), (15.0, 5.0), (12.0, 8.0)]);
            }
            p.text(pt(12.0, 13.5), eframe::egui::Align2::CENTER_CENTER, "15", eframe::egui::FontId::proportional(7.5 * s), color);
        }
        Icon::Settings => {
            p.circle_stroke(pt(12.0, 12.0), 6.2 * s, st);
            p.circle_stroke(pt(12.0, 12.0), 2.6 * s, st);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (c, sn) = (a.cos(), a.sin());
                line(&[(12.0 + 6.2 * c, 12.0 + 6.2 * sn), (12.0 + 9.0 * c, 12.0 + 9.0 * sn)]);
            }
        }
        Icon::Chevron => line(&[(9.5, 6.0), (15.5, 12.0), (9.5, 18.0)]),
        Icon::ChevronDown => line(&[(6.0, 9.5), (12.0, 15.5), (18.0, 9.5)]),
        Icon::Music => {
            line(&[(9.0, 18.0), (9.0, 5.0), (19.0, 3.5), (19.0, 16.0)]);
            p.circle_filled(pt(6.5, 18.0), 2.6 * s, color);
            p.circle_filled(pt(16.5, 16.0), 2.6 * s, color);
        }
        Icon::Bell => {
            line(&[(6.0, 17.0), (6.0, 11.0)]);
            arc(12.0, 11.0, 6.0, PI, TAU);
            line(&[(18.0, 11.0), (18.0, 17.0)]);
            line(&[(4.0, 17.0), (20.0, 17.0)]);
            arc(12.0, 18.5, 2.0, 0.0, PI);
        }
    }
}
