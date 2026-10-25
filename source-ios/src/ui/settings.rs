//! Settings, grouped like Spotify's. Rows that don't apply at once say when they do.

use super::theme::{self, col};
use super::widgets;
use crate::app::{Action, App};
use crate::config::{Autostart, EQ_BANDS, EQ_PRESETS, LibraryView, Loudness, Quality};
use crate::model::s;
use eframe::egui::{self, Align, Align2, CornerRadius, Frame, Layout, Margin, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui, pos2, vec2};
use std::cell::Cell;

/// The search, and the section being drawn.
struct Ctx<'a> {
    q: &'a str,
    /// The section's title matches: show all of it.
    whole: Cell<bool>,
    /// The section's title, waiting for its first matching row.
    title: Cell<Option<&'static str>>,
}

impl Ctx<'_> {
    fn matches(&self, text: &str) -> bool {
        self.q.is_empty() || text.to_lowercase().contains(self.q)
    }

    /// Whether a row shows; the first one in a section brings its title.
    fn reveal(&self, ui: &mut Ui, label: &str, note: &str) -> bool {
        if !(self.whole.get() || self.matches(label) || self.matches(note)) {
            return false;
        }
        if let Some(title) = self.title.take() {
            heading(ui, title);
        }
        true
    }
}

fn heading(ui: &mut Ui, title: &str) {
    ui.add_space(theme::sp(2.25));
    ui.label(RichText::new(title).font(theme::regular(18.0)));
    ui.add_space(theme::sp(0.5));
}

/// Starts a section. While searching, its title shows above its first
/// matching row, or the whole section shows when the title matches.
fn section(ui: &mut Ui, c: &Ctx, title: &'static str) {
    let whole = c.matches(title);
    c.whole.set(whole);
    c.title.set(None);
    if whole {
        heading(ui, title);
    } else {
        c.title.set(Some(title));
    }
}

/// A label on the left and a control on the right, with `note` under the label.
fn row(ui: &mut Ui, c: &Ctx, label: &str, note: &str, control: impl FnOnce(&mut Ui)) {
    if !c.reveal(ui, label, note) {
        return;
    }
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.vertical(|ui| {
            // Controls get 260 points, unless the window is too narrow (zoomed in) for
            // the label to keep 40% of the row.
            ui.set_max_width((ui.available_width() - 260.0).max(ui.available_width() * 0.4));
            ui.label(RichText::new(label).color(col::text()).size(14.0));
            if !note.is_empty() {
                ui.label(RichText::new(note).color(col::sub()).size(12.0));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), control);
    });
}

fn toggle(ui: &mut Ui, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let fill = col::raised().lerp_to_gamma(col::accent(), t);
    ui.painter().rect_filled(rect, CornerRadius::same(11), fill);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    ui.painter().circle_filled(pos2(x, rect.center().y), 8.0, if *on { col::on_accent() } else { col::text() });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

const RESTART: &str = "Applies when the playback engine restarts.";
const LIVE: &str = "";

pub fn show(app: &mut App, ui: &mut Ui) {
    ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: 32, right: 32, top: 24, bottom: 48 }).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(900.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new("Settings").font(theme::regular(32.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let resp = ui.add(egui::TextEdit::singleline(&mut app.settings_query).hint_text("Search in Settings").desired_width(220.0));
                    if app.focus_search {
                        resp.request_focus();
                        app.focus_search = false;
                    }
                });
            });
            let q = app.settings_query.trim().to_lowercase();
            let c = Ctx { q: &q, whole: Cell::new(true), title: Cell::new(None) };
            // Settings change only from a click, a key, text or a drag: only then is the
            // copy made to see whether they did.
            let input = ui.input(|i| {
                i.pointer.any_down()
                    || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::PointerButton { .. } | egui::Event::Ime(_)))
            });
            let before = input.then(|| app.settings.clone());
            body(app, ui, &c);
            if let Some(before) = before
                && before != app.settings
            {
                app.mark_settings_dirty();
                if before.autostart != app.settings.autostart {
                    app.apply_autostart();
                }
            }
        });
    });
}

