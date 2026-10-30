//! The assistant panel: billing up top, the conversation with playlist
//! previews and approval requests, then the model, usage and composer at the
//! bottom. Also the assistant's section of Settings.

use super::theme::{self, col, px, sp};
use super::widgets;
use crate::ai::tools::{self, ApprovalKind, Dest, DraftStatus};
use crate::ai::{Billing, Line, Models, openai};
use crate::app::{Action, App, Route};
use crate::assistant_app::DraftEdit;
use eframe::egui::{self, Align, Key, Layout, Margin, RichText, ScrollArea, Sense, Stroke, Ui, vec2};

fn small(ui: &mut Ui, text: impl Into<String>, color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text.into()).font(theme::regular(11.5)).color(color)).wrap());
}

/// One quiet line: "Using ChatGPT plan · Manage usage", or the API-key note.
fn header(app: &mut App, ui: &mut Ui) {
    let font = theme::regular(11.5);
    ui.horizontal(|ui| match app.ai_billing() {
        Billing::ChatGptPlan => {
            ui.label(RichText::new("Using ChatGPT plan").font(font.clone()).color(col::muted()));
            ui.label(RichText::new("·").font(font.clone()).color(col::muted()));
            if widgets::link(ui, "Manage usage", font.clone(), col::sub()).on_hover_text("ChatGPT settings → Usage").clicked() {
                app.open_manage_usage();
            }
        }
        Billing::ApiKey => {
            ui.label(RichText::new("Your API key · billed by OpenAI").font(font.clone()).color(col::muted()));
        }
    });
    if let Models::Failed(e) = &app.ai.models {
        small(ui, format!("Couldn't list models: {e}"), col::accent());
    }
}

/// The model, as a small menu beside "new chat".
fn model_menu(app: &mut App, ui: &mut Ui) {
    let current = app.ai_model();
    let models = match &app.ai.models {
        Models::Ready(l) => l.clone(),
        _ => Vec::new(),
    };
    let shown = current.as_ref().and_then(|c| models.iter().find(|m| &m.slug == c)).map(|m| m.name.clone()).unwrap_or_else(|| match &app.ai.models {
        Models::Loading => "loading…".into(),
        Models::Failed(_) => "no models".into(),
        _ => "choose a model".into(),
    });
    egui::ComboBox::from_id_salt("ai-model").selected_text(RichText::new(shown).font(theme::regular(11.0)).color(col::sub())).width(px(110.0)).show_ui(ui, |ui| {
        for m in &models {
            if ui.selectable_label(current.as_deref() == Some(m.slug.as_str()), &m.name).clicked() {
                app.act(Action::AiSetModel(m.slug.clone()));
            }
        }
        ui.separator();
        if ui.selectable_label(false, "refresh list").clicked() {
            app.act(Action::AiRefreshModels);
        }
    });
}

/// When the assistant can't be used yet: why, and what to do.
fn not_ready(app: &mut App, ui: &mut Ui, why: &str) {
    ui.add_space(sp(2.0));
    if app.ai.limit_reached {
        // The documented usage-limit message: Manage usage first.
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("ChatGPT").font(theme::regular(11.5)).color(col::muted()));
            ui.label(RichText::new("Usage limit reached").font(theme::regular(15.0)));
            small(ui, "Review your plan or this app's limit in ChatGPT settings.", col::sub());
            ui.add_space(sp(1.0));
            ui.horizontal(|ui| {
                if widgets::bracket_button(ui, None, "Manage usage", true).clicked() {
                    app.open_manage_usage();
                }
                if widgets::bracket_button(ui, None, "try again", false).on_hover_text("Clear this notice and send requests again").clicked() {
                    app.act(Action::AiLimitAcknowledged);
                }
            });
        });
        return;
    }
    widgets::unavailable(ui, "assistant", why);
    ui.add_space(sp(1.5));
    ui.vertical_centered(|ui| {
        if widgets::bracket_button(ui, None, "open settings", true).clicked() {
            app.settings_query = "assistant".into();
            app.act(Action::Navigate(Route::Settings));
        }
    });
}

