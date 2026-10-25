//! The command bar (⌘K): `> search your library`. Typing lists matching
//! commands and library items; Enter runs the highlighted one, or searches
//! Spotify for the text when nothing matches. ↑/↓ move, Esc closes.

use super::icons::{self, Icon};
use super::library;
use super::theme::{self, col, px, sp};
use super::widgets;
use crate::app::{Action, App, LibFilter, Modal, NpView, RightPanel, Route};
use eframe::egui::{self, Align2, Id, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

pub fn field_id() -> Id {
    Id::new("command-bar")
}

fn rect_id() -> Id {
    Id::new("command-bar-rect")
}

fn results_id() -> Id {
    Id::new("command-results-rect")
}

/// Set when Enter was pressed in the field. The field drops focus on Enter
/// before the result list sees the key, so it leaves this note.
fn enter_id() -> Id {
    Id::new("command-bar-enter")
}

type Run = fn(&mut App);

/// Every command: (name, extra search words, key hint, what it does).
const COMMANDS: &[(&str, &str, &str, Run)] = &[
    ("play / pause", "resume stop", "space", |a| a.act(Action::TogglePlay)),
    ("next song", "skip forward track", "⌘ →", |a| a.act(Action::Next)),
    ("previous song", "back track", "⌘ ←", |a| a.act(Action::Prev)),
    ("shuffle on/off", "random", "⌘ S", |a| {
        let s = !a.pb.shuffle;
        a.act(Action::SetShuffle(s))
    }),
    ("repeat: off / all / one", "loop", "⌘ R", |a| a.act(Action::CycleRepeat)),
    ("like current song", "save heart", "⌥ ⇧ B", |a| {
        let uri = a.pb.uri().to_string();
        if !uri.is_empty() {
            let saved = a.is_saved(&uri).unwrap_or(false);
            a.act(Action::Save { uri, save: !saved });
        }
    }),
    // As the shortcuts: on Now Playing these are views of their own (the side panel isn't shown).
    ("queue", "up next", "⌥ ⇧ Q", |a| if a.route == Route::NowPlaying { a.np_view = NpView::Queue } else { a.act(Action::SetRight(RightPanel::Queue)) }),
    ("devices", "connect speaker phone", "⌥ ⇧ D", |a| if a.route == Route::NowPlaying { a.np_view = NpView::Devices } else { a.act(Action::SetRight(RightPanel::Devices)) }),
    ("lyrics", "words karaoke", "L", |a| {
        a.np_view = NpView::Lyrics;
        a.act(Action::Navigate(Route::NowPlaying))
    }),
    ("now playing", "current track screen", "⌥ ⇧ J", |a| a.act(Action::Navigate(Route::NowPlaying))),
    ("theme panel", "colors appearance opacity blur", "⌘ T", |a| a.theme_panel = true),
    ("reload theme.toml", "theme refresh", "", |a| {
        a.theme.reload();
        match a.theme.error() {
            Some(e) => a.toast(format!("theme.toml wasn't applied: {e}")),
            None => a.toast("Reloaded theme.toml"),
        }
    }),
    ("open theme.toml in your editor", "theme edit config", "", |a| {
        let _ = a.theme.ensure_file();
        let _ = open::that(a.theme.path());
    }),
    ("library", "music playlists", "⌥ ⇧ 0", |a| a.act(Action::Navigate(Route::Library))),
    ("home", "start", "⌥ ⇧ H", |a| a.act(Action::Navigate(Route::Home))),
    ("search spotify", "find", "⌘ L", |a| {
        a.focus_search = true;
        a.act(Action::Navigate(Route::Search))
    }),
    ("liked songs", "favorites favourites hearts", "⌥ ⇧ S", |a| a.act(Action::Navigate(Route::LikedSongs))),
    ("your episodes", "podcasts saved", "", |a| a.act(Action::Navigate(Route::YourEpisodes))),
    ("albums", "library", "⌥ ⇧ 4", |a| {
        a.lib_tab = 0;
        a.lib_filter = LibFilter::Albums;
        a.act(Action::Navigate(Route::Library))
    }),
    ("artists", "library following", "⌥ ⇧ 3", |a| {
        a.lib_tab = 0;
        a.lib_filter = LibFilter::Artists;
        a.act(Action::Navigate(Route::Library))
    }),
    ("podcasts", "shows library", "⌥ ⇧ 2", |a| {
        a.lib_tab = 1;
        a.lib_filter = LibFilter::Podcasts;
        a.act(Action::Navigate(Route::Library))
    }),
    ("downloads", "offline downloaded", "", |a| {
        a.lib_tab = 0;
        a.lib_filter = LibFilter::Downloaded;
        a.act(Action::Navigate(Route::Library))
    }),
    ("recents", "history recently played", "⌥ ⇧ R", |a| a.act(Action::Navigate(Route::Recents))),
    ("local files", "mp3 computer", "", |a| a.act(Action::Navigate(Route::LocalFiles))),
    ("dj", "ai dj x mix spotify play", "", |a| a.act(Action::PlayContext { context: crate::dj::URI.into(), track: None })),
    ("profile", "account me", "", |a| {
        let id = a.user_id();
        a.act(Action::Navigate(Route::Profile(id)))
    }),
    ("settings", "preferences", "⌘ ,", |a| a.act(Action::Navigate(Route::Settings))),
    ("activity log", "network requests", "", |a| a.act(Action::Navigate(Route::Activity))),
    ("new playlist", "create", "⌘ N", |a| a.act(Action::CreatePlaylist { name: "New Playlist".into(), uris: vec![] })),
    ("download what's playing", "offline save", "", |a| {
        let ctx = a.pb.context_uri.clone();
        let uri = if ctx.is_empty() { a.pb.uri().to_string() } else { ctx };
        if !uri.is_empty() {
            let song = a.pb.item.as_ref().map(|i| crate::model::s(i, "name").to_string()).unwrap_or_default();
            let name = super::right::context_name(a).unwrap_or(song);
            a.act(Action::Download { uri, name });
        }
    }),
    ("miniplayer", "small", "⌥ ⇧ M", |a| a.act(Action::ToggleMini)),
    ("full screen", "fullscreen", super::FULL_SCREEN_KEY, |a| a.act(Action::ToggleFullscreen)),
    ("keyboard shortcuts", "keys help", "⌘ /", |a| a.modal = Some(Modal::Shortcuts)),
    ("about glass", "version", "", |a| a.modal = Some(Modal::About)),
    ("log out", "sign out", "⇧ ⌘ W", |a| {
        a.modal =
            Some(Modal::Confirm { title: "Log out?".into(), body: "Glass will forget your Spotify session on this computer.".into(), confirm: "Log out".into(), action: Box::new(Action::LogOut) })
    }),
];

enum Hit {
    Command(usize),
    Item {
        name: String,
        sub: String,
        uri: String,
        route: Option<Route>,
    },
    Search(String),
    /// "? …": ask the assistant.
    Ask(String),
}

/// Every word of `q` (lowercase) appears in `hay` (lowercase).
fn matches(hay: &str, q: &str) -> bool {
    q.split_whitespace().all(|w| hay.contains(w))
}

fn hits(app: &mut App) -> Vec<Hit> {
    // "? make an hour of city pop" goes to the assistant.
    if let Some(ask) = app.cmd_text.trim().strip_prefix('?') {
        return vec![Hit::Ask(ask.trim().to_string())];
    }
    let q = app.cmd_text.trim().trim_start_matches('>').trim().to_lowercase();
    let mut out = Vec::new();
    if q.is_empty() {
        // A starting point: the most useful commands.
        const START: [&str; 6] = ["play / pause", "queue", "devices", "lyrics", "search spotify", "liked songs"];
        out.extend(START.iter().filter_map(|n| COMMANDS.iter().position(|c| c.0 == *n)).map(Hit::Command));
        return out;
    }
    for (i, (name, words, _, _)) in COMMANDS.iter().enumerate() {
        if matches(&format!("{name} {words}"), &q) {
            out.push(Hit::Command(i));
        }
    }
    out.truncate(5);
    // Liked Songs and the like first, then the library; only matches are copied.
    let specials = library::specials(app);
    let (all, _) = library::entries(app);
    for e in specials.iter().chain(all.iter()).filter(|e| matches(&e.find, &q)).take(6) {
        out.push(Hit::Item { name: e.name.clone(), sub: e.sub.clone(), uri: e.uri.clone(), route: e.special.clone().or_else(|| super::menus::route_for(&e.value)) });
    }
    out.push(Hit::Search(app.cmd_text.trim().trim_start_matches('>').trim().to_string()));
    out
}

fn run(app: &mut App, hit: &Hit) {
    match hit {
        Hit::Command(i) => (COMMANDS[*i].3)(app),
        Hit::Item { uri, route, .. } => {
            if app.route == Route::Library {
                app.lib_selected = Some(uri.clone());
            } else if let Some(r) = route {
                app.act(Action::Navigate(r.clone()));
            }
        }
        Hit::Search(q) => {
            app.search_text = q.clone();
            app.search_committed = q.clone();
            app.act(Action::Navigate(Route::Search));
        }
        // A request runs in the background (the answer arrives as a toast);
        // the panel opens only to type, or to show why it can't answer.
        Hit::Ask(q) if !q.is_empty() && app.ai_ready().is_ok() => {
            app.act(Action::AiSend(q.clone()));
            if !app.ai_panel_visible() {
                app.toast("Asking the assistant…");
            }
        }
        Hit::Ask(q) => {
            if app.route == Route::NowPlaying {
                app.act(Action::Back);
            }
            app.act(Action::SetRight(crate::app::RightPanel::Assistant));
            if q.is_empty() {
                app.ai.focus_input = true;
            } else {
                app.act(Action::AiSend(q.clone()));
            }
        }
    }
    app.cmd_text.clear();
    app.cmd_open = false;
    app.cmd_index = 0;
}

/// The bar itself. On the library page its text also filters the list.
pub fn bar(app: &mut App, ui: &mut Ui, rect: Rect) {
    ui.ctx().data_mut(|d| d.insert_temp(rect_id(), rect));
    let focused = ui.ctx().memory(|m| m.has_focus(field_id()));
    let stroke = if focused { Stroke::new(1.2, col::accent()) } else { Stroke::new(1.0, col::border()) };
    ui.painter().rect(rect, theme::small_radius(), if focused { col::raised() } else { col::surface() }, stroke, StrokeKind::Inside);
    let icon = Rect::from_center_size(pos2(rect.left() + px(24.0), rect.center().y), vec2(px(17.0), px(17.0)));
    icons::paint(ui.painter(), icon, Icon::Search, col::sub());
    let prompt_x = rect.left() + px(46.0);
    ui.painter().text(pos2(prompt_x, rect.center().y), Align2::LEFT_CENTER, ">", theme::regular(13.5), if focused { col::accent() } else { col::sub() });

    let key_w = widgets::keycap(ui, pos2(rect.right() - px(60.0), rect.center().y), "⌘ K");
    let field = Rect::from_min_max(pos2(prompt_x + px(18.0), rect.top() + px(10.0)), pos2(rect.right() - px(70.0).max(key_w + sp(2.0)), rect.bottom() - px(10.0)));
    ui.scope_builder(UiBuilder::new().max_rect(field), |ui| {
        let hint = if app.route == Route::Library { "search your library" } else { "search, go to, or run a command" };
        let te = egui::TextEdit::singleline(&mut app.cmd_text)
            .id(field_id())
            .frame(egui::Frame::NONE)
            .hint_text(RichText::new(hint).color(col::muted()))
            .font(theme::regular(13.5))
            .text_color(col::text())
            .desired_width(field.width());
        let r = ui.add(te);
        if app.focus_cmd {
            r.request_focus();
            app.focus_cmd = false;
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            ui.ctx().data_mut(|d| d.insert_temp(enter_id(), true));
        }
        if r.gained_focus() || r.changed() {
            app.cmd_open = true;
            if r.changed() {
                app.cmd_index = 0;
            }
        }
    });
    let bg = ui.interact(Rect::from_min_max(rect.min, pos2(prompt_x + px(14.0), rect.max.y)), ui.id().with("cmd-click"), Sense::click());
    if bg.clicked() {
        ui.ctx().memory_mut(|m| m.request_focus(field_id()));
    }
}

/// The result list under the bar, while it's open.
pub fn palette(app: &mut App, ctx: &egui::Context) {
    let focused = ctx.memory(|m| m.has_focus(field_id()));
    if !app.cmd_open {
        return;
    }
    let enter = ctx.data_mut(|d| d.remove_temp::<bool>(enter_id())).unwrap_or(false);
    if enter {
        let list = hits(app);
        if let Some(hit) = list.get(app.cmd_index.min(list.len().saturating_sub(1))) {
            run(app, hit);
        }
        return;
    }
    // Clicking elsewhere closes it; a click on a result still lands.
    let over = ctx.data(|d| d.get_temp::<Rect>(results_id())).zip(ctx.pointer_hover_pos()).is_some_and(|(r, p)| r.contains(p));
    if !focused && !over {
        app.cmd_open = false;
        return;
    }
    let Some(rect) = ctx.data(|d| d.get_temp::<Rect>(rect_id())) else { return };
    let list = hits(app);
    if list.is_empty() {
        return;
    }
    if ctx.input(|i| i.key_pressed(Key::ArrowDown)) {
        app.cmd_index = (app.cmd_index + 1).min(list.len() - 1);
    }
    if ctx.input(|i| i.key_pressed(Key::ArrowUp)) {
        app.cmd_index = app.cmd_index.saturating_sub(1);
    }
    app.cmd_index = app.cmd_index.min(list.len() - 1);

    let mut picked = None;
    let shown = egui::Area::new(Id::new("command-results")).fixed_pos(pos2(rect.left(), rect.bottom() + sp(0.5))).order(egui::Order::Foreground).show(ctx, |ui| {
        theme::popup().show(ui, |ui| {
            ui.set_width(rect.width() - sp(1.5));
            for (i, hit) in list.iter().enumerate() {
                let h = px(34.0);
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
                let on = i == app.cmd_index;
                let (glyph, name, sub, hint) = match hit {
                    Hit::Command(c) => (">", COMMANDS[*c].0.to_string(), String::new(), COMMANDS[*c].2.to_string()),
                    Hit::Item { name, sub, .. } => ("♪", name.clone(), sub.clone(), "enter".to_string()),
                    Hit::Search(q) => ("⌕", format!("search spotify for “{q}”"), String::new(), "enter".to_string()),
                    Hit::Ask(q) if q.is_empty() => ("?", "ask the assistant, or tell it what to play".to_string(), String::new(), "⌘ J".to_string()),
                    Hit::Ask(q) => ("?", format!("ask the assistant: “{q}”"), String::new(), "enter".to_string()),
                };
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, &name));
                if on || resp.hovered() {
                    ui.painter().rect(
                        r,
                        theme::small_radius(),
                        if on { col::selection() } else { col::hover() },
                        if on { Stroke::new(1.0, col::accent_a(0.8)) } else { Stroke::NONE },
                        StrokeKind::Inside,
                    );
                }
                let x = r.left() + sp(1.25);
                ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, glyph, theme::regular(13.0), if on { col::accent() } else { col::muted() });
                let w = widgets::text_trunc(ui, pos2(x + px(22.0), r.center().y), &name, theme::regular(13.0), col::text(), r.width() * 0.6);
                if !sub.is_empty() {
                    widgets::text_trunc(ui, pos2(x + px(22.0) + w + sp(1.5), r.center().y), &sub, theme::regular(12.0), col::muted(), r.width() * 0.3);
                }
                if !hint.is_empty() {
                    ui.painter().text(pos2(r.right() - sp(1.25), r.center().y), Align2::RIGHT_CENTER, widgets::keys(&hint), theme::regular(11.5), col::muted());
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    picked = Some(i);
                }
            }
            ui.add_space(sp(0.5));
            ui.horizontal(|ui| {
                ui.label(RichText::new("↑↓ move   enter run   esc close").font(theme::regular(11.0)).color(col::muted()));
            });
        });
    });
    ctx.data_mut(|d| d.insert_temp(results_id(), shown.response.rect));
    if let Some(i) = picked {
        run(app, &list[i]);
        ctx.memory_mut(|m| m.surrender_focus(field_id()));
    }
}
