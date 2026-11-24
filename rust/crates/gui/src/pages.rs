//! The pages of the player window (2.0, `docs/player-spec.md`): album cards,
//! the album page, song tables, artists, genres, search, Now Playing with the
//! lyrics, and Settings with every function of the 1.x app.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Galley, Layout, Rect, RichText, Sense, Stroke, Ui};
use realhires::musiclib::{self, Album, Track};
use realhires::playerstate;
use realhires::pyunicode::casefold;
use realhires::settings::{Settings, GROUPS};

use crate::icons::{self, Icon};
use crate::look::{self, Palette};
use crate::player::{cover, fmt_time, NpHits, NpTab, Page, Player, ShownLyrics, SongCol};

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

/// What a piece of a link line opens (Now Playing, the LCD).
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Artist(String),
    Album,
}

fn text_width(ui: &Ui, text: &str, font: &FontId) -> f32 {
    ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE).size().x)
}

/// "Artist, Artist — Album" in pieces: each artist its own link, the album name a link when
/// `album_link`, the separators plain; cut with "…" to fit `width` (the shorter part keeps
/// its room), as the Python app's `artist_line()`.
pub fn artist_line(ui: &Ui, font: FontId, color: Color32, artists: &[String], album: &str, album_link: bool, width: f32) -> Vec<(Arc<Galley>, Option<Link>)> {
    let sep = " — ";
    let galley = |t: &str| ui.ctx().fonts_mut(|f| f.layout_no_wrap(t.to_string(), font.clone(), color));
    let joined = artists.join(", ");
    let (aw, sw, bw) = (text_width(ui, &joined, &font), text_width(ui, sep, &font), text_width(ui, album, &font));
    let (a_room, b_room) = if aw + sw + bw <= width {
        (aw, bw)
    } else {
        let room = (width - sw).max(0.0);
        if aw <= room / 2.0 {
            (aw, room - aw)
        } else if bw <= room / 2.0 {
            (room - bw, bw)
        } else {
            (room / 2.0, room / 2.0)
        }
    };
    let mut out = vec![];
    let mut used = 0.0;
    for (i, name) in artists.iter().enumerate() {
        if i > 0 {
            let cw = text_width(ui, ", ", &font);
            if used + cw > a_room + 0.5 {
                break;
            }
            out.push((galley(", "), None));
            used += cw;
        }
        let nw = text_width(ui, name, &font);
        if used + nw <= a_room + 0.5 {
            out.push((galley(name), Some(Link::Artist(name.clone()))));
            used += nw;
        } else {
            if a_room - used > 12.0 {
                out.push((elided(ui, name, font.clone(), color, a_room - used), Some(Link::Artist(name.clone()))));
            }
            break;
        }
    }
    out.push((galley(sep), None));
    out.push((elided(ui, album, font, color, b_room + 0.5), album_link.then_some(Link::Album)));
    out
}

/// Paints link pieces centred at `center` with `painter`; the links get the pointer cursor and
/// an underline on hover.  Returns the rectangles of the links and the link clicked.
pub fn paint_links(ui: &Ui, painter: &egui::Painter, pieces: Vec<(Arc<Galley>, Option<Link>)>, center: egui::Pos2, color: Color32, id: egui::Id) -> (Vec<(Rect, Link)>, Option<Link>) {
    let total: f32 = pieces.iter().map(|(g, _)| g.size().x).sum();
    let mut x = center.x - total / 2.0;
    let mut rects = vec![];
    let mut clicked = None;
    for (i, (g, link)) in pieces.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x, center.y - g.size().y / 2.0), g.size());
        x += g.size().x;
        painter.galley(r.min, g, color);
        let Some(link) = link else { continue };
        let resp = ui.interact(r, id.with(i), Sense::click());
        if resp.hovered() {
            painter.line_segment([pos2(r.left(), r.bottom() - 1.0), pos2(r.right(), r.bottom() - 1.0)], Stroke::new(1.0, color));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            clicked = Some(link.clone());
        }
        rects.push((r, link));
    }
    (rects, clicked)
}