fn body(app: &mut App, ui: &mut Ui, c: &Ctx) {
    // ---------------------------------------------------------------- Account
    section(ui, c, "Account");
    if let Some(me) = app.me.as_ref() {
        // Spotify often leaves the email out.
        let who: Vec<&str> = [s(me, "display_name"), s(me, "email"), s(me, "country")].into_iter().filter(|p| !p.is_empty()).collect();
        let plan = match s(me, "product") {
            "premium" => "Premium",
            "free" | "open" => "Free",
            other => other,
        };
        row(ui, c, "Signed in as", &who.join(" · "), |ui| {
            ui.label(RichText::new(plan).color(col::accent()));
        });
    }
    row(ui, c, "Edit login methods", "Opens your Spotify account page in the browser.", |ui| {
        if ui.button("Edit").clicked() {
            let _ = open::that("https://www.spotify.com/account/overview/");
        }
    });
    row(ui, c, "Log out", "Forgets the session stored on this computer.", |ui| {
        if ui.button("Log out").clicked() {
            app.act(Action::LogOut);
        }
    });

    // ---------------------------------------------------------------- Appearance
    section(ui, c, "Appearance");
    let path = app.theme.path().display().to_string();
    row(ui, c, "Theme file", &path, |ui| {
        if ui.button("Reload").on_hover_text("Glass also reloads it by itself when the file changes").clicked() {
            app.theme.reload();
        }
        if ui.button("Edit in editor").clicked() {
            let _ = app.theme.ensure_file();
            let _ = open::that(app.theme.path());
        }
        if ui.button("Theme panel").clicked() {
            app.theme_panel = true;
        }
    });
    let status = match app.theme.error() {
        Some(e) => format!("Not applied: Glass keeps the last good theme until this is fixed.\n{e}"),
        None if app.theme.warnings().is_empty() => "Applied. Edits show up as soon as the file is saved.".to_string(),
        None => app.theme.warnings().join("\n"),
    };
    row(ui, c, "Theme status", &status, |_| {});
    row(ui, c, "Background effect", crate::window_fx::blur_status(), |_| {});
    let t = app.theme.theme();
    row(ui, c, "Font", &format!("{} · {} pt (set in theme.toml)", t.font_family, t.font_size), |_| {});
    let motion = if t.reduce_motion { "On" } else { "Off" };
    row(ui, c, "Reduce motion", &format!("{motion}. Follows the system setting; [motion] reduce in theme.toml overrides it."), |_| {});

    // ---------------------------------------------------------------- Assistant
    section(ui, c, "Assistant");
    if c.reveal(ui, "Assistant ChatGPT OpenAI API key model", "") {
        super::assistant::settings(app, ui);
    }

    // ---------------------------------------------------------------- Lyrics
    section(ui, c, "Lyrics");
    lyrics_settings(app, ui, c);

    // ---------------------------------------------------------------- Downloads
    section(ui, c, "Downloads");
    let (left, failed) = app.downloads.pending();
    let mb = app.downloads.total_bytes() as f64 / 1_048_576.0;
    let n = app.downloads.count();
    let summary = format!("{n} downloaded · {mb:.0} MB{}", if left > 0 { format!(" · {left} still downloading") } else { String::new() });
    row(ui, c, "Downloaded", &summary, |ui| {
        if n > 0 && ui.button("Remove all downloads").clicked() {
            app.modal = Some(crate::app::Modal::Confirm {
                title: "Remove all downloads?".into(),
                body: "Downloaded songs will stream again when you play them.".into(),
                confirm: "Remove".into(),
                action: Box::new(Action::RemoveAllDownloads),
            });
        }
    });
    if failed > 0 {
        let what = if failed == 1 { "1 song couldn't be downloaded".to_string() } else { format!("{failed} songs couldn't be downloaded") };
        row(ui, c, "Couldn't download", &format!("{what}. Glass tries again the next time it starts."), |_| {});
    }
    row(
        ui,
        c,
        "How downloads work",
        "Songs are kept in Glass's own cache, encrypted exactly as Spotify serves them, and play only in Glass while you're signed in. They start instantly and don't stream again; each play still needs a connection to unlock. Downloads use the streaming quality below, so changing it means downloading again.",
        |_| {},
    );

    // ------------------------------------------------------------- Save songs
    section(ui, c, "Save songs");
    save_song_settings(app, ui, c);

    // ---------------------------------------------------------------- Library
    section(ui, c, "Your Library");
    row(ui, c, "Use compact library layout", LIVE, |ui| {
        let mut on = app.settings.library_view == LibraryView::Compact;
        if toggle(ui, &mut on) {
            app.settings.library_view = if on { LibraryView::Compact } else { LibraryView::List };
        }
    });
    row(ui, c, "Show Local Files", LIVE, |ui| {
        toggle(ui, &mut app.settings.show_local_files);
    });
    let inbox = crate::share::inbox();
    row(
        ui,
        c,
        "Share with Glass on your other devices",
        &format!(
            "Glass on your other devices on this network, signed in to this account, finds this computer by itself: it can see and get the songs in Local Files, and send songs here. Right-click songs to send them; playlists get their missing local songs from your other devices. Received songs go to {}.",
            inbox.display()
        ),
        |ui| {
            toggle(ui, &mut app.settings.share_local_files);
        },
    );
    if app.settings.show_local_files {
        let dirs = app.settings.local_dirs.clone();
        for d in dirs {
            row(ui, c, "Show songs from", &d.display().to_string(), |ui| {
                if ui.button("Remove").clicked() {
                    app.settings.local_dirs.retain(|x| x != &d);
                    app.act(Action::RescanLocal);
                }
            });
        }
        row(ui, c, "Add a source", "Pick a folder of music files (MP3, M4A, AAC, FLAC, Ogg, WAV or AIFF).", |ui| {
            if ui.button("Add a source").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                if crate::local::add_source(&mut app.settings.local_dirs, dir) {
                    app.act(Action::RescanLocal);
                } else {
                    app.toast("That folder is already in Local Files");
                }
            }
        });
    }

    // ---------------------------------------------------------------- Display
    section(ui, c, "Display");
    row(ui, c, "Open the now-playing screen when playback starts", LIVE, |ui| {
        toggle(ui, &mut app.settings.now_playing_on_play);
    });
    row(ui, c, "Show desktop notifications when the song changes", "Uses your system's notifications.", |ui| {
        toggle(ui, &mut app.settings.track_notifications);
    });
    row(ui, c, "Match Now Playing to the album's colors", "Now Playing, its lyrics and visualizer, and the player bar take their colors from the cover of what's playing.", |ui| {
        toggle(ui, &mut app.settings.album_colors);
    });

    // ---------------------------------------------------------------- Playback
    section(ui, c, "Playback");
    row(ui, c, "Gapless playback", RESTART, |ui| {
        toggle(ui, &mut app.settings.gapless);
    });
    row(ui, c, "Autoplay — keep playing similar content when your music ends", RESTART, |ui| {
        toggle(ui, &mut app.settings.autoplay);
    });
    row(ui, c, "DJ voice — X talks between the DJ's songs", LIVE, |ui| {
        toggle(ui, &mut app.settings.dj_voice);
    });
    row(ui, c, "Mono audio — left and right speakers play the same audio", "Applies instantly.", |ui| {
        toggle(ui, &mut app.settings.mono);
    });
    row(ui, c, "Normalize volume — same volume level for all songs and podcasts", RESTART, |ui| {
        toggle(ui, &mut app.settings.normalize);
    });
    if app.settings.normalize {
        row(ui, c, "Volume level", "Loud may reduce audio quality. Applies when the playback engine restarts.", |ui| {
            egui::ComboBox::from_id_salt("loud").selected_text(format!("{:?}", app.settings.loudness)).show_ui(ui, |ui| {
                for l in [Loudness::Quiet, Loudness::Normal, Loudness::Loud] {
                    ui.selectable_value(&mut app.settings.loudness, l, format!("{l:?}"));
                }
            });
        });
    }
    if c.reveal(ui, "Equalizer", "") {
        ui.add_space(theme::sp(1.0));
        equalizer(app, ui);
    }

    // ---------------------------------------------------------------- Audio quality
    section(ui, c, "Audio quality");
    row(ui, c, "Streaming quality", "Also the quality of downloads. Applies when the playback engine restarts.", |ui| {
        egui::ComboBox::from_id_salt("quality").selected_text(app.settings.streaming_quality.label()).show_ui(ui, |ui| {
            for q in Quality::ALL {
                ui.selectable_value(&mut app.settings.streaming_quality, q, q.label());
            }
        });
    });

    // ---------------------------------------------------------------- Explicit
    section(ui, c, "Explicit content");
    row(ui, c, "Allow explicit content", "When off, explicit songs are greyed out and skipped.", |ui| {
        toggle(ui, &mut app.settings.allow_explicit);
    });

    // ---------------------------------------------------------------- Storage
    section(ui, c, "Storage");
    let dir = crate::config::cache_dir().join("audio");
    row(ui, c, "Cache", &format!("Audio kept for faster playback; downloads aren't touched. {}", dir.display()), |ui| {
        if ui.button("Clear cache").clicked() {
            app.act(Action::ClearAudioCache);
        }
    });
    row(ui, c, "Cache size limit (MB)", "Glass trims the cache to this every 10 minutes.", |ui| {
        ui.add(egui::DragValue::new(&mut app.settings.cache_limit_mb).range(256..=50_000).speed(32));
    });

    // ---------------------------------------------------------------- Startup
    section(ui, c, "Startup and window behavior");
    row(
        ui,
        c,
        "Open Glass automatically after you log into the computer",
        if cfg!(target_os = "macos") { "Writes ~/Library/LaunchAgents/app.glass.player.plist." } else { "Writes ~/.config/autostart/glass.desktop." },
        |ui| {
            egui::ComboBox::from_id_salt("autostart")
                .selected_text(match app.settings.autostart {
                    Autostart::Off => "No",
                    Autostart::Normal => "Yes",
                    Autostart::Minimized => "Minimized",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.settings.autostart, Autostart::Off, "No");
                    ui.selectable_value(&mut app.settings.autostart, Autostart::Normal, "Yes");
                    ui.selectable_value(&mut app.settings.autostart, Autostart::Minimized, "Minimized");
                });
        },
    );
    row(ui, c, "Close button should minimize the window", LIVE, |ui| {
        toggle(ui, &mut app.settings.close_minimizes);
    });

    // ---------------------------------------------------------------- Proxy
    section(ui, c, "Proxy");
    row(ui, c, "HTTP proxy", "For example http://host:8080. Applies after restarting Glass.", |ui| {
        ui.add(egui::TextEdit::singleline(&mut app.settings.proxy).hint_text("No proxy").desired_width(240.0));
    });

    // ---------------------------------------------------------------- Spotify Connect / Advanced
    section(ui, c, "Spotify Connect");
    row(ui, c, "Device name", "How this computer appears in other Spotify apps. Applies when the playback engine restarts.", |ui| {
        ui.add(egui::TextEdit::singleline(&mut app.settings.device_name).desired_width(200.0));
    });
    row(ui, c, "Playback engine", &app.engine_status.clone(), |ui| {
        if ui.button("Restart engine").clicked() {
            app.act(Action::RestartEngine);
        }
    });
    row(ui, c, "Client ID", "The OAuth client Glass signs in with. Changing it requires logging in again.", |ui| {
        ui.add(egui::TextEdit::singleline(&mut app.settings.client_id).desired_width(260.0).font(egui::TextStyle::Monospace));
    });

    section(ui, c, "Updates");
    use crate::app::Update;
    let build = if crate::update::RELEASE { "" } else { " (a development build: it doesn't update itself)" };
    row(ui, c, "Version", &format!("Glass {}{build}", crate::update::VERSION), |_| {});
    let auto_note = if cfg!(target_os = "macos") {
        "Download new versions in the background and install them when Glass quits."
    } else {
        "Check for new versions in the background (to update, build the new package)."
    };
    row(ui, c, "Update automatically", auto_note, |ui| {
        toggle(ui, &mut app.settings.auto_update);
    });
    let status = match &app.update {
        Update::Idle if app.settings.auto_update && crate::update::RELEASE => "Checks shortly after Glass starts, then every 6 hours.".to_string(),
        Update::Idle => "Not checked yet.".to_string(),
        Update::Checking => "Checking…".to_string(),
        Update::UpToDate => "Glass is up to date.".to_string(),
        Update::Ready { version, .. } => format!("Glass {version} is downloaded: it installs when you quit, or restart now."),
        Update::Available { version, .. } if cfg!(target_os = "linux") => format!("Glass {version} is out: build the new package to update."),
        Update::Available { version, .. } => format!("Glass {version} is out: get it from the releases page (this copy can't install it itself)."),
        Update::Installed => "Updated: the new version opens next time.".to_string(),
        Update::Failed(e) => format!("Couldn't check: {e}"),
    };
    let ready = matches!(app.update, Update::Ready { .. });
    row(ui, c, "Check for updates", &status, |ui| {
        if ready {
            if ui.button("Restart to update").clicked() {
                app.act(Action::RestartToUpdate);
            }
        } else if ui.add_enabled(!matches!(app.update, Update::Checking), egui::Button::new("Check now")).clicked() {
            app.act(Action::CheckForUpdates);
        }
    });
    if let Update::Ready { version, notes, .. } | Update::Available { version, notes } = &app.update
        && !notes.trim().is_empty()
    {
        let notes = notes.trim().to_string();
        row(ui, c, &format!("What's new in {version}"), &notes, |_| {});
    }
}

