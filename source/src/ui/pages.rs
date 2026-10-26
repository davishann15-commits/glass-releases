//! Main-view pages. Each asks `App` for a few Web-API-shaped paths (answered
//! by `sp`) and renders them top to bottom.

use super::icons::{self, Icon};
use super::menus::{self, route_for, subtitle_for};
use super::theme::{self, col};
use super::tracklist::{self, Opts, rows_from_items, rows_plain};
use super::widgets::{self, card_grid, chip, icon_btn, link, paint_cover, section_title};
use crate::api::enc;
use crate::app::{Action, App, Modal, Route, SearchFilter};
use crate::model::{self, s};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, ScrollArea, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub fn show(app: &mut App, ui: &mut Ui) {
    let route = app.route.clone();
    if route == Route::Settings {
        return super::settings::show(app, ui);
    }
    // Each page keeps its own scroll offset, so Back returns to where you were.
    let mut sa = ScrollArea::vertical().id_salt(("main", &route)).auto_shrink([false, false]);
    if app.scroll_top {
        sa = sa.vertical_scroll_offset(0.0);
        app.scroll_top = false;
    }
    sa.show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: 24, right: 24, top: 20, bottom: 40 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match route {
                Route::Library | Route::NowPlaying => {}
                Route::Home => home(app, ui),
                Route::Search => search(app, ui),
                Route::Genre { id, name } => genre(app, ui, &id, &name),
                Route::Playlist(id) => playlist(app, ui, &id),
                Route::Album(id) => album(app, ui, &id),
                Route::Artist(id) => artist(app, ui, &id),
                Route::Discography(id) => discography(app, ui, &id),
                Route::Show(id) => show_page(app, ui, &id),
                Route::Episode(id) => episode(app, ui, &id),
                Route::Audiobook(id) => audiobook(app, ui, &id),
                Route::Track(id) => track(app, ui, &id),
                Route::Profile(id) => profile(app, ui, &id),
                Route::LikedSongs => liked(app, ui),
                Route::YourEpisodes => your_episodes(app, ui),
                Route::LocalFiles => super::local_library::show(app, ui),
                Route::LocalAlbum { artist, album } => super::local_library::album_page(app, ui, &artist, &album),
                Route::Device { id, name } => device_page(app, ui, &id, &name),
                Route::LocalPlaylist(id) => local_playlist(app, ui, &id),
                Route::Recents => recents(app, ui),
                Route::Folder(id) => folder(app, ui, &id),
                Route::Activity => activity(app, ui),
                Route::Settings => {}
            }
        });
    });
}

/// Fetch or show a spinner / retryable error.
fn need(app: &mut App, ui: &mut Ui, path: &str) -> Option<Arc<Value>> {
    if let Some(v) = app.want(path) {
        return Some(v);
    }
    show_wait(app, ui, path);
    None
}

fn need_all(app: &mut App, ui: &mut Ui, path: &str, cap: usize) -> Option<Arc<Value>> {
    if let Some(v) = app.want_all(path, cap) {
        return Some(v);
    }
    show_wait(app, ui, path);
    None
}

fn show_wait(app: &mut App, ui: &mut Ui, path: &str) {
    if let Some(e) = app.load_error(path) {
        if widgets::error_box(ui, &e) {
            app.act(Action::Invalidate(path.to_string()));
        }
    } else {
        widgets::loading(ui);
    }
}

/// `[ play ]  [ shuffle ]`, save, download and more, as on every collection page.
fn action_row(app: &mut App, ui: &mut Ui, context: &str, save_uri: Option<&str>, menu: impl FnOnce(&mut App, &mut Ui)) {
    ui.add_space(theme::sp(2.5));
    // Wraps on a narrow page (e.g. beside the assistant) instead of running under the side panel.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = theme::sp(1.5);
        let here = !context.is_empty() && app.pb.context_uri == context;
        let playing = here && app.pb.is_playing;
        if widgets::bracket_button(ui, Some(if playing { Icon::Pause } else { Icon::Play }), if playing { "pause" } else { "play" }, true).clicked() {
            if here {
                app.act(Action::TogglePlay);
            } else if !context.is_empty() {
                app.act(Action::PlayContext { context: context.to_string(), track: None });
            }
        }
        if widgets::bracket_button(ui, Some(Icon::Shuffle), if app.pb.shuffle { "shuffle on" } else { "shuffle" }, false).clicked() {
            let sh = !app.pb.shuffle;
            app.act(Action::SetShuffle(sh));
        }
        if let Some(uri) = save_uri {
            let saved = app.is_saved(uri) == Some(true);
            let tip = if saved { "Remove from Your Library" } else { "Save to Your Library" };
            if icon_btn(ui, if saved { Icon::Check } else { Icon::Plus }, 18.0, tip, saved).clicked() {
                app.act(Action::Save { uri: uri.to_string(), save: !saved });
            }
        }
        if !context.is_empty() && !context.starts_with("spotify:artist:") {
            let name = ui.ctx().data(|d| d.get_temp::<String>(egui::Id::new("page-title"))).unwrap_or_default();
            download_toggle(app, ui, context, &name, 18.0);
        }
        let more = icon_btn(ui, Icon::More, 18.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| menu(app, ui));
    });
    ui.add_space(theme::sp(1.5));
}

/// Download a collection to this computer (songs start at once; each play still
/// needs a connection), or remove it; shows how far it's got.
pub(super) fn download_toggle(app: &mut App, ui: &mut Ui, uri: &str, name: &str, size: f32) {
    let kept = app.downloads.is_kept(uri);
    let progress = app.downloads.progress(uri);
    let tip = match progress {
        Some((done, total)) if done < total => format!("Downloading {done} of {total} — click to remove"),
        Some(_) => "Downloaded — click to remove".to_string(),
        None => "Download to this computer".to_string(),
    };
    if icon_btn(ui, Icon::Download, size, &tip, kept).clicked() {
        app.act(if kept { Action::RemoveDownload(uri.to_string()) } else { Action::Download { uri: uri.to_string(), name: name.to_string() } });
    }
    if let Some((done, total)) = progress.filter(|(d, t)| d < t) {
        ui.label(RichText::new(format!("{done}/{total}")).font(theme::regular(11.5)).color(col::muted()));
    }
}

/// A grid of cards (one row of them when `one_row`), borrowing the items.
fn cards<'a>(app: &mut App, ui: &mut Ui, items: impl IntoIterator<Item = &'a Value>, one_row: bool) {
    let items: Vec<&Value> = items.into_iter().filter(|v| !v.is_null()).collect();
    let playing_ctx = app.pb.context_uri.clone();
    let is_playing = app.pb.is_playing;
    card_grid(ui, items.len(), one_row, |ui, i, w| {
        let v = items[i];
        let round = matches!(s(v, "type"), "artist" | "user");
        let uri = s(v, "uri");
        let r = widgets::card(ui, model::image(v, 300).as_deref(), s(v, "name").if_empty(s(v, "display_name")), &subtitle_for(v), round, w, is_playing && uri == playing_ctx);
        if r.open
            && let Some(route) = route_for(v)
        {
            app.act(Action::Navigate(route));
        }
        if r.play {
            if uri == playing_ctx {
                app.act(Action::TogglePlay);
            } else if s(v, "type") == "track" || s(v, "type") == "episode" {
                app.act(Action::PlayUris { uris: vec![uri.to_string()], index: 0 });
            } else {
                app.act(Action::PlayContext { context: uri.to_string(), track: None });
            }
        }
        r.response.context_menu(|ui| menus::any(app, ui, v));
    });
}

trait IfEmpty {
    fn if_empty<'a>(&'a self, other: &'a str) -> &'a str;
}
impl IfEmpty for str {
    fn if_empty<'a>(&'a self, other: &'a str) -> &'a str {
        if self.is_empty() { other } else { self }
    }
}

// ------------------------------------------------------------------- Home

fn recent_contexts(app: &mut App) -> Vec<Value> {
    let Some(rp) = app.want("/me/player/recently-played?limit=50") else { return vec![] };
    let uid = app.user_id();
    let mine = app.want_all("/me/playlists?limit=50", 2000);
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for it in model::items(&rp) {
        let track = &it["track"];
        let ctx = it.get("context").filter(|c| !c.is_null());
        let uri = ctx.map(|c| s(c, "uri").to_string()).unwrap_or_else(|| s(&track["album"], "uri").to_string());
        if uri.is_empty() || !seen.insert(uri.clone()) {
            continue;
        }
        let v = match model::kind_of(&uri) {
            "album" => Some(track["album"].clone()),
            "playlist" => mine
                .as_ref()
                .and_then(|m| model::items(m).iter().find(|p| s(p, "uri") == uri).cloned())
                .or_else(|| app.want(&format!("/playlists/{}?fields=id,uri,type,name,images,owner(display_name,id)", model::id_of(&uri))).map(|v| (*v).clone())),
            "artist" => app.want(&format!("/artists/{}", model::id_of(&uri))).map(|v| (*v).clone()),
            "show" => app.want(&format!("/shows/{}", model::id_of(&uri))).map(|v| (*v).clone()),
            "collection" | "user" if uri.ends_with(":collection") => Some(json!({ "type": "collection", "uri": format!("spotify:user:{uid}:collection"), "name": "Liked Songs" })),
            _ => None,
        };
        if let Some(v) = v {
            out.push(v);
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

/// Spotify's DJ: X picks songs for you and talks between them.
fn dj_page(app: &mut App, ui: &mut Ui) {
    let here = crate::dj::is_dj(&app.pb.context_uri);
    widgets::entity_header(ui, Some(crate::dj::COVER), false, "spotify", "DJ", |ui| {
        ui.label(RichText::new("Your personal DJ, X: songs you love, ones you used to, and new picks, with X introducing them as they play.").font(theme::regular(13.0)).color(col::sub()));
    });
    ui.add_space(theme::sp(1.25));
    ui.horizontal(|ui| {
        if widgets::play_btn(ui, here && app.pb.is_playing, 56.0).clicked() {
            app.act(if here { Action::TogglePlay } else { Action::PlayContext { context: crate::dj::URI.into(), track: None } });
        }
        if here && widgets::outline_button(ui, "Next pick").clicked() {
            app.act(Action::Next);
        }
    });
    ui.add_space(theme::sp(1.25));
    if let Some(line) = app.dj_caption().map(str::to_string) {
        ui.label(RichText::new(format!("“{line}”")).font(theme::regular(16.0)).color(col::text()));
        widgets::subtle(ui, "— X");
    } else if !app.settings.dj_voice {
        widgets::subtle(ui, "X's voice is off (Settings → Playback): the DJ plays its picks without talking.");
    }
}

/// A banner for the DJ at the top of Home.
fn dj_banner(app: &mut App, ui: &mut Ui) {
    let here = crate::dj::is_dj(&app.pb.context_uri);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "DJ"));
    let hovered = ui.rect_contains_pointer(rect);
    ui.painter().rect_filled(rect, theme::small_radius(), if hovered { col::hover() } else { col::raised() });
    let c = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 8.0), vec2(56.0, 56.0));
    paint_cover(ui, c, Some(crate::dj::COVER), false);
    ui.painter().text(pos2(c.right() + 14.0, rect.center().y - 10.0), Align2::LEFT_CENTER, "DJ", theme::regular(17.0), col::text());
    let sub = app
        .dj_caption()
        .filter(|_| here)
        .map(|l| format!("X: {l}"))
        .unwrap_or_else(|| if app.settings.dj_voice { "Songs picked for you, with X talking between them" } else { "Songs picked for you" }.into());
    widgets::text_trunc(ui, pos2(c.right() + 14.0, rect.center().y + 12.0), &sub, theme::regular(12.5), col::sub(), rect.width() - 160.0);
    let pr = Rect::from_center_size(pos2(rect.right() - 36.0, rect.center().y), vec2(36.0, 36.0));
    ui.painter().circle(pr.center(), 18.0, col::overlay(), egui::Stroke::new(1.5, col::accent()));
    icons::paint(ui.painter(), Rect::from_center_size(pr.center(), vec2(15.0, 15.0)), if here && app.pb.is_playing { Icon::Pause } else { Icon::Play }, col::accent());
    let play = ui.interact(pr.expand(4.0), resp.id.with("play"), Sense::click());
    if play.clicked() {
        app.act(if here { Action::TogglePlay } else { Action::PlayContext { context: crate::dj::URI.into(), track: None } });
    } else if resp.clicked() {
        app.act(Action::Navigate(Route::Playlist(model::id_of(crate::dj::URI).into())));
    }
    ui.add_space(theme::sp(1.5));
}