/// Where each local song is in the library, by the id the assistant knows it by.
/// Built once per scan of the library, not every frame.
fn local_ids(app: &App) -> std::sync::Arc<std::collections::HashMap<String, usize>> {
    let local = app.local_tracks.as_deref().map(Vec::as_slice).unwrap_or_default();
    let key = (local.as_ptr() as usize, local.len());
    let id = egui::Id::new("ai-local-ids");
    type Ids = std::sync::Arc<std::collections::HashMap<String, usize>>;
    if let Some((k, ids)) = app.ctx.data(|d| d.get_temp::<((usize, usize), Ids)>(id))
        && k == key
    {
        return ids;
    }
    let ids: Ids = std::sync::Arc::new(tools::track_ids(local).into_iter().enumerate().map(|(i, id)| (id, i)).collect());
    app.ctx.data_mut(|d| d.insert_temp(id, (key, ids.clone())));
    ids
}

/// A preview row: (title — artist, length), from the local library or what the assistant saw on Spotify.
fn preview_rows(app: &App, d: &tools::Draft) -> Vec<Option<(String, u64)>> {
    match d.dest {
        Dest::Spotify => d.tracks.iter().map(|u| app.ai.catalog.get(u).map(|i| (format!("{} — {}", i.name, i.by), i.duration_ms))).collect(),
        Dest::Local => {
            let local = app.local_tracks.as_deref().map(Vec::as_slice).unwrap_or_default();
            let ids = local_ids(app);
            d.tracks.iter().map(|i| ids.get(i).and_then(|&k| local.get(k)).map(|t| (format!("{} — {}", t.title, t.artist), t.duration_ms))).collect()
        }
    }
}