/// Share ▸ "Download as …" (see `save_song`).
fn save_song_settings(app: &mut App, ui: &mut Ui, c: &Ctx) {
    use crate::save_song::{BITRATES, Format};
    row(
        ui,
        c,
        "Format",
        "Share → Download as … saves the song in this format. The audio comes from JioSaavn or YouTube Music, matched to the Spotify song, not from Spotify. M4A keeps that audio exactly as it is (up to 320 kbps); MP3 converts it so it plays anywhere.",
        |ui| {
            egui::ComboBox::from_id_salt("save-format").selected_text(app.settings.save_format.name()).show_ui(ui, |ui| {
                for f in Format::ALL {
                    ui.selectable_value(&mut app.settings.save_format, f, f.name());
                }
            });
        },
    );
    if app.settings.save_format == Format::Mp3 {
        row(ui, c, "MP3 bitrate", "Higher keeps more of the source's quality.", |ui| {
            egui::ComboBox::from_id_salt("save-bitrate").selected_text(format!("{} kbps", app.settings.save_mp3_kbps)).show_ui(ui, |ui| {
                for kbps in BITRATES {
                    ui.selectable_value(&mut app.settings.save_mp3_kbps, kbps, format!("{kbps} kbps"));
                }
            });
        });
    }
    let folder = app.save_options().dir;
    row(ui, c, "Folder", &format!("Songs are saved to {}.", folder.display()), |ui| {
        if app.settings.save_dir.is_some() && ui.button("Reset").clicked() {
            app.settings.save_dir = None;
        }
        if ui.button("Change…").clicked()
            && let Some(dir) = rfd::FileDialog::new().pick_folder()
        {
            app.settings.save_dir = Some(dir);
        }
    });
    if !app.settings.show_local_files {
        return;
    }
    // A source above the folder (~/Music for ~/Music/Glass) already shows its songs:
    // there's nothing to switch off.
    if let Some(parent) = app.settings.local_dirs.iter().find(|d| **d != folder && folder.starts_with(d)) {
        row(ui, c, "Show saved songs in Local Files", &format!("Already in Local Files (it's inside {}).", super::widgets::home_relative(parent)), |_| {});
    } else {
        let mut on = app.settings.local_dirs.contains(&folder);
        row(ui, c, "Show saved songs in Local Files", "Adds the folder above to Local Files.", |ui| {
            if toggle(ui, &mut on) {
                if on {
                    let _ = std::fs::create_dir_all(&folder);
                    crate::local::add_source(&mut app.settings.local_dirs, folder.clone());
                } else {
                    app.settings.local_dirs.retain(|d| d != &folder);
                }
                app.act(Action::RescanLocal);
            }
        });
    }
}

