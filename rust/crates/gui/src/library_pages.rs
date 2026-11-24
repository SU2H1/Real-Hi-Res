//! The pages added to the player after its first version (2.0,
//! `docs/player-spec.md` "Additions"): the Library start page, Folders,
//! Playlists, Recently Played and Your Stats with its details.

use std::collections::HashMap;

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, Ui};
use realhires::musiclib::{self, Track};
use realhires::paths::{abspath, file_name};
use realhires::stats::{self, PERIODS};

use crate::icons::{self, Icon};
use crate::look::{self, Palette};
use crate::pages::elided;
use crate::player::{cover, NameAction, Page, Player};

/// The colours of the summary cards, one after the other.
const CARD_COLORS: [u32; 10] =
    [0xfa2d48, 0x7c5cff, 0x0a84ff, 0x30b06e, 0xff9f0a, 0xe0457b, 0x00a3a3, 0x5856d6, 0xc0673a, 0x8e8e93];

fn page_title(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).font(look::title(28.0)).color(pal.text));
    ui.add_space(12.0);
}

fn pill(ui: &mut Ui, pal: &Palette, icon: Icon, label: &str) -> egui::Response {
    let font = look::title(13.0);
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x) + 46.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(110.0), 32.0), Sense::click());
    let fill = if resp.hovered() { pal.accent.gamma_multiply(0.85) } else { pal.accent };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(18.0, 18.0)), icon, Color32::WHITE);
    ui.painter().text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, font, Color32::WHITE);
    resp
}

fn back_link(ui: &mut Ui, pal: &Palette, label: &str) -> egui::Response {
    let font = look::text(13.0);
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), pal.accent).size().x) + 22.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), vec2(16.0, 16.0)), Icon::Back, pal.accent);
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, label, font, pal.accent);
    ui.add_space(6.0);
    resp
}

fn plural(n: usize, word: &str) -> String {
    format!("{} {word}{}", stats::thousands(n as i64), if n == 1 { "" } else { "s" })
}

fn minutes_text(tracks: &[&Track]) -> String {
    let total: f64 = tracks.iter().map(|t| t.duration.max(0.0)).sum();
    let m = (total / 60.0).round() as i64;
    if m >= 60 {
        format!("{} hr {} min", m / 60, m % 60)
    } else {
        format!("{m} minute{}", if m == 1 { "" } else { "s" })
    }
}