fn draft_card(app: &mut App, ui: &mut Ui, d: &tools::Draft) {
    let rows = preview_rows(app, d);
    let total: u64 = rows.iter().flatten().map(|r| r.1).sum();
    let busy = app.ai.busy();
    let spotify = d.dest == Dest::Spotify;
    let editable = !busy && !matches!(d.status, DraftStatus::Saved(_) | DraftStatus::Saving);
    egui::Frame::NONE.stroke(Stroke::new(1.0, col::border())).corner_radius(theme::small_radius()).inner_margin(Margin::same(sp(1.25) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(if spotify { "Spotify preview" } else { "preview" }).font(theme::regular(11.0)).color(col::muted()));
            if editable {
                // The name can be edited in place; it applies when the field loses focus.
                let id = egui::Id::new(("draft-name", &d.id));
                let mut name = ui.ctx().data(|m| m.get_temp::<String>(id)).unwrap_or_else(|| d.name.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut name).font(theme::regular(14.5)).desired_width(ui.available_width()).frame(egui::Frame::NONE));
                if r.lost_focus() && name.trim() != d.name && !name.trim().is_empty() {
                    app.act(Action::AiEditDraft { draft: d.id.clone(), edit: DraftEdit::Rename(name.clone()) });
                }
                if r.has_focus() {
                    ui.ctx().data_mut(|m| m.insert_temp(id, name));
                } else {
                    ui.ctx().data_mut(|m| m.remove::<String>(id));
                }
            } else {
                ui.label(RichText::new(&d.name).font(theme::regular(14.5)).color(col::text()));
            }
        });
        let replaces = match d.dest {
            Dest::Local => d.replaces.as_ref().and_then(|id| app.playlists.get(id)).map(|p| p.name.clone()),
            Dest::Spotify => d.replaces.as_ref().map(|id| app.ai.catalog.get(&format!("spotify:playlist:{id}")).map(|i| i.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "your playlist".into())),
        };
        let mut meta = format!("{} · {}", crate::model::songs(d.tracks.len() as u64), crate::model::fmt_ms(total));
        if let Some(r) = &replaces {
            meta.push_str(&if spotify { format!(" · would be added to “{r}”") } else { format!(" · would replace “{r}”") });
        }
        small(ui, meta, col::sub());
        ui.add_space(sp(0.5));
        let n = d.tracks.len();
        // Long previews scroll inside the card, so its name and buttons stay in view.
        ScrollArea::vertical().id_salt(("draft-rows", &d.id)).max_height(px(220.0)).auto_shrink([false, true]).show(ui, |ui| {
            for (i, row_info) in rows.iter().enumerate() {
                let Some((label, ms)) = row_info else {
                    small(ui, format!("{:02}  {}", i + 1, if spotify { "(song details unavailable)" } else { "(file no longer in the library)" }), col::muted());
                    continue;
                };
                let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), px(22.0)), Sense::hover());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone()));
                let w = row.width();
                let y = row.center().y;
                ui.painter().text(egui::pos2(row.left(), y), egui::Align2::LEFT_CENTER, format!("{:02}", i + 1), theme::regular(11.0), col::muted());
                let controls = if editable { px(54.0) } else { 0.0 };
                widgets::text_trunc(ui, egui::pos2(row.left() + px(22.0), y), label, theme::regular(12.0), col::text(), w - px(64.0) - controls);
                ui.painter().text(egui::pos2(row.right() - controls - px(2.0), y), egui::Align2::RIGHT_CENTER, crate::model::fmt_ms(*ms), theme::regular(11.0), col::sub());
                if editable {
                    let mut x = row.right() - controls;
                    for (label, edit, tip, on) in
                        [("↑", DraftEdit::Move(i, -1), "Move up", i > 0), ("↓", DraftEdit::Move(i, 1), "Move down", i + 1 < n), ("×", DraftEdit::Remove(i), "Remove from the preview", true)]
                    {
                        let r = egui::Rect::from_min_size(egui::pos2(x, row.top()), vec2(px(18.0), row.height()));
                        x += px(18.0);
                        if !on {
                            continue;
                        }
                        let b = ui.interact(r, resp.id.with(label), Sense::click());
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, theme::regular(12.0), if b.hovered() { col::accent() } else { col::muted() });
                        if b.on_hover_text(tip).clicked() {
                            app.act(Action::AiEditDraft { draft: d.id.clone(), edit });
                        }
                    }
                }
            }
        });
        ui.add_space(sp(0.75));
        match &d.status {
            DraftStatus::Saved(pid) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(if spotify { "saved to Spotify ✓" } else { "saved ✓" }).font(theme::regular(12.0)).color(col::accent()));
                    let pid = pid.clone();
                    if widgets::link(ui, "open", theme::regular(12.0), col::sub()).clicked() {
                        app.act(Action::Navigate(if spotify { Route::Playlist(pid) } else { Route::LocalPlaylist(pid) }));
                    }
                });
            }
            DraftStatus::Saving => {
                ui.label(RichText::new("saving to Spotify…").font(theme::regular(12.0)).color(col::sub()));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
            _ => {
                let asked = app.ai.approvals.iter().find(|a| matches!(&a.kind, ApprovalKind::SaveDraft { draft_id, .. } if *draft_id == d.id)).cloned();
                if asked.is_some() {
                    small(
                        ui,
                        match (spotify, replaces.is_some()) {
                            (false, true) => "The assistant asks to replace the playlist with this preview.",
                            (false, false) => "The assistant asks to save this as a new playlist.",
                            (true, true) => "The assistant asks to add these songs to your playlist.",
                            (true, false) => "The assistant asks to save this as a new Spotify playlist.",
                        },
                        col::sub(),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        let save_label = match (&replaces, spotify) {
                            (Some(r), false) => format!("replace “{}”", r.chars().take(18).collect::<String>()),
                            (Some(r), true) => format!("add to “{}”", r.chars().take(18).collect::<String>()),
                            (None, true) => "save to Spotify".into(),
                            (None, false) => "save".into(),
                        };
                        if widgets::bracket_button(ui, None, &save_label, true).on_hover_text("Save this preview (your approval)").clicked() {
                            app.act(Action::AiSaveDraft { draft: d.id.clone(), replace: d.replaces.clone() });
                        }
                        if replaces.is_some() && widgets::bracket_button(ui, None, "save as new", false).clicked() {
                            app.act(Action::AiSaveDraft { draft: d.id.clone(), replace: None });
                        }
                        if let Some(a) = &asked {
                            if widgets::bracket_button(ui, None, "decline", false).clicked() {
                                app.act(Action::AiDecline(a.id.clone()));
                            }
                        } else if widgets::bracket_button(ui, None, "discard", false).clicked() {
                            app.act(Action::AiDiscardDraft(d.id.clone()));
                        }
                    });
                });
            }
        }
    });
}