fn lyrics_settings(app: &mut App, ui: &mut Ui, c: &Ctx) {
    use crate::lyrics::service::{ProviderSetting, SPOTIFY_NOTE};
    use crate::lyrics::translate::{Engine, Mode};
    let cfg = app.config.lyrics().clone();
    let connected = app.spotify_source().is_some();
    let note = match cfg.provider {
        ProviderSetting::Auto if cfg.fallback => "Auto: LRCLIB first, then Spotify's lyrics for Spotify songs LRCLIB doesn't have.",
        ProviderSetting::Auto => "Auto: LRCLIB only, since fallback is off.",
        ProviderSetting::Lrclib => "LRCLIB only.",
        ProviderSetting::Spotify => "Spotify only: Spotify's lyrics for songs played from Spotify.",
    };
    row(ui, c, "Lyrics source", note, |ui| {
        // Right to left: Spotify, LRCLIB, Auto.
        for (p, label) in [(ProviderSetting::Spotify, if connected { "Spotify" } else { "Spotify · not connected" }), (ProviderSetting::Lrclib, "LRCLIB"), (ProviderSetting::Auto, "Auto")] {
            if widgets::chip(ui, label, cfg.provider == p) && cfg.provider != p {
                app.act(Action::LyricsProvider(p));
            }
        }
    });
    if cfg.provider == ProviderSetting::Spotify {
        row(ui, c, "Spotify lyrics", SPOTIFY_NOTE, |_| {});
    }
    // Translation: how lyrics are shown, into which language, and by whom.
    row(ui, c, "Translation", "Show the original, the original with a translation under each line, or the translation alone. T in the lyrics view switches too.", |ui| {
        for m in [Mode::Only, Mode::Below, Mode::Off] {
            if widgets::chip(ui, m.label(), cfg.translation == m) && cfg.translation != m {
                app.act(Action::LyricsTranslation(m));
            }
        }
    });
    if cfg.translation != Mode::Off {
        let target = crate::lyrics::translate::target(&cfg.translate_to);
        // What's typed lives in egui's memory until the field loses focus.
        let id = egui::Id::new("settings-translate-to");
        let mut lang = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| cfg.translate_to.clone());
        row(ui, c, "Translate into", &format!("{}. \"system\" follows your system language; or type a code such as en, es or ja.", crate::lyrics::translate::language_name(&target)), |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut lang).desired_width(90.0));
            if r.lost_focus() && lang.trim() != cfg.translate_to {
                app.set_config("lyrics.translate_to", lang.trim().to_string().into());
            }
            if r.has_focus() {
                ui.data_mut(|d| d.insert_temp(id, lang.clone()));
            } else {
                ui.data_mut(|d| d.remove::<String>(id));
            }
        });
        let apple = crate::lyrics::translate::apple_available();
        let who = match cfg.translator {
            Engine::Auto if apple => "Auto: Spotify's own translation when it has one, then Apple's on-device translation (free, private), then the assistant if it's on.",
            Engine::Auto => "Auto: Spotify's own translation when it has one, then the assistant if it's on.",
            Engine::Apple if apple => {
                "Apple's on-device translation: free and private; nothing leaves this Mac. Languages are downloaded in System Settings → General → Language & Region → Translation Languages."
            }
            Engine::Apple => "Apple's on-device translation isn't available here (it needs macOS 26 and a release build): choose auto or assistant.",
            Engine::Assistant => "The assistant's model: works on any system and with any language; sends the lyrics to OpenAI and counts toward your plan or key.",
        };
        row(ui, c, "Translator", who, |ui| {
            for (e, label) in [(Engine::Assistant, "Assistant"), (Engine::Apple, "Apple on-device"), (Engine::Auto, "Auto")] {
                if e == Engine::Apple && !apple {
                    continue;
                }
                if widgets::chip(ui, label, cfg.translator == e) && cfg.translator != e {
                    app.set_config("lyrics.translator", e.name().into());
                }
            }
        });
    }
    let value = if cfg.offset_ms == 0 { "0.0 s".to_string() } else { format!("{:+.1} s", cfg.offset_ms as f64 / 1000.0) };
    row(
        ui,
        c,
        "Timing offset (all songs)",
        &format!(
            "{value}. Positive shows lyrics earlier, negative later (Bluetooth speakers often need a little negative). To fix one song, use [ and ] on the now-playing screen; Glass keeps that song's timing."
        ),
        |ui| {
            if cfg.offset_ms != 0 && ui.button("Reset").clicked() {
                app.act(Action::LyricsOffset(0));
            }
            if ui.button("+0.1 s").clicked() {
                app.act(Action::LyricsOffset(cfg.offset_ms + 100));
            }
            if ui.button("−0.1 s").clicked() {
                app.act(Action::LyricsOffset(cfg.offset_ms - 100));
            }
        },
    );
    let path = app.config.path().display().to_string();
    row(ui, c, "Settings file", &path, |ui| {
        if ui.button("Reload").on_hover_text("Glass also reloads it by itself when the file changes").clicked() {
            app.config.reload();
        }
        if ui.button("Edit in editor").clicked() {
            let _ = app.config.ensure_file();
            let _ = open::that(app.config.path());
        }
    });
    if let Some(e) = app.config.error() {
        row(ui, c, "Settings file status", &format!("Not applied: Glass keeps the last good settings until this is fixed.\n{e}"), |_| {});
    } else if !app.config.warnings().is_empty() {
        row(ui, c, "Settings file status", &app.config.warnings().join("\n"), |_| {});
    }
    // Counting files every frame would be wasteful; every few seconds is plenty.
    let id = egui::Id::new("lyrics-cache-usage");
    let now = ui.input(|i| i.time);
    let usage = match ui.ctx().data(|d| d.get_temp::<(f64, usize, u64)>(id)) {
        Some((t, n, b)) if now - t < 5.0 => (n, b),
        _ => {
            let (n, b) = app.lyrics_cache.usage();
            ui.ctx().data_mut(|d| d.insert_temp(id, (now, n, b)));
            (n, b)
        }
    };
    row(
        ui,
        c,
        "Saved lyrics",
        &format!(
            "{} · {:.0} KB. Matches are kept 30 days and misses a day. Clearing also removes saved translations; lyrics you picked by hand stay.",
            crate::model::songs(usage.0 as u64),
            usage.1 as f64 / 1024.0
        ),
        |ui| {
            if (usage.0 > 0 || usage.1 > 0) && ui.button("Clear").clicked() {
                app.act(Action::ClearLyricsCache);
                ui.ctx().data_mut(|d| d.remove::<(f64, usize, u64)>(id));
            }
        },
    );
    row(
        ui,
        c,
        "About LRCLIB",
        "Lyrics come from LRCLIB (lrclib.net), a free lyrics database kept by its community. When lyrics are on screen, Glass sends LRCLIB the song's title, artist, album and length, then keeps the answer on this computer. Lyrics belong to their rights holders; Glass shows them for your personal use and never shares them.",
        |_| {},
    );
}