impl Player {
    /// Up to four album covers in a square (playlists).
    fn mosaic(&mut self, p: &egui::Painter, rect: Rect, paths: &[String], name: &str) {
        let mut keys: Vec<(String, Vec<String>)> = vec![];
        for path in paths {
            if let Some(a) = self.lib.album_of(path) {
                if !keys.iter().any(|(k, _)| *k == a.key) {
                    keys.push((a.key.clone(), a.tracks.iter().take(4).map(|t| t.path.clone()).collect()));
                }
            }
            if keys.len() == 4 {
                break;
            }
        }
        if keys.len() < 4 {
            let tex = keys.first().and_then(|(k, ps)| self.covers.get(k, ps));
            cover(p, rect, if keys.is_empty() { None } else { tex.as_ref() }, name, 8);
            return;
        }
        let half = rect.width() / 2.0;
        for (i, (k, ps)) in keys.iter().enumerate() {
            let r = Rect::from_min_size(rect.min + vec2((i % 2) as f32 * half, (i / 2) as f32 * half), vec2(half, half));
            let tex = self.covers.get(k, ps);
            cover(p, r, tex.as_ref(), name, 0);
        }
        p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(0.0, Color32::TRANSPARENT), egui::StrokeKind::Inside);
    }

    // -------------------------------------------------------------- library ---

    pub fn library_home(&mut self, ui: &mut Ui, pal: &Palette) {
        let recent_n = self.recently_played().len();
        let year = {
            let saved = self.stats_period.clone();
            self.stats_period = "This year".into();
            let st = self.stats();
            self.stats_period = saved;
            st
        };
        let l = &self.lib;
        let tiles: Vec<(Icon, &str, String, Page)> = vec![
            (Icon::Note, "All Songs", plural(l.tracks.len(), "song"), Page::AllSongs),
            (Icon::Folder, "Folders", plural(self.scanner.cfg.library_folders.len(), "folder"), Page::Folders(None)),
            (Icon::Albums, "Albums", plural(l.albums.len(), "album"), Page::Albums),
            (Icon::Person, "Artists", plural(l.artists.len(), "artist"), Page::Artists),
            (Icon::People, "Album Artists", plural(l.album_artists.len(), "album artist"), Page::AlbumArtists),
            (Icon::Genres, "Genres", plural(l.genres.len(), "genre"), Page::Genres),
            (Icon::Calendar, "Years", plural(l.years.len(), "year"), Page::Years),
            (Icon::Composer, "Composers", plural(l.composers.len(), "composer"), Page::Composers),
            (Icon::Playlist, "Playlists", plural(self.state.playlists.len(), "playlist"), Page::Playlists),
            (Icon::Heart, "Liked Songs", plural(self.state.liked.len(), "song"), Page::Liked),
            (Icon::ThumbDown, "Disliked Songs", plural(self.state.disliked.len(), "song"), Page::Disliked),
            (Icon::Clock, "Recently Played", plural(recent_n, "song"), Page::RecentlyPlayed),
            (Icon::Albums, "Recently Added", plural(l.albums.len().min(50), "album"), Page::Recent),
            (Icon::Chart, "Your Stats", format!("{} minutes this year", stats::thousands((year.seconds / 60.0).round() as i64)), Page::Stats),
        ];
        egui::ScrollArea::vertical().id_salt("library-home").auto_shrink([false, false]).show(ui, |ui| {
            page_title(ui, pal, "Library");
            if self.lib.tracks.is_empty() {
                ui.label(RichText::new(self.library_status().unwrap_or_else(|| {
                    "Add your music folder in Settings → Library to fill your library.".into()
                })).color(pal.secondary));
                ui.add_space(8.0);
                if self.library_status().is_none() && pill(ui, pal, Icon::Folder, "Choose a music folder…").clicked() {
                    self.add_library_folder();
                }
                ui.add_space(14.0);
            }
            let gap = 12.0;
            let width = ui.available_width();
            let cols = ((width + gap) / (210.0 + gap)).floor().max(1.0) as usize;
            let tw = (width + gap) / cols as f32 - gap;
            for row in tiles.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (icon, name, count, page) in row {
                        let (rect, resp) = ui.allocate_exact_size(vec2(tw, 74.0), Sense::click());
                        let fill = if resp.hovered() { pal.hover } else { pal.lcd };
                        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
                        icons::paint(ui.painter(), Rect::from_center_size(rect.left_top() + vec2(26.0, 26.0), vec2(22.0, 22.0)), *icon, pal.accent);
                        let g = elided(ui, name, look::title(14.0), pal.text, tw - 56.0);
                        ui.painter().galley(rect.left_top() + vec2(46.0, 16.0), g, pal.text);
                        let g = elided(ui, count, look::text(12.0), pal.secondary, tw - 56.0);
                        ui.painter().galley(rect.left_top() + vec2(46.0, 38.0), g, pal.secondary);
                        if resp.clicked() {
                            self.go(page.clone());
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            if year.logged > 0 {
                ui.add_space(14.0);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
                let top = look::parse_hex("#fa2d48");
                let fill = if resp.hovered() { top.gamma_multiply(0.9) } else { top };
                ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
                ui.painter().text(rect.left_center() + vec2(20.0, -10.0), Align2::LEFT_CENTER, "Your Stats", look::title(16.0), Color32::WHITE);
                ui.painter().text(
                    rect.left_center() + vec2(20.0, 12.0),
                    Align2::LEFT_CENTER,
                    format!(
                        "{} minutes of music this year, {} by {}.",
                        stats::thousands((year.seconds / 60.0).round() as i64),
                        plural(year.songs, "song"),
                        plural(year.artists, "artist")
                    ),
                    look::text(13.0),
                    Color32::WHITE,
                );
                if resp.clicked() {
                    self.stats_period = "This year".into();
                    self.go(Page::Stats);
                }
            }
            if !self.lib.recent.is_empty() {
                ui.add_space(18.0);
                ui.label(RichText::new("Recently Added").font(look::title(18.0)).color(pal.text));
                ui.add_space(8.0);
                let albums: Vec<usize> = self.lib.recent.iter().take(12).copied().collect();
                egui::ScrollArea::horizontal().id_salt("home-recent").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 18.0;
                        for a in albums {
                            self.album_card(ui, pal, a, 150.0);
                        }
                    });
                });
            }
        });
    }

    // -------------------------------------------------------------- folders ---

    pub fn folders_page(&mut self, ui: &mut Ui, pal: &Palette, folder: Option<String>) {
        let roots: Vec<String> = self.scanner.cfg.library_folders.iter().map(|f| abspath(f)).collect();
        let sep = std::path::MAIN_SEPARATOR;
        let under = |root: &str, t: &Track| t.path.starts_with(&format!("{}{sep}", root.trim_end_matches(sep)));
        // with one library folder, Folders opens it at once
        let folder = folder.or_else(|| (roots.len() == 1).then(|| roots[0].clone()));
        let Some(folder) = folder else {
            page_title(ui, pal, "Folders");
            if roots.is_empty() {
                ui.label(RichText::new("No music folder yet: add one in Settings → Library.").color(pal.secondary));
                return;
            }
            let rows: Vec<(String, String, usize)> = roots
                .iter()
                .map(|r| (r.clone(), r.clone(), self.lib.tracks.iter().filter(|t| under(r, t)).count()))
                .collect();
            self.folder_rows(ui, pal, &rows);
            return;
        };
        // breadcrumb from the library folder that holds it
        let root = roots.iter().filter(|r| folder == **r || folder.starts_with(&format!("{}{sep}", r.trim_end_matches(sep)))).max_by_key(|r| r.len()).cloned();
        ui.horizontal_wrapped(|ui| {
            if roots.len() > 1 && back_link(ui, pal, "Folders").clicked() {
                self.go(Page::Folders(None));
            }
            if let Some(root) = &root {
                let mut path = root.clone();
                let mut parts = vec![(file_name(root), root.clone())];
                for piece in folder[root.len()..].split(sep).filter(|p| !p.is_empty()) {
                    path = format!("{}{sep}{piece}", path.trim_end_matches(sep));
                    parts.push((piece.to_string(), path.clone()));
                }
                for (i, (name, p)) in parts.iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("›").color(pal.secondary));
                    }
                    let last = i + 1 == parts.len();
                    let text = RichText::new(name).color(if last { pal.text } else { pal.accent });
                    if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() && !last {
                        self.go(Page::Folders(Some(p.clone())));
                    }
                }
            }
        });
        ui.add_space(6.0);
        let (subs, songs) = musiclib::folder_view(&self.lib.tracks, &folder);
        page_title(ui, pal, &file_name(&folder));
        let mut all: Vec<Track> =
            self.lib.tracks.iter().filter(|t| t.path.starts_with(&format!("{}{sep}", folder.trim_end_matches(sep)))).cloned().collect();
        all.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<String> = all.iter().map(|t| t.path.clone()).collect();
        if !paths.is_empty() {
            ui.horizontal(|ui| {
                let name = file_name(&folder);
                if pill(ui, pal, Icon::Play, "Play").clicked() {
                    self.play_list(paths.clone(), Some(&paths[0]), &name);
                }
                if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                    self.shuffle_list(paths.clone(), &name);
                }
            });
            ui.add_space(10.0);
        }
        if !subs.is_empty() {
            self.folder_rows(ui, pal, &subs);
            ui.add_space(10.0);
        }
        if !songs.is_empty() {
            let list: Vec<String> = songs.iter().map(|t| t.path.clone()).collect();
            let name = file_name(&folder);
            self.song_table_kind(ui, pal, &list, &name, "folders", false);
        }
    }

    fn folder_rows(&mut self, ui: &mut Ui, pal: &Palette, rows: &[(String, String, usize)]) {
        for (path, name, n) in rows {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
            }
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(20.0, 20.0)), Icon::Folder, pal.accent);
            let g = elided(ui, name, look::text(13.5), pal.text, rect.width() - 160.0);
            ui.painter().galley(pos2(rect.left() + 38.0, rect.center().y - g.size().y / 2.0), g, pal.text);
            ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, plural(*n, "song"), look::text(12.0), pal.secondary);
            if resp.clicked() {
                self.go(Page::Folders(Some(path.clone())));
            }
        }
    }

    // ------------------------------------------------------------ playlists ---

    pub fn playlists_page(&mut self, ui: &mut Ui, pal: &Palette) {
        page_title(ui, pal, "Playlists");
        if pill(ui, pal, Icon::Playlist, "New Playlist…").clicked() {
            self.ask_name("New Playlist", "New Playlist", NameAction::NewPlaylist(vec![]));
        }
        ui.add_space(14.0);
        if self.state.playlists.is_empty() {
            ui.label(
                RichText::new("You have no playlists yet. Make one here, or choose “Add to playlist” in any song's menu.")
                    .color(pal.secondary),
            );
            return;
        }
        let lists: Vec<(String, String, Vec<String>)> =
            self.state.playlists.iter().map(|p| (p.id.clone(), p.name.clone(), p.songs.clone())).collect();
        let gap = 22.0;
        let width = ui.available_width();
        let cols = ((width + gap) / (180.0 + gap)).floor().max(1.0) as usize;
        let size = ((width + gap) / cols as f32 - gap).min(220.0);
        egui::ScrollArea::vertical().id_salt("playlists").auto_shrink([false, false]).show(ui, |ui| {
            for row in lists.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (id, name, songs) in row {
                        let (rect, resp) = ui.allocate_exact_size(vec2(size, size + 44.0), Sense::click());
                        let crect = Rect::from_min_size(rect.min, vec2(size, size));
                        let p = ui.painter().clone();
                        self.mosaic(&p, crect, songs, name);
                        if resp.hovered() {
                            ui.painter().rect_filled(crect, CornerRadius::same(8), Color32::from_black_alpha(30));
                        }
                        let g = elided(ui, name, look::title(13.0), pal.text, size);
                        ui.painter().galley(pos2(rect.left(), crect.bottom() + 6.0), g, pal.text);
                        ui.painter().text(pos2(rect.left(), crect.bottom() + 24.0), Align2::LEFT_TOP, plural(songs.len(), "song"), look::text(12.0), pal.secondary);
                        if resp.clicked() {
                            self.go(Page::Playlist(id.clone()));
                        }
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
        });
    }

    pub fn playlist_page(&mut self, ui: &mut Ui, pal: &Palette, id: &str) {
        let Some(pl) = self.state.playlist(id).cloned() else {
            self.go(Page::Playlists);
            return;
        };
        if back_link(ui, pal, "Playlists").clicked() {
            self.go(Page::Playlists);
            return;
        }
        let songs = pl.songs.clone();
        ui.horizontal(|ui| {
            let (crect, _) = ui.allocate_exact_size(vec2(180.0, 180.0), Sense::hover());
            let p = ui.painter().clone();
            self.mosaic(&p, crect, &songs, &pl.name);
            ui.add_space(22.0);
            ui.vertical(|ui| {
                ui.add_space(60.0);
                ui.label(RichText::new(&pl.name).font(look::title(26.0)).color(pal.text));
                let tracks: Vec<&Track> = songs.iter().filter_map(|p| self.lib.track(p)).collect();
                ui.label(RichText::new(format!("{}, {}", plural(songs.len(), "song"), minutes_text(&tracks))).color(pal.secondary));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if !songs.is_empty() {
                        if pill(ui, pal, Icon::Play, "Play").clicked() {
                            self.play_list(songs.clone(), None, &pl.name);
                        }
                        if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                            self.shuffle_list(songs.clone(), &pl.name);
                        }
                    }
                    if ui.button("Rename…").clicked() {
                        self.ask_name("Rename Playlist", &pl.name, NameAction::RenamePlaylist(pl.id.clone()));
                    }
                    if ui.button("Delete…").clicked() {
                        self.confirm = Some(crate::player::ConfirmAction::DeletePlaylist(pl.id.clone()));
                    }
                });
            });
        });
        ui.add_space(16.0);
        if songs.is_empty() {
            ui.label(RichText::new("This playlist is empty. Add songs with “Add to playlist” in any song's menu.").color(pal.secondary));
            return;
        }
        self.table_playlist = Some(pl.id.clone());
        self.song_table(ui, pal, &songs, &pl.name, false);
    }

    // ------------------------------------------------------ recently played ---

    pub fn recently_played_page(&mut self, ui: &mut Ui, pal: &Palette) {
        page_title(ui, pal, "Recently Played");
        let rows = self.recently_played();
        if rows.is_empty() {
            ui.label(RichText::new("Songs you play appear here.").color(pal.secondary));
            return;
        }
        ui.horizontal(|ui| {
            let list: Vec<String> = rows.iter().map(|(p, _)| p.clone()).collect();
            if pill(ui, pal, Icon::Play, "Play").clicked() {
                self.play_list(list, None, "Recently Played");
            }
            if ui.button("Clear history…").clicked() {
                self.confirm = Some(crate::player::ConfirmAction::ClearHistory);
            }
        });
        ui.add_space(10.0);
        let list: Vec<String> = rows.iter().map(|(p, _)| p.clone()).collect();
        let played: HashMap<String, f64> = rows.into_iter().collect();
        self.song_table_ex(ui, pal, &list, "Recently Played", false, Some(&played));
    }

    // ------------------------------------------------------------ your stats ---

    fn period_menu(&mut self, ui: &mut Ui) {
        let mut period = self.stats_period.clone();
        egui::ComboBox::from_id_salt("stats-period").selected_text(period.clone()).width(150.0).show_ui(ui, |ui| {
            for p in PERIODS {
                ui.selectable_value(&mut period, p.to_string(), p);
            }
        });
        if period != self.stats_period {
            self.stats_period = period;
        }
    }

    pub fn stats_page(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Your Stats").font(look::title(28.0)).color(pal.text));
            ui.add_space(16.0);
            self.period_menu(ui);
        });
        ui.add_space(12.0);
        let st = self.stats();
        let cards = stats::cards(&st);
        if cards.is_empty() {
            ui.label(RichText::new("No listening yet in this period. Play some music and come back.").color(pal.secondary));
            return;
        }
        egui::ScrollArea::vertical().id_salt("stats").auto_shrink([false, false]).show(ui, |ui| {
            let gap = 14.0;
            let width = ui.available_width();
            let cols = ((width + gap) / (300.0 + gap)).floor().clamp(1.0, 3.0) as usize;
            let cw = (width + gap) / cols as f32 - gap;
            for (r, row) in cards.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (i, (label, big, detail)) in row.iter().enumerate() {
                        let k = r * cols + i;
                        let height = 140.0;
                        let (rect, _) = ui.allocate_exact_size(vec2(cw, height), Sense::hover());
                        let base = crate::style::hex(CARD_COLORS[k % CARD_COLORS.len()]);
                        ui.painter().rect_filled(rect, CornerRadius::same(14), base);
                        let p = ui.painter_at(rect);
                        p.text(rect.left_top() + vec2(18.0, 16.0), Align2::LEFT_TOP, label, look::title(12.5), Color32::from_white_alpha(220));
                        let g = elided(ui, big, look::title(if k == 0 { 40.0 } else { 26.0 }), Color32::WHITE, cw - 36.0);
                        p.galley(rect.left_top() + vec2(18.0, 38.0), g, Color32::WHITE);
                        let g = elided(ui, detail, look::text(13.0), Color32::from_white_alpha(230), cw - 36.0);
                        p.galley(pos2(rect.left() + 18.0, rect.bottom() - 30.0), g, Color32::WHITE);
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
            ui.add_space(8.0);
            if pill(ui, pal, Icon::Chart, "Details").clicked() {
                self.go(Page::StatsDetails);
            }
        });
    }

    pub fn stats_details_page(&mut self, ui: &mut Ui, pal: &Palette) {
        if back_link(ui, pal, "Your Stats").clicked() {
            self.go(Page::Stats);
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Details").font(look::title(28.0)).color(pal.text));
            ui.add_space(16.0);
            self.period_menu(ui);
        });
        ui.add_space(10.0);
        let st = self.stats();
        let lib = stats::library_stats(&self.lib.tracks);
        egui::ScrollArea::vertical().id_salt("stats-details").auto_shrink([false, false]).show(ui, |ui| {
            if st.logged == 0 {
                ui.label(RichText::new("No listening yet in this period. Play some music and come back.").color(pal.secondary));
            } else {
                section(ui, pal, "Totals");
                let totals = [
                    ("Minutes", stats::thousands((st.seconds / 60.0).round() as i64)),
                    ("Plays", stats::thousands(st.plays)),
                    ("Songs", stats::thousands(st.songs as i64)),
                    ("Artists", stats::thousands(st.artists as i64)),
                    ("Albums", stats::thousands(st.albums as i64)),
                    ("Genres", stats::thousands(st.genres as i64)),
                    ("Days with music", stats::thousands(st.days as i64)),
                    ("Longest streak", plural(st.streak, "day")),
                ];
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(10.0, 10.0);
                    for (k, v) in totals {
                        let (rect, _) = ui.allocate_exact_size(vec2(160.0, 62.0), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(10), pal.lcd);
                        ui.painter().text(rect.left_top() + vec2(14.0, 10.0), Align2::LEFT_TOP, k, look::text(12.0), pal.secondary);
                        ui.painter().text(rect.left_top() + vec2(14.0, 28.0), Align2::LEFT_TOP, v, look::title(20.0), pal.text);
                    }
                });
                section(ui, pal, "By hour of the day (minutes)");
                let hours: Vec<String> = (0..24).map(|h| if h % 3 == 0 { stats::hour_label(h) } else { String::new() }).collect();
                bars(ui, pal, &st.by_hour.map(|x| x / 60.0), &hours);
                section(ui, pal, "By day of the week (minutes)");
                let days: Vec<String> = stats::WEEKDAYS.iter().map(|d| d[..3].to_string()).collect();
                bars(ui, pal, &st.by_weekday.map(|x| x / 60.0), &days);
                if st.by_month.len() > 1 {
                    section(ui, pal, "By month (minutes)");
                    let vals: Vec<f64> = st.by_month.iter().map(|(_, s)| s / 60.0).collect();
                    let labels: Vec<String> = st
                        .by_month
                        .iter()
                        .map(|(m, _)| {
                            let (y, mo) = m.split_at(4);
                            let mo: usize = mo.trim_start_matches('-').parse().unwrap_or(1);
                            format!("{} {y}", stats::MONTHS[mo.clamp(1, 12) - 1])
                        })
                        .collect();
                    bars(ui, pal, &vals, &labels);
                }
                ui.add_space(8.0);
                ui.columns(2, |cols| {
                    top_list(&mut cols[0], pal, "Top songs", &st.top_songs, true);
                    top_list(&mut cols[1], pal, "Top artists", &st.top_artists, false);
                });
                ui.columns(2, |cols| {
                    top_list(&mut cols[0], pal, "Top albums", &st.top_albums, true);
                    top_list(&mut cols[1], pal, "Top genres", &st.top_genres, false);
                });
            }
            section(ui, pal, "Your library");
            let gb = lib.bytes as f64 / 1e9;
            ui.label(
                RichText::new(format!(
                    "{}, {} by {}; {} of music, {}.",
                    plural(lib.songs, "song"),
                    plural(lib.albums, "album"),
                    plural(lib.artists, "artist"),
                    {
                        let m = (lib.seconds / 60.0).round() as i64;
                        if m >= 60 { format!("{} hr {} min", stats::thousands(m / 60), m % 60) } else { format!("{m} min") }
                    },
                    if gb >= 1.0 { format!("{} GB", realhires::pyfmt::f(gb, 1)) } else { format!("{} MB", realhires::pyfmt::f(gb * 1000.0, 0)) }
                ))
                .color(pal.text),
            );
            ui.add_space(6.0);
            ui.columns(4, |cols| {
                kv_list(&mut cols[0], pal, "Quality", &lib.qualities.iter().map(|(k, n)| (k.clone(), *n)).collect::<Vec<_>>());
                kv_list(&mut cols[1], pal, "Formats", &lib.formats);
                kv_list(
                    &mut cols[2],
                    pal,
                    "Sample rates",
                    &lib.rates.iter().map(|(r, n)| (if *r > 0 { crate::eqwin::khz(*r) } else { "?".into() }, *n)).collect::<Vec<_>>(),
                );
                kv_list(&mut cols[3], pal, "Bit depths", &lib.bits);
            });
        });
    }
}