fn home(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (i, name) in ["all", "music", "podcasts", "audiobooks"].iter().enumerate() {
            if chip(ui, name, app.home_filter == i as u8) {
                app.home_filter = i as u8;
            }
        }
    });
    ui.add_space(theme::sp(1.5));
    ui.label(RichText::new(model::greeting().to_lowercase()).font(theme::regular(14.0)).color(col::sub()));
    ui.label(RichText::new("home_").font(theme::regular(40.0)));
    ui.add_space(theme::sp(1.25));
    let f = app.home_filter;
    if f <= 1 {
        dj_banner(app, ui);
    }

    if f <= 1 {
        let recents = recent_contexts(app);
        // Shortcut grid (Spotify's 8 tiles).
        let tiles: Vec<Value> = recents.iter().take(8).cloned().collect();
        let cols = if ui.available_width() > 900.0 { 4 } else { 2 };
        let gap = 8.0;
        let tw = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
        for chunk in tiles.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for v in chunk {
                    shortcut_tile(app, ui, v, tw);
                }
            });
            ui.add_space(gap);
        }
        if recents.len() > 8 {
            section_title(ui, "Jump back in", false);
            cards(app, ui, recents[8..].iter().filter(|v| s(v, "type") != "collection"), true);
        }
        if let Some(v) = app.want("/me/top/artists?limit=12&time_range=short_term")
            && !model::items(&v).is_empty()
        {
            section_title(ui, "Your top artists", false);
            cards(app, ui, model::items(&v), true);
        }
        if let Some(v) = app.want("/me/top/tracks?limit=12&time_range=short_term") {
            let mut seen = std::collections::HashSet::new();
            let albums: Vec<&Value> = model::items(&v).iter().filter_map(|t| t.get("album")).filter(|a| seen.insert(s(a, "id"))).collect();
            if !albums.is_empty() {
                section_title(ui, "On repeat", false);
                cards(app, ui, albums, true);
            }
        }
        if let Some(v) = app.want_all("/me/playlists?limit=50", 2000)
            && !model::items(&v).is_empty()
        {
            if section_title(ui, "Your playlists", true) {
                app.lib_tab = 0;
                app.lib_filter = crate::app::LibFilter::Playlists;
                app.act(Action::Navigate(Route::Library));
            }
            cards(app, ui, model::items(&v), true);
        }
        if let Some(v) = app.want_all("/me/albums?limit=50", 2000)
            && !model::items(&v).is_empty()
        {
            section_title(ui, "Saved albums", false);
            cards(app, ui, model::items(&v).iter().map(|i| &i["album"]), true);
        }
    }
    if f == 0 || f == 2 {
        if let Some(v) = app.want_all("/me/shows?limit=50", 500)
            && !model::items(&v).is_empty()
        {
            section_title(ui, "Your shows", false);
            cards(app, ui, model::items(&v).iter().map(|i| &i["show"]), f == 0);
        }
        if let Some(v) = app.want("/me/episodes?limit=10") {
            let eps: Vec<Value> = model::items(&v).iter().map(|i| i["episode"].clone()).collect();
            if !eps.is_empty() {
                if section_title(ui, "Your Episodes", true) {
                    app.act(Action::Navigate(Route::YourEpisodes));
                }
                for e in eps.iter().take(if f == 2 { 10 } else { 3 }) {
                    episode_row(app, ui, e, true, None);
                }
            }
        }
    }
    if (f == 0 || f == 3)
        && let Some(v) = app.want_all("/me/audiobooks?limit=50", 500)
    {
        let books: Vec<&Value> = model::items(&v).iter().map(|i| i.get("audiobook").unwrap_or(i)).collect();
        if !books.is_empty() {
            section_title(ui, "Your audiobooks", false);
            cards(app, ui, books, f == 0);
        } else if f == 3 {
            widgets::unavailable(ui, "no audiobooks yet", "Audiobooks you save appear here.");
        }
    }
}

fn shortcut_tile(app: &mut App, ui: &mut Ui, v: &Value, w: f32) {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 56.0), Sense::click());
    ui.painter().rect_filled(rect, theme::small_radius(), if resp.hovered() { col::selection() } else { col::raised() });
    let c = Rect::from_min_size(rect.min, vec2(56.0, 56.0));
    if s(v, "type") == "collection" {
        ui.painter().rect_filled(c, CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, Color32::from_rgb(110, 80, 230));
        icons::paint(ui.painter(), Rect::from_center_size(c.center(), vec2(22.0, 22.0)), Icon::HeartFilled, col::text());
    } else {
        paint_cover(ui, c, model::image(v, 64).as_deref(), s(v, "type") == "artist");
    }
    let uri = s(v, "uri").to_string();
    let here = app.pb.context_uri == uri;
    widgets::text_trunc(ui, pos2(c.right() + 12.0, rect.center().y), s(v, "name"), theme::regular(14.0), if here { col::accent() } else { col::text() }, rect.width() - 56.0 - 64.0);
    let b = Rect::from_center_size(pos2(rect.right() - 26.0, rect.center().y), vec2(34.0, 34.0));
    let pr = ui.interact(b, resp.id.with("p"), Sense::click());
    if ui.rect_contains_pointer(rect) || here {
        ui.painter().circle(b.center(), 16.0, col::accent_a(0.12), egui::Stroke::new(1.3, col::accent()));
        icons::paint(ui.painter(), Rect::from_center_size(b.center(), vec2(13.0, 13.0)), if here && app.pb.is_playing { Icon::Pause } else { Icon::Play }, col::accent());
    }
    let play = pr.clicked();
    if play {
        if here { app.act(Action::TogglePlay) } else { app.act(Action::PlayContext { context: uri.clone(), track: None }) }
    } else if resp.clicked() {
        if s(v, "type") == "collection" {
            app.act(Action::Navigate(Route::LikedSongs));
        } else if let Some(r) = route_for(v) {
            app.act(Action::Navigate(r));
        }
    }
    let v = v.clone();
    resp.context_menu(|ui| menus::any(app, ui, &v));
}

// ------------------------------------------------------------------- Search

fn remember_search(app: &mut App) {
    let q = app.search_committed.trim().to_string();
    if q.is_empty() {
        return;
    }
    app.settings.recent_searches.retain(|r| r != &q);
    app.settings.recent_searches.insert(0, q);
    app.settings.recent_searches.truncate(10);
    app.mark_settings_dirty();
}

/// `⌕ > search spotify` — the search page's own field.
fn search_field(app: &mut App, ui: &mut Ui) {
    let w = ui.available_width().min(theme::px(640.0));
    let (r, _) = ui.allocate_exact_size(vec2(w, theme::px(46.0)), Sense::hover());
    let id = egui::Id::new("search-field");
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    ui.painter().rect(
        r,
        theme::small_radius(),
        if focused { col::raised() } else { col::surface() },
        egui::Stroke::new(if focused { 1.2 } else { 1.0 }, if focused { col::accent() } else { col::border() }),
        egui::StrokeKind::Inside,
    );
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + theme::px(24.0), r.center().y), vec2(theme::px(17.0), theme::px(17.0))), Icon::Search, col::sub());
    ui.painter().text(pos2(r.left() + theme::px(46.0), r.center().y), Align2::LEFT_CENTER, ">", theme::regular(14.0), if focused { col::accent() } else { col::sub() });
    let field = Rect::from_min_max(pos2(r.left() + theme::px(64.0), r.top() + theme::px(11.0)), pos2(r.right() - theme::px(14.0), r.bottom() - theme::px(11.0)));
    ui.scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
        let te = egui::TextEdit::singleline(&mut app.search_text)
            .id(id)
            .frame(egui::Frame::NONE)
            .hint_text(RichText::new("search spotify — songs, artists, albums, podcasts").color(col::muted()))
            .font(theme::regular(14.0))
            .text_color(col::text())
            .desired_width(field.width());
        let resp = ui.add(te);
        if app.focus_search {
            resp.request_focus();
            app.focus_search = false;
        }
        if resp.changed() {
            app.search_edit_at = Some(std::time::Instant::now());
        }
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            app.search_edit_at = Some(std::time::Instant::now() - Duration::from_secs(1));
        }
    });
    ui.add_space(theme::sp(2.5));
}