fn equalizer(app: &mut App, ui: &mut Ui) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Equalizer").font(theme::regular(16.0)));
            toggle(ui, &mut app.settings.eq_enabled);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Reset").clicked() {
                    app.settings.eq_gains = [0.0; 6];
                    app.settings.eq_preset = "Flat".into();
                }
                egui::ComboBox::from_id_salt("eq-preset").selected_text(app.settings.eq_preset.clone()).show_ui(ui, |ui| {
                    for (name, gains) in EQ_PRESETS {
                        if ui.selectable_label(app.settings.eq_preset == *name, *name).clicked() {
                            app.settings.eq_preset = (*name).into();
                            app.settings.eq_gains = *gains;
                            app.settings.eq_enabled = true;
                        }
                    }
                    let _ = ui.selectable_label(app.settings.eq_preset == "Manual", "Manual");
                });
                ui.label(RichText::new("Presets").color(col::sub()));
            });
        });
        ui.label(RichText::new("Applies instantly to music played in Glass.").color(col::sub()).size(12.0));
        ui.add_space(theme::sp(1.25));
        // Curve with draggable band handles, like Spotify's.
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, 180.0), Sense::hover());
        let enabled = app.settings.eq_enabled;
        let y_of = |db: f32| egui::remap(db, 12.0..=-12.0, rect.top() + 12.0..=rect.bottom() - 24.0);
        for db in [-12.0, 0.0, 12.0] {
            let y = y_of(db);
            ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, if db == 0.0 { col::border() } else { col::divider() }));
            ui.painter().text(pos2(rect.left(), y - 2.0), Align2::LEFT_BOTTOM, format!("{db:+.0}dB"), theme::regular(10.0), col::sub());
        }
        let xs: Vec<f32> = (0..EQ_BANDS.len()).map(|i| rect.left() + 50.0 + (w - 80.0) * i as f32 / (EQ_BANDS.len() - 1) as f32).collect();
        let mut pts: Vec<Pos2> = Vec::new();
        for (i, x) in xs.iter().enumerate() {
            pts.push(pos2(*x, y_of(app.settings.eq_gains[i])));
        }
        let color = if enabled { col::accent() } else { col::muted() };
        // Smooth curve (Catmull–Rom), as Spotify's settings page draws it.
        let mut curve = Vec::new();
        for i in 0..pts.len() - 1 {
            let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
            let (p1, p2) = (pts[i], pts[i + 1]);
            let p3 = if i + 2 < pts.len() { pts[i + 2] } else { pts[i + 1] };
            for k in 0..=16 {
                let t = k as f32 / 16.0;
                let t2 = t * t;
                let t3 = t2 * t;
                let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
                curve.push(pos2(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
            }
        }
        ui.painter().add(egui::Shape::line(curve, Stroke::new(2.0, color)));
        for (i, p) in pts.iter().enumerate() {
            let id = ui.id().with(("eq", i));
            let hr = Rect::from_center_size(*p, vec2(24.0, 24.0));
            let resp = ui.interact(hr, id, Sense::drag());
            if resp.dragged()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let db = egui::remap_clamp(pos.y, rect.top() + 12.0..=rect.bottom() - 24.0, 12.0..=-12.0);
                app.settings.eq_gains[i] = (db * 10.0).round() / 10.0;
                app.settings.eq_preset = "Manual".into();
                app.settings.eq_enabled = true;
            }
            let r = if resp.hovered() || resp.dragged() { 8.0 } else { 6.0 };
            ui.painter().circle_filled(*p, r, col::text());
            if resp.hovered() || resp.dragged() {
                ui.painter().text(*p - vec2(0.0, 14.0), Align2::CENTER_BOTTOM, format!("{:+.1}dB", app.settings.eq_gains[i]), theme::medium(11.0), col::text());
            }
            ui.painter().text(pos2(xs[i], rect.bottom() - 6.0), Align2::CENTER_BOTTOM, EQ_BANDS[i].1, theme::regular(11.0), col::sub());
        }
    });
}