/// Recommendations: ▶ plays a song's 30-second preview (what's playing
/// pauses for it), "+ add" puts it in the playlist they're for, or one picked
/// from a menu; "add all" adds the rest.
fn picks_card(app: &mut App, ui: &mut Ui, p: &tools::Picks) {
    let target = p.for_playlist.as_ref().map(|id| (id.clone(), app.ai_playlist_name(id)));
    let preview = app.ai_preview.as_ref().map(|x| (x.uri.clone(), x.voice.is_some(), x.elapsed()));
    egui::Frame::NONE.stroke(Stroke::new(1.0, col::border())).corner_radius(theme::small_radius()).inner_margin(Margin::same(sp(1.25) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("recommendations").font(theme::regular(11.0)).color(col::muted()));
            ui.label(RichText::new(&p.title).font(theme::regular(14.5)).color(col::text()));
        });
        let meta = match &target {
            Some((_, name)) => format!("{} for “{name}”, none already in it · ▶ plays 30 seconds", crate::model::songs(p.songs.len() as u64)),
            None => format!("{} · ▶ plays 30 seconds, + adds to a playlist", crate::model::songs(p.songs.len() as u64)),
        };
        small(ui, meta, col::sub());
        ui.add_space(sp(0.5));
        for song in &p.songs {
            let Some(it) = app.ai.catalog.get(&song.uri) else { continue };
            let label = format!("{} — {}", it.name, it.by);
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), px(24.0)), Sense::hover());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone()));
            let y = row.center().y;
            // ▶ / … / ■
            let mine = preview.as_ref().filter(|(u, ..)| *u == song.uri);
            let (icon, tip) = match mine {
                None => ("▶", "Play a 30-second preview"),
                Some((_, false, _)) => ("…", "Loading the preview"),
                Some((_, true, _)) => ("■", "Stop the preview"),
            };
            let play = egui::Rect::from_min_size(row.min, vec2(px(20.0), row.height()));
            let b = ui.interact(play, resp.id.with("preview"), Sense::click());
            b.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{tip}: {}", it.name)));
            ui.painter().text(play.center(), egui::Align2::CENTER_CENTER, icon, theme::regular(12.0), if b.hovered() || mine.is_some() { col::accent() } else { col::sub() });
            if b.on_hover_text(tip).clicked() {
                app.act(Action::AiPreview(song.uri.clone()));
            }
            // "+ add" (or where it went), then the length (or the preview's time left).
            let add_w = px(78.0);
            let add = egui::Rect::from_min_max(egui::pos2(row.right() - add_w, row.top()), row.max);
            let time = match mine {
                Some((_, true, t)) => format!("0:{:02}", 30u64.saturating_sub(t.as_secs())),
                _ => crate::model::fmt_ms(it.duration_ms),
            };
            ui.painter().text(egui::pos2(add.left() - px(6.0), y), egui::Align2::RIGHT_CENTER, time, theme::regular(11.0), if mine.is_some() { col::accent() } else { col::sub() });
            widgets::text_trunc(ui, egui::pos2(play.right() + px(4.0), y), &label, theme::regular(12.0), col::text(), add.left() - play.right() - px(48.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(add).layout(Layout::right_to_left(Align::Center)), |ui| {
                let added = |id: &str| song.added_to.iter().any(|x| x == id);
                match &target {
                    Some((id, name)) if added(id) => {
                        ui.label(RichText::new("added ✓").font(theme::regular(11.5)).color(col::accent())).on_hover_text(format!("In “{name}”"));
                    }
                    Some((id, name)) => {
                        if widgets::link(ui, "+ add", theme::regular(11.5), col::sub()).on_hover_text(format!("Add to “{name}”")).clicked() {
                            app.act(Action::AiAddPicks { card: p.id.clone(), uri: Some(song.uri.clone()), playlist: id.clone() });
                        }
                    }
                    None => {
                        let text = if song.added_to.is_empty() { "+ add" } else { "added ✓" };
                        ui.menu_button(RichText::new(text).font(theme::regular(11.5)).color(if song.added_to.is_empty() { col::sub() } else { col::accent() }), |ui| {
                            ui.set_min_width(px(200.0));
                            egui::ScrollArea::vertical().max_height(px(320.0)).show(ui, |ui| {
                                for pl in super::menus::writable_playlists(app) {
                                    let id = crate::model::s(&pl, "id").to_string();
                                    let name = crate::model::s(&pl, "name").to_string();
                                    let shown = if added(&id) { format!("{name}  ✓") } else { name };
                                    if widgets::menu_item(ui, &shown) && !added(&id) {
                                        app.act(Action::AiAddPicks { card: p.id.clone(), uri: Some(song.uri.clone()), playlist: id });
                                    }
                                }
                            });
                        });
                    }
                }
            });
            if !song.why.is_empty() {
                ui.horizontal_top(|ui| {
                    ui.add_space(px(24.0));
                    small(ui, &song.why, col::muted());
                });
            }
        }
        ui.add_space(sp(0.75));
        ui.horizontal_wrapped(|ui| {
            if let Some((id, name)) = &target {
                let left = p.songs.iter().filter(|x| !x.added_to.contains(id)).count();
                if left > 1 && widgets::bracket_button(ui, None, &format!("add all to “{}”", name.chars().take(18).collect::<String>()), true).clicked() {
                    app.act(Action::AiAddPicks { card: p.id.clone(), uri: None, playlist: id.clone() });
                }
            }
            if widgets::bracket_button(ui, None, "dismiss", false).clicked() {
                app.act(Action::AiDismissPicks(p.id.clone()));
            }
        });
    });
    if preview.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn approval_card(app: &mut App, ui: &mut Ui, a: &tools::Approval) {
    let text = match &a.kind {
        ApprovalKind::QueueMany { ids, mode } => {
            let spotify = ids.first().is_some_and(|i| i.starts_with("spotify:"));
            let n = crate::model::songs(ids.len() as u64);
            match mode {
                tools::QueueMode::PlayNow => format!("The assistant asks to play {n}."),
                tools::QueueMode::Next => format!("The assistant asks to play {n} next."),
                tools::QueueMode::End if spotify => format!("The assistant asks to add {n} to your Spotify queue."),
                tools::QueueMode::End => format!("The assistant asks to add {n} to the end of the queue."),
            }
        }
        ApprovalKind::Like { ids } => format!("The assistant asks to add {} to your Liked Songs.", crate::model::songs(ids.len() as u64)),
        ApprovalKind::SaveDraft { .. } => return,
    };
    egui::Frame::NONE.stroke(Stroke::new(1.0, col::accent())).corner_radius(theme::small_radius()).inner_margin(Margin::same(sp(1.25) as i8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(text).font(theme::regular(12.5)));
        ui.horizontal(|ui| {
            if widgets::bracket_button(ui, None, "approve", true).clicked() {
                app.act(Action::AiApprove(a.id.clone()));
            }
            if widgets::bracket_button(ui, None, "decline", false).clicked() {
                app.act(Action::AiDecline(a.id.clone()));
            }
        });
    });
}