fn search(app: &mut App, ui: &mut Ui) {
    search_field(app, ui);
    if let Some(t) = app.search_edit_at {
        if t.elapsed() > Duration::from_millis(350) {
            app.search_committed = app.search_text.trim().to_string();
            app.search_edit_at = None;
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(120));
        }
    }
    if app.search_text.trim().is_empty() {
        app.search_committed.clear();
    }
    let q = app.search_committed.clone();
    if q.is_empty() {
        if !app.settings.recent_searches.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Recent searches").font(theme::regular(22.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if link(ui, "Clear recent searches", theme::medium(13.0), col::sub()).clicked() {
                        app.settings.recent_searches.clear();
                        app.mark_settings_dirty();
                    }
                });
            });
            ui.add_space(theme::sp(0.75));
            ui.horizontal_wrapped(|ui| {
                for r in app.settings.recent_searches.clone() {
                    if chip(ui, &r, false) {
                        app.search_text = r.clone();
                        app.search_committed = r;
                    }
                }
            });
            ui.add_space(theme::sp(1.75));
        }
        ui.label(RichText::new("Browse all").font(theme::regular(22.0)));
        ui.add_space(theme::sp(1.0));
        let Some(v) = need(app, ui, "/browse/categories?limit=50") else { return };
        let cats = v["categories"]["items"].as_array().cloned().unwrap_or_default();
        card_grid(ui, cats.len(), false, |ui, i, w| {
            let c = &cats[i];
            let (rect, resp) = ui.allocate_exact_size(vec2(w, w * 0.56), Sense::click());
            let tint = c["color"].as_str().and_then(|h| crate::theme_config::Rgba::parse(h).ok()).map(|c| c.with_alpha(0.22).color32()).unwrap_or(col::surface());
            ui.painter().rect_filled(rect, theme::small_radius(), if resp.hovered() { col::hover() } else { col::surface() });
            ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(theme::px(3.0), rect.height())), egui::CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, tint.gamma_multiply(3.0));
            ui.painter().rect_stroke(rect, theme::small_radius(), egui::Stroke::new(1.0, if resp.hovered() { col::border() } else { col::divider() }), egui::StrokeKind::Inside);
            if let Some(u) = c["icons"][0]["url"].as_str() {
                widgets::mark_drawn(ui.ctx(), u);
                let ir = Rect::from_min_size(rect.right_bottom() - vec2(w * 0.38, w * 0.38) + vec2(8.0, 8.0), vec2(w * 0.34, w * 0.34));
                egui::Image::new(u.to_string()).corner_radius(theme::small_radius()).rotate(0.4, egui::Vec2::splat(0.5)).paint_at(ui, ir);
            }
            ui.painter().text(rect.left_top() + vec2(theme::px(16.0), theme::px(14.0)), Align2::LEFT_TOP, s(c, "name").to_lowercase(), theme::regular(15.0), col::text());
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.act(Action::Navigate(Route::Genre { id: s(c, "id").into(), name: s(c, "name").into() }));
            }
        });
        return;
    }

    ui.horizontal(|ui| {
        for (f, name) in [
            (SearchFilter::All, "All"),
            (SearchFilter::Songs, "Songs"),
            (SearchFilter::Artists, "Artists"),
            (SearchFilter::Albums, "Albums"),
            (SearchFilter::Playlists, "Playlists"),
            (SearchFilter::Podcasts, "Podcasts & Shows"),
            (SearchFilter::Episodes, "Episodes"),
            (SearchFilter::Audiobooks, "Audiobooks"),
        ] {
            if chip(ui, name, app.search_filter == f) {
                app.search_filter = f;
            }
        }
    });
    ui.add_space(theme::sp(1.75));
    let qe = enc(&q);
    let typed = |t: &str| format!("/search?q={qe}&type={t}&limit=10");
    let key = match app.search_filter {
        SearchFilter::All => None,
        SearchFilter::Songs => Some(("track", "tracks")),
        SearchFilter::Artists => Some(("artist", "artists")),
        SearchFilter::Albums => Some(("album", "albums")),
        SearchFilter::Playlists => Some(("playlist", "playlists")),
        SearchFilter::Podcasts => Some(("show", "shows")),
        SearchFilter::Episodes => Some(("episode", "episodes")),
        SearchFilter::Audiobooks => Some(("audiobook", "audiobooks")),
    };
    if let Some((t, _)) = key {
        let Some(v) = need_all(app, ui, &typed(t), 50) else { return };
        let items: Vec<Value> = model::items(&v).iter().filter(|i| !i.is_null()).cloned().collect();
        if items.is_empty() {
            no_results(ui, &q);
            return;
        }
        match t {
            "track" => {
                tracklist::table(app, ui, &rows_plain(&items), &Opts { album_col: true, cover: true, header: true, ..Default::default() });
                if ui.ui_contains_pointer() && ui.input(|i| i.pointer.any_click()) {
                    remember_search(app);
                }
            }
            "episode" => {
                for e in &items {
                    episode_row(app, ui, e, true, None);
                }
            }
            _ => cards(app, ui, &items, false),
        }
        return;
    }

    let path = format!("/search?q={qe}&type=track,artist,album,playlist,show,episode,audiobook&limit=10");
    let Some(v) = need(app, ui, &path) else { return };
    let list = |k: &str| -> Vec<Value> { v[k]["items"].as_array().map(|a| a.iter().filter(|i| !i.is_null()).cloned().collect()).unwrap_or_default() };
    let (tracks, artists, albums, playlists, shows, episodes, books) = (list("tracks"), list("artists"), list("albums"), list("playlists"), list("shows"), list("episodes"), list("audiobooks"));
    if [&tracks, &artists, &albums, &playlists, &shows, &episodes, &books].iter().all(|l| l.is_empty()) {
        no_results(ui, &q);
        return;
    }
    let ql = q.to_lowercase();
    let top = artists.iter().find(|a| s(a, "name").to_lowercase() == ql).or_else(|| tracks.first()).or_else(|| artists.first()).cloned();
    let wide = ui.available_width() > 760.0;
    let top_and_songs = |app: &mut App, ui: &mut Ui| {
        let lw = if wide { ui.available_width() * 0.4 } else { ui.available_width() };
        ui.vertical(|ui| {
            ui.set_width(lw);
            ui.label(RichText::new("Top result").font(theme::regular(22.0)));
            ui.add_space(theme::sp(1.0));
            if let Some(t) = &top {
                let (rect, resp) = ui.allocate_exact_size(vec2(lw, 220.0), Sense::click());
                ui.painter().rect_filled(rect, theme::radius(), if resp.hovered() { col::selection() } else { col::raised() });
                let round = s(t, "type") == "artist";
                paint_cover(ui, Rect::from_min_size(rect.min + vec2(20.0, 20.0), vec2(92.0, 92.0)), model::image(t, 300).as_deref(), round);
                widgets::text_trunc(ui, rect.min + vec2(20.0, 140.0), s(t, "name"), theme::regular(30.0), col::text(), rect.width() - 40.0);
                widgets::text_trunc(ui, rect.min + vec2(20.0, 176.0), &subtitle_for(t), theme::regular(13.5), col::sub(), rect.width() - 100.0);
                let b = Rect::from_center_size(rect.right_bottom() - vec2(44.0, 44.0), vec2(48.0, 48.0));
                if ui.rect_contains_pointer(rect) {
                    ui.painter().circle(b.center(), 23.0, col::accent_a(0.12), egui::Stroke::new(1.5, col::accent()));
                    icons::paint(ui.painter(), Rect::from_center_size(b.center(), vec2(18.0, 18.0)), Icon::Play, col::accent());
                }
                let pr = ui.interact(b, resp.id.with("p"), Sense::click());
                if pr.clicked() {
                    remember_search(app);
                    if s(t, "type") == "track" {
                        app.act(Action::PlayUris { uris: vec![s(t, "uri").into()], index: 0 });
                    } else {
                        app.act(Action::PlayContext { context: s(t, "uri").into(), track: None });
                    }
                } else if resp.clicked() {
                    remember_search(app);
                    if let Some(r) = route_for(t) {
                        app.act(Action::Navigate(r));
                    }
                }
                let t2 = t.clone();
                resp.context_menu(|ui| menus::any(app, ui, &t2));
            }
        });
        if !tracks.is_empty() {
            ui.vertical(|ui| {
                ui.label(RichText::new("Songs").font(theme::regular(22.0)));
                ui.add_space(theme::sp(1.0));
                let rows = rows_plain(&tracks);
                tracklist::table(app, ui, &rows, &Opts { cover: true, limit: Some(4), ..Default::default() });
            });
        }
    };
    // The top result is a song or an artist: with neither, the page starts with what matched.
    if top.is_some() {
        if wide {
            ui.horizontal_top(|ui| top_and_songs(app, ui));
        } else {
            ui.vertical(|ui| top_and_songs(app, ui));
        }
    }
    for (title, items) in [("Artists", artists), ("Albums", albums), ("Playlists", playlists), ("Podcasts", shows), ("Episodes", episodes), ("Audiobooks", books)] {
        if items.is_empty() {
            continue;
        }
        if section_title(ui, title, true) {
            app.search_filter = match title {
                "Artists" => SearchFilter::Artists,
                "Albums" => SearchFilter::Albums,
                "Playlists" => SearchFilter::Playlists,
                "Podcasts" => SearchFilter::Podcasts,
                "Episodes" => SearchFilter::Episodes,
                _ => SearchFilter::Audiobooks,
            };
        }
        cards(app, ui, &items, true);
    }
    if ui.ui_contains_pointer() && ui.input(|i| i.pointer.any_click()) {
        remember_search(app);
    }
}

fn no_results(ui: &mut Ui, q: &str) {
    widgets::unavailable(ui, &format!("No results found for “{q}”"), "Check the spelling, or try fewer or different words.");
}

fn genre(app: &mut App, ui: &mut Ui, id: &str, name: &str) {
    ui.label(RichText::new(name).font(theme::regular(48.0)));
    ui.add_space(theme::sp(1.5));
    let path = format!("/browse/categories/{id}/playlists?limit=50");
    match app.want(&path) {
        Some(v) => {
            let items = v["playlists"]["items"].as_array().cloned().unwrap_or_default();
            cards(app, ui, &items, false);
        }
        None => {
            if app.load_error(&path).is_some() {
                widgets::unavailable(ui, "couldn't load this category", "Spotify didn't return this page. Searching for its name may still find its playlists.");
                if ui.button(format!("Search for “{name}”")).clicked() {
                    app.search_text = name.to_string();
                    app.search_committed = name.to_string();
                    app.act(Action::Navigate(Route::Search));
                }
            } else {
                widgets::loading(ui);
            }
        }
    }
}

// ------------------------------------------------------------------- Playlist

fn playlist(app: &mut App, ui: &mut Ui, id: &str) {
    if crate::dj::is_dj(&format!("spotify:playlist:{id}")) {
        return dj_page(app, ui);
    }
    let Some(p) = need(app, ui, &format!("/playlists/{id}")) else { return };
    let me = app.user_id();
    let own = p["owner"]["id"].as_str() == Some(me.as_str());
    let editable = own || model::b(&p, "collaborative");
    let tracks_path = format!("/playlists/{id}/tracks?limit=100");
    let items_path = format!("/playlists/{id}/items?limit=100");
    let items = match app.want_all(&tracks_path, 10_000) {
        Some(v) => Some(v),
        None if app.load_error(&tracks_path).is_some() => app.want_all(&items_path, 10_000),
        None => None,
    };
    let rows = items.as_ref().map(|v| rows_from_items(model::items(v))).unwrap_or_default();
    let total = p["tracks"]["total"].as_u64().or_else(|| p["items"]["total"].as_u64()).unwrap_or(rows.len() as u64);
    let dur: u64 = rows.iter().map(|r| model::u(r.item, "duration_ms")).sum();

    let kind = if model::b(&p, "collaborative") {
        "Collaborative Playlist"
    } else if model::b(&p, "public") {
        "Public Playlist"
    } else {
        "Playlist"
    };
    let cover = widgets::entity_header(ui, model::image(&p, 300).as_deref(), false, kind, s(&p, "name"), |ui| {
        let desc = model::plain(s(&p, "description"));
        if !desc.is_empty() {
            ui.label(RichText::new(desc).color(col::sub()).size(13.5));
            ui.end_row();
        }
        if link(ui, s(&p["owner"], "display_name"), theme::regular(13.5), col::text()).clicked() {
            app.act(Action::Navigate(Route::Profile(s(&p["owner"], "id").into())));
        }
        let saves = model::u(&p["followers"], "total");
        if saves > 0 {
            widgets::dot(ui);
            widgets::subtle(ui, if saves == 1 { "1 save".to_string() } else { format!("{} saves", model::fmt_count(saves)) });
        }
        widgets::dot(ui);
        widgets::subtle(ui, format!("{}{}", model::songs(total), if dur > 0 { format!(", {}", model::fmt_long(dur)) } else { String::new() }));
    });
    if own && cover.clicked() {
        app.modal = Some(Modal::EditPlaylist {
            id: id.into(),
            name: s(&p, "name").into(),
            desc: model::plain(s(&p, "description")),
            public: model::b(&p, "public"),
            collaborative: model::b(&p, "collaborative"),
        });
    }
    let uri = s(&p, "uri").to_string();
    action_row(app, ui, &uri, if own { None } else { Some(&uri) }, |app, ui| menus::playlist(app, ui, &p));
    ui.horizontal(|ui| {
        tracklist::controls(app, ui, true);
    });
    if items.is_none() {
        // The path being waited for: the newer one once the first has failed.
        let waiting = if app.load_error(&tracks_path).is_some() { &items_path } else { &tracks_path };
        show_wait(app, ui, waiting);
        return;
    }
    if rows.is_empty() && own {
        ui.add_space(theme::sp(2.5));
        hairline_label(ui, "Let's find something for your playlist");
        find_for_playlist(app, ui, id);
        return;
    }
    tracklist::table(app, ui, &rows, &Opts { album_col: true, added_col: true, cover: true, header: true, playlist: Some((id.to_string(), editable)), context: Some(uri), ..Default::default() });
    if own {
        ui.add_space(theme::sp(2.5));
        hairline_label(ui, "Find more");
        find_for_playlist(app, ui, id);
    }
}

fn hairline_label(ui: &mut Ui, t: &str) {
    widgets::hairline(ui);
    ui.add_space(theme::sp(1.25));
    ui.label(RichText::new(t).font(theme::regular(20.0)));
    ui.add_space(theme::sp(0.75));
}

fn find_for_playlist(app: &mut App, ui: &mut Ui, pid: &str) {
    let field = ui.add(egui::TextEdit::singleline(&mut app.pl_find).hint_text("Search for songs").desired_width(360.0));
    let (edited, now) = (egui::Id::new("pl-find-edited"), ui.input(|i| i.time));
    if field.changed() {
        ui.data_mut(|d| d.insert_temp(edited, now));
    }
    let q = app.pl_find.trim().to_string();
    if q.chars().count() < 2 {
        return;
    }
    // Searches once typing pauses, not on every key.
    let quiet = now - ui.data(|d| d.get_temp::<f64>(edited)).unwrap_or(0.0);
    if quiet < 0.35 {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.36 - quiet));
        widgets::loading(ui);
        return;
    }
    let path = format!("/search?q={}&type=track&limit=10", enc(&q));
    let Some(v) = app.want(&path) else {
        widgets::loading(ui);
        return;
    };
    for t in model::arr(&v["tracks"], "items") {
        ui.horizontal(|ui| {
            widgets::cover(ui, model::image(t, 64).as_deref(), 40.0, false);
            ui.vertical(|ui| {
                ui.label(RichText::new(s(t, "name")).color(col::text()));
                widgets::subtle(ui, model::artists_line(t));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::outline_button(ui, "Add").clicked() {
                    app.act(Action::AddToPlaylist { playlist: pid.into(), uris: vec![s(t, "uri").into()] });
                }
            });
        });
    }
}

