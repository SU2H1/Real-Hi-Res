//! The pages of the player window (2.0, `docs/player-spec.md`): album cards,
//! the album page, song tables, artists, genres, search, Now Playing with the
//! lyrics, and Settings with every function of the 1.x app.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Galley, Layout, Rect, RichText, Sense, Stroke, Ui};
use realhires::musiclib::{self, Album, Track};
use realhires::pyunicode::casefold;
use realhires::settings::{Settings, GROUPS};

use crate::icons::{self, Icon};
use crate::look::{self, Palette};
use crate::player::{cell_text, cover, Page, Player, ShownLyrics, SongCol};

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
        let needs_songs = !matches!(
            page,
            Page::Settings | Page::NowPlaying | Page::Quality | Page::Library | Page::Stats | Page::StatsDetails
                | Page::Playlists | Page::Playlist(_) | Page::RecentlyPlayed
        );
        if self.lib.tracks.is_empty() && needs_songs {
            self.empty_library(ui, pal);
            return;
        }
        self.table_playlist = None;
        match page {
            Page::Library => self.library_home(ui, pal),
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
            Page::Artists => self.group_grid(ui, pal, "artists"),
            Page::AlbumArtists => self.group_grid(ui, pal, "album_artists"),
            Page::Genres => self.group_grid(ui, pal, "genres"),
            Page::Years => self.group_grid(ui, pal, "years"),
            Page::Composers => self.group_grid(ui, pal, "composers"),
            Page::Group(kind, name) => self.group_item_page(ui, pal, kind, &name),
            Page::AllSongs => {
                page_title(ui, pal, "All Songs");
                // sorted once, not every frame: that made scrolling a big library lag
                let sort = Some(self.song_sort);
                let list = self.cached_list("all", sort, |p| p.sorted_songs(p.lib.tracks.iter().collect()).iter().map(|t| t.path.clone()).collect());
                ui.horizontal(|ui| {
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(list.to_vec(), Some(&list[0]), "All Songs");
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(list.to_vec(), "All Songs");
                    }
                });
                ui.add_space(10.0);
                self.song_table(ui, pal, &list, "All Songs", true);
            }
            Page::Folders(folder) => self.folders_page(ui, pal, folder),
            Page::Playlists => self.playlists_page(ui, pal),
            Page::Playlist(id) => self.playlist_page(ui, pal, &id),
            Page::Liked | Page::Disliked => {
                let liked = page == Page::Liked;
                let title = if liked { "Liked Songs" } else { "Disliked Songs" };
                page_title(ui, pal, title);
                let sort = Some(self.song_sort);
                let list = self.cached_list(if liked { "liked" } else { "disliked" }, sort, |p| {
                    let songs: Vec<&Track> = if liked {
                        p.liked_songs()
                    } else {
                        p.lib.tracks.iter().filter(|t| p.state.is_disliked(&t.path)).collect()
                    };
                    p.sorted_songs(songs).iter().map(|t| t.path.clone()).collect()
                });
                if list.is_empty() {
                    let hint = if liked {
                        "Songs you like appear here. Click the heart next to a song, or in the playback bar while it plays."
                    } else {
                        "Songs you dislike appear here, and shuffling leaves them out. Dislike a song with the thumbs-down button in the playback bar or from a song's menu."
                    };
                    ui.label(RichText::new(hint).color(pal.secondary));
                } else {
                    ui.horizontal(|ui| {
                        if pill(ui, pal, Icon::Play, "Play").clicked() {
                            self.play_list(list.to_vec(), Some(&list[0]), title);
                        }
                        if liked && pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                            self.shuffle_list(list.to_vec(), title);
                        }
                    });
                    ui.add_space(10.0);
                    self.song_table(ui, pal, &list, title, true);
                }
            }
            Page::RecentlyPlayed => self.recently_played_page(ui, pal),
            Page::Stats => self.stats_page(ui, pal),
            Page::StatsDetails => self.stats_details_page(ui, pal),
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
            ui.label(RichText::new("Universal Music Player plays your music and checks whether its hi-res and lossless files are real.").color(pal.secondary));
            ui.add_space(14.0);
            if pill(ui, pal, Icon::Albums, "Choose a music folder…").clicked() {
                self.add_library_folder();
            }
        });
    }

    pub fn album_card(&mut self, ui: &mut Ui, pal: &Palette, album: usize, size: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size + 44.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let a = &self.lib.albums[album];
        let key = a.key.clone();
        let (title, artist) = (a.title.clone(), a.artist.clone());
        let tex = self.covers.get(&key, &self.lib.album_paths[album]);
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
                let paths: Vec<String> = self.lib.albums[album].tracks.iter().map(|t| t.path.clone()).collect();
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
        if self.album_sort_for() != key {
            // another album: in track order again
            self.set_album_sort_for(key);
        }
        ui.horizontal(|ui| {
            let (crect, _) = ui.allocate_exact_size(vec2(220.0, 220.0), Sense::hover());
            let tex = self.covers.get(&a.key, &self.lib.album_paths[idx]);
            cover(ui.painter(), crect, tex.as_ref(), &a.title, 10);
            ui.add_space(22.0);
            ui.vertical(|ui| {
                ui.add_space(70.0);
                ui.label(RichText::new(&a.title).font(look::title(26.0)).color(pal.text));
                // each artist of the field its own link to its page
                let names = {
                    let v = musiclib::split_artists(&a.artist, &self.lib.known);
                    if v.is_empty() && !a.artist.is_empty() { vec![a.artist.clone()] } else { v }
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (i, n) in names.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new(", ").font(look::title(18.0)).color(pal.secondary));
                        }
                        let r = ui.add(egui::Label::new(RichText::new(n).font(look::title(18.0)).color(pal.accent)).sense(Sense::click()));
                        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.hovered() {
                            ui.painter().line_segment([r.rect.left_bottom(), r.rect.right_bottom()], Stroke::new(1.0, pal.accent));
                        }
                        if r.clicked() {
                            self.open_artist(n);
                        }
                    }
                });
                let refs: Vec<&Track> = a.tracks.iter().collect();
                let meta: Vec<String> = [a.genre.clone(), a.year.clone(), minutes(&refs)].into_iter().filter(|s| !s.is_empty()).collect();
                ui.label(RichText::new(meta.join(" · ")).color(pal.secondary));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(paths.clone(), Some(&paths[0]), &a.title);
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(paths.clone(), &a.title);
                    }
                    ui.add_space(6.0);
                    if ui
                        .add_enabled(self.edit_info.is_none(), egui::Button::new("Edit album info…"))
                        .on_hover_text("Change the album, album artist, genre, release date… of all its songs")
                        .clicked()
                    {
                        self.edit_album_info(&a);
                    }
                });
            });
        });
        ui.add_space(18.0);
        // the columns the user chose for album pages (# first by default), in track order
        // until a heading is clicked
        let sort = self.album_sort;
        let list = self.cached_list(&format!("album:{key}"), sort, |_| {
            crate::player::sort_tracks(a.tracks.iter().collect(), sort).iter().map(|t| t.path.clone()).collect()
        });
        self.song_table_kind(ui, pal, &list, &a.title, "album", true);
    }

    /// The right-click menu of a song.
    pub fn song_menu(&mut self, ui: &mut Ui, path: &str) {
        let liked = self.state.is_liked(path);
        if ui.button(if liked { "Unlike" } else { "Like" }).clicked() {
            self.toggle_like(path);
            ui.close();
        }
        let disliked = self.state.is_disliked(path);
        if ui.button(if disliked { "Remove dislike" } else { "Dislike" }).clicked() {
            self.toggle_dislike(path);
            ui.close();
        }
        ui.menu_button("Add to playlist", |ui| {
            let lists: Vec<(String, String)> = self.state.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
            for (id, name) in lists {
                if ui.button(name).clicked() {
                    self.add_to_playlist(&id, &[path.to_string()]);
                    ui.close();
                }
            }
            if !self.state.playlists.is_empty() {
                ui.separator();
            }
            if ui.button("New Playlist…").clicked() {
                self.ask_name("New Playlist", "New Playlist", crate::player::NameAction::NewPlaylist(vec![path.to_string()]));
                ui.close();
            }
        });
        if let Some(id) = self.table_playlist.clone() {
            ui.separator();
            if ui.button("Remove from this playlist").clicked() {
                self.state.remove_from_playlist(&id, &[path.to_string()]);
                self.save_state();
                ui.close();
            }
            if ui.button("Move up").clicked() {
                self.state.move_in_playlist(&id, path, -1);
                self.save_state();
                ui.close();
            }
            if ui.button("Move down").clicked() {
                self.state.move_in_playlist(&id, path, 1);
                self.save_state();
                ui.close();
            }
            ui.separator();
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
        if ui.add_enabled(self.edit_info.is_none(), egui::Button::new("Edit info…")).clicked() {
            self.edit_song_info(path);
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

    /// A table of songs (Songs, Liked Songs, a genre, search results) with the columns
    /// the user chose for song lists.
    pub fn song_table(&mut self, ui: &mut Ui, pal: &Palette, list: &[String], name: &str, sortable: bool) {
        self.song_table_kind(ui, pal, list, name, "songs", sortable);
    }

    /// A table with the columns the user chose for this kind of list ("songs",
    /// "folders", "album"); a right click on its headings chooses them.
    pub fn song_table_kind(&mut self, ui: &mut Ui, pal: &Palette, list: &[String], name: &str, kind: &'static str, sortable: bool) {
        let cols: Vec<SongCol> = self.state.columns_of(kind).iter().filter_map(|c| SongCol::from_name(c)).collect();
        self.table(ui, pal, list, name, cols, Some(kind), sortable, None);
    }

    /// Recently Played: its own columns, with "Played" instead of Genre.
    pub fn song_table_ex(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        list: &[String],
        name: &str,
        sortable: bool,
        played: Option<&HashMap<String, f64>>,
    ) {
        let cols = vec![SongCol::Like, SongCol::Title, SongCol::Artist, SongCol::Album, SongCol::Played, SongCol::Time, SongCol::Quality];
        self.table(ui, pal, list, name, cols, None, sortable && played.is_none(), played);
    }

    /// The heading's menu: a check item per column (Title stays) and Reset columns.
    fn columns_menu(&mut self, ui: &mut Ui, kind: &'static str) {
        let shown = self.state.columns_of(kind);
        let mut changed = false;
        for name in realhires::playerstate::SONG_COLUMNS {
            let Some(col) = SongCol::from_name(name) else { continue };
            let mut on = shown.iter().any(|c| c == name);
            let r = ui.add_enabled(name != "title", egui::Checkbox::new(&mut on, col.info().1));
            if r.changed() {
                self.state.toggle_column(kind, name);
                changed = true;
            }
        }
        ui.separator();
        if ui.button("Reset columns").clicked() {
            self.state.reset_columns(kind);
            changed = true;
            ui.close();
        }
        if changed {
            self.save_state();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn table(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        list: &[String],
        name: &str,
        cols: Vec<SongCol>,
        kind: Option<&'static str>,
        sortable: bool,
        played: Option<&HashMap<String, f64>>,
    ) {
        let w = ui.available_width();
        let heart = cols.contains(&SongCol::Like);
        let heart_w = if heart { 30.0 } else { 6.0 };
        let shown: Vec<SongCol> = cols.iter().copied().filter(|c| *c != SongCol::Like).collect();
        let total: f32 = shown.iter().map(|c| c.info().2).sum::<f32>().max(0.1);
        let usable = w - heart_w;
        let mut xs = vec![];
        let mut ws = vec![];
        let mut x = heart_w;
        for c in &shown {
            xs.push(x);
            let cw = usable * c.info().2 / total;
            ws.push(cw);
            x += cw;
        }
        let sort = kind.map(|k| self.sort_of(k)).unwrap_or(Some(self.song_sort));
        // header
        let (hrect, hresp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
        let mut menu_open = false;
        for (i, col) in shown.iter().enumerate() {
            let r = Rect::from_min_size(pos2(hrect.left() + xs[i], hrect.top()), vec2(ws[i], 26.0));
            let can_sort = sortable && *col != SongCol::Played;
            let resp = ui.interact(r, ui.id().with(("song-head", name, i)), if can_sort { Sense::click() } else { Sense::hover() });
            let mut text = col.info().0.to_string();
            if can_sort && sort.is_some_and(|(c, _)| c == *col) {
                text += if sort.is_some_and(|(_, rev)| rev) { "  ▼" } else { "  ▲" };
            }
            let color = if resp.hovered() && can_sort { pal.text } else { pal.secondary };
            let right = col.info().3;
            let (px, align) = if right { (r.right() - 6.0, Align2::RIGHT_CENTER) } else { (r.left() + 4.0, Align2::LEFT_CENTER) };
            ui.painter().text(pos2(px, r.center().y), align, text, look::title(11.5), color);
            if resp.clicked() && can_sort {
                match kind {
                    Some(k) => self.set_sort(k, *col),
                    None => {
                        self.song_sort = if self.song_sort.0 == *col { (*col, !self.song_sort.1) } else { (*col, false) };
                    }
                }
            }
            if let Some(k) = kind {
                resp.context_menu(|ui| {
                    menu_open = true;
                    self.columns_menu(ui, k);
                });
            }
        }
        if let Some(k) = kind {
            hresp.context_menu(|ui| {
                menu_open = true;
                self.columns_menu(ui, k);
            });
        }
        let _ = menu_open;
        ui.painter().line_segment([hrect.left_bottom(), hrect.right_bottom()], Stroke::new(0.5, pal.separator));
        let now = self.now.as_ref().map(|n| n.path.clone());
        let mut play = None;
        let mut like = None;
        let qcol = shown.iter().position(|c| *c == SongCol::Quality);
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("songs", name)).show_rows(ui, 30.0, list.len(), |ui, range| {
            for i in range {
                let path = &list[i];
                let Some(t) = self.lib.track(path) else {
                    // a playlist song whose file is gone
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                    let p = ui.painter_at(rect);
                    let g = elided(ui, &realhires::paths::file_name(path), look::text(13.0), pal.secondary.gamma_multiply(0.7), usable * 0.5);
                    p.galley(pos2(rect.left() + heart_w + 4.0, rect.center().y - g.size().y / 2.0), g, pal.secondary);
                    p.text(pos2(rect.left() + heart_w + usable * 0.55, rect.center().y), Align2::LEFT_CENTER, "Not in the library", look::text(12.0), pal.secondary.gamma_multiply(0.7));
                    let p2 = path.clone();
                    resp.context_menu(|ui| self.song_menu(ui, &p2));
                    continue;
                };
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                let selected = self.selected_song.as_deref() == Some(path.as_str());
                let playing = now.as_deref() == Some(path.as_str());
                if selected {
                    ui.painter().rect_filled(rect, CornerRadius::same(5), pal.hover);
                } else if i % 2 == 1 {
                    ui.painter().rect_filled(rect, CornerRadius::same(5), if pal.dark { Color32::from_white_alpha(6) } else { Color32::from_black_alpha(6) });
                }
                let p = ui.painter_at(rect);
                let heart_rect = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(16.0, 16.0));
                let liked = self.state.is_liked(path);
                if heart && (liked || resp.hovered()) {
                    icons::paint(&p, heart_rect, if liked { Icon::HeartFilled } else { Icon::Heart }, pal.accent);
                }
                for (k, col) in shown.iter().enumerate() {
                    let v = match col {
                        SongCol::Played => played_label(played.and_then(|m| m.get(path).copied()).unwrap_or(0.0)),
                        SongCol::Num if playing => {
                            icons::paint(&p, Rect::from_center_size(pos2(rect.left() + xs[k] + ws[k] - 12.0, rect.center().y), vec2(15.0, 15.0)), Icon::Speaker, pal.accent);
                            continue;
                        }
                        c => cell_text(t, *c),
                    };
                    let first = *col == SongCol::Title;
                    let color = if first && playing { pal.accent } else if first { pal.text } else { pal.secondary };
                    let font = if first { look::text(13.0) } else { look::text(12.5) };
                    let g = elided(ui, &v, font, color, ws[k] - 10.0);
                    let x = if col.info().3 { rect.left() + xs[k] + ws[k] - 6.0 - g.size().x } else { rect.left() + xs[k] + 4.0 };
                    p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, color);
                }
                let mut resp = resp;
                if let Some(q) = qcol {
                    let cell = Rect::from_min_size(pos2(rect.left() + xs[q], rect.top()), vec2(ws[q], rect.height()));
                    if ui.rect_contains_pointer(cell) {
                        // the Quality cell's tooltip: the resolution ("24-bit/96 kHz", "MP3 · 320 kbps · 44.1 kHz")
                        resp = resp.on_hover_text_at_pointer(musiclib::quality_detail(t));
                    }
                }
                if resp.clicked() {
                    if heart && resp.interact_pointer_pos().is_some_and(|q| heart_rect.expand(5.0).contains(q)) {
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

    // ------------------------------------------------------ grouping pages ---

    /// Artists, Album Artists, Genres, Years and Composers: a card each, like the albums
    /// (a mosaic of up to four of its album covers, the name, "N albums · M songs"); a
    /// click opens its page.  On Genres, Ctrl/Cmd-click selects several to merge.
    fn group_grid(&mut self, ui: &mut Ui, pal: &Palette, kind: &'static str) {
        let (_, title, noun) = crate::player::GROUPINGS.iter().copied().find(|g| g.0 == kind).unwrap_or(crate::player::GROUPINGS[0]);
        let n = self.lib.groups(kind).len();
        let selected = if kind == "genres" { self.group_selected.len() } else { 0 };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(title).font(look::title(28.0)).color(pal.text));
                let mut sub = crate::style::plural(n, noun);
                if kind == "genres" {
                    let m = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
                    sub += &if selected > 0 { format!(" · {selected} selected") } else { format!(" · {m}-click genres to select several and merge them") };
                }
                ui.label(RichText::new(sub).color(pal.secondary));
            });
            if selected >= 2 {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Clear selection").clicked() {
                        self.group_selected.clear();
                    }
                    if pill(ui, pal, Icon::Genres, &format!("Merge {selected} genres…")).clicked() {
                        let names = self.group_selected.clone();
                        self.ask_merge_groups(kind, names);
                    }
                });
            }
        });
        ui.add_space(12.0);
        let gap = 22.0;
        let width = ui.available_width();
        let cols = ((width + gap) / (180.0 + gap)).floor().max(1.0) as usize;
        let size = ((width + gap) / cols as f32 - gap).min(240.0);
        let rows = n.div_ceil(cols);
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("groups", kind)).show_rows(ui, size + 44.0 + gap, rows, |ui, range| {
            for r in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for i in (r * cols)..((r + 1) * cols).min(n) {
                        self.group_card(ui, pal, kind, i, size);
                    }
                });
                ui.add_space(gap - ui.spacing().item_spacing.y);
            }
        });
    }

    /// Up to four album covers, 2 x 2 (fewer repeat), or the one cover there is.
    fn group_mosaic(&mut self, p: &egui::Painter, rect: Rect, albums: &[usize], name: &str) {
        let shown: Vec<usize> = albums.iter().take(4).copied().collect();
        if shown.len() <= 1 {
            let tex = shown.first().and_then(|&a| self.covers.get(&self.lib.albums[a].key, &self.lib.album_paths[a]));
            cover(p, rect, tex.as_ref(), name, 8);
            return;
        }
        let half = rect.width() / 2.0;
        for q in 0..4 {
            let a = shown[q % shown.len()];
            let r = Rect::from_min_size(rect.min + vec2((q % 2) as f32 * half, (q / 2) as f32 * half), vec2(half, half));
            let tex = self.covers.get(&self.lib.albums[a].key, &self.lib.album_paths[a]);
            let radius = CornerRadius {
                nw: if q == 0 { 8 } else { 0 },
                ne: if q == 1 { 8 } else { 0 },
                sw: if q == 2 { 8 } else { 0 },
                se: if q == 3 { 8 } else { 0 },
            };
            match tex {
                Some(t) => {
                    p.add(egui::epaint::RectShape::filled(r, radius, Color32::WHITE).with_texture(t.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))));
                }
                None => {
                    p.rect_filled(r, radius, crate::style::placeholder_color(&self.lib.albums[a].title));
                }
            }
        }
    }

    fn group_card(&mut self, ui: &mut Ui, pal: &Palette, kind: &'static str, i: usize, size: f32) {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size + 44.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let name = self.lib.groups(kind)[i].name.clone();
        let songs = self.lib.groups(kind)[i].tracks.len();
        let albums = self.lib.group_albums.get(kind).and_then(|v| v.get(i)).cloned().unwrap_or_default();
        let crect = Rect::from_min_size(rect.min, vec2(size, size));
        self.group_mosaic(ui.painter(), crect, &albums, &name);
        let is_sel = kind == "genres" && self.group_selected.iter().any(|s| *s == name);
        if is_sel {
            ui.painter().rect_stroke(crect.expand(3.0), CornerRadius::same(10), Stroke::new(3.0, pal.accent), egui::StrokeKind::Inside);
            let c = crect.right_top() + vec2(-20.0, 20.0);
            ui.painter().circle_filled(c, 13.0, pal.accent);
            icons::paint(ui.painter(), Rect::from_center_size(c, vec2(16.0, 16.0)), Icon::Check, Color32::WHITE);
        } else if resp.hovered() {
            ui.painter().rect_filled(crect, CornerRadius::same(8), Color32::from_black_alpha(30));
        }
        let g = elided(ui, &name, look::title(13.0), pal.text, size);
        ui.painter().galley(pos2(rect.left(), crect.bottom() + 6.0), g, pal.text);
        let sub = format!("{} · {}", crate::style::plural(albums.len(), "album"), crate::style::plural(songs, "song"));
        let g = elided(ui, &sub, look::text(12.0), pal.secondary, size);
        ui.painter().galley(pos2(rect.left(), crect.bottom() + 24.0), g, pal.secondary);
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            let toggle = kind == "genres" && ui.input(|i| i.modifiers.command);
            if toggle {
                if is_sel {
                    self.group_selected.retain(|s| *s != name);
                } else {
                    self.group_selected.push(name.clone());
                }
            } else {
                self.group_selected.clear();
                self.go(Page::Group(kind, name.clone()));
            }
        }
        resp.context_menu(|ui| {
            if is_sel && self.group_selected.len() > 1 {
                let k = self.group_selected.len();
                if ui.button(format!("Merge {k} genres…")).clicked() {
                    let names = self.group_selected.clone();
                    self.ask_merge_groups(kind, names);
                    ui.close();
                }
                if ui.button("Clear selection").clicked() {
                    self.group_selected.clear();
                    ui.close();
                }
            } else {
                self.group_menu(ui, kind, &name, false);
            }
        });
    }

    /// The menu of a genre, year, composer or artist: Open, Play, Shuffle, and for genres,
    /// composers and years Rename… (genres also Merge into…).
    pub fn group_menu(&mut self, ui: &mut Ui, kind: &'static str, name: &str, here: bool) {
        let songs = |p: &Player| -> Vec<String> {
            p.lib.find_group(kind, name).map(|i| p.lib.groups(kind)[i].tracks.iter().map(|t| t.path.clone()).collect()).unwrap_or_default()
        };
        if !here && ui.button("Open").clicked() {
            self.go(Page::Group(kind, name.to_string()));
            ui.close();
        }
        if ui.button("Play").clicked() {
            let list = songs(self);
            if !list.is_empty() {
                self.play_list(list.clone(), Some(&list[0]), name);
            }
            ui.close();
        }
        if ui.button("Shuffle").clicked() {
            let list = songs(self);
            if !list.is_empty() {
                self.shuffle_list(list, name);
            }
            ui.close();
        }
        if matches!(kind, "genres" | "composers" | "years") {
            ui.separator();
            let noun = realhires::tagedit::group_noun(kind);
            if ui.add_enabled(self.tag_job.is_none(), egui::Button::new(format!("Rename {noun}…"))).clicked() {
                self.ask_rename_group(kind, name);
                ui.close();
            }
            if kind == "genres" && ui.add_enabled(self.tag_job.is_none(), egui::Button::new("Merge into…")).clicked() {
                self.ask_merge_groups(kind, vec![name.to_string()]);
                ui.close();
            }
        }
    }

    /// One artist, album artist, genre, year or composer: its albums (artists) or songs,
    /// Play, Shuffle and "…" (its menu).
    fn group_item_page(&mut self, ui: &mut Ui, pal: &Palette, kind: &'static str, name: &str) {
        if back_link(ui, pal, "Back").clicked() {
            self.go_back();
            return;
        }
        let noun = crate::player::GROUPINGS.iter().find(|g| g.0 == kind).map(|g| g.2).unwrap_or("artist");
        let Some(gi) = self.lib.find_group(kind, name) else {
            page_title(ui, pal, name);
            ui.label(RichText::new(format!("No songs have this {noun} any more.")).color(pal.secondary));
            return;
        };
        let gname = self.lib.groups(kind)[gi].name.clone();
        let albums = self.lib.group_albums.get(kind).and_then(|v| v.get(gi)).cloned().unwrap_or_default();
        let songs = self.lib.groups(kind)[gi].tracks.len();
        let cards = matches!(kind, "artists" | "album_artists");
        let id = format!("group:{kind}:{gname}");
        let sort = Some(self.song_sort);
        let list = self.cached_list(&id, if cards { None } else { sort }, |p| {
            let g = &p.lib.groups(kind)[gi];
            if cards {
                // album order, as the albums are shown
                let mut v: Vec<&Track> = g.tracks.iter().collect();
                v.sort_by_cached_key(|t| (p.lib.album_index.get(&t.song().album_key()).copied().unwrap_or(usize::MAX), t.disc, t.track));
                v.iter().map(|t| t.path.clone()).collect()
            } else {
                crate::player::sort_tracks(g.tracks.iter().collect(), sort).iter().map(|t| t.path.clone()).collect()
            }
        });
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(&gname).font(look::title(28.0)).color(pal.text));
                let mut cap = noun.to_string();
                if let Some(f) = cap.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                let sub = if cards {
                    format!("{cap} · {}, {}", crate::style::plural(albums.len(), "album"), crate::style::plural(songs, "song"))
                } else {
                    format!("{cap} · {}", crate::style::plural(songs, "song"))
                };
                ui.label(RichText::new(sub).color(pal.secondary));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if matches!(kind, "genres" | "composers" | "years") {
                    ui.menu_button("…", |ui| self.group_menu(ui, kind, &gname, true));
                }
                if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                    self.shuffle_list(list.to_vec(), &gname);
                }
                if pill(ui, pal, Icon::Play, "Play").clicked() && !list.is_empty() {
                    self.play_list(list.to_vec(), Some(&list[0]), &gname);
                }
            });
        });
        ui.add_space(12.0);
        if cards {
            self.album_grid(ui, pal, &albums);
        } else {
            self.song_table(ui, pal, &list, &gname, true);
        }
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
                if ui.button(format!("About {}", realhires::APP_NAME)).clicked() {
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
                    if group == "Appearance" {
                        if !self.font_used.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new(format!("Font in use: {}.", self.font_used)).color(pal.secondary));
                        }
                        ui.add_space(14.0);
                        ui.label(RichText::new("Themes").font(look::title(15.0)).color(pal.text));
                        ui.add_space(4.0);
                        self.theme_editor(ui, pal);
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
                    if ui.button("Scan library").on_hover_text("Check the library folders for fake hi-res and fake lossless files in the quality checker").clicked() {
                        self.check_library();
                    }
                    if ui.button("Download lyrics for the whole library…").clicked() {
                        self.lyrics_for_library();
                    }
                    if ui.button("Open the quality checker").clicked() {
                        self.go(Page::Quality);
                    }
                    if ui.button("Clear the play history…").clicked() {
                        self.confirm = Some(crate::player::ConfirmAction::ClearHistory);
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

impl Player {
    /// Settings → Appearance: the themes folder, and make, edit and export themes.
    fn theme_editor(&mut self, ui: &mut Ui, pal: &Palette) {
        use realhires::themes;
        if self.file_themes_read.elapsed() > std::time::Duration::from_secs(2) {
            self.reload_file_themes();
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(2));
        let chosen = self.settings_draft().theme.clone();
        let dark_now = ui.ctx().global_style().visuals.dark_mode;
        let all = self.all_themes();
        let current = themes::resolve(&chosen, dark_now, &all);
        let custom = self.custom_themes.iter().position(|t| t.name == chosen);
        let file = self.theme_file(&chosen).map(|p| p.to_path_buf());
        ui.label(RichText::new("Theme files (.json) put into the themes folder appear in this menu.").color(pal.secondary));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open themes folder").on_hover_text(themes::folder(None).to_string_lossy()).clicked() {
                let dir = themes::folder(None);
                match std::fs::create_dir_all(&dir) {
                    Ok(()) => crate::style::open_with_default_app(&dir.to_string_lossy()),
                    Err(e) => {
                        self.scanner.info = Some(("Themes folder".into(), format!("The themes folder could not be made: {e}.")))
                    }
                }
            }
            if ui.button("Import theme…").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Universal Music Player theme", &["json"]).pick_file() {
                    self.import_theme_path(&path);
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button("New theme…").on_hover_text("A copy of the theme in use, to change as you like").clicked() {
                let name = themes::copy_name(&current.name, &all);
                let _ = self.apply_name(crate::player::NameAction::NewTheme(current.name.clone()), &name);
            }
            if ui.button("Export…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Universal Music Player theme", &["json"])
                    .set_file_name(format!("{}.json", current.name))
                    .save_file()
                {
                    if let Err(e) = std::fs::write(&path, themes::export_theme(&current)) {
                        self.scanner.info = Some(("Export theme".into(), format!("The theme could not be saved: {e}.")));
                    }
                }
            }
            if let Some(i) = custom {
                if ui.button("Rename…").clicked() {
                    let n = self.custom_themes[i].name.clone();
                    self.ask_name("Rename Theme", &n.clone(), crate::player::NameAction::RenameTheme(n));
                }
                if ui.button("Delete…").clicked() {
                    self.confirm = Some(crate::player::ConfirmAction::DeleteTheme(self.custom_themes[i].name.clone()));
                }
            }
        });
        if let Some(f) = file {
            ui.add_space(6.0);
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(
                RichText::new(format!(
                    "“{chosen}” comes from the file {name} in the themes folder. Change that file, or make a copy you can \
                     change here with New theme…"
                ))
                .color(pal.secondary),
            );
            return;
        }
        ui.add_space(6.0);
        let Some(i) = custom else {
            ui.label(
                RichText::new(format!(
                    "“{}” is a built-in theme and cannot be changed; “New theme…” makes a copy you can edit.",
                    if chosen == themes::SYSTEM { format!("{} ({})", chosen, current.name) } else { chosen.clone() }
                ))
                .color(pal.secondary),
            );
            return;
        };
        let mut t = self.custom_themes[i].clone();
        let before = t.clone();
        egui::Grid::new("theme-colours").num_columns(4).spacing([14.0, 8.0]).show(ui, |ui| {
            ui.label("Dark theme");
            ui.checkbox(&mut t.dark, "");
            ui.end_row();
            for (k, key) in themes::COLOR_KEYS.iter().enumerate() {
                ui.label(themes::color_label(key));
                let mut c = look::parse_hex(&t.colors[k]);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                    t.colors[k] = look::to_hex(c);
                }
                if k % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        if t != before {
            self.custom_themes[i] = t;
            self.themes_changed();
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

/// "Today 14:05", "Yesterday 09:12" or "24 Nov 2026".
pub fn played_label(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let now = realhires::stats::local_parts(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0));
    let t = realhires::stats::local_parts(ts);
    let day = |p: &realhires::stats::LocalParts| chrono::NaiveDate::from_ymd_opt(p.year, p.month, p.day);
    let secs = ts.floor() as i64;
    let minute = chrono::DateTime::from_timestamp(secs, 0).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default();
    match (day(&now), day(&t)) {
        (Some(a), Some(b)) if a == b => format!("Today {minute}"),
        (Some(a), Some(b)) if a.pred_opt() == Some(b) => format!("Yesterday {minute}"),
        _ => realhires::stats::day_label(&format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)),
    }
}