pub fn panel(app: &mut App, ui: &mut Ui) {
    let pad = sp(2.25) as i8;
    egui::Frame::NONE.inner_margin(Margin { left: pad, right: pad, top: sp(1.0) as i8, bottom: pad }).show(ui, |ui| {
        if !app.ai_enabled() {
            not_ready(app, ui, "The assistant is off. It's optional: turn it on in Settings → Assistant.");
            return;
        }
        header(app, ui);
        ui.add_space(sp(0.5));
        widgets::hairline(ui);
        if let Err(why) = app.ai_ready() {
            not_ready(app, ui, &why);
            return;
        }
        // Conversation (scrolls), then the composer pinned to the bottom.
        let composer_h = px(96.0);
        let h = (ui.available_height() - composer_h).max(px(120.0));
        ScrollArea::vertical().id_salt("ai-transcript").max_height(h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            // Taken out for the frame and put back after (nothing changes it meanwhile:
            // actions run after the frame), so a long chat isn't copied every frame.
            let lines = std::mem::take(&mut app.ai.transcript);
            let last = lines.len().saturating_sub(1);
            for (i, l) in lines.iter().enumerate() {
                ui.add_space(sp(0.75));
                match l {
                    Line::User(t) => {
                        ui.label(RichText::new("you").font(theme::regular(10.5)).color(col::muted()));
                        ui.add(egui::Label::new(RichText::new(t).font(theme::regular(13.0)).color(col::text())).wrap());
                    }
                    Line::Assistant(t) => {
                        ui.label(RichText::new("glass").font(theme::regular(10.5)).color(col::accent()));
                        let shown = if t.is_empty() && i == last && app.ai.busy() { "…" } else { t.as_str() };
                        ui.add(egui::Label::new(RichText::new(shown).font(theme::regular(13.0)).color(col::sub())).wrap());
                    }
                    Line::Note(t) => small(ui, format!("· {t}"), col::muted()),
                    Line::Error(t) => {
                        small(ui, t, col::accent());
                        if i == last && app.ai.last_request.is_some() && !app.ai.busy() {
                            if widgets::link(ui, "retry", theme::regular(12.0), col::sub()).clicked() {
                                app.act(Action::AiRetry);
                            }
                            if t.contains("Usage limit") && widgets::link(ui, "Manage usage", theme::regular(12.0), col::accent()).clicked() {
                                app.open_manage_usage();
                            }
                        }
                    }
                }
            }
            app.ai.transcript = lines;
            let approvals = std::mem::take(&mut app.ai.approvals);
            for a in &approvals {
                ui.add_space(sp(1.0));
                approval_card(app, ui, a);
            }
            app.ai.approvals = approvals;
            let drafts = std::mem::take(&mut app.ai.drafts);
            for d in drafts.iter().rev() {
                ui.add_space(sp(1.0));
                draft_card(app, ui, d);
            }
            app.ai.drafts = drafts;
            // Recommendations last, the newest nearest the composer (the panel opens at the bottom).
            let picks = std::mem::take(&mut app.ai.picks);
            for p in &picks {
                ui.add_space(sp(1.0));
                picks_card(app, ui, p);
            }
            app.ai.picks = picks;
        });
        widgets::hairline(ui);
        ui.add_space(sp(0.5));
        // Status / stop.
        ui.horizontal(|ui| {
            if let Some(r) = &app.ai.running {
                ui.label(RichText::new(&r.status).font(theme::regular(11.5)).color(col::sub()));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::link(ui, "stop  esc", theme::regular(11.5), col::accent()).clicked() {
                        app.act(Action::AiCancel);
                    }
                });
            } else {
                model_menu(app, ui);
                let u = app.ai.usage;
                let mut usage = if u.input + u.output > 0 { format!("{} / {} tokens", kfmt(u.input), kfmt(u.output)) } else { String::new() };
                if app.ai_billing() == Billing::ApiKey
                    && let Some(m) = app.ai_model()
                    && let Some(usd) = openai::estimate_usd(&m, u)
                    && u.input + u.output > 0
                {
                    usage.push_str(&format!(" · ≈ ${usd:.4} (estimate)"));
                }
                ui.label(RichText::new(usage).font(theme::regular(10.5)).color(col::muted())).on_hover_text("This session: tokens in / out");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::link(ui, "new chat", theme::regular(11.5), col::sub()).clicked() {
                        app.act(Action::AiNewChat);
                    }
                    if app.ai_undo.is_some() && widgets::link(ui, "undo", theme::regular(11.5), col::sub()).on_hover_text("Undo the last change the assistant made").clicked() {
                        app.act(Action::AiUndo);
                    }
                });
            }
        });
        // ⇧Enter is the field's own new line (at the cursor); plain Enter sends, below.
        let edit = egui::TextEdit::multiline(&mut app.ai.input)
            .hint_text("ask, or tell Glass what to play…")
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .font(theme::regular(13.0))
            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, Key::Enter));
        let resp = ui.add(edit);
        if app.ai.focus_input {
            resp.request_focus();
            app.ai.focus_input = false;
        }
        // Enter sends; Esc stops a running request.
        if resp.has_focus() {
            let (send, stop) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, Key::Enter), app.ai.busy() && i.consume_key(egui::Modifiers::NONE, Key::Escape)));
            if stop {
                app.act(Action::AiCancel);
            }
            if send && !app.ai.input.trim().is_empty() {
                let text = std::mem::take(&mut app.ai.input);
                app.act(Action::AiSend(text));
            }
        }
    });
}