// ------------------------------------------------------------------- Album

/// An album's songs, each with the album attached (album tracks come without it, and
/// rows link back through it). `all` is the full track list when the album object
/// only embeds the first page. Built once per answer, not every frame.
pub(super) fn album_tracks(ui: &Ui, album: &Arc<Value>, all: Option<&Arc<Value>>) -> Arc<Vec<Value>> {
    type Tracks = Arc<Vec<Value>>;
    let id = egui::Id::new("album-tracks");
    let key = (Arc::as_ptr(album) as usize, all.map_or(0, |a| Arc::as_ptr(a) as usize));
    if let Some((k, tracks)) = ui.data(|d| d.get_temp::<((usize, usize), Tracks)>(id))
        && k == key
    {
        return tracks;
    }
    let stub = json!({ "id": s(album, "id"), "name": s(album, "name"), "uri": s(album, "uri"), "images": album["images"].clone() });
    let source = all.map(|a| model::items(a)).unwrap_or_else(|| model::items(&album["tracks"]));
    let tracks: Tracks = Arc::new(
        source
            .iter()
            .map(|t| {
                let mut t = t.clone();
                if t.get("album").is_none_or(Value::is_null) {
                    t["album"] = stub.clone();
                }
                t
            })
            .collect(),
    );
    ui.data_mut(|d| d.insert_temp(id, (key, tracks.clone())));
    tracks
}

fn album(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(a) = need(app, ui, &format!("/albums/{id}")) else { return };
    let total = model::u(&a["tracks"], "total");
    // Past the first page, the whole list comes from the album's tracks endpoint.
    let all = if (model::items(&a["tracks"]).len() as u64) < total { app.want_all(&format!("/albums/{id}/tracks?limit=50"), 2000) } else { None };
    let tracks = album_tracks(ui, &a, all.as_ref());
    let dur: u64 = tracks.iter().map(|t| model::u(t, "duration_ms")).sum();
    let kind = match s(&a, "album_type") {
        "single" => "Single",
        "compilation" => "Compilation",
        _ => "Album",
    };
    widgets::entity_header(ui, model::image(&a, 300).as_deref(), false, kind, s(&a, "name"), |ui| {
        if let Some(artist) = model::arr(&a, "artists").first() {
            let aid = s(artist, "id").to_string();
            if let Some(full) = app.want(&format!("/artists/{aid}")) {
                widgets::cover(ui, model::image(&full, 64).as_deref(), 24.0, true);
            }
        }
        widgets::artist_links(app, ui, &a, 13.5, col::text());
        widgets::dot(ui);
        widgets::subtle(ui, model::year(s(&a, "release_date")));
        widgets::dot(ui);
        widgets::subtle(ui, format!("{}, {}", model::songs(tracks.len() as u64), model::fmt_long(dur)));
    });
    let uri = s(&a, "uri").to_string();
    action_row(app, ui, &uri, Some(&uri), |app, ui| menus::album(app, ui, &a));
    tracklist::controls(app, ui, true);
    tracklist::table(app, ui, &rows_plain(&tracks), &Opts { track_numbers: true, discs: true, header: true, context: Some(uri), ..Default::default() });
    ui.add_space(theme::sp(2.0));
    widgets::subtle(ui, model::fmt_date(s(&a, "release_date")));
    for c in model::arr(&a, "copyrights") {
        let sym = if s(c, "type") == "P" { "℗" } else { "©" };
        let text = s(c, "text");
        let text = if text.starts_with('©') || text.starts_with('℗') || text.starts_with("(C)") || text.starts_with("(P)") { text.to_string() } else { format!("{sym} {text}") };
        ui.label(RichText::new(text).size(11.5).color(col::sub()));
    }
    if let Some(artist) = model::arr(&a, "artists").first().cloned() {
        let aid = s(&artist, "id").to_string();
        if let Some(v) = app.want(&format!("/artists/{aid}/albums?include_groups=album,single&limit=12")) {
            let others: Vec<Value> = model::items(&v).iter().filter(|x| s(x, "id") != id).cloned().collect();
            if !others.is_empty() {
                if section_title(ui, &format!("More by {}", s(&artist, "name")), true) {
                    app.act(Action::Navigate(Route::Discography(aid)));
                }
                cards(app, ui, &others, true);
            }
        }
    }
}

// ------------------------------------------------------------------- Artist

fn artist(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(a) = need(app, ui, &format!("/artists/{id}")) else { return };
    // Banner
    let w = ui.available_width();
    let (banner, _) = ui.allocate_exact_size(vec2(w, 260.0), Sense::hover());
    if let Some(u) = model::image(&a, 640) {
        widgets::mark_drawn(ui.ctx(), &u);
        let img = egui::Image::new(u).corner_radius(theme::radius());
        // Crop the square image to the banner's aspect ratio.
        let aspect = banner.height() / banner.width();
        let uv = Rect::from_min_max(pos2(0.0, 0.5 - aspect / 2.0), pos2(1.0, 0.5 + aspect / 2.0));
        img.uv(uv).paint_at(ui, banner);
    }
    // Fade to dark at the bottom so the name stays readable.
    let steps = 24;
    let top = banner.top() + banner.height() * 0.35;
    let h = (banner.bottom() - top) / steps as f32;
    for i in 0..steps {
        let a = (i as f32 / steps as f32 * 150.0) as u8;
        let r = Rect::from_min_max(pos2(banner.left(), top + h * i as f32), pos2(banner.right(), top + h * (i + 1) as f32));
        let cr = if i == steps - 1 { CornerRadius { nw: 0, ne: 0, sw: 10, se: 10 } } else { CornerRadius::ZERO };
        ui.painter().rect_filled(r, cr, Color32::from_black_alpha(a));
    }
    widgets::caps(ui, banner.left_bottom() + vec2(22.0, -theme::px(92.0)), "artist", 11.0, col::sub());
    ui.painter().text(
        banner.left_bottom() + vec2(20.0, -theme::px(42.0)),
        Align2::LEFT_BOTTOM,
        s(&a, "name"),
        theme::regular(if s(&a, "name").chars().count() > 24 { 30.0 } else { 40.0 }),
        col::text(),
    );
    let listeners =
        a["monthly_listeners"].as_u64().map(|n| format!("{} monthly listeners", model::fmt_count(n))).unwrap_or_else(|| format!("{} followers", model::fmt_count(model::u(&a["followers"], "total"))));
    ui.painter().text(banner.left_bottom() + vec2(22.0, -theme::px(20.0)), Align2::LEFT_BOTTOM, listeners, theme::regular(13.0), col::sub());

    let uri = s(&a, "uri").to_string();
    ui.add_space(theme::sp(2.0));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let here = app.pb.context_uri == uri;
        if widgets::play_btn(ui, here && app.pb.is_playing, 56.0).clicked() {
            if here { app.act(Action::TogglePlay) } else { app.act(Action::PlayContext { context: uri.clone(), track: None }) }
        }
        if icon_btn(ui, Icon::Shuffle, 26.0, "Shuffle", app.pb.shuffle).clicked() {
            let sh = !app.pb.shuffle;
            app.act(Action::SetShuffle(sh));
        }
        let following = app.is_saved(&uri) == Some(true);
        if widgets::outline_button(ui, if following { "Following" } else { "Follow" }).clicked() {
            app.act(Action::Save { uri: uri.clone(), save: !following });
        }
        let more = icon_btn(ui, Icon::More, 26.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| menus::artist(app, ui, &a));
    });

    if let Some(v) = app.want(&format!("/artists/{id}/top-tracks?market=from_token")) {
        let tracks = model::arr(&v, "tracks").to_vec();
        if !tracks.is_empty() {
            section_title(ui, "Popular", false);
            let limit = if app.artist_more { 10 } else { 5 };
            let saved_filter = std::mem::take(&mut app.page_filter);
            tracklist::table(app, ui, &rows_plain(&tracks), &Opts { cover: true, limit: Some(limit), ..Default::default() });
            app.page_filter = saved_filter;
            if link(ui, if app.artist_more { "Show less" } else { "See more" }, theme::regular(13.5), col::sub()).clicked() {
                app.artist_more = !app.artist_more;
            }
        }
    }

    if let Some(v) = app.want_all(&format!("/artists/{id}/albums?include_groups=album,single,compilation&limit=50"), 200) {
        let all = model::items(&v);
        if !all.is_empty() {
            if section_title(ui, "Discography", true) {
                app.act(Action::Navigate(Route::Discography(id.into())));
            }
            disco_chips(app, ui);
            let list = filter_disco(app, all);
            cards(app, ui, list, true);
        }
    }
    if let Some(v) = app.want(&format!("/artists/{id}/related-artists")) {
        let rel = model::arr(&v, "artists");
        if !rel.is_empty() {
            section_title(ui, "Fans also like", false);
            cards(app, ui, rel, true);
        }
    }
    if let Some(v) = app.want(&format!("/artists/{id}/albums?include_groups=appears_on&limit=20")) {
        let ap = model::items(&v);
        if !ap.is_empty() {
            section_title(ui, "Appears On", false);
            cards(app, ui, ap, true);
        }
    }
    section_title(ui, "About", false);
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width().min(700.0));
        ui.horizontal(|ui| {
            widgets::cover(ui, model::image(&a, 300).as_deref(), 120.0, false);
            ui.vertical(|ui| {
                ui.label(RichText::new(format!("{} followers", model::fmt_count(model::u(&a["followers"], "total")))).font(theme::regular(18.0)));
                let genres: Vec<&str> = model::arr(&a, "genres").iter().filter_map(Value::as_str).collect();
                if !genres.is_empty() {
                    widgets::subtle(ui, genres.join(" · "));
                }
                if let Some(n) = a["monthly_listeners"].as_u64() {
                    widgets::subtle(ui, format!("{} monthly listeners", model::fmt_count(n)));
                }
                let bio = s(&a, "biography");
                if !bio.is_empty() {
                    ui.add_space(theme::sp(0.75));
                    ui.add(egui::Label::new(RichText::new(bio).font(theme::regular(12.5)).color(col::sub())).wrap());
                }
            });
        });
    });
}

fn disco_chips(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for (i, n) in ["All releases", "Albums", "Singles and EPs", "Compilations"].iter().enumerate() {
            if chip(ui, n, app.disco_filter == i as u8) {
                app.disco_filter = i as u8;
            }
        }
    });
    ui.add_space(theme::sp(1.25));
}

/// The releases the discography chips pick, newest first.
fn filter_disco<'a>(app: &App, all: &'a [Value]) -> Vec<&'a Value> {
    let mut list: Vec<&Value> = all
        .iter()
        .filter(|x| match app.disco_filter {
            1 => s(x, "album_type") == "album",
            2 => s(x, "album_type") == "single" || s(x, "album_type") == "ep",
            3 => s(x, "album_type") == "compilation",
            _ => true,
        })
        .collect();
    list.sort_by(|a, b| s(b, "release_date").cmp(s(a, "release_date")));
    list
}

fn discography(app: &mut App, ui: &mut Ui, id: &str) {
    let name = app.want(&format!("/artists/{id}")).map(|a| s(&a, "name").to_string()).unwrap_or_default();
    if link(ui, &name, theme::regular(28.0), col::text()).clicked() {
        app.act(Action::Navigate(Route::Artist(id.into())));
    }
    ui.add_space(theme::sp(1.0));
    disco_chips(app, ui);
    let Some(v) = need_all(app, ui, &format!("/artists/{id}/albums?include_groups=album,single,compilation&limit=50"), 500) else { return };
    let list = filter_disco(app, model::items(&v));
    cards(app, ui, list, false);
}

// ------------------------------------------------------------------- Podcasts