fn section(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(text).font(look::title(17.0)).color(pal.text));
    ui.add_space(6.0);
}

/// A bar chart with a label under every bar that has one.
fn bars(ui: &mut Ui, pal: &Palette, values: &[f64], labels: &[String]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(900.0), 130.0), Sense::hover());
    let p = ui.painter_at(rect);
    let max = values.iter().copied().fold(0.0f64, f64::max).max(1e-9);
    let n = values.len().max(1) as f32;
    let slot = rect.width() / n;
    let chart = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 18.0));
    p.line_segment([chart.left_bottom(), chart.right_bottom()], Stroke::new(0.5, pal.separator));
    for (i, v) in values.iter().enumerate() {
        let h = (v / max) as f32 * (chart.height() - 14.0);
        let x = rect.left() + slot * i as f32;
        let bar = Rect::from_min_max(pos2(x + slot * 0.18, chart.bottom() - h), pos2(x + slot * 0.82, chart.bottom()));
        p.rect_filled(bar, CornerRadius::same(3), pal.accent);
        if *v > 0.0 && slot > 26.0 {
            p.text(pos2(bar.center().x, bar.top() - 2.0), Align2::CENTER_BOTTOM, format!("{}", v.round() as i64), look::text(9.5), pal.secondary);
        }
        if let Some(l) = labels.get(i).filter(|l| !l.is_empty()) {
            p.text(pos2(x + slot / 2.0, rect.bottom() - 2.0), Align2::CENTER_BOTTOM, l, look::text(10.0), pal.secondary);
        }
    }
}

fn top_list(ui: &mut Ui, pal: &Palette, title: &str, rows: &[stats::Row], with_artist: bool) {
    section(ui, pal, title);
    for (i, r) in rows.iter().take(50).enumerate() {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.label(RichText::new(format!("{:>2}", i + 1)).color(pal.secondary).monospace());
                let mut text = r.name.clone();
                if with_artist && !r.artist.is_empty() {
                    text = format!("{text} — {}", r.artist);
                }
                let w = (ui.available_width() - 140.0).max(60.0);
                let g = elided(ui, &text, look::text(13.0), pal.text, w);
                ui.label(g);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} · {} min",
                            if r.plays == 1 { "1 play".to_string() } else { format!("{} plays", stats::thousands(r.plays)) },
                            stats::thousands((r.seconds / 60.0).round() as i64)
                        ))
                        .color(pal.secondary),
                    );
                });
            });
        });
    }
}

fn kv_list(ui: &mut Ui, pal: &Palette, title: &str, rows: &[(String, usize)]) {
    ui.label(RichText::new(title).font(look::title(13.5)).color(pal.text));
    for (k, n) in rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new(k).color(pal.text));
            ui.label(RichText::new(stats::thousands(*n as i64)).color(pal.secondary));
        });
    }
}