/// Details' format line: "FLAC · 24-bit/96 kHz · 2,304 kbps" (lossy: "MP3 · 44.1 kHz · 320
/// kbps"); unknown parts are left out (Python `format_line()`).
pub fn format_line(t: &Track) -> String {
    let mut parts = vec![];
    if !t.codec.is_empty() {
        parts.push(t.codec.to_uppercase());
    }
    if t.sample_rate > 0 {
        let khz = crate::eqwin::khz(t.sample_rate);
        if t.is_lossy || t.bits == 0 {
            parts.push(khz);
        } else {
            parts.push(format!("{}-bit{}/{khz}", t.bits, if t.is_float { " float" } else { "" }));
        }
    }
    if t.bit_rate > 0 {
        let k = (t.bit_rate as f64 / 1000.0).round_ties_even() as i64;
        let digits = k.to_string();
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        parts.push(format!("{grouped} kbps"));
    }
    if parts.is_empty() {
        "—".into()
    } else {
        parts.join(" · ")
    }
}

/// Now Playing's Details: (label, value) rows (Python `song_details()`). `verdict` is the
/// saved Quality check (verdict label, summary); `artists` the artists of the artist field.
pub fn song_details(t: &Track, album_tracks: usize, comment: &str, verdict: Option<&(String, String)>, artists: &[String]) -> Vec<(&'static str, String)> {
    let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
    let track_no = if t.track > 0 && album_tracks >= t.track as usize && album_tracks > 1 {
        format!("{} of {album_tracks}", t.track)
    } else if t.track > 0 {
        t.track.to_string()
    } else {
        "—".into()
    };
    let artist = if !artists.is_empty() && !t.artist.is_empty() { artists.join(", ") } else { t.artist.clone() };
    let mut rows = vec![
        ("Title", t.display_title()),
        ("Artist", dash(&artist)),
        ("Album", dash(&t.album)),
        ("Album artist", dash(&t.album_artist)),
        ("Composer", dash(&t.composer)),
        ("Genre", dash(&t.genre)),
        ("Year", dash(&t.year)),
        ("Track", track_no),
        ("Disc", if t.disc > 0 { t.disc.to_string() } else { "—".into() }),
        ("Duration", fmt_time(t.duration)),
        ("Format", format_line(t)),
        ("File", t.path.clone()),
    ];
    if !comment.trim().is_empty() {
        rows.push(("Comment", comment.trim().to_string()));
    }
    match verdict {
        None => rows.push(("Quality check", "Not checked yet".into())),
        Some((v, why)) => {
            rows.push(("Quality check", v.clone()));
            if !why.is_empty() {
                rows.push(("Why", why.clone()));
            }
        }
    }
    rows
}

/// A segmented switch (Now Playing's Lyrics | Up Next | Details): a rounded panel in the LCD
/// colour, the chosen segment raised in the page colour.  Returns the segment clicked.
fn segmented<T: Copy + PartialEq>(ui: &mut Ui, pal: &Palette, options: &[(T, &str)], value: T) -> Option<T> {
    let font = look::title(13.0);
    let seg = options.iter().map(|(_, t)| text_width(ui, t, &font)).fold(0.0, f32::max) + 30.0;
    let (rect, _) = ui.allocate_exact_size(vec2(seg * options.len() as f32 + 4.0, 30.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(9), pal.lcd);
    let mut chosen = None;
    for (i, (key, text)) in options.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 2.0 + i as f32 * seg, rect.top() + 2.0), vec2(seg, 26.0));
        let resp = ui.interact(r, ui.scope_id().with(("segment", i)), Sense::click());
        let on = *key == value;
        if on {
            ui.painter().rect_filled(r, CornerRadius::same(7), pal.bg);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(7), pal.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *text, font.clone(), if on { pal.text } else { pal.secondary });
        if resp.clicked() {
            chosen = Some(*key);
        }
    }
    chosen
}