/// One episode. `show` is the show it's from, for its menu, when the episode doesn't say.
fn episode_row(app: &mut App, ui: &mut Ui, e: &Value, with_show: bool, show: Option<&Value>) {
    let uri = s(e, "uri").to_string();
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 132.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        // Same height as a drawn row, so the list doesn't shift while scrolling.
        widgets::hairline(ui);
        return;
    }
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
    }
    let c = Rect::from_min_size(rect.min + vec2(8.0, 10.0), vec2(112.0, 112.0));
    paint_cover(ui, c, model::image(e, 300).as_deref(), false);
    let x = c.right() + 16.0;
    let tw = rect.right() - x - 12.0;
    let current = app.pb.uri() == uri;
    widgets::text_trunc(ui, pos2(x, rect.top() + 22.0), s(e, "name"), theme::regular(16.0), if current { col::accent() } else { col::text() }, tw);
    let mut y = rect.top() + 44.0;
    if with_show && let Some(show) = e.get("show") {
        widgets::text_trunc(ui, pos2(x, y), s(show, "name"), theme::regular(13.0), col::text(), tw);
        y += 18.0;
    }
    let desc = s(e, "description");
    let job = ui.painter().layout(desc.chars().take(300).collect(), theme::regular(12.5), col::sub(), tw);
    for row in job.rows.iter().take(2) {
        ui.painter().text(pos2(x, y - 7.0), Align2::LEFT_TOP, row.text(), theme::regular(12.5), col::sub());
        y += 16.0;
    }
    let resume = &e["resume_point"];
    let dur = model::u(e, "duration_ms");
    let pos = model::u(resume, "resume_position_ms");
    let meta = if model::b(resume, "fully_played") {
        format!("{} • Played", model::fmt_date(s(e, "release_date")))
    } else if pos > 0 {
        format!("{} • {} left", model::fmt_date(s(e, "release_date")), model::fmt_long(dur.saturating_sub(pos)))
    } else {
        format!("{} • {}", model::fmt_date(s(e, "release_date")), model::fmt_long(dur))
    };
    let by = rect.bottom() - 26.0;
    let b = Rect::from_center_size(pos2(x + 16.0, by), vec2(32.0, 32.0));
    let pr = ui.interact(b, resp.id.with("p"), Sense::click());
    ui.painter().circle(b.center(), 15.0, col::accent_a(0.10), egui::Stroke::new(1.3, col::accent()));
    icons::paint(ui.painter(), Rect::from_center_size(b.center(), vec2(12.0, 12.0)), if current && app.pb.is_playing { Icon::Pause } else { Icon::Play }, col::accent());
    ui.painter().text(pos2(x + 44.0, by), Align2::LEFT_CENTER, meta, theme::regular(12.5), col::sub());
    if pos > 0 && dur > 0 && !model::b(resume, "fully_played") {
        let bar = Rect::from_min_size(pos2(x + 230.0, by - 2.0), vec2(80.0, 4.0));
        ui.painter().rect_filled(bar, CornerRadius::same(2), col::track());
        ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(80.0 * pos as f32 / dur as f32, 4.0)), CornerRadius::same(2), col::text());
    }
    let saved = app.is_saved(&uri) == Some(true);
    let sr = Rect::from_center_size(pos2(rect.right() - 60.0, by), vec2(20.0, 20.0));
    let sresp = ui.interact(sr.expand(4.0), resp.id.with("s"), Sense::click()).on_hover_text(if saved { "Remove from Your Episodes" } else { "Save to Your Episodes" });
    icons::paint(ui.painter(), sr, if saved { Icon::Check } else { Icon::Plus }, if saved { col::accent() } else { col::sub() });
    if sresp.clicked() {
        app.act(Action::Save { uri: uri.clone(), save: !saved });
    }
    if pr.clicked() {
        if current { app.act(Action::TogglePlay) } else { app.act(Action::PlayUris { uris: vec![uri.clone()], index: 0 }) }
    } else if resp.clicked() && !sresp.clicked() {
        app.act(Action::Navigate(Route::Episode(s(e, "id").into())));
    }
    resp.context_menu(|ui| match show {
        Some(sh) => {
            let mut e = e.clone();
            e["show"] = json!({ "id": s(sh, "id"), "name": s(sh, "name"), "uri": s(sh, "uri") });
            menus::track(app, ui, &e, None, &[]);
        }
        None => menus::track(app, ui, e, None, &[]),
    });
    widgets::hairline(ui);
}

fn show_page(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(sh) = need(app, ui, &format!("/shows/{id}")) else { return };
    widgets::entity_header(ui, model::image(&sh, 300).as_deref(), false, "Podcast", s(&sh, "name"), |ui| {
        ui.label(RichText::new(s(&sh, "publisher")).font(theme::regular(16.0)));
    });
    let uri = s(&sh, "uri").to_string();
    ui.add_space(theme::sp(2.0));
    ui.horizontal(|ui| {
        let saved = app.is_saved(&uri) == Some(true);
        if widgets::outline_button(ui, if saved { "Following" } else { "Follow" }).clicked() {
            app.act(Action::Save { uri: uri.clone(), save: !saved });
        }
        let more = icon_btn(ui, Icon::More, 24.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| menus::show(app, ui, &sh));
    });
    let wide = ui.available_width() > 860.0;
    let eps_ui = |app: &mut App, ui: &mut Ui| {
        ui.label(RichText::new("All Episodes").font(theme::regular(22.0)));
        ui.add_space(theme::sp(0.75));
        if let Some(v) = need_all(app, ui, &format!("/shows/{id}/episodes?limit=50"), 1000) {
            for e in model::items(&v).iter().filter(|e| !e.is_null()) {
                episode_row(app, ui, e, false, Some(&sh));
            }
        }
    };
    let about = |ui: &mut Ui| {
        theme::card().show(ui, |ui| {
            ui.label(RichText::new("About").font(theme::regular(20.0)));
            ui.add_space(theme::sp(0.75));
            ui.label(RichText::new(model::plain(s(&sh, "description"))).size(13.5).color(col::sub()));
            ui.add_space(theme::sp(0.75));
            let n = model::u(&sh, "total_episodes");
            widgets::subtle(ui, if n == 1 { "1 episode".to_string() } else { format!("{n} episodes") });
        });
    };
    ui.add_space(theme::sp(2.0));
    if wide {
        ui.horizontal_top(|ui| {
            let w = ui.available_width();
            ui.vertical(|ui| {
                ui.set_width(w * 0.64);
                eps_ui(app, ui);
            });
            ui.vertical(|ui| {
                ui.set_width(w * 0.34);
                about(ui);
            });
        });
    } else {
        about(ui);
        ui.add_space(theme::sp(1.5));
        eps_ui(app, ui);
    }
}

fn episode(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(e) = need(app, ui, &format!("/episodes/{id}")) else { return };
    widgets::entity_header(ui, model::image(&e, 300).as_deref(), false, "Podcast Episode", s(&e, "name"), |ui| {
        if let Some(show) = e.get("show")
            && link(ui, s(show, "name"), theme::regular(16.0), col::text()).clicked()
        {
            app.act(Action::Navigate(Route::Show(s(show, "id").into())));
        }
    });
    ui.add_space(theme::sp(1.0));
    widgets::subtle(ui, format!("{} • {}", model::fmt_date(s(&e, "release_date")), model::fmt_long(model::u(&e, "duration_ms"))));
    let uri = s(&e, "uri").to_string();
    ui.add_space(theme::sp(1.5));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let current = app.pb.uri() == uri;
        if widgets::play_btn(ui, current && app.pb.is_playing, 56.0).clicked() {
            if current { app.act(Action::TogglePlay) } else { app.act(Action::PlayUris { uris: vec![uri.clone()], index: 0 }) }
        }
        let saved = app.is_saved(&uri) == Some(true);
        if icon_btn(ui, if saved { Icon::Check } else { Icon::Plus }, 26.0, if saved { "Remove from Your Episodes" } else { "Save to Your Episodes" }, saved).clicked() {
            app.act(Action::Save { uri: uri.clone(), save: !saved });
        }
        let more = icon_btn(ui, Icon::More, 26.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| menus::track(app, ui, &e, None, &[]));
    });
    section_title(ui, "Episode Description", false);
    ui.label(RichText::new(s(&e, "description")).size(14.0).color(col::sub()));
    if let Some(show) = e.get("show") {
        ui.add_space(theme::sp(1.75));
        if widgets::outline_button(ui, "See all episodes").clicked() {
            app.act(Action::Navigate(Route::Show(s(show, "id").into())));
        }
    }
}

fn audiobook(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(b) = need(app, ui, &format!("/audiobooks/{id}")) else { return };
    widgets::entity_header(ui, model::image(&b, 300).as_deref(), false, "Audiobook", s(&b, "name"), |ui| {
        ui.label(RichText::new(model::artists_line(&b)).font(theme::regular(15.0)));
        let narrators: Vec<&str> = model::arr(&b, "narrators").iter().map(|n| s(n, "name")).collect();
        if !narrators.is_empty() {
            widgets::dot(ui);
            widgets::subtle(ui, format!("Narrated by {}", narrators.join(", ")));
        }
    });
    let uri = s(&b, "uri").to_string();
    action_row(app, ui, &uri, Some(&uri), |app, ui| menus::audiobook(app, ui, &b));
    section_title(ui, "Description", false);
    ui.label(RichText::new(model::plain(s(&b, "html_description").if_empty(s(&b, "description")))).size(14.0).color(col::sub()));
    section_title(ui, "Chapters", false);
    let Some(v) = need_all(app, ui, &format!("/audiobooks/{id}/chapters?limit=50"), 2000) else { return };
    for (i, c) in model::items(&v).iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        if resp.hovered() {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
        }
        let current = app.pb.uri() == s(c, "uri");
        ui.painter().text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, (i + 1).to_string(), theme::regular(13.0), col::sub());
        widgets::text_trunc(ui, pos2(rect.left() + 48.0, rect.center().y - 8.0), s(c, "name"), theme::regular(14.5), if current { col::accent() } else { col::text() }, rect.width() - 140.0);
        ui.painter().text(pos2(rect.left() + 48.0, rect.center().y + 11.0), Align2::LEFT_CENTER, model::fmt_long(model::u(c, "duration_ms")), theme::regular(12.0), col::sub());
        if resp.double_clicked() {
            app.act(Action::PlayContext { context: uri.clone(), track: Some(s(c, "uri").into()) });
        }
    }
}

// ------------------------------------------------------------------- Track

fn track(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(t) = need(app, ui, &format!("/tracks/{id}")) else { return };
    widgets::entity_header(ui, model::image(&t, 300).as_deref(), false, "Song", s(&t, "name"), |ui| {
        widgets::artist_links(app, ui, &t, 13.5, col::text());
        if let Some(a) = t.get("album") {
            widgets::dot(ui);
            if link(ui, s(a, "name"), theme::regular(13.5), col::text()).clicked() {
                app.act(Action::Navigate(Route::Album(s(a, "id").into())));
            }
            widgets::dot(ui);
            widgets::subtle(ui, model::year(s(a, "release_date")));
        }
        widgets::dot(ui);
        widgets::subtle(ui, model::fmt_ms(model::u(&t, "duration_ms")));
        widgets::dot(ui);
        widgets::subtle(ui, format!("Popularity {}", model::u(&t, "popularity")));
    });
    let uri = s(&t, "uri").to_string();
    let album_uri = t.get("album").map(|a| s(a, "uri").to_string()).unwrap_or_default();
    ui.add_space(theme::sp(2.0));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let current = app.pb.uri() == uri;
        if widgets::play_btn(ui, current && app.pb.is_playing, 56.0).clicked() {
            if current {
                app.act(Action::TogglePlay);
            } else if !album_uri.is_empty() {
                app.act(Action::PlayContext { context: album_uri.clone(), track: Some(uri.clone()) });
            } else {
                app.act(Action::PlayUris { uris: vec![uri.clone()], index: 0 });
            }
        }
        let saved = app.is_saved(&uri) == Some(true);
        if icon_btn(ui, if saved { Icon::HeartFilled } else { Icon::Heart }, 26.0, if saved { "Remove from Liked Songs" } else { "Save to Liked Songs" }, saved).clicked() {
            app.act(Action::Save { uri: uri.clone(), save: !saved });
        }
        let more = icon_btn(ui, Icon::More, 26.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| menus::track(app, ui, &t, None, &[]));
    });
    let artists = model::arr(&t, "artists").to_vec();
    ui.add_space(theme::sp(1.5));
    for a in &artists {
        let aid = s(a, "id").to_string();
        let img = app.want(&format!("/artists/{aid}")).and_then(|v| model::image(&v, 160));
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(600.0), 96.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
        }
        paint_cover(ui, Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(80.0, 80.0)), img.as_deref(), true);
        ui.painter().text(rect.min + vec2(104.0, 34.0), Align2::LEFT_CENTER, "Artist", theme::regular(12.5), col::sub());
        ui.painter().text(rect.min + vec2(104.0, 56.0), Align2::LEFT_CENTER, s(a, "name"), theme::regular(16.0), col::text());
        if resp.clicked() {
            app.act(Action::Navigate(Route::Artist(aid)));
        }
    }
    if let Some(a) = artists.first() {
        let aid = s(a, "id").to_string();
        if let Some(v) = app.want(&format!("/artists/{aid}/top-tracks?market=from_token")) {
            let tracks = model::arr(&v, "tracks").to_vec();
            if !tracks.is_empty() {
                ui.add_space(theme::sp(1.0));
                widgets::subtle(ui, "Popular songs by");
                ui.label(RichText::new(s(a, "name")).font(theme::regular(22.0)));
                tracklist::table(app, ui, &rows_plain(&tracks), &Opts { cover: true, limit: Some(5), ..Default::default() });
            }
        }
        if let Some(v) = app.want(&format!("/artists/{aid}/albums?include_groups=album,single&limit=12")) {
            if section_title(ui, &format!("Releases by {}", s(a, "name")), true) {
                app.act(Action::Navigate(Route::Discography(aid.clone())));
            }
            cards(app, ui, model::items(&v), true);
        }
    }
}