fn kfmt(n: u64) -> String {
    if n >= 1000 { format!("{:.1}k", n as f64 / 1000.0) } else { n.to_string() }
}

/// The Settings section: opt-in, billing, account, API key, model, data use.
pub fn settings(app: &mut App, ui: &mut Ui) {
    let cfg = app.config.assistant().clone();
    let sub = |ui: &mut Ui, t: &str| {
        ui.add(egui::Label::new(RichText::new(t).color(col::sub()).size(12.0)).wrap());
    };
    let mut on = cfg.enabled;
    ui.horizontal(|ui| {
        if ui.checkbox(&mut on, RichText::new("Turn on the assistant").size(14.0)).changed() {
            app.act(Action::AiEnable(on));
        }
    });
    if !cfg.enabled {
        sub(ui, crate::ai::data_note(cfg.spotify));
        return;
    }
    let mut spotify = cfg.spotify;
    ui.horizontal(|ui| {
        if ui.checkbox(&mut spotify, RichText::new("Let it use my Spotify library and playback").size(14.0)).changed() {
            app.act(Action::AiUseSpotify(spotify));
        }
    });
    sub(
        ui,
        "Liked Songs, your playlists, listening history and search, so it can answer questions, play and queue songs, like them and build playlists. Spotify's Developer Terms restrict sending Spotify data to AI services; with this off, the assistant only sees your local files.",
    );
    sub(ui, crate::ai::data_note(cfg.spotify));
    ui.add_space(sp(1.0));
    let billing = app.ai_billing();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Pay for requests with").size(14.0));
        if ui.selectable_label(billing == Billing::ChatGptPlan, "my ChatGPT plan").clicked() && billing != Billing::ChatGptPlan {
            app.act(Action::AiSetBilling(Billing::ChatGptPlan));
        }
        if ui.selectable_label(billing == Billing::ApiKey, "my OpenAI API key").clicked() && billing != Billing::ApiKey {
            app.act(Action::AiSetBilling(Billing::ApiKey));
        }
    });
    ui.add_space(sp(0.5));
    match billing {
        Billing::ChatGptPlan => {
            sub(
                ui,
                "Works with ChatGPT Plus and Pro. Requests count toward your plan's limits (on Plus, the five-hour limit is shared with your other apps). Glass can't see your ChatGPT conversations.",
            );
            let active = app.ai.accounts.active().cloned();
            ui.add_space(sp(0.5));
            match &active {
                None => {
                    if app.ai.signing_in.is_some() {
                        ui.horizontal(|ui| {
                            ui.label("Waiting for the browser…");
                            if ui.button("Cancel").clicked() {
                                app.act(Action::AiCancelSignIn);
                            }
                        });
                    } else if widgets::bracket_button(ui, None, "Continue with ChatGPT", true).clicked() {
                        app.act(Action::AiSignIn { reconsent: false, new_account: true });
                    }
                    let saved: Vec<_> = app.ai.accounts.registrations().iter().filter(|r| r.subject.is_some()).cloned().collect();
                    for r in saved {
                        ui.horizontal(|ui| {
                            sub(ui, &format!("Saved: {}", r.label()));
                            if ui.button("Continue with this account").clicked() {
                                app.act(Action::AiSelectAccount(r.client_id.clone()));
                                app.act(Action::AiSignIn { reconsent: false, new_account: false });
                            }
                        });
                    }
                }
                Some(r) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(r.label()).size(14.0));
                        ui.label(
                            RichText::new(if r.plan_enabled { "· ChatGPT plan usage enabled" } else { "· plan usage not enabled" })
                                .color(if r.plan_enabled { col::accent() } else { col::sub() })
                                .size(12.5),
                        );
                    });
                    ui.horizontal(|ui| {
                        if !r.plan_enabled && ui.button("Enable ChatGPT plan use").on_hover_text("Opens the browser to ask for permission again").clicked() {
                            app.act(Action::AiSignIn { reconsent: true, new_account: false });
                        }
                        if ui.button("Manage usage").on_hover_text("ChatGPT settings → Usage").clicked() {
                            app.open_manage_usage();
                        }
                        if ui.button("Sign out").clicked() {
                            app.act(Action::AiSignOut);
                        }
                        if ui.button("Add another account").clicked() {
                            app.act(Action::AiSignIn { reconsent: false, new_account: true });
                        }
                    });
                    let others: Vec<_> = app.ai.accounts.registrations().iter().filter(|x| x.client_id != r.client_id && x.subject.is_some()).cloned().collect();
                    for o in others {
                        ui.horizontal(|ui| {
                            sub(ui, &format!("Also saved: {}", o.label()));
                            if ui.button("Switch").clicked() {
                                app.act(Action::AiSelectAccount(o.client_id.clone()));
                            }
                        });
                    }
                    sub(
                        ui,
                        &format!("Your sign-in is kept in {}. To revoke access, sign out here (Glass revokes the session), or disconnect Glass in ChatGPT settings.", app.ai.auth.store.describe()),
                    );
                }
            }
            if let Some(n) = &app.ai.account_note {
                ui.add_space(sp(0.5));
                ui.label(RichText::new(n).color(col::accent()).size(12.5));
            }
        }
        Billing::ApiKey => {
            ui.label(
                RichText::new("API usage is billed by OpenAI to the account that owns the key, separately from any ChatGPT plan. Glass never switches to it on its own.")
                    .color(col::accent())
                    .size(12.5),
            );
            ui.add_space(sp(0.5));
            if app.ai.has_api_key() {
                ui.horizontal(|ui| {
                    sub(ui, &format!("A key is saved in {}.", app.ai.auth.store.describe()));
                    if ui.button("Remove key").clicked() {
                        app.act(Action::AiForgetApiKey);
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut app.ai.key_field).password(true).hint_text("sk-…").desired_width(px(260.0)));
                    if ui.button("Save key").clicked() {
                        app.act(Action::AiSaveApiKey);
                    }
                });
                sub(ui, &format!("Typed here only — the key goes straight into {} and is never shown, logged or written to a file.", app.ai.auth.store.describe()));
            }
        }
    }
    ui.add_space(sp(1.0));
    ui.horizontal(|ui| {
        ui.label(RichText::new("Model").size(14.0));
        match (app.ai.models.clone(), app.ai_model()) {
            (Models::Ready(list), m) => {
                let shown = m.as_ref().and_then(|m| list.iter().find(|x| &x.slug == m)).map(|x| x.name.clone()).unwrap_or_else(|| "choose a model".into());
                egui::ComboBox::from_id_salt("ai-model-settings").selected_text(shown).show_ui(ui, |ui| {
                    for x in &list {
                        if ui.selectable_label(m.as_deref() == Some(x.slug.as_str()), &x.name).clicked() {
                            app.act(Action::AiSetModel(x.slug.clone()));
                        }
                    }
                });
            }
            (Models::Loading, _) => {
                ui.label("loading…");
            }
            (Models::Failed(e), _) => {
                ui.label(RichText::new(e.as_str()).color(col::accent()).size(12.5));
            }
            (Models::Unknown, _) => {
                let when = if billing == Billing::ApiKey { "available once a key is saved" } else { "available after signing in" };
                ui.label(RichText::new(when).color(col::sub()).size(12.5));
            }
        }
    });
    sub(ui, "GPT-6 Luna is chosen when your account offers it. Glass keeps your choice and never switches to a pricier model by itself.");
}