/// A filled pill with white text (the "Back to the current line" button), at `rect`.
fn floating_pill(ui: &mut Ui, pal: &Palette, rect: Rect, label: &str, id: egui::Id) -> bool {
    let resp = ui.interact(rect, id, Sense::click());
    let fill = if resp.hovered() { pal.accent.gamma_multiply(0.85) } else { pal.accent };
    ui.painter().rect_filled(rect, CornerRadius::same(16), fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, look::title(13.0), Color32::WHITE);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
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
            Page::Artists => self.group_page(ui, pal, "artists"),
            Page::AlbumArtists => self.group_page(ui, pal, "album_artists"),
            Page::Genres => self.group_page(ui, pal, "genres"),
            Page::Years => self.group_page(ui, pal, "years"),
            Page::Composers => self.group_page(ui, pal, "composers"),
            Page::AllSongs => {
                page_title(ui, pal, "All Songs");
                let list: Vec<String> = self.sorted_songs(self.lib.tracks.iter().collect()).iter().map(|t| t.path.clone()).collect();
                ui.horizontal(|ui| {
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(list.clone(), Some(&list[0]), "All Songs");
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(list.clone(), "All Songs");
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
                let songs: Vec<&Track> = if liked {
                    self.liked_songs()
                } else {
                    self.lib.tracks.iter().filter(|t| self.state.is_disliked(&t.path)).collect()
                };
                let list: Vec<String> = self.sorted_songs(songs).iter().map(|t| t.path.clone()).collect();
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
                            self.play_list(list.clone(), Some(&list[0]), title);
                        }
                        if liked && pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                            self.shuffle_list(list.clone(), title);
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
            self.get_lyrics(vec![path.to_string()]);
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

    /// A table of songs (Songs, Liked Songs, a genre, search results).
    pub fn song_table(&mut self, ui: &mut Ui, pal: &Palette, list: &[String], name: &str, sortable: bool) {
        self.song_table_ex(ui, pal, list, name, sortable, None);
    }

    /// The same, with a "Played" column instead of Genre (Recently Played).
    pub fn song_table_ex(
        &mut self,
        ui: &mut Ui,
        pal: &Palette,
        list: &[String],
        name: &str,
        sortable: bool,
        played: Option<&HashMap<String, f64>>,
    ) {
        let w = ui.available_width();
        let cols: [(SongCol, &str, f32); 6] = [
            (SongCol::Title, "Title", 0.30),
            (SongCol::Artist, "Artist", 0.20),
            (SongCol::Album, "Album", 0.22),
            (SongCol::Genre, if played.is_some() { "Played" } else { "Genre" }, 0.11),
            (SongCol::Time, "Time", 0.06),
            (SongCol::Quality, "Quality", 0.11),
        ];
        let sortable = sortable && played.is_none();
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
                let Some(t) = self.lib.track(path).cloned() else {
                    // a playlist song whose file is gone
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                    let p = ui.painter_at(rect);
                    let g = elided(ui, &realhires::paths::file_name(path), look::text(13.0), pal.secondary.gamma_multiply(0.7), usable * 0.5);
                    p.galley(pos2(rect.left() + xs[0] + 4.0, rect.center().y - g.size().y / 2.0), g, pal.secondary);
                    p.text(pos2(rect.left() + xs[2] + 4.0, rect.center().y), Align2::LEFT_CENTER, "Not in the library", look::text(12.0), pal.secondary.gamma_multiply(0.7));
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
                let heart = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(16.0, 16.0));
                let liked = self.state.is_liked(path);
                if liked || resp.hovered() {
                    icons::paint(&p, heart, if liked { Icon::HeartFilled } else { Icon::Heart }, pal.accent);
                }
                let fourth = match played {
                    Some(m) => played_label(m.get(path).copied().unwrap_or(0.0)),
                    None => t.genre.clone(),
                };
                let vals = [t.display_title(), t.display_artist(), t.display_album(), fourth, fmt_time(t.duration), musiclib::quality(&t)];
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

    pub fn side_list(&mut self, ui: &mut Ui, pal: &Palette, names: &[(String, usize)], selected: &Option<String>) -> Option<String> {
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

    /// Artists, Album Artists (album cards), Genres, Years, Composers (song
    /// tables): the names on the left, the chosen one's music on the right.
    fn group_page(&mut self, ui: &mut Ui, pal: &Palette, kind: &'static str) {
        let (title, groups, cards) = match kind {
            "artists" => ("Artists", &self.lib.artists, true),
            "album_artists" => ("Album Artists", &self.lib.album_artists, true),
            "genres" => ("Genres", &self.lib.genres, false),
            "years" => ("Years", &self.lib.years, false),
            _ => ("Composers", &self.lib.composers, false),
        };
        page_title(ui, pal, title);
        let names: Vec<(String, usize)> = groups.iter().map(|g| (g.name.clone(), g.tracks.len())).collect();
        if self.group_sel.get(kind).is_none_or(|s| !names.iter().any(|(n, _)| n == s)) {
            if let Some((n, _)) = names.first() {
                self.group_sel.insert(kind, n.clone());
            }
        }
        let sel = self.group_sel.get(kind).cloned();
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(240.0, height), Layout::top_down(Align::Min), |ui| {
                if let Some(p) = self.side_list(ui, pal, &names, &sel) {
                    self.group_sel.insert(kind, p);
                }
            });
            ui.add_space(20.0);
            ui.vertical(|ui| {
                let Some(name) = sel else { return };
                let groups = match kind {
                    "artists" => &self.lib.artists,
                    "album_artists" => &self.lib.album_artists,
                    "genres" => &self.lib.genres,
                    "years" => &self.lib.years,
                    _ => &self.lib.composers,
                };
                let Some(g) = groups.iter().find(|g| g.name == name) else { return };
                let tracks = g.tracks.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).font(look::title(22.0)).color(pal.text));
                    ui.add_space(14.0);
                    let list: Vec<String> = tracks.iter().map(|t| t.path.clone()).collect();
                    if pill(ui, pal, Icon::Play, "Play").clicked() {
                        self.play_list(list.clone(), Some(&list[0]), &name);
                    }
                    if pill(ui, pal, Icon::Shuffle, "Shuffle").clicked() {
                        self.shuffle_list(list, &name);
                    }
                });
                ui.add_space(12.0);
                if cards {
                    let mut keys: Vec<String> = vec![];
                    for t in &tracks {
                        let key = t.song().album_key();
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                    let albums: Vec<usize> = keys.iter().filter_map(|k| self.lib.album_index.get(k).copied()).collect();
                    self.album_grid(ui, pal, &albums);
                } else {
                    let list: Vec<String> = self.sorted_songs(tracks.iter().collect()).iter().map(|t| t.path.clone()).collect();
                    self.song_table(ui, pal, &list, &name, true);
                }
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

    /// Now Playing (2.1): the cover with the title and "Artist — Album" as one block in the
    /// middle of the left half (the album name opens the album, each artist name that artist,
    /// the cover the Cover window); on the right a switch Lyrics | Up Next | Details.  With
    /// the lyrics hidden (the Lyrics button on this page) only the block, in the middle.
    fn now_playing(&mut self, ui: &mut Ui, pal: &Palette) {
        self.np_hits = NpHits::default();
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
        let has_album = self.lib.album_of(&path).is_some();
        let paths: Vec<String> = self.lib.album_of(&path).map(|a| a.tracks.iter().map(|t| t.path.clone()).collect()).unwrap_or_else(|| vec![path.clone()]);
        let tex = self.covers.get(&key, &paths);
        let lyrics = self.lyrics_of_now();
        let area = ui.available_rect_before_wrap();
        let right = !self.lyrics_hidden;
        let left = if right { Rect::from_min_max(area.min, pos2(area.center().x, area.bottom())) } else { area };
        // the block: the cover, the title and "Artist — Album" (80 px), in the middle of `left`
        let text_h = 80.0;
        let size = (left.width() - 80.0).min(left.height() - 80.0 - text_h).clamp(120.0, 480.0);
        let crect = Rect::from_min_size(pos2(left.center().x - size / 2.0, left.center().y - (size + text_h) / 2.0), vec2(size, size));
        cover(ui.painter(), crect, tex.as_ref(), &track.display_album(), 12);
        let cresp = ui.interact(crect, ui.scope_id().with("np-cover"), Sense::click());
        if cresp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        self.np_hits.cover = Some(crect);
        if cresp.clicked() {
            self.open_cover_window(&path);
        }
        let title = elided(ui, &track.display_title(), look::title(20.0), pal.text, left.width() - 60.0);
        ui.painter().galley(pos2(left.center().x - title.size().x / 2.0, crect.bottom() + 30.0 - title.size().y / 2.0), title, pal.text);
        let artists = self.lib.artists_of(&track);
        let pieces = artist_line(ui, look::text(14.0), pal.secondary, &artists, &track.display_album(), has_album, left.width() - 60.0);
        let (rects, clicked) = paint_links(ui, ui.painter(), pieces, pos2(left.center().x, crect.bottom() + 58.0), pal.secondary, ui.scope_id().with("np-links"));
        self.np_hits.links = rects;
        match clicked {
            Some(Link::Artist(name)) => return self.open_artist(&name),
            Some(Link::Album) => return self.open_album_of(&path),
            None => {}
        }
        if !right {
            return; // the lyrics are hidden (the Lyrics button): the block only
        }
        let rrect = Rect::from_min_max(pos2(area.center().x + 10.0, area.top()), area.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rrect), |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                if let Some(tab) = segmented(ui, pal, &NpTab::ALL, self.np_tab) {
                    self.set_np_tab(tab);
                }
                ui.add_space(8.0);
            });
            match self.np_tab {
                NpTab::Lyrics => self.np_lyrics(ui, pal, &path, lyrics),
                NpTab::UpNext => self.np_up_next(ui, pal),
                NpTab::Details => self.np_details(ui, pal, &track),
            }
        });
    }

    /// Now Playing's Lyrics: the label and A− / A+, then the lyrics (synced ones follow the
    /// song unless the user scrolled the current line away; then "Back to the current line").
    fn np_lyrics(&mut self, ui: &mut Ui, pal: &Palette, path: &str, lyrics: Option<ShownLyrics>) {
        let Some(lyrics) = lyrics else {
            ui.add_space(ui.available_height() * 0.3);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No lyrics for this song.").color(pal.secondary));
                ui.add_space(8.0);
                let r = pill(ui, pal, Icon::Lyrics, "Get lyrics");
                self.np_hits.get_lyrics = Some(r.rect);
                if r.clicked() {
                    self.get_lyrics(vec![path.to_string()]);
                }
            });
            return;
        };
        let (label, source) = self.lyrics_label_of_now();
        let size = self.state.lyrics_size.clamp(playerstate::LYRICS_SIZE_MIN, playerstate::LYRICS_SIZE_MAX);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&label).font(look::title(13.0)).color(pal.text));
            ui.label(RichText::new(&source).font(look::text(11.5)).color(pal.secondary));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                let small = |t: &str| egui::Button::new(RichText::new(t).font(look::title(13.0))).fill(pal.lcd).corner_radius(8).min_size(vec2(44.0, 26.0));
                let bigger = ui.add_enabled(size < playerstate::LYRICS_SIZE_MAX, small("A+")).on_hover_text("Larger lyrics");
                let smaller = ui.add_enabled(size > playerstate::LYRICS_SIZE_MIN, small("A−")).on_hover_text("Smaller lyrics");
                self.np_hits.size_buttons = vec![smaller.rect, bigger.rect];
                if bigger.clicked() {
                    self.lyrics_size_step(1);
                }
                if smaller.clicked() {
                    self.lyrics_size_step(-1);
                }
            });
        });
        ui.add_space(4.0);
        let lrect = ui.available_rect_before_wrap();
        let font = look::title(size as f32);
        let plain_font = look::title(playerstate::plain_lyrics_size(size) as f32);
        let pos_ms = (self.engine.position().unwrap_or(0.0) * 1000.0) as u32;
        let mut seek_to = None;
        let user_scrolled = ui.rect_contains_pointer(lrect) && ui.input(|i| i.smooth_scroll_delta.y != 0.0);
        let back = std::mem::take(&mut self.lyrics_back);
        let mut cur_in_view = None;
        let mut cur_line = None;
        egui::ScrollArea::vertical().id_salt("lyrics").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(if matches!(lyrics, ShownLyrics::Synced(_)) { lrect.height() * 0.3 } else { 6.0 });
            match &lyrics {
                ShownLyrics::Synced(lines) => {
                    let cur = lines.iter().rposition(|(ms, _)| *ms <= pos_ms);
                    cur_line = cur;
                    let seen = cur.unwrap_or(0); // before the first line: the first one
                    for (i, (ms, text)) in lines.iter().enumerate() {
                        let is_cur = Some(i) == cur;
                        let color = if is_cur { pal.text } else { pal.secondary.gamma_multiply(0.75) };
                        let shown = if text.trim().is_empty() { "…".to_string() } else { text.clone() };
                        let resp = ui.add(egui::Label::new(RichText::new(shown).font(font.clone()).color(color)).wrap().sense(Sense::click()));
                        ui.add_space(10.0);
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() {
                            seek_to = Some(*ms as f64 / 1000.0);
                        }
                        if i == seen {
                            cur_in_view = Some(ui.clip_rect().contains_rect(resp.rect));
                            if back {
                                resp.scroll_to_me_animation(Some(Align::Center), egui::style::ScrollAnimation::none());
                            }
                        }
                        if is_cur && !back && !self.lyrics_away && self.lyrics_line != Some(i) {
                            resp.scroll_to_me_animation(Some(Align::Center), egui::style::ScrollAnimation::duration(0.35));
                        }
                    }
                }
                ShownLyrics::Plain(text) => {
                    ui.add(egui::Label::new(RichText::new(text).font(plain_font.clone()).color(pal.text)).wrap());
                }
                ShownLyrics::Instrumental => {
                    ui.label(RichText::new("This song is instrumental.").font(plain_font.clone()).color(pal.secondary));
                }
            }
            ui.add_space(lrect.height() * 0.5);
        });
        if matches!(lyrics, ShownLyrics::Synced(_)) {
            if seek_to.is_some() || back {
                self.lyrics_away = false; // follow the line clicked, or the current one again
                if seek_to.is_some() {
                    self.lyrics_line = None;
                }
            } else if user_scrolled || self.lyrics_check {
                // the user scrolls: following stops while the current line is out of view (checked
                // once more after the last scroll, since a scroll shows a frame later)
                if let Some(v) = cur_in_view {
                    self.lyrics_away = !v;
                }
                self.lyrics_check = user_scrolled;
                ui.ctx().request_repaint();
            } else if self.lyrics_away && cur_in_view == Some(true) {
                self.lyrics_away = false; // the current line came back into view
            }
            if !self.lyrics_away && seek_to.is_none() {
                self.lyrics_line = cur_line;
            }
            if self.lyrics_away {
                let w = text_width(ui, "Back to the current line", &look::title(13.0)) + 36.0;
                let r = Rect::from_center_size(pos2(lrect.center().x, lrect.bottom() - 34.0), vec2(w, 32.0));
                self.np_hits.back = Some(r);
                if floating_pill(ui, pal, r, "Back to the current line", ui.scope_id().with("lyrics-back")) {
                    self.lyrics_back = true;
                    self.lyrics_away = false;
                    ui.ctx().request_repaint();
                }
            }
        }
        if let Some(s) = seek_to {
            self.seek(s);
        }
    }

    /// Now Playing's Up Next: the queue, as in the Up Next panel (double-click plays a song).
    fn np_up_next(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            if !self.queue_name.is_empty() {
                ui.label(RichText::new(format!("Playing from {}", self.queue_name)).color(pal.secondary));
            }
        });
        ui.add_space(6.0);
        let upcoming = self.queue.as_ref().map(|q| q.up_next(50)).unwrap_or_default();
        self.np_hits.up_next = upcoming.clone();
        if upcoming.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Nothing comes after this song.").color(pal.secondary));
            });
            return;
        }
        let mut jump = None;
        egui::ScrollArea::vertical().id_salt("np-up-next").auto_shrink([false, false]).show(ui, |ui| {
            for (i, path) in upcoming.iter().enumerate() {
                let t = self.lib.track(path);
                let title = t.map(|t| t.display_title()).unwrap_or_else(|| path.clone());
                let artist = t.map(|t| t.display_artist()).unwrap_or_default();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 12.0, 44.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
                }
                let p = ui.painter_at(rect);
                p.text(rect.left_top() + vec2(20.0, 5.0), Align2::LEFT_TOP, title, look::title(13.0), pal.text);
                p.text(rect.left_top() + vec2(20.0, 24.0), Align2::LEFT_TOP, artist, look::text(11.5), pal.secondary);
                if resp.double_clicked() {
                    jump = Some(i);
                }
            }
        });
        if let Some(i) = jump {
            self.jump_up_next(i);
        }
    }

    /// Now Playing's Details: the song's description; its artists are links.
    fn np_details(&mut self, ui: &mut Ui, pal: &Palette, track: &Track) {
        let (comment, verdict) = self.details_extra(&track.path);
        let n_album = self.lib.album_of(&track.path).map(|a| a.tracks.len()).unwrap_or(0);
        let artists = if track.artist.is_empty() { vec![] } else { musiclib::split_artists(&track.artist, &self.lib.known_artists) };
        let rows = song_details(track, n_album, &comment, verdict.as_ref(), &artists);
        self.np_hits.details = rows.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        let mut open = None;
        let label_w = rows.iter().map(|(k, _)| text_width(ui, k, &look::text(13.0))).fold(0.0, f32::max) + 18.0;
        egui::ScrollArea::vertical().id_salt("np-details").auto_shrink([false, false]).show(ui, |ui| {
            let value_w = (ui.available_width() - label_w - 44.0).max(120.0);
            for (k, v) in &rows {
                ui.horizontal_top(|ui| {
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        ui.set_width(label_w);
                        ui.label(RichText::new(*k).font(look::text(13.0)).color(pal.secondary));
                    });
                    ui.vertical(|ui| {
                        ui.set_width(value_w);
                        if *k == "Artist" && !artists.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                for (i, name) in artists.iter().enumerate() {
                                    if i > 0 {
                                        ui.label(RichText::new(", ").font(look::text(13.0)).color(pal.text));
                                    }
                                    let r = ui.add(egui::Link::new(RichText::new(name).font(look::text(13.0)).color(pal.text)));
                                    self.np_hits.detail_links.push((r.rect, name.clone()));
                                    if r.clicked() {
                                        open = Some(name.clone());
                                    }
                                }
                            });
                        } else {
                            ui.add(egui::Label::new(RichText::new(v).font(look::text(13.0)).color(pal.text)).wrap());
                        }
                    });
                });
                ui.add_space(7.0);
            }
        });
        if let Some(name) = open {
            self.open_artist(&name);
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