// ------------------------------------------------------------------- Profile

fn profile(app: &mut App, ui: &mut Ui, id: &str) {
    let me_id = app.user_id();
    let is_me = id.is_empty() || id == me_id;
    let path = if is_me { "/me".to_string() } else { format!("/users/{}", enc(id)) };
    let Some(u) = need(app, ui, &path) else { return };
    let lists = if is_me { app.want_all("/me/playlists?limit=50", 2000) } else { app.want_all(&format!("/users/{}/playlists?limit=50", enc(id)), 500) };
    // Your own profile shows your public playlists, as others see it.
    let playlists: Vec<&Value> =
        lists.as_deref().map(|v| model::items(v).iter().filter(|p| !is_me || (p["owner"]["id"].as_str() == Some(me_id.as_str()) && model::b(p, "public"))).collect()).unwrap_or_default();
    let follows = if is_me { app.want_all("/me/following?type=artist&limit=50", 2000) } else { None };
    let following: &[Value] = follows.as_deref().map(model::items).unwrap_or(&[]);
    widgets::entity_header(ui, model::image(&u, 300).as_deref(), true, "Profile", s(&u, "display_name"), |ui| {
        widgets::subtle(ui, format!("{} Public Playlists", playlists.len()));
        widgets::dot(ui);
        widgets::subtle(ui, format!("{} Followers", model::fmt_count(model::u(&u["followers"], "total"))));
        if is_me {
            widgets::dot(ui);
            widgets::subtle(ui, format!("{} Following", following.len()));
        }
    });
    ui.add_space(theme::sp(2.0));
    ui.horizontal(|ui| {
        let more = icon_btn(ui, Icon::More, 24.0, "More options", false);
        egui::Popup::menu(&more).show(|ui| menus::user(app, ui, &u));
    });
    if is_me {
        if let Some(v) = app.want("/me/top/artists?limit=10&time_range=short_term")
            && !model::items(&v).is_empty()
        {
            section_title(ui, "Top artists this month", false);
            widgets::subtle(ui, "Only visible to you");
            cards(app, ui, model::items(&v), true);
        }
        if let Some(v) = app.want("/me/top/tracks?limit=10&time_range=short_term")
            && !model::items(&v).is_empty()
        {
            section_title(ui, "Top songs this month", false);
            widgets::subtle(ui, "Only visible to you");
            tracklist::table(app, ui, &rows_plain(model::items(&v)), &Opts { album_col: true, cover: true, limit: Some(4), ..Default::default() });
        }
    }
    if !playlists.is_empty() {
        section_title(ui, "Public Playlists", false);
        cards(app, ui, playlists, false);
    }
    if !following.is_empty() {
        section_title(ui, "Following", false);
        cards(app, ui, following, true);
    }
}

// ------------------------------------------------------------------- Collections

fn gradient_tile(ui: &mut Ui, icon: Icon) {
    let size = theme::px(180.0);
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().rect(r, theme::small_radius(), col::accent_a(0.14), egui::Stroke::new(1.0, col::accent_a(0.5)), egui::StrokeKind::Inside);
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(size * 0.32, size * 0.32)), icon, col::accent());
}

fn collection_header(app: &mut App, ui: &mut Ui, icon: Icon, title: &str, meta: String) {
    let name = app.me.as_ref().map(|m| s(m, "display_name").to_string()).unwrap_or_default();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("page-title"), title.to_string()));
    ui.horizontal_top(|ui| {
        gradient_tile(ui, icon);
        ui.add_space(theme::sp(3.0));
        ui.vertical(|ui| {
            ui.add_space(theme::px(70.0));
            widgets::caps_label(ui, "collection", 11.0, col::muted());
            ui.label(RichText::new(title).font(theme::regular(30.0)));
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).font(theme::regular(13.0)));
                widgets::dot(ui);
                widgets::subtle(ui, meta);
            });
        });
    });
}

fn liked(app: &mut App, ui: &mut Ui) {
    let uid = app.user_id();
    let items = app.want_all("/me/tracks?limit=50", 10_000);
    let n = items.as_ref().map(|v| model::u(v, "total")).unwrap_or(0);
    collection_header(app, ui, Icon::HeartFilled, "Liked Songs", model::songs(n));
    let ctx = format!("spotify:user:{uid}:collection");
    action_row(app, ui, &ctx, None, |app, ui| {
        if widgets::menu_item(ui, "Add all to queue")
            && let Some(v) = app.want_all("/me/tracks?limit=50", 10_000)
        {
            let uris: Vec<String> = model::items(&v).iter().map(|i| s(&i["track"], "uri").to_string()).collect();
            app.act(Action::Queue(uris));
        }
    });
    tracklist::controls(app, ui, true);
    let Some(v) = items else {
        show_wait(app, ui, "/me/tracks?limit=50");
        return;
    };
    let rows = rows_from_items(model::items(&v));
    // Everything here is liked; recorded (once per answer) so hearts render without extra requests.
    if mark_saved_once(ui, "liked-songs", &v) {
        for r in &rows {
            let uri = s(r.item, "uri");
            if !app.saved.contains_key(uri) {
                app.saved.insert(uri.to_string(), true);
            }
        }
    }
    if rows.is_empty() {
        widgets::unavailable(ui, "songs you like will appear here", "Click the heart on any song to save it here.");
        return;
    }
    tracklist::table(app, ui, &rows, &Opts { album_col: true, added_col: true, cover: true, header: true, context: Some(ctx), ..Default::default() });
}

fn your_episodes(app: &mut App, ui: &mut Ui) {
    let items = app.want_all("/me/episodes?limit=50", 2000);
    let n = items.as_ref().map(|v| model::u(v, "total")).unwrap_or(0);
    collection_header(app, ui, Icon::Bell, "Your Episodes", if n == 1 { "1 episode".to_string() } else { format!("{n} episodes") });
    let Some(v) = items else {
        show_wait(app, ui, "/me/episodes?limit=50");
        return;
    };
    let eps: Vec<&Value> = model::items(&v).iter().map(|i| &i["episode"]).filter(|e| !e.is_null()).collect();
    if eps.is_empty() {
        widgets::unavailable(ui, "add to Your Episodes", "Click + on an episode to save it here.");
        return;
    }
    ui.add_space(theme::sp(1.5));
    ui.horizontal(|ui| {
        if widgets::play_btn(ui, false, 56.0).clicked() {
            let uris: Vec<String> = eps.iter().map(|e| s(e, "uri").to_string()).collect();
            app.act(Action::PlayUris { uris, index: 0 });
        }
    });
    ui.add_space(theme::sp(1.0));
    if mark_saved_once(ui, "your-episodes", &v) {
        for e in &eps {
            app.saved.entry(s(e, "uri").to_string()).or_insert(true);
        }
    }
    for e in &eps {
        episode_row(app, ui, e, true, None);
    }
}

/// True the first time a page sees this answer (`v`), to do something once per answer
/// rather than every frame.
fn mark_saved_once(ui: &Ui, page: &str, v: &Arc<Value>) -> bool {
    let id = egui::Id::new(("seen-answer", page));
    let ptr = Arc::as_ptr(v) as usize;
    if ui.data(|d| d.get_temp::<usize>(id)) == Some(ptr) {
        return false;
    }
    ui.data_mut(|d| d.insert_temp(id, ptr));
    true
}

/// Glass on your other devices on this network (click one to see its songs and get
/// some), and songs going to and from them.
pub(super) fn devices(app: &mut App, ui: &mut Ui) {
    if !app.settings.share_local_files {
        return;
    }
    if app.share_peers.is_empty() {
        widgets::subtle(ui, "Glass on your other devices (a Mac, a Linux PC or an iPhone) shows up here by itself when it's open on this network and signed in to this Spotify account.");
    } else {
        for p in app.share_peers.clone() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(520.0), 44.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &p.name));
            if resp.hovered() {
                ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(20.0, 20.0)), Icon::Devices, col::accent());
            widgets::text_trunc(ui, pos2(rect.left() + 42.0, rect.center().y - 8.0), &p.name, theme::medium(13.5), col::text(), rect.width() - 70.0);
            widgets::text_trunc(ui, pos2(rect.left() + 42.0, rect.center().y + 9.0), "see its songs and get them here", theme::regular(12.0), col::muted(), rect.width() - 70.0);
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(14.0, 14.0)), Icon::Chevron, col::muted());
            widgets::focus_ring(ui, &resp, rect, theme::small_radius());
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                app.act(Action::Navigate(Route::Device { id: p.id.clone(), name: p.name.clone() }));
            }
        }
        widgets::subtle(ui, "Right-click songs to send them; playlists get their missing local songs from these devices.");
    }
    for note in app.share_others.iter().filter_map(|p| p.update_note()) {
        widgets::subtle(ui, note);
    }
    transfers(app, ui);
    ui.add_space(theme::sp(1.0));
}

/// The last few songs going to and from other devices.
fn transfers(app: &App, ui: &mut Ui) {
    use crate::share::State;
    for t in app.transfers.iter().rev().take(3) {
        let what = |n: usize| if n == 1 { "a song".to_string() } else { model::songs(n as u64) };
        let at = t.done.min(t.files.saturating_sub(1)) + 1;
        let text = match (&t.state, t.sending) {
            (State::Running, true) if t.files == 1 => format!("Sending a song to {}", t.peer),
            (State::Running, false) if t.files == 1 => format!("Receiving a song from {}", t.peer),
            (State::Running, true) => format!("Sending {at} of {} to {}", t.files, t.peer),
            (State::Running, false) => format!("Receiving {at} of {} from {}", t.files, t.peer),
            (State::Done, true) => format!("Sent {} to {}", what(t.done), t.peer),
            (State::Done, false) => format!("Got {} from {}", what(t.done), t.peer),
            (State::Failed(why), _) => format!("{} {}: {why}", if t.sending { "Couldn't send to" } else { "Couldn't receive from" }, t.peer),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(text).font(theme::regular(12.5)).color(col::sub()));
            if t.state == State::Running && t.total > 0 {
                ui.add(egui::ProgressBar::new(t.bytes as f32 / t.total as f32).desired_width(160.0).desired_height(6.0));
            }
        });
    }
}

