//! Dialogs over the window: confirmations, playlist and folder details, song info, the assistant welcome, shortcuts and about.

use super::SHORTCUTS;
use super::theme::{self, col};
use crate::app::{Action, App, Modal, Route};
use crate::model;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use serde_json::json;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut modal) = app.modal.take() else { return };
    let mut keep = true;
    let mut actions: Vec<Action> = Vec::new();
    let resp =
        egui::Modal::new("modal".into()).backdrop_color(Color32::from_black_alpha(140)).frame(theme::popup().inner_margin(egui::Margin::same(24)).corner_radius(theme::radius())).show(ctx, |ui| {
            ui.set_max_width(520.0);
            match &mut modal {
                Modal::Shortcuts => {
                    ui.label(RichText::new("Keyboard shortcuts").font(theme::regular(22.0)));
                    ui.label(RichText::new(super::widgets::keys("Press ⌘ / to show or hide this.")).color(col::sub()));
                    ui.add_space(theme::sp(1.0));
                    egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                        for (section, rows) in SHORTCUTS {
                            ui.add_space(theme::sp(1.0));
                            ui.label(RichText::new(*section).font(theme::regular(15.0)));
                            for (what, keys) in rows.iter() {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(*what).color(col::text()));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(RichText::new(super::widgets::keys(keys)).monospace().color(col::sub()));
                                    });
                                });
                            }
                        }
                    });
                    ui.add_space(theme::sp(1.0));
                    if ui.button("Close").clicked() {
                        keep = false;
                    }
                }
                Modal::About => {
                    ui.label(RichText::new("Glass").font(theme::regular(28.0)));
                    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(col::sub()));
                    ui.add_space(theme::sp(1.0));
                    ui.label("A quiet, transparent Spotify player, written in Rust.");
                    ui.add_space(theme::sp(0.75));
                    ui.label(
                        RichText::new(
                            "Built on librespot (audio, Spotify Connect and Spotify's own services), egui and rodio. Glass isn't affiliated with Spotify. Playback requires Spotify Premium.",
                        )
                        .color(col::sub()),
                    );
                    ui.add_space(theme::sp(1.25));
                    ui.horizontal(|ui| {
                        if ui.button("What Glass stores and sends").clicked() {
                            actions.push(Action::Navigate(Route::Activity));
                            keep = false;
                        }
                        if let Some(path) = crate::config::notices_file()
                            && ui.button("Open-source licenses").clicked()
                        {
                            let _ = open::that(path);
                        }
                        if ui.button("Close").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::EditPlaylist { id, name, desc, public, collaborative } => {
                    ui.label(RichText::new("Edit details").font(theme::regular(22.0)));
                    ui.add_space(theme::sp(1.25));
                    ui.horizontal_top(|ui| {
                        let cover = app.want(&format!("/playlists/{id}")).and_then(|p| model::image(&p, 300));
                        super::widgets::cover(ui, cover.as_deref(), 160.0, false);
                        ui.vertical(|ui| {
                            ui.add(egui::TextEdit::singleline(name).hint_text("Add a name").desired_width(300.0).font(theme::regular(16.0)));
                            ui.add(egui::TextEdit::multiline(desc).hint_text("Add an optional description").desired_width(300.0).desired_rows(4));
                            // Spotify keeps collaborative playlists private.
                            let shared = *collaborative;
                            ui.add_enabled(!shared, egui::Checkbox::new(public, "Show on your profile (public)")).on_disabled_hover_text("Collaborative playlists can't be public");
                            ui.checkbox(collaborative, "Collaborative (others can add songs)");
                        });
                    });
                    ui.add_space(theme::sp(1.25));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if ui.button(RichText::new("Save").font(theme::regular(14.0))).clicked() && !name.trim().is_empty() {
                            // A collaborative playlist can't be public.
                            let public = *public && !*collaborative;
                            actions
                                .push(Action::UpdatePlaylist { id: id.clone(), body: json!({ "name": name.trim(), "description": desc.trim(), "public": public, "collaborative": *collaborative }) });
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::NewFolder { name, add } => {
                    ui.label(RichText::new("Create folder").font(theme::regular(20.0)));
                    ui.add_space(theme::sp(1.0));
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(320.0));
                    // Focus the name to start with; Tab can still reach the buttons.
                    if ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.label(RichText::new("Folders are kept by Glass on this computer.").size(11.5).color(col::sub()));
                    ui.add_space(theme::sp(1.0));
                    // Enter in the field confirms (Enter on a focused Cancel button is that button's click).
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if (ui.button("Create").clicked() || enter) && !name.trim().is_empty() {
                            actions.push(Action::CreateFolder { name: name.trim().into(), add: add.clone() });
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::RenameFolder { id, name } => {
                    ui.label(RichText::new("Rename folder").font(theme::regular(20.0)));
                    ui.add_space(theme::sp(1.0));
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(320.0));
                    if ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.add_space(theme::sp(1.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if (ui.button("Save").clicked() || enter) && !name.trim().is_empty() {
                            actions.push(Action::RenameFolder { id: id.clone(), name: name.trim().into() });
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::EditTags(ed) => {
                    ui.label(RichText::new("Edit song info").font(theme::regular(22.0)));
                    let name = ed.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    ui.label(RichText::new(name).size(12.0).color(col::sub()));
                    ui.add_space(theme::sp(1.25));
                    let mut enter = false;
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(140.0, 140.0), egui::Sense::hover());
                            super::widgets::paint_cover(ui, r, ed.preview.as_deref(), false);
                            ui.add_space(theme::sp(0.75));
                            ui.horizontal(|ui| {
                                if ui.button("Choose image…").clicked() {
                                    actions.push(Action::ChooseCover);
                                }
                                if ed.preview.is_some() && ui.button("Remove").on_hover_text("Remove the cover from the file").clicked() {
                                    ed.cover = crate::local::Cover::Remove;
                                    ed.preview = None;
                                }
                            });
                        });
                        ui.add_space(theme::sp(2.0));
                        egui::Grid::new("tag-fields").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                            let t = &mut ed.tags;
                            for (label, value, hint) in
                                [("Title", &mut t.title, ""), ("Artist", &mut t.artist, ""), ("Album", &mut t.album, ""), ("Year", &mut t.year, "e.g. 2019"), ("Genre", &mut t.genre, "")]
                            {
                                ui.label(RichText::new(label).color(col::sub()));
                                let r = ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(240.0));
                                enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(theme::sp(1.0));
                    ui.label(
                        RichText::new("Playlists find local songs by title, artist and length, so changing the title or artist can stop a playlist from finding this file.")
                            .size(11.5)
                            .color(col::muted()),
                    );
                    if let Some(e) = &ed.error {
                        ui.add_space(theme::sp(0.5));
                        ui.label(RichText::new(e.as_str()).color(col::accent()));
                    }
                    ui.add_space(theme::sp(1.25));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let save = ui.add_enabled(!ed.saving, egui::Button::new(RichText::new(if ed.saving { "Saving…" } else { "Save" }).font(theme::regular(14.0))));
                        if (save.clicked() || enter) && !ed.saving {
                            actions.push(Action::SaveTags);
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::AiWelcome => {
                    // Shown once, after the first sign-in with ChatGPT plan usage enabled.
                    ui.label(RichText::new("You're using your ChatGPT plan").font(theme::regular(20.0)));
                    ui.add_space(theme::sp(0.75));
                    ui.label(RichText::new("Eligible usage in this app uses your ChatGPT plan. Manage usage in your ChatGPT settings.").color(col::sub()));
                    ui.add_space(theme::sp(1.5));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if ui.button(RichText::new("Got it").font(theme::regular(14.0))).clicked() {
                            actions.push(Action::AiDismissWelcome);
                            keep = false;
                        }
                        if ui.button("Manage usage").clicked() {
                            let _ = open::that(crate::ai::oauth::MANAGE_USAGE_URL);
                        }
                    });
                }
                Modal::Confirm { title, body, confirm, action } => {
                    ui.label(RichText::new(title.as_str()).font(theme::regular(20.0)));
                    ui.add_space(theme::sp(0.75));
                    ui.label(RichText::new(body.as_str()).color(col::sub()));
                    ui.add_space(theme::sp(1.75));
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if ui.button(RichText::new(confirm.as_str()).font(theme::regular(14.0))).clicked() {
                            actions.push((**action).clone());
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
            }
        });
    if resp.should_close() {
        keep = false;
    }
    if keep && app.modal.is_none() {
        app.modal = Some(modal);
    }
    for a in actions {
        app.act(a);
    }
}
