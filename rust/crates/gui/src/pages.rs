//! The pages of the player window (2.0, `docs/player-spec.md`): album cards,
//! the album page, song tables, artists, genres, search, Now Playing with the
//! lyrics, and Settings with every function of the 1.x app.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Galley, Layout, Rect, RichText, Sense, Stroke, Ui};
use realhires::musiclib::{self, Album, Track};
use realhires::pyunicode::casefold;
use realhires::settings::{Settings, GROUPS};

use crate::icons::{self, Icon};
use crate::look::{self, Palette};
use crate::player::{cover, fmt_time, Page, Player, ShownLyrics, SongCol};

/// One line of text, cut with "…" to `width`.
pub fn elided(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping { max_width: width.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

fn page_title(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).font(look::title(28.0)).color(pal.text));
    ui.add_space(12.0);
}

/// A filled accent button with an icon (Play, Shuffle).
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

fn minutes(tracks: &[&Track]) -> String {
    let total: f64 = tracks.iter().map(|t| t.duration.max(0.0)).sum();
    let m = (total / 60.0).round() as i64;
    let songs = crate::style::plural(tracks.len(), "song");
    if m >= 60 {
        format!("{songs}, {} hr {} min", m / 60, m % 60)
    } else {
        format!("{songs}, {m} minute{}", if m == 1 { "" } else { "s" })
    }
}

fn matches(t: &Track, q: &str) -> bool {
    [t.display_title(), t.display_artist(), t.display_album(), t.genre.clone(), t.album_artist.clone()]
        .iter()
        .any(|f| casefold(f).contains(q))
}