/// Another device's Local Files, listed by it: pick songs and get them here (as
/// on iOS). Songs already here are marked and can't be picked.
fn device_page(app: &mut App, ui: &mut Ui, id: &str, name: &str) {
    // Asked for when the page opens.
    if app.device.as_ref().is_none_or(|d| d.peer != id) {
        app.act(Action::BrowseDevice(id.to_string()));
    }
    ui.label(RichText::new(name).font(theme::regular(30.0)).color(col::text()));
    let Some(d) = app.device.as_ref() else { return };
    let songs = match &d.songs {
        None => {
            widgets::subtle(ui, "Asking it for its songs…");
            widgets::loading(ui);
            return;
        }
        Some(Err(e)) => {
            let e = e.clone();
            widgets::unavailable(ui, "couldn't load the songs", &e);
            ui.add_space(theme::sp(1.0));
            ui.vertical_centered(|ui| {
                if widgets::bracket_button(ui, None, "try again", false).clicked() {
                    app.act(Action::BrowseDevice(id.to_string()));
                }
            });
            return;
        }
        Some(Ok(songs)) => songs,
    };
    if songs.is_empty() {
        widgets::unavailable(ui, "no songs", &format!("{name} has nothing in Local Files."));
        return;
    }
    let q = app.page_filter.trim().to_lowercase();
    // Artist, album, title: how Local Files lists songs.
    let key = |f: &crate::share::FileMeta| (f.artist.to_lowercase(), f.album.to_lowercase(), f.title.to_lowercase());
    let mut shown: Vec<(usize, &crate::share::FileMeta, bool)> =
        d.list().enumerate().map(|(i, (f, here))| (i, f, here)).filter(|(_, f, _)| q.is_empty() || format!("{} {} {}", f.title, f.artist, f.album).to_lowercase().contains(&q)).collect();
    shown.sort_by_cached_key(|(_, f, _)| key(f));
    let missing = d.list().filter(|(_, here)| !here).count();
    let picked: Vec<String> = d.list().filter(|(f, here)| !here && app.device_picks.contains(&f.id)).map(|(f, _)| f.id.clone()).collect();
    let bytes: u64 = d.list().filter(|(f, here)| !here && app.device_picks.contains(&f.id)).map(|(f, _)| f.size).sum();
    let shown_missing: Vec<String> = shown.iter().filter(|(_, _, here)| !here).map(|(_, f, _)| f.id.clone()).collect();
    let rows: Vec<(String, String, String, u64, bool)> = shown
        .iter()
        .map(|(_, f, here)| (f.id.clone(), f.title.clone(), [f.artist.as_str(), f.album.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "), f.duration_ms, *here))
        .collect();
    let count = |n: usize| if n == 1 { "1 song".to_string() } else { format!("{} songs", model::fmt_count(n as u64)) };
    widgets::subtle(ui, format!("{} · {}", count(songs.len()), if missing == 0 { "all on this computer".to_string() } else { format!("{} not on this computer", count(missing)) }));
    transfers(app, ui);
    ui.add_space(theme::sp(1.0));

    // What's picked, and getting it.
    ui.horizontal(|ui| {
        let get = widgets::bracket_button(ui, Some(Icon::Download), &if picked.is_empty() { "get".to_string() } else { format!("get {}", count(picked.len())) }, true);
        if get.clicked() && !picked.is_empty() {
            app.act(Action::GetFromDevice { peer: id.to_string(), ids: picked.clone() });
        }
        let all_picked = !shown_missing.is_empty() && shown_missing.iter().all(|id| app.device_picks.contains(id));
        if !shown_missing.is_empty() && !all_picked && widgets::bracket_button(ui, None, "pick all", false).clicked() {
            app.device_picks.extend(shown_missing.iter().cloned());
        } else if !app.device_picks.is_empty() && all_picked && widgets::bracket_button(ui, None, "pick none", false).clicked() {
            app.device_picks.clear();
        }
        ui.add_space(theme::sp(1.0));
        let note = if picked.is_empty() { "Click songs to pick them".to_string() } else { format!("{} picked · {}", count(picked.len()), fmt_bytes(bytes)) };
        ui.label(RichText::new(note).font(theme::regular(12.5)).color(col::sub()));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add(egui::TextEdit::singleline(&mut app.page_filter).hint_text(RichText::new("filter").color(col::muted())).font(theme::regular(12.5)).desired_width(theme::px(180.0)));
        });
    });
    ui.add_space(theme::sp(1.0));
    if rows.is_empty() {
        widgets::subtle(ui, "Nothing matches the filter.");
        return;
    }
    let w = ui.available_width();
    for (pos, (song, title, sub, ms, here)) in rows.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 40.0), if *here { Sense::hover() } else { Sense::click() });
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let on = app.device_picks.contains(song);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, !*here, on, title));
        if resp.hovered() && !*here {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
        } else if pos % 2 == 1 {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover().gamma_multiply(0.35));
        }
        let mark = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0));
        if *here {
            icons::paint(ui.painter(), mark, Icon::Check, col::muted());
        } else {
            ui.painter().rect(
                mark,
                CornerRadius::same(4),
                if on { col::accent() } else { Color32::TRANSPARENT },
                egui::Stroke::new(1.2, if on { col::accent() } else { col::muted() }),
                egui::StrokeKind::Inside,
            );
            if on {
                icons::paint(ui.painter(), mark.shrink(2.0), Icon::Check, col::on_accent());
            }
        }
        let tw = rect.width() - 140.0;
        let color = if *here { col::muted() } else { col::text() };
        widgets::text_trunc(ui, pos2(rect.left() + 42.0, rect.center().y - 8.0), if title.is_empty() { "Untitled" } else { title }, theme::regular(13.5), color, tw);
        widgets::text_trunc(ui, pos2(rect.left() + 42.0, rect.center().y + 9.0), if *here { "on this computer" } else { sub }, theme::regular(12.0), col::muted(), tw);
        ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, model::fmt_ms(*ms), theme::regular(12.5), col::sub());
        widgets::focus_ring(ui, &resp, rect, theme::small_radius());
        if resp.clicked() && !app.device_picks.remove(song) {
            app.device_picks.insert(song.clone());
        }
    }
}

/// Glass's local playlists, at the top of Local Files.
pub(super) fn local_playlists_list(app: &mut App, ui: &mut Ui) {
    if app.playlists.list.is_empty() {
        return;
    }
    ui.label(RichText::new("playlists").font(theme::regular(15.0)).color(col::sub()));
    ui.add_space(theme::sp(0.5));
    // Just what each row shows (not every playlist's list of files, every frame).
    let lists: Vec<(String, String, usize)> = app.playlists.list.iter().map(|p| (p.id.clone(), p.name.clone(), p.tracks.len())).collect();
    for (id, name, songs) in lists {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let hovered = ui.rect_contains_pointer(rect);
        if hovered {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
        }
        if resp.dnd_hover_payload::<super::SongDrag>().is_some() {
            ui.painter().rect_stroke(rect, theme::small_radius(), egui::Stroke::new(1.5, col::accent()), egui::StrokeKind::Inside);
        }
        if let Some(d) = resp.dnd_release_payload::<super::SongDrag>() {
            app.act(Action::AddToLocalPlaylist { id: id.clone(), paths: d.files.clone() });
        }
        let pr = egui::Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(16.0, 16.0));
        let play = ui.interact(pr.expand(6.0), resp.id.with("play"), Sense::click());
        icons::paint(
            ui.painter(),
            pr,
            Icon::Play,
            if play.hovered() {
                col::accent()
            } else if hovered {
                col::text()
            } else {
                col::muted()
            },
        );
        widgets::text_trunc(ui, pos2(rect.left() + 44.0, rect.center().y), &name, theme::regular(14.0), col::text(), rect.width() * 0.6);
        ui.painter().text(pos2(rect.right() - 12.0, rect.center().y), Align2::RIGHT_CENTER, model::songs(songs as u64), theme::regular(12.5), col::sub());
        if play.on_hover_text("Play").clicked() {
            app.act(Action::PlayLocalPlaylist { id: id.clone(), index: 0 });
        } else if resp.clicked() {
            app.act(Action::Navigate(Route::LocalPlaylist(id)));
        }
    }
    ui.add_space(theme::sp(1.5));
}

/// Where each file is in Local Files, by path: built once per scan of the library.
fn local_index(ui: &Ui, all: &Arc<Vec<crate::local::LocalTrack>>) -> Arc<std::collections::HashMap<std::path::PathBuf, usize>> {
    type Index = Arc<std::collections::HashMap<std::path::PathBuf, usize>>;
    let id = egui::Id::new("local-index");
    let key = Arc::as_ptr(all) as usize;
    if let Some((k, index)) = ui.data(|d| d.get_temp::<(usize, Index)>(id))
        && k == key
    {
        return index;
    }
    let index: Index = Arc::new(all.iter().enumerate().map(|(i, t)| (t.path.clone(), i)).collect());
    ui.data_mut(|d| d.insert_temp(id, (key, index.clone())));
    index
}

/// A Glass local playlist: its files, in order.
fn local_playlist(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(p) = app.playlists.get(id).cloned() else {
        if app.playlists.just_deleted(id) {
            widgets::unavailable(ui, "playlist deleted", "Undo brings it back, songs and order included.");
        } else {
            widgets::unavailable(ui, "playlist not found", "It may have been deleted.");
        }
        if app.playlists.can_undo() {
            ui.vertical_centered(|ui| {
                if widgets::bracket_button(ui, None, "undo last playlist change", false).clicked() {
                    app.act(Action::UndoPlaylistChange);
                }
            });
        }
        return;
    };
    let all = app.local_tracks.clone().unwrap_or_default();
    let by_path = local_index(ui, &all);
    let rows: Vec<Option<&crate::local::LocalTrack>> = p.tracks.iter().map(|path| by_path.get(path).and_then(|&i| all.get(i))).collect();
    let total: u64 = rows.iter().flatten().map(|t| t.duration_ms).sum();
    widgets::caps_label(ui, "local playlist", 11.0, col::muted());
    ui.label(RichText::new(&p.name).font(theme::regular(28.0)).color(col::text()));
    if !p.description.is_empty() {
        ui.label(RichText::new(&p.description).font(theme::regular(13.0)).color(col::sub()));
    }
    let missing = rows.iter().filter(|r| r.is_none()).count();
    let mut meta = format!("{} · {} · files on this computer", crate::model::songs((rows.len() - missing) as u64), crate::model::fmt_ms(total));
    if missing > 0 {
        meta.push_str(&format!(" · {missing} missing"));
    }
    ui.label(RichText::new(meta).font(theme::regular(12.5)).color(col::muted()));
    ui.add_space(theme::sp(1.25));
    ui.horizontal(|ui| {
        if widgets::bracket_button(ui, Some(Icon::Play), "play", true).clicked() {
            app.act(Action::PlayLocalPlaylist { id: p.id.clone(), index: 0 });
        }
        if widgets::bracket_button(ui, None, "add files", false).on_hover_text("Add songs from this computer (or drop them here)").clicked() {
            menus::pick_local_files(app, crate::app::FileTarget::Local { id: p.id.clone(), name: p.name.clone() });
        }
        if widgets::bracket_button(ui, None, "delete", false).clicked() {
            app.modal = Some(crate::app::Modal::Confirm {
                title: format!("Delete “{}”?", p.name),
                body: "The playlist is removed from Glass (you can undo). Your music files aren't touched.".into(),
                confirm: "Delete".into(),
                action: Box::new(Action::DeleteLocalPlaylist(p.id.clone())),
            });
        }
        if app.playlists.can_undo() && widgets::bracket_button(ui, None, "undo last change", false).clicked() {
            app.act(Action::UndoPlaylistChange);
        }
    });
    ui.add_space(theme::sp(1.25));
    let mut playable = 0usize;
    for (n, row) in rows.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
        resp.context_menu(|ui| {
            if let Some(t) = row {
                if widgets::menu_item(ui, "Play") {
                    app.act(Action::PlayLocalPlaylist { id: p.id.clone(), index: rows[..n].iter().flatten().count() });
                }
                menus::add_local_to_playlist(app, ui, &[(*t).clone()]);
                menus::send_to_device(app, ui, &[(*t).clone()]);
            }
            if widgets::menu_item(ui, "Remove from this playlist") {
                app.act(Action::RemoveFromLocalPlaylist { id: p.id.clone(), index: n });
            }
            if let Some(t) = row
                && widgets::menu_item(ui, crate::local::REVEAL)
            {
                crate::local::reveal(&t.path);
            }
        });
        let Some(t) = row else {
            if ui.is_rect_visible(rect) {
                ui.painter().text(
                    pos2(rect.left() + 16.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    format!("{:02}  file missing: {}", n + 1, p.tracks[n].file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()),
                    theme::regular(12.5),
                    col::muted(),
                );
            }
            continue;
        };
        let index = playable;
        playable += 1;
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &t.title));
        if !ui.is_rect_visible(rect) {
            continue;
        }
        if ui.rect_contains_pointer(rect) {
            ui.painter().rect_filled(rect, theme::small_radius(), col::hover());
        }
        let w = rect.width();
        ui.painter().text(pos2(rect.left() + 16.0, rect.center().y), Align2::LEFT_CENTER, format!("{:02}", n + 1), theme::regular(13.0), col::muted());
        let c = egui::Rect::from_min_size(pos2(rect.left() + 48.0, rect.center().y - 20.0), vec2(40.0, 40.0));
        paint_cover(ui, c, t.cover_uri().as_deref(), false);
        widgets::text_trunc(ui, pos2(c.right() + 12.0, rect.center().y - 8.0), &t.title, theme::regular(14.5), col::text(), w * 0.5);
        widgets::text_trunc(ui, pos2(c.right() + 12.0, rect.center().y + 10.0), &t.artist, theme::regular(12.5), col::sub(), w * 0.5);
        ui.painter().text(pos2(rect.right() - 16.0, rect.center().y), Align2::RIGHT_CENTER, model::fmt_ms(t.duration_ms), theme::regular(13.0), col::sub());
        if resp.clicked() && !resp.double_clicked() {
            app.act(Action::PlayLocalPlaylist { id: p.id.clone(), index });
        }
    }
}

fn recents(app: &mut App, ui: &mut Ui) {
    ui.label(RichText::new("Recents").font(theme::regular(32.0)));
    widgets::subtle(ui, "The last 50 songs you played, from Spotify's listening history.");
    ui.add_space(theme::sp(1.25));
    let Some(v) = need(app, ui, "/me/player/recently-played?limit=50") else { return };
    let mut groups: Vec<(String, Vec<tracklist::Row>)> = Vec::new();
    for it in model::items(&v) {
        let day = model::fmt_date(s(it, "played_at"));
        let row = tracklist::Row { item: &it["track"], added_at: "" };
        match groups.last_mut() {
            Some((d, rows)) if *d == day => rows.push(row),
            _ => groups.push((day, vec![row])),
        }
    }
    for (day, rows) in groups {
        ui.add_space(theme::sp(1.25));
        ui.label(RichText::new(&day).font(theme::regular(18.0)));
        // Each day's table apart: the same song on two days would share its row's id.
        ui.push_id(&day, |ui| tracklist::table(app, ui, &rows, &Opts { album_col: true, cover: true, ..Default::default() }));
    }
}

fn folder(app: &mut App, ui: &mut Ui, id: &str) {
    let Some(f) = app.settings.folders.iter().find(|f| f.id == id).cloned() else {
        widgets::unavailable(ui, "folder not found", "It may have been deleted.");
        return;
    };
    ui.horizontal(|ui| {
        gradient_tile(ui, Icon::Folder);
        ui.add_space(theme::sp(1.5));
        ui.vertical(|ui| {
            ui.add_space(theme::sp(11.25));
            ui.label(RichText::new("Folder").font(theme::medium(13.0)));
            ui.label(RichText::new(&f.name).font(theme::regular(56.0)));
            let n = f.playlists.len();
            widgets::subtle(ui, if n == 1 { "1 playlist".to_string() } else { format!("{n} playlists") });
        });
    });
    ui.add_space(theme::sp(2.0));
    ui.horizontal(|ui| {
        if widgets::outline_button(ui, "Rename").clicked() {
            app.modal = Some(Modal::RenameFolder { id: f.id.clone(), name: f.name.clone() });
        }
        if widgets::outline_button(ui, "Delete folder").clicked() {
            app.modal = Some(Modal::Confirm {
                title: "Delete folder?".into(),
                body: format!("This will delete “{}”. Its playlists stay in Your Library.", f.name),
                confirm: "Delete".into(),
                action: Box::new(Action::DeleteFolder(f.id.clone())),
            });
        }
    });
    ui.add_space(theme::sp(1.5));
    let mine = app.want_all("/me/playlists?limit=50", 2000);
    let items: Vec<Value> = f.playlists.iter().filter_map(|u| mine.as_ref().and_then(|m| model::items(m).iter().find(|p| s(p, "uri") == u).cloned())).collect();
    if items.is_empty() {
        widgets::unavailable(ui, "this folder looks empty", "Start adding playlists by dragging and dropping them onto the folder in Your Library.");
    }
    cards(app, ui, &items, false);
}

// ------------------------------------------------------------------- Activity (transparency)

fn dir_size(p: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten().map(|e| if e.file_type().is_ok_and(|t| t.is_dir()) { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()
}

/// What the Activity page shows of Glass's files: each file's size (`None` when it
/// isn't there), then the audio and lyrics caches.
type Sizes = (Vec<Option<u64>>, u64, u64);

/// `Sizes`, measured on another thread at most every ten seconds (the audio cache
/// is thousands of files) and kept in between.
fn sizes(ctx: &egui::Context, files: Vec<std::path::PathBuf>, audio: std::path::PathBuf, lyrics: std::path::PathBuf) -> Option<Arc<Sizes>> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<(Option<Instant>, Option<Arc<Sizes>>)> = Mutex::new((None, None));
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.0.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
        last.0 = Some(Instant::now());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = (files.iter().map(|p| std::fs::metadata(p).ok().map(|m| m.len())).collect(), dir_size(&audio), dir_size(&lyrics));
            LAST.lock().unwrap_or_else(|e| e.into_inner()).1 = Some(Arc::new(found));
            ctx.request_repaint();
        });
    }
    last.1.clone()
}

fn fmt_bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

fn activity(app: &mut App, ui: &mut Ui) {
    ui.label(RichText::new("Activity").font(theme::regular(32.0)));
    widgets::subtle(ui, "Everything Glass sends, receives and stores.");
    ui.add_space(theme::sp(1.75));

    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Who Glass talks to").font(theme::regular(16.0)));
        ui.add_space(theme::sp(0.5));
        for (host, why) in [
            ("accounts.spotify.com", "Sign-in (OAuth with PKCE) and token refresh"),
            ("Spotify spclient + api-partner.spotify.com", "Library, search, pages and playback commands (logged below)"),
            ("lrclib.net", "Lyrics: the title, artist, album and length of the song whose lyrics are on screen (logged below)"),
            ("Spotify access points + audio CDN", "Encrypted audio streams and Spotify Connect, via librespot"),
            ("i.scdn.co / mosaic.scdn.co / lexicon-assets.spotifycdn.com", "Cover art, artist images and the DJ's cover"),
            ("github.com", "Checking for and downloading updates (release builds)"),
            ("jiosaavn.com, youtube.com and their CDNs", "Only when you save songs as files: finding and downloading them"),
            ("api.openai.com, auth.openai.com", "Only with the assistant on: sign-in, your requests, and lyrics it translates"),
            ("your other Glass computers", "Only with sharing on: songs you send, over an encrypted connection on this network"),
        ] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(host).font(theme::medium(13.5)));
                widgets::subtle(ui, format!("— {why}"));
            });
        }
        widgets::subtle(ui, "No analytics, telemetry or crash reporting.");
    });
    ui.add_space(theme::sp(1.25));
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("What Glass stores").font(theme::regular(16.0)));
        ui.add_space(theme::sp(0.5));
        let cfg = crate::config::config_dir();
        let cache = crate::config::cache_dir();
        let edited = crate::theme_config::config_dir();
        let files = [
            ("settings.json", cfg.join("settings.json"), "Your preferences, folders and pins"),
            ("session.json", cfg.join("session.json"), "Spotify refresh token (readable only by you)"),
            ("credentials.json", cfg.join("credentials.json"), "librespot's reusable login (readable only by you)"),
            ("device_id", cfg.join("device_id"), "This computer's Spotify Connect ID"),
            ("data_device_id", cfg.join("data_device_id"), "The ID of Glass's connection for your library and pages"),
            ("queries.json", cfg.join("queries.json"), "Spotify's current page queries, read from Spotify.app"),
            ("downloads.json", cfg.join("downloads.json"), "What you downloaded"),
            ("local_playlists.json", cfg.join("local_playlists.json"), "Your local playlists"),
            ("lyrics_choices.json", cfg.join("lyrics_choices.json"), "Lyrics you picked, and timing you nudged"),
            ("theme.toml", edited.join("theme.toml"), "Your theme"),
            ("config.toml", edited.join("config.toml"), "Lyrics and assistant settings"),
        ];
        let measured = sizes(ui.ctx(), files.iter().map(|f| f.1.clone()).collect(), cache.join("audio"), cache.join("lyrics"));
        let shown = |n: Option<u64>| n.map(fmt_bytes).unwrap_or_else(|| "not present".into());
        for (i, (name, _, what)) in files.iter().enumerate() {
            // Only macOS has a Spotify.app to read them from.
            if *name == "queries.json" && !cfg!(target_os = "macos") {
                continue;
            }
            let size = match &measured {
                Some(m) => shown(m.0.get(i).copied().flatten()),
                None => "…".into(),
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(*name).font(theme::medium(13.5)));
                widgets::subtle(ui, format!("{what} · {size}"));
            });
        }
        let (audio, lyrics) = measured.as_ref().map_or(("…".into(), "…".into()), |m| (fmt_bytes(m.1), fmt_bytes(m.2)));
        ui.horizontal(|ui| {
            ui.label(RichText::new("audio cache").font(theme::medium(13.5)));
            widgets::subtle(ui, format!("Encrypted audio files · {audio} of {} MB", app.settings.cache_limit_mb));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("lyrics cache").font(theme::medium(13.5)));
            widgets::subtle(ui, format!("LRCLIB answers and saved translations · {lyrics}"));
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("assistant").font(theme::medium(13.5)));
            let keychain = if cfg!(target_os = "macos") { "Keychain" } else { "system keyring" };
            widgets::subtle(ui, format!("The ChatGPT app registration; its sign-in and any API key are in the {keychain}"));
        });
        // The data folder, the hand-edited files' folder (on macOS another one), and the caches.
        let mut folders = vec![cfg.clone()];
        if edited != cfg {
            folders.push(edited);
        }
        folders.push(cache);
        for dir in folders {
            ui.horizontal(|ui| {
                if ui.button(crate::local::REVEAL).clicked() {
                    let _ = open::that(&dir);
                }
                widgets::subtle(ui, dir.display().to_string());
            });
        }
        ui.add_space(theme::sp(0.5));
        widgets::subtle(ui, format!("Playback engine: {}", app.engine_status));
    });

    ui.add_space(theme::sp(1.25));
    let entries = app.log.snapshot();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("requests ({})", entries.len())).font(theme::regular(18.0)));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Clear").clicked() {
                app.log.clear();
            }
        });
    });
    ui.add_space(theme::sp(0.75));
    for e in entries.iter().rev() {
        // Only the rows on screen are laid out (there can be 500).
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        if !ui.is_rect_visible(row) {
            continue;
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui.label(RichText::new(model::clock(e.at)).monospace().size(12.0).color(col::sub()));
            let mc = match e.method.as_str() {
                "GET" => col::sub(),
                "DELETE" => col::error(),
                _ => col::accent(),
            };
            ui.add_sized([56.0, 16.0], egui::Label::new(RichText::new(&e.method).monospace().size(12.0).color(mc)));
            let ok = e.status.starts_with('2');
            ui.add_sized([40.0, 16.0], egui::Label::new(RichText::new(&e.status).monospace().size(12.0).color(if ok { col::text() } else { col::error() })));
            ui.add_sized([60.0, 16.0], egui::Label::new(RichText::new(format!("{} ms", e.millis)).monospace().size(12.0).color(col::sub())));
            ui.add(egui::Label::new(RichText::new(&e.path).monospace().size(12.0)).truncate());
        });
    }
}