impl Player {
    pub fn content(&mut self, ui: &mut Ui, pal: &Palette) {
        let page = self.page.clone();
        if self.lib.tracks.is_empty() && !matches!(page, Page::Settings | Page::NowPlaying | Page::Quality) {
            self.empty_library(ui, pal);
            return;
        }
        match page {
            Page::Recent => {
                page_title(ui, pal, "Recently Added");
                let list = self.lib.recent.clone();
                self.album_grid(ui, pal, &list);
            }
            Page::Albums => {
                page_title(ui, pal, "Albums");
                let list: Vec<usize> = (0..self.lib.albums.len()).collect();
                self.album_grid(ui, pal, &list);
            }
            Page::Album(key) => self.album_page(ui, pal, &key),
            Page::Artists => self.artists_page(ui, pal),
            Page::Songs => {
                page_title(ui, pal, "Songs");
                let list: Vec<String> = self.sorted_songs(self.lib.tracks.iter().collect()).iter().map(|t| t.path.clone()).collect();
                self.song_table(ui, pal, &list, "Songs", true);
            }
            Page::Genres => self.genres_page(ui, pal),
            Page::Liked => {
                page_title(ui, pal, "Liked Songs");
                let list: Vec<String> = self.sorted_songs(self.liked_songs()).iter().map(|t| t.path.clone()).collect();
                if list.is_empty() {
                    ui.label(RichText::new("Songs you like appear here. Click the heart next to a song, or in the playback bar while it plays.").color(pal.secondary));
                } else {
                    ui.horizontal(|ui| {
                        if pill(ui, pal, Icon::Play, "Play").clicked() {
                            self.play_list(list.clone(), Some(&list[0]), "Liked Songs");
                        }
                        if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                            self.shuffle_list(list.clone(), "Liked Songs");
                        }
                    });
                    ui.add_space(10.0);
                    self.song_table(ui, pal, &list, "Liked Songs", true);
                }
            }
            Page::Search => self.search_page(ui, pal),
            Page::NowPlaying => self.now_playing(ui, pal),
            Page::Settings => self.settings_page(ui, pal),
            Page::Quality => {}
        }
    }

    fn empty_library(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.add_space(ui.available_height() * 0.3);
        ui.vertical_centered(|ui| {
            icons::paint(ui.painter(), Rect::from_center_size(ui.cursor().center_top() + vec2(0.0, 30.0), vec2(64.0, 64.0)), Icon::Note, pal.secondary);
            ui.add_space(72.0);
            if let Some(s) = self.library_status() {
                ui.label(RichText::new(s).font(look::title(18.0)).color(pal.text));
                return;
            }
            let has_folders = !self.scanner.cfg.library_folders.is_empty();
            ui.label(RichText::new(if has_folders { "No songs found in your music folders" } else { "Add your music folder in Settings to start" }).font(look::title(18.0)).color(pal.text));
            ui.add_space(6.0);
            ui.label(RichText::new("Real-Hi-Res plays your music and checks whether its hi-res and lossless files are real.").color(pal.secondary));
            ui.add_space(14.0);
            if pill(ui, pal, Icon::Albums, "Choose a music folder…").clicked() {
                self.add_library_folder();
            }
        });
    }

    fn album_card(&mut self, ui: &mut Ui, pal: &Palette, album: usize, size: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size + 44.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let a = &self.lib.albums[album];
        let key = a.key.clone();
        let (title, artist) = (a.title.clone(), a.artist.clone());
        let paths: Vec<String> = a.tracks.iter().map(|t| t.path.clone()).collect();
        let tex = self.covers.get(&key, &paths);
        let crect = Rect::from_min_size(rect.min, vec2(size, size));
        cover(ui.painter(), crect, tex.as_ref(), &title, 8);
        if resp.hovered() {
            ui.painter().rect_filled(crect, CornerRadius::same(8), Color32::from_black_alpha(40));
            let play = Rect::from_center_size(crect.left_bottom() + vec2(24.0, -24.0), vec2(32.0, 32.0));
            ui.painter().circle_filled(play.center(), 16.0, pal.accent);
            icons::paint(ui.painter(), play.shrink(7.0), Icon::Play, Color32::WHITE);
        }
        let g = elided(ui, &title, look::title(13.0), pal.text, size);
        ui.painter().galley(pos2(rect.left(), crect.bottom() + 6.0), g, pal.text);
        let g = elided(ui, &artist, look::text(12.0), pal.secondary, size);
        ui.painter().galley(pos2(rect.left(), crect.bottom() + 24.0), g, pal.secondary);
        if resp.clicked() {
            let quick = resp.interact_pointer_pos().is_some_and(|p| (p - (crect.left_bottom() + vec2(24.0, -24.0))).length() < 17.0);
            if quick {
                self.play_list(paths, None, &title);
            } else {
                self.go(Page::Album(key));
            }
        }
    }

    fn album_grid(&mut self, ui: &mut Ui, pal: &Palette, albums: &[usize]) {
        let gap = 22.0;
        let width = ui.available_width();
        let cols = ((width + gap) / (180.0 + gap)).floor().max(1.0) as usize;
        let size = ((width + gap) / cols as f32 - gap).min(240.0);
        let rows = albums.len().div_ceil(cols);
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("albums", self.page.title())).show_rows(
            ui,
            size + 44.0 + gap,
            rows,
            |ui, range| {
                for r in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for &a in albums.iter().skip(r * cols).take(cols) {
                            self.album_card(ui, pal, a, size);
                        }
                    });
                    ui.add_space(gap - ui.spacing().item_spacing.y);
                }
            },
        );
    }

    fn album_page(&mut self, ui: &mut Ui, pal: &Palette, key: &str) {
        let Some(&idx) = self.lib.album_index.get(key) else {
            self.go(Page::Albums);
            return;
        };
        if !self.back.is_empty() && back_link(ui, pal, "Back").clicked() {
            self.go_back();
            return;
        }
        let a: Album = self.lib.albums[idx].clone();
        let paths: Vec<String> = a.tracks.iter().map(|t| t.path.clone()).collect();
        ui.horizontal(|ui| {
            let (crect, _) = ui.allocate_exact_size(vec2(220.0, 220.0), Sense::hover());
            let tex = self.covers.get(&a.key, &paths);
            cover(ui.painter(), crect, tex.as_ref(), &a.title, 10);
            ui.add_space(22.0);
            ui.vertical(|ui| {
                ui.add_space(70.0);
                ui.label(RichText::new(&a.title).font(look::title(26.0)).color(pal.text));
                ui.label(RichText::new(&a.artist).font(look::title(18.0)).color(pal.accent));
                let meta: Vec<&str> = [a.genre.as_str(), a.year.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                if !meta.is_empty() {
                    ui.label(RichText::new(meta.join(" · ")).color(pal.secondary));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(paths.clone(), Some(&paths[0]), &a.title);
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(paths.clone(), &a.title);
                    }
                });
            });
        });
        ui.add_space(18.0);
        let now = self.now.as_ref().map(|n| n.path.clone());
        let mut play = None;
        let mut like = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("album", key)).show(ui, |ui| {
            let multi_disc = a.tracks.iter().any(|t| t.disc > 1);
            let mut last_disc = 0;
            for t in &a.tracks {
                if multi_disc && t.disc != last_disc {
                    last_disc = t.disc;
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("Disc {}", t.disc.max(1))).font(look::title(13.0)).color(pal.secondary));
                }
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                let selected = self.selected_song.as_deref() == Some(t.path.as_str());
                let playing = now.as_deref() == Some(t.path.as_str());
                if selected {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover.gamma_multiply(0.6));
                }
                ui.painter().line_segment([rect.left_bottom() + vec2(36.0, 0.0), rect.right_bottom()], Stroke::new(0.5, pal.separator));
                let p = ui.painter_at(rect);
                if playing {
                    icons::paint(&p, Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0)), Icon::Speaker, pal.accent);
                } else {
                    let n = if t.track > 0 { t.track.to_string() } else { String::new() };
                    p.text(pos2(rect.left() + 26.0, rect.center().y), Align2::RIGHT_CENTER, n, look::text(13.0), pal.secondary);
                }
                let title_color = if playing { pal.accent } else { pal.text };
                let g = elided(ui, &t.display_title(), look::text(13.5), title_color, rect.width() - 200.0);
                p.galley(pos2(rect.left() + 40.0, rect.center().y - g.size().y / 2.0), g, title_color);
                if t.artist != a.artist && !t.artist.is_empty() {
                    // shown only when a track's artist differs from the album's
                }
                p.text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, fmt_time(t.duration), look::text(12.5), pal.secondary);
                let heart = Rect::from_center_size(pos2(rect.right() - 76.0, rect.center().y), vec2(18.0, 18.0));
                let liked = self.state.is_liked(&t.path);
                if liked || resp.hovered() {
                    icons::paint(&p, heart, if liked { Icon::HeartFilled } else { Icon::Heart }, pal.accent);
                }
                if resp.clicked() {
                    if resp.interact_pointer_pos().is_some_and(|q| heart.expand(4.0).contains(q)) {
                        like = Some(t.path.clone());
                    } else {
                        self.selected_song = Some(t.path.clone());
                    }
                }
                if resp.double_clicked() {
                    play = Some(t.path.clone());
                }
                let path = t.path.clone();
                resp.context_menu(|ui| self.song_menu(ui, &path));
            }
            ui.add_space(10.0);
            let refs: Vec<&Track> = a.tracks.iter().collect();
            ui.label(RichText::new(minutes(&refs)).color(pal.secondary));
        });
        if let Some(p) = like {
            self.toggle_like(&p);
        }
        if let Some(p) = play {
            self.play_list(paths, Some(&p), &a.title);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().egui_wants_keyboard_input() {
            if let Some(p) = self.selected_song.clone().filter(|p| a.tracks.iter().any(|t| t.path == *p)) {
                let paths: Vec<String> = a.tracks.iter().map(|t| t.path.clone()).collect();
                self.play_list(paths, Some(&p), &a.title);
            }
        }
    }

    /// The right-click menu of a song.
    fn song_menu(&mut self, ui: &mut Ui, path: &str) {
        let liked = self.state.is_liked(path);
        if ui.button(if liked { "Unlike" } else { "Like" }).clicked() {
            self.toggle_like(path);
            ui.close();
        }
        if let Some(a) = self.lib.album_of(path) {
            let key = a.key.clone();
            if ui.button("Go to the album").clicked() {
                self.go(Page::Album(key));
                ui.close();
            }
        }
        if ui.button("Get lyrics…").clicked() {
            self.scanner.download_lyrics(vec![path.to_string()]);
            ui.close();
        }
        if ui.button("Check its quality").clicked() {
            self.scanner.clear();
            self.scanner.add_source(path);
            self.scanner.start_scan(None, false);
            self.go(Page::Quality);
            ui.close();
        }
        if ui.button("Show in folder").clicked() {
            crate::style::reveal_in_file_manager(path);
            ui.close();
        }
    }

    /// A table of songs (Songs, Liked Songs, a genre, search results).
    pub fn song_table(&mut self, ui: &mut Ui, pal: &Palette, list: &[String], name: &str, sortable: bool) {
        let w = ui.available_width();
        let cols: [(SongCol, &str, f32); 6] = [
            (SongCol::Title, "Title", 0.30),
            (SongCol::Artist, "Artist", 0.20),
            (SongCol::Album, "Album", 0.22),
            (SongCol::Genre, "Genre", 0.11),
            (SongCol::Time, "Time", 0.06),
            (SongCol::Quality, "Quality", 0.11),
        ];
        let heart_w = 30.0;
        let usable = w - heart_w;
        let mut xs = vec![];
        let mut x = heart_w;
        for (_, _, f) in &cols {
            xs.push(x);
            x += usable * f;
        }
        // header
        let (hrect, _) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
        for (i, (col, label, f)) in cols.iter().enumerate() {
            let r = Rect::from_min_size(pos2(hrect.left() + xs[i], hrect.top()), vec2(usable * f, 26.0));
            let resp = ui.interact(r, ui.id().with(("song-head", *label)), if sortable { Sense::click() } else { Sense::hover() });
            let mut text = label.to_string();
            if sortable && self.song_sort.0 == *col {
                text += if self.song_sort.1 { "  ▼" } else { "  ▲" };
            }
            let color = if resp.hovered() { pal.text } else { pal.secondary };
            ui.painter().text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, text, look::title(11.5), color);
            if resp.clicked() {
                if self.song_sort.0 == *col {
                    self.song_sort.1 = !self.song_sort.1;
                } else {
                    self.song_sort = (*col, false);
                }
            }
        }
        ui.painter().line_segment([hrect.left_bottom(), hrect.right_bottom()], Stroke::new(0.5, pal.separator));
        let now = self.now.as_ref().map(|n| n.path.clone());
        let mut play = None;
        let mut like = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("songs", name)).show_rows(ui, 30.0, list.len(), |ui, range| {
            for i in range {
                let path = &list[i];
                let Some(t) = self.lib.track(path).cloned() else { continue };
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                let selected = self.selected_song.as_deref() == Some(path.as_str());
                let playing = now.as_deref() == Some(path.as_str());
                if selected {
                    ui.painter().rect_filled(rect, CornerRadius::same(5), pal.hover);
                } else if i % 2 == 1 {
                    ui.painter().rect_filled(rect, CornerRadius::same(5), if pal.dark { Color32::from_white_alpha(6) } else { Color32::from_black_alpha(6) });
                }
                let p = ui.painter_at(rect);
                let heart = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(16.0, 16.0));
                let liked = self.state.is_liked(path);
                if liked || resp.hovered() {
                    icons::paint(&p, heart, if liked { Icon::HeartFilled } else { Icon::Heart }, pal.accent);
                }
                let vals = [t.display_title(), t.display_artist(), t.display_album(), t.genre.clone(), fmt_time(t.duration), musiclib::quality(&t)];
                for (k, v) in vals.iter().enumerate() {
                    let color = if k == 0 && playing { pal.accent } else if k == 0 { pal.text } else { pal.secondary };
                    let font = if k == 0 { look::text(13.0) } else { look::text(12.5) };
                    let g = elided(ui, v, font, color, usable * cols[k].2 - 10.0);
                    p.galley(pos2(rect.left() + xs[k] + 4.0, rect.center().y - g.size().y / 2.0), g, color);
                }
                if resp.clicked() {
                    if resp.interact_pointer_pos().is_some_and(|q| heart.expand(5.0).contains(q)) {
                        like = Some(path.clone());
                    } else {
                        self.selected_song = Some(path.clone());
                    }
                }
                if resp.double_clicked() {
                    play = Some(path.clone());
                }
                let p2 = path.clone();
                resp.context_menu(|ui| self.song_menu(ui, &p2));
            }
        });
        if let Some(p) = like {
            self.toggle_like(&p);
        }
        if let Some(p) = play {
            self.play_list(list.to_vec(), Some(&p), name);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().egui_wants_keyboard_input() {
            if let Some(p) = self.selected_song.clone().filter(|p| list.contains(p)) {
                self.play_list(list.to_vec(), Some(&p), name);
            }
        }
    }

    fn side_list(&mut self, ui: &mut Ui, pal: &Palette, names: &[(String, usize)], selected: &Option<String>) -> Option<String> {
        let mut pick = None;
        egui::ScrollArea::vertical().id_salt(("side", self.page.title())).auto_shrink([false, false]).show(ui, |ui| {
            for (name, n) in names {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
                let sel = selected.as_deref() == Some(name.as_str());
                if sel {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.accent);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
                }
                let color = if sel { Color32::WHITE } else { pal.text };
                let g = elided(ui, name, look::text(13.0), color, rect.width() - 50.0);
                ui.painter().galley(pos2(rect.left() + 10.0, rect.center().y - g.size().y / 2.0), g, color);
                ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, n.to_string(), look::text(11.0), if sel { Color32::WHITE } else { pal.secondary });
                if resp.clicked() {
                    pick = Some(name.clone());
                }
            }
        });
        pick
    }

    fn artists_page(&mut self, ui: &mut Ui, pal: &Palette) {
        page_title(ui, pal, "Artists");
        let names: Vec<(String, usize)> = self.lib.artists.iter().map(|g| (g.name.clone(), g.tracks.len())).collect();
        if self.artist_sel.as_ref().is_none_or(|s| !names.iter().any(|(n, _)| n == s)) {
            self.artist_sel = names.first().map(|(n, _)| n.clone());
        }
        let sel = self.artist_sel.clone();
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(240.0, height), Layout::top_down(Align::Min), |ui| {
                if let Some(p) = self.side_list(ui, pal, &names, &sel) {
                    self.artist_sel = Some(p);
                }
            });
            ui.add_space(20.0);
            ui.vertical(|ui| {
                let Some(name) = sel else { return };
                let Some(g) = self.lib.artists.iter().find(|g| g.name == name) else { return };
                let paths: Vec<String> = g.tracks.iter().map(|t| t.path.clone()).collect();
                let keys: Vec<String> = {
                    let mut k: Vec<String> = vec![];
                    for t in &g.tracks {
                        let key = t.song().album_key();
                        if !k.contains(&key) {
                            k.push(key);
                        }
                    }
                    k
                };
                ui.label(RichText::new(&name).font(look::title(22.0)).color(pal.text));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(paths.clone(), Some(&paths[0]), &name);
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(paths.clone(), &name);
                    }
                });
                ui.add_space(14.0);
                let albums: Vec<usize> = keys.iter().filter_map(|k| self.lib.album_index.get(k).copied()).collect();
                self.album_grid(ui, pal, &albums);
            });
        });
    }

    fn genres_page(&mut self, ui: &mut Ui, pal: &Palette) {
        page_title(ui, pal, "Genres");
        let names: Vec<(String, usize)> = self.lib.genres.iter().map(|g| (g.name.clone(), g.tracks.len())).collect();
        if self.genre_sel.as_ref().is_none_or(|s| !names.iter().any(|(n, _)| n == s)) {
            self.genre_sel = names.first().map(|(n, _)| n.clone());
        }
        let sel = self.genre_sel.clone();
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(220.0, height), Layout::top_down(Align::Min), |ui| {
                if let Some(p) = self.side_list(ui, pal, &names, &sel) {
                    self.genre_sel = Some(p);
                }
            });
            ui.add_space(20.0);
            ui.vertical(|ui| {
                let Some(name) = sel else { return };
                let Some(g) = self.lib.genres.iter().find(|g| g.name == name) else { return };
                let list: Vec<String> = self.sorted_songs(g.tracks.iter().collect()).iter().map(|t| t.path.clone()).collect();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).font(look::title(22.0)).color(pal.text));
                    ui.add_space(14.0);
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(list.clone(), Some(&list[0]), &name);
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(list.clone(), &name);
                    }
                });
                ui.add_space(10.0);
                self.song_table(ui, pal, &list, &name, true);
            });
        });
    }

    fn search_page(&mut self, ui: &mut Ui, pal: &Palette) {
        let q = casefold(&self.search_query);
        page_title(ui, pal, &format!("Results for “{}”", self.search_query));
        let albums: Vec<usize> = self
            .lib
            .albums
            .iter()
            .enumerate()
            .filter(|(_, a)| casefold(&a.title).contains(&q) || casefold(&a.artist).contains(&q) || casefold(&a.genre).contains(&q))
            .map(|(i, _)| i)
            .take(60)
            .collect();
        let songs: Vec<String> = self.lib.tracks.iter().filter(|t| matches(t, &q)).take(2000).map(|t| t.path.clone()).collect();
        if albums.is_empty() && songs.is_empty() {
            ui.label(RichText::new("Nothing in the library matches.").color(pal.secondary));
            return;
        }
        if !albums.is_empty() {
            ui.label(RichText::new("Albums").font(look::title(17.0)).color(pal.text));
            ui.add_space(6.0);
            egui::ScrollArea::horizontal().id_salt("search-albums").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 18.0;
                    for a in &albums {
                        self.album_card(ui, pal, *a, 150.0);
                    }
                });
            });
            ui.add_space(14.0);
        }
        if !songs.is_empty() {
            ui.label(RichText::new("Songs").font(look::title(17.0)).color(pal.text));
            ui.add_space(4.0);
            let name = format!("Search: {}", self.search_query);
            self.song_table(ui, pal, &songs, &name, false);
        }
    }

    // --------------------------------------------------------- now playing ---

    fn now_playing(&mut self, ui: &mut Ui, pal: &Palette) {
        let Some(path) = self.now.as_ref().map(|n| n.path.clone()) else {
            ui.add_space(ui.available_height() * 0.35);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Nothing is playing").font(look::title(20.0)).color(pal.text));
                ui.label(RichText::new("Pick an album or a song, or press Play.").color(pal.secondary));
            });
            return;
        };
        let track = self.lib.track(&path).cloned().unwrap_or_else(|| Track { path: path.clone(), ..Default::default() });
        let key = track.song().album_key();
        let paths: Vec<String> = self.lib.album_of(&path).map(|a| a.tracks.iter().map(|t| t.path.clone()).collect()).unwrap_or_else(|| vec![path.clone()]);
        let tex = self.covers.get(&key, &paths);
        let lyrics = self.lyrics_of_now();
        let area = ui.available_rect_before_wrap();
        let show_lyrics = matches!(lyrics, Some(ShownLyrics::Synced(_)) | Some(ShownLyrics::Plain(_)));
        if !show_lyrics {
            // the cover in the centre, with the title under it
            let size = (area.height() - 150.0).min(area.width() - 40.0).clamp(120.0, 480.0);
            let crect = Rect::from_center_size(pos2(area.center().x, area.top() + 20.0 + size / 2.0), vec2(size, size));
            cover(ui.painter(), crect, tex.as_ref(), &track.display_album(), 12);
            let p = ui.painter();
            p.text(pos2(area.center().x, crect.bottom() + 26.0), Align2::CENTER_CENTER, track.display_title(), look::title(20.0), pal.text);
            p.text(pos2(area.center().x, crect.bottom() + 50.0), Align2::CENTER_CENTER, format!("{} — {}", track.display_artist(), track.display_album()), look::text(14.0), pal.secondary);
            let label_y = crect.bottom() + 70.0;
            ui.scope_builder(egui::UiBuilder::new().max_rect(Rect::from_center_size(pos2(area.center().x, label_y + 22.0), vec2(320.0, 40.0))), |ui| {
                ui.vertical_centered(|ui| {
                    if matches!(lyrics, Some(ShownLyrics::Instrumental)) {
                        ui.label(RichText::new("Instrumental").color(pal.secondary));
                    } else {
                        ui.label(RichText::new("No lyrics for this song.").color(pal.secondary));
                        if ui.button("Get lyrics").clicked() {
                            self.scanner.download_lyrics(vec![path.clone()]);
                        }
                    }
                });
            });
            return;
        }
        // cover on the left, lyrics on the right
        let half = area.width() / 2.0;
        let size = (half - 60.0).min(area.height() - 120.0).clamp(120.0, 480.0);
        let crect = Rect::from_center_size(pos2(area.left() + half / 2.0, area.top() + 10.0 + size / 2.0), vec2(size, size));
        cover(ui.painter(), crect, tex.as_ref(), &track.display_album(), 12);
        ui.painter().text(pos2(crect.center().x, crect.bottom() + 24.0), Align2::CENTER_CENTER, track.display_title(), look::title(18.0), pal.text);
        ui.painter().text(pos2(crect.center().x, crect.bottom() + 46.0), Align2::CENTER_CENTER, format!("{} — {}", track.display_artist(), track.display_album()), look::text(13.0), pal.secondary);
        let lrect = Rect::from_min_max(pos2(area.left() + half + 10.0, area.top()), area.max);
        let pos_ms = (self.engine.position().unwrap_or(0.0) * 1000.0) as u32;
        let mut seek_to = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(lrect), |ui| {
            if ui.rect_contains_pointer(lrect) && ui.input(|i| i.smooth_scroll_delta.y != 0.0) {
                self.lyrics_scrolled = Some(Instant::now());
            }
            let follow = self.lyrics_scrolled.is_none_or(|t| t.elapsed() > Duration::from_secs(3));
            egui::ScrollArea::vertical().id_salt("lyrics").auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(lrect.height() * 0.3);
                match &lyrics {
                    Some(ShownLyrics::Synced(lines)) => {
                        let cur = lines.iter().rposition(|(ms, _)| *ms <= pos_ms);
                        for (i, (ms, text)) in lines.iter().enumerate() {
                            let is_cur = Some(i) == cur;
                            let color = if is_cur { pal.text } else { pal.secondary.gamma_multiply(0.75) };
                            let shown = if text.trim().is_empty() { "…".to_string() } else { text.clone() };
                            let resp = ui.add(egui::Label::new(RichText::new(shown).font(look::title(24.0)).color(color)).wrap().sense(Sense::click()));
                            ui.add_space(10.0);
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.clicked() {
                                seek_to = Some(*ms as f64 / 1000.0);
                                self.lyrics_scrolled = None;
                            }
                            if is_cur && follow && self.lyrics_line != Some(i) {
                                resp.scroll_to_me_animation(Some(Align::Center), egui::style::ScrollAnimation::duration(0.35));
                            }
                        }
                        if follow {
                            self.lyrics_line = cur;
                        }
                    }
                    Some(ShownLyrics::Plain(text)) => {
                        ui.add(egui::Label::new(RichText::new(text).font(look::title(18.0)).color(pal.text)).wrap());
                    }
                    _ => {}
                }
                ui.add_space(lrect.height() * 0.5);
            });
        });
        if let Some(s) = seek_to {
            self.seek(s);
        }
    }

    // ------------------------------------------------------------ settings ---

    fn settings_page(&mut self, ui: &mut Ui, pal: &Palette) {
        page_title(ui, pal, "Settings");
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(180.0, height), Layout::top_down(Align::Min), |ui| {
                for (i, g) in GROUPS.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 30.0), Sense::click());
                    let sel = self.settings_group == i;
                    if sel {
                        ui.painter().rect_filled(rect, CornerRadius::same(6), pal.accent);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
                    }
                    ui.painter().text(pos2(rect.left() + 10.0, rect.center().y), Align2::LEFT_CENTER, *g, look::text(13.0), if sel { Color32::WHITE } else { pal.text });
                    if resp.clicked() {
                        self.settings_group = i;
                    }
                }
                ui.add_space(12.0);
                if ui.button("About Real-Hi-Res").clicked() {
                    self.scanner.show_about();
                }
            });
            ui.add_space(18.0);
            ui.vertical(|ui| {
                let group = GROUPS[self.settings_group.min(GROUPS.len() - 1)];
                ui.label(RichText::new(group).font(look::title(20.0)).color(pal.text));
                ui.add_space(8.0);
                egui::ScrollArea::vertical().id_salt("settings-page").auto_shrink([false, false]).show(ui, |ui| {
                    self.settings_actions(ui, pal, group);
                    let mut draft = self.settings_draft().clone();
                    let before = draft.clone();
                    let mut error = None;
                    egui::Grid::new(("settings-grid", group)).num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                        for info in Settings::field_info().into_iter().filter(|i| i.group == group) {
                            if let Some(e) = crate::dialogs::setting_row(ui, &info, &mut draft, &self.scanner.choices) {
                                error = Some(e);
                            }
                        }
                    });
                    if let Some(e) = error {
                        ui.label(RichText::new(e).color(pal.accent));
                    }
                    if draft != before {
                        *self.settings_draft() = draft;
                        self.settings_changed();
                    }
                    if group == "Appearance" && !self.font_used.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new(format!("Font in use: {}.", self.font_used)).color(pal.secondary));
                    }
                    ui.add_space(16.0);
                    if ui.button("Restore the defaults of this group").clicked() {
                        let defaults = Settings::default();
                        let draft = self.settings_draft();
                        for info in Settings::field_info().into_iter().filter(|i| i.group == group) {
                            if let Some(v) = defaults.get(info.name) {
                                let _ = draft.set(info.name, v);
                            }
                        }
                        self.settings_changed();
                    }
                });
            });
        });
    }

    /// The buttons above a settings group: what the 1.x app did from its menus.
    fn settings_actions(&mut self, ui: &mut Ui, pal: &Palette, group: &str) {
        let mut any = true;
        match group {
            "Library" => {
                ui.horizontal_wrapped(|ui| {
                    let busy = self.library_status().is_some();
                    if ui.add_enabled(!busy, egui::Button::new("Update the library now")).clicked() {
                        self.update_library();
                    }
                    if ui.button("Check the library for fake files").clicked() {
                        self.check_library();
                    }
                    if ui.button("Download lyrics for the whole library…").clicked() {
                        self.lyrics_for_library();
                    }
                    if ui.button("Open the quality checker").clicked() {
                        self.go(Page::Quality);
                    }
                });
                let n = self.lib.tracks.len();
                let text = match self.library_status() {
                    Some(s) => s,
                    None => format!("{} in {}.", crate::style::plural(n, "song"), crate::style::plural(self.lib.albums.len(), "album")),
                };
                ui.label(RichText::new(text).color(pal.secondary));
            }
            "Playback" => {
                ui.horizontal(|ui| {
                    if ui.button("Equalizer…").clicked() {
                        self.eq_open = true;
                    }
                    if ui.button("List the output devices again").clicked() {
                        self.fill_choices();
                    }
                });
            }
            "Lyrics" => {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Download lyrics for the whole library…").clicked() {
                        self.lyrics_for_library();
                    }
                });
            }
            "Saved results" => {
                match &self.scanner.store {
                    Some(st) => {
                        let s = st.stats();
                        ui.label(RichText::new(format!(
                            "{} saved for {} in {} ({} MB, of which graphs {} MB).",
                            crate::style::plural(s.results as usize, "result"),
                            crate::style::plural(s.files as usize, "file"),
                            s.path.display(),
                            realhires::pyfmt::f(s.disk_bytes as f64 / 1e6, 1),
                            realhires::pyfmt::f(s.graph_bytes as f64 / 1e6, 1)
                        )).color(pal.secondary));
                    }
                    None => {
                        ui.label(RichText::new("Saving results is turned off, or the saved results cannot be opened.").color(pal.secondary));
                    }
                }
                if ui.button("Delete all saved results…").clicked() {
                    self.scanner.forget_saved_ask();
                }
            }
            "Detection" | "Scanning" | "Display" | "Fixing" => {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Open the quality checker").clicked() {
                        self.go(Page::Quality);
                    }
                });
            }
            _ => any = false,
        }
        if any {
            ui.add_space(12.0);
        }
    }
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
