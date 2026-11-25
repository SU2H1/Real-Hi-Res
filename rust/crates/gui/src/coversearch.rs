//! Search Cover Online → MusicBrainz… (2.5, `docs/player-spec.md`, "2.5", item 9), the
//! counterpart of Python's `player_coverart.py`.
//!
//! `CoverSearch` belongs to the Cover window (it is opened in it and closed with it): it
//! looks the album up on MusicBrainz (`coverart::find_releases()`, through the existing
//! client and its one request a second) and shows the front covers of the releases
//! found, from the Cover Art Archive, as a grid of thumbnails with each release's title,
//! artist, year, country and format.  A click on one downloads the cover as it was
//! uploaded and shows it large with its size in pixels; Use this cover hands it to the
//! Cover window, which sets it exactly as Change cover… does.
//!
//! One thread does everything, one request at a time: the search, then the thumbnails
//! in order, a chosen cover first.  Only the album's artist and title are sent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Rect, RichText, Sense};
use realhires::coverart::{self, CoverArtClient, CoverArtError};
use realhires::i18n::tr_text;
use realhires::musicbrainz::{MbError, MusicBrainzClient, Release};
use realhires::stats::thousands;
use realhires::{covers, tagedit, tr};

use crate::look::{self, Palette};
use crate::pages::elided;

/// A thumbnail in the grid, in points.
pub const THUMB: f32 = 148.0;
const GAP: f32 = 14.0;
/// Three lines under it: title, artist, year · country · format.
const CAPTION: f32 = 54.0;
/// The chosen cover is made at most this large (egui scales it to fit).
const LARGE: u32 = 1024;

enum Job {
    Search(u64, String, String),
    Thumb(u64, Release),
}

#[derive(Default)]
struct Jobs {
    list: Vec<Job>,
    /// the chosen cover, before the thumbnails
    full: Option<(u64, Release)>,
}

enum Msg {
    Found(u64, Vec<Release>),
    Thumb(u64, Release, Option<egui::ColorImage>),
    Full(u64, Release, Vec<u8>, Option<egui::ColorImage>, String),
    /// (search, what failed, the sentence, the archive asked to slow down)
    Error(u64, &'static str, String, bool),
}

pub struct Tile {
    pub release: Release,
    tex: egui::TextureHandle,
}

/// What the window asks of the Cover window after a frame.
pub enum Outcome {
    Open,
    Closed,
    /// Use this cover: the picture, and what to call it when it cannot be used
    Use(Vec<u8>, String),
}

pub struct CoverSearch {
    pub album: String,
    pub artist: String,
    /// what was searched for last
    pub searched: String,
    gen: Arc<Mutex<u64>>,
    pub searching: bool,
    /// found on MusicBrainz
    pub releases: Vec<Release>,
    /// of them, how many were asked of the archive
    pub asked: usize,
    /// every thumbnail was asked for
    pub done: bool,
    /// those with a cover
    pub tiles: Vec<Tile>,
    /// the release whose cover is shown large
    pub chosen: Option<Release>,
    /// that cover as it was uploaded
    pub picture: Option<Vec<u8>>,
    large: Option<egui::TextureHandle>,
    /// the covers downloaded as they were uploaded, by release id: never twice
    full: std::collections::HashMap<String, (Vec<u8>, Option<egui::TextureHandle>, String)>,
    pub details: String,
    pub loading: bool,
    /// what went wrong (English; shown with `tr_text()`)
    pub error: String,
    jobs: Arc<(Mutex<Jobs>, Condvar)>,
    stop: Arc<AtomicBool>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    /// where the tiles were drawn (for the tests)
    pub tile_rects: Vec<Rect>,
    /// the window, as drawn last
    pub rect: Option<Rect>,
}

fn decode(png: Option<Vec<u8>>) -> Option<egui::ColorImage> {
    let rgba = image::load_from_memory(&png?).ok()?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

fn work(jobs: Arc<(Mutex<Jobs>, Condvar)>, gen: Arc<Mutex<u64>>, stop: Arc<AtomicBool>, tx: Sender<Msg>, ctx: egui::Context, server: String) {
    let mut mb = MusicBrainzClient::new(&server).with_stop(stop.clone());
    let mut art = CoverArtClient::new(None).with_stop(stop.clone());
    let current = |g: u64| *gen.lock().unwrap_or_else(|e| e.into_inner()) == g;
    loop {
        // the next job: a chosen cover first, then the search and the thumbnails in order
        let (job, full) = {
            let (lock, cv) = &*jobs;
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(f) = q.full.take() {
                    break (None, Some(f));
                }
                if !q.list.is_empty() {
                    break (Some(q.list.remove(0)), None);
                }
                q = cv.wait_timeout(q, std::time::Duration::from_millis(500)).unwrap_or_else(|e| e.into_inner()).0;
            }
        };
        let fail = |g: u64, what: &'static str, e: CoverArtError| -> bool {
            match e {
                CoverArtError::Stopped => return false,
                CoverArtError::Busy(m) => {
                    // nothing more is asked of the archive now
                    jobs.0.lock().unwrap_or_else(|e| e.into_inner()).list.retain(|j| !matches!(j, Job::Thumb(..)));
                    let _ = tx.send(Msg::Error(g, what, m, true));
                }
                CoverArtError::Failed(m) => {
                    let _ = tx.send(Msg::Error(g, what, m, false));
                }
            }
            true
        };
        if let Some((g, r)) = full {
            if !current(g) {
                continue;
            }
            match art.front(&r.id, false) {
                Ok(Some(data)) => match tagedit::check_picture(&data, "") {
                    Ok(info) => {
                        let text = covers::details_text(info.width, info.height, covers::image_format(&data), data.len());
                        let image = decode(covers::fit_png(&data, LARGE, false));
                        let _ = tx.send(Msg::Full(g, r, data, image, text));
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::Error(g, "full", e.to_string(), false));
                    }
                },
                Ok(None) => {
                    let _ = tx.send(Msg::Error(g, "full", "The Cover Art Archive has no cover for this release.".into(), false));
                }
                Err(e) => {
                    if !fail(g, "full", e) {
                        return;
                    }
                }
            }
            ctx.request_repaint();
            continue;
        }
        match job {
            Some(Job::Search(g, album, artist)) if current(g) => match coverart::find_releases(&mut mb, &album, &artist) {
                Ok(found) => {
                    {
                        let mut q = jobs.0.lock().unwrap_or_else(|e| e.into_inner());
                        if current(g) {
                            q.list.extend(found.iter().cloned().map(|r| Job::Thumb(g, r)));
                        }
                    }
                    let _ = tx.send(Msg::Found(g, found));
                }
                Err(MbError::Stopped) => return,
                Err(MbError::Failed(m)) => {
                    let _ = tx.send(Msg::Error(g, "search", m, false));
                }
            },
            Some(Job::Thumb(g, r)) if current(g) => match art.front(&r.id, true) {
                Ok(data) => {
                    let image = data.and_then(|d| decode(covers::fit_png(&d, (THUMB * 2.0) as u32, true)));
                    let _ = tx.send(Msg::Thumb(g, r, image));
                }
                Err(e) => {
                    if !fail(g, "thumb", e) {
                        return;
                    }
                }
            },
            _ => {}
        }
        ctx.request_repaint();
    }
}

impl CoverSearch {
    /// `album`, `artist`: what MusicBrainz is asked first (`coverart::search_terms()` of the
    /// song's tags); `server`: the MusicBrainz server of the settings.
    pub fn new(ctx: &egui::Context, album: &str, artist: &str, server: &str) -> CoverSearch {
        let (tx, rx) = mpsc::channel();
        let jobs: Arc<(Mutex<Jobs>, Condvar)> = Default::default();
        let (gen, stop) = (Arc::new(Mutex::new(0u64)), Arc::new(AtomicBool::new(false)));
        {
            let (jobs, gen, stop, ctx, server) = (jobs.clone(), gen.clone(), stop.clone(), ctx.clone(), server.to_string());
            let _ = std::thread::Builder::new().name("cover-search".into()).spawn(move || work(jobs, gen, stop, tx, ctx, server));
        }
        let mut w = CoverSearch {
            album: album.to_string(),
            artist: artist.to_string(),
            searched: String::new(),
            gen,
            searching: false,
            releases: vec![],
            asked: 0,
            done: false,
            tiles: vec![],
            chosen: None,
            picture: None,
            large: None,
            full: Default::default(),
            details: String::new(),
            loading: false,
            error: String::new(),
            jobs,
            stop,
            rx,
            ctx: ctx.clone(),
            tile_rects: vec![],
            rect: None,
        };
        if !album.trim().is_empty() {
            w.search();
        }
        w
    }

    fn generation(&self) -> u64 {
        *self.gen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Search: the releases called so on MusicBrainz, then their covers.
    pub fn search(&mut self) {
        let (album, artist) = (tagedit::clean_value(&self.album), tagedit::clean_value(&self.artist));
        if album.is_empty() || self.searching {
            return;
        }
        self.searched = album.clone();
        let g = {
            let mut g = self.gen.lock().unwrap_or_else(|e| e.into_inner());
            *g += 1;
            *g
        };
        self.releases.clear();
        self.tiles.clear();
        (self.asked, self.done, self.searching) = (0, false, true);
        (self.chosen, self.picture, self.large, self.loading) = (None, None, None, false);
        self.error.clear();
        let (lock, cv) = &*self.jobs;
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.list = vec![Job::Search(g, album, artist)];
        q.full = None;
        cv.notify_all();
    }

    /// A click on a cover: it is downloaded as it was uploaded and shown large.
    pub fn choose(&mut self, index: usize) {
        let Some(r) = self.tiles.get(index).map(|t| t.release.clone()) else { return };
        self.error.clear();
        if let Some((data, tex, text)) = self.full.get(&r.id).cloned() {
            // downloaded before
            (self.picture, self.large, self.loading, self.details) = (Some(data), tex, false, text);
        } else {
            (self.picture, self.large, self.loading) = (None, None, true);
            self.details.clear();
            let (lock, cv) = &*self.jobs;
            lock.lock().unwrap_or_else(|e| e.into_inner()).full = Some((self.generation(), r.clone()));
            cv.notify_all();
        }
        self.chosen = Some(r);
    }

    /// Back to the covers.
    pub fn back(&mut self) {
        (self.chosen, self.picture, self.large, self.loading) = (None, None, None, false);
        self.error.clear();
        self.jobs.0.lock().unwrap_or_else(|e| e.into_inner()).full = None;
    }

    fn poll(&mut self) {
        let g = self.generation();
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Found(gen, found) if gen == g => {
                    self.searching = false;
                    self.done = found.is_empty();
                    self.releases = found;
                }
                Msg::Thumb(gen, r, image) if gen == g => {
                    self.asked += 1;
                    if let Some(image) = image {
                        let tex = self.ctx.load_texture(format!("cover search {}", r.id), image, egui::TextureOptions::LINEAR);
                        self.tiles.push(Tile { release: r, tex });
                    }
                    self.done = self.asked >= self.releases.len();
                }
                Msg::Full(gen, r, data, image, text) if gen == g => {
                    let tex = image.map(|i| self.ctx.load_texture(format!("cover search large {}", r.id), i, egui::TextureOptions::LINEAR));
                    self.full.insert(r.id.clone(), (data.clone(), tex.clone(), text.clone()));
                    if self.chosen.as_ref().is_some_and(|c| c.id == r.id) {
                        self.loading = false;
                        self.picture = Some(data);
                        self.large = tex;
                        self.details = text;
                    }
                }
                Msg::Error(gen, what, text, busy) if gen == g => {
                    match what {
                        "search" => (self.searching, self.done) = (false, true),
                        "thumb" => {
                            self.asked = if busy { self.releases.len() } else { self.asked + 1 };
                            self.done = self.asked >= self.releases.len();
                        }
                        _ => self.loading = false,
                    }
                    if what != "thumb" || busy {
                        self.error = text; // (one thumbnail that fails is just not shown)
                    }
                }
                _ => {}
            }
        }
    }

    /// The line under the search fields.
    pub fn status_text(&self) -> String {
        if !self.error.is_empty() {
            return tr_text(&self.error).to_string();
        }
        let album = &self.searched;
        if album.is_empty() {
            return tr!("Type the album's name and press Search.").to_string();
        }
        if self.searching {
            return tr!("Looking for “{album}” on MusicBrainz…", album = album);
        }
        let (n, k) = (self.releases.len(), self.tiles.len());
        if n == 0 {
            return tr!("MusicBrainz knows no release called “{album}”. Change the words and search again.", album = album);
        }
        if !self.done {
            return tr!("Getting the covers from the Cover Art Archive ({i} of {n})…", i = (self.asked + 1).min(n), n = n);
        }
        match (k, n) {
            (0, 1) => tr!("MusicBrainz knows 1 release, but the Cover Art Archive has no cover for it.").to_string(),
            (0, _) => tr!("MusicBrainz knows {n} releases, but the Cover Art Archive has no cover for them.", n = n),
            (1, _) => tr!("1 cover found. Click it to see it large.").to_string(),
            _ => tr!("{n} covers found. Click one to see it large.", n = k),
        }
    }

    /// The chosen release as one line: "Title — Artist  ·  2001 · JP · CD".
    pub fn caption(&self) -> String {
        let Some(r) = &self.chosen else { return String::new() };
        let head = if r.artist.is_empty() { r.title.clone() } else { tr!("{title} — {artist}", title = r.title, artist = r.artist) };
        let facts = coverart::release_facts(r);
        if facts.is_empty() {
            head
        } else {
            format!("{head}  ·  {facts}")
        }
    }

    /// Draw the window.  `all_songs`, `also_folder`: the Cover window's choice, shown with
    /// the chosen cover; `n`: the album's songs; `idle`: the Cover window can take a cover now.
    #[allow(clippy::too_many_arguments)]
    pub fn show(&mut self, ctx: &egui::Context, pal: &Palette, esc: bool, all_songs: &mut bool, also_folder: &mut bool, n: usize, idle: bool) -> Outcome {
        self.poll();
        let mut result = Outcome::Open;
        let mut open = true;
        let (mut close, mut search, mut choose, mut back, mut use_it) = (false, false, None, false, false);
        self.tile_rects.clear();
        let screen = ctx.content_rect();
        let resp = egui::Window::new(tr!("Search Cover on MusicBrainz"))
            .id(egui::Id::unique("player-cover-search"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(vec2(700.0_f32.min(screen.width() - 40.0), 620.0_f32.min(screen.height() - 60.0)))
            .min_size(vec2(520.0, 380.0))
            .pivot(Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr!("Album")).color(pal.secondary));
                    let w = ((ui.available_width() - 220.0) * 0.55).max(120.0);
                    let a = ui.add(egui::TextEdit::singleline(&mut self.album).desired_width(w));
                    ui.label(RichText::new(tr!("Artist")).color(pal.secondary));
                    let w = (ui.available_width() - 90.0).max(90.0);
                    let b = ui.add(egui::TextEdit::singleline(&mut self.artist).desired_width(w));
                    let enter = (a.lost_focus() || b.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add_enabled(!self.searching, egui::Button::new(RichText::new(tr!("Search")).strong())).clicked() || enter {
                        search = true;
                    }
                });
                ui.label(RichText::new(self.status_text()).small().color(pal.secondary));
                ui.add_space(4.0);
                // the pictures take what the lines under them leave, so a small window shrinks them
                let large = self.chosen.is_some();
                let line = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y;
                let credit_lines = if ui.available_width() < 620.0 { 3.0 } else { 2.0 };
                let under = 44.0 + credit_lines * line + if large { 3.0 * line + 14.0 + if n > 1 { 2.0 * line + 8.0 } else { 0.0 } } else { 0.0 };
                let height = (ui.available_height() - under).max(120.0);
                let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
                if large {
                    let p = ui.painter_at(area);
                    match &self.large {
                        Some(tex) => {
                            let size = tex.size_vec2();
                            let scale = (area.width() / size.x).min(area.height() / size.y).min(1.0);
                            let rect = Rect::from_center_size(area.center(), size * scale);
                            let shape = egui::epaint::RectShape::filled(rect, CornerRadius::same(6), Color32::WHITE)
                                .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                            p.add(shape);
                        }
                        None => {
                            let text = if self.loading { tr!("Getting the cover…") } else { tr!("The picture cannot be shown.") };
                            p.text(area.center(), Align2::CENTER_CENTER, text, look::text(14.0), pal.secondary);
                        }
                    }
                } else {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                        egui::ScrollArea::vertical().id_salt("cover-search-grid").auto_shrink([false, false]).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                                for (i, tile) in self.tiles.iter().enumerate() {
                                    let (r, resp) = ui.allocate_exact_size(vec2(THUMB, THUMB + CAPTION), Sense::click());
                                    self.tile_rects.push(r);
                                    if !ui.is_rect_visible(r) {
                                        continue;
                                    }
                                    let p = ui.painter();
                                    if resp.hovered() {
                                        p.rect_filled(r.expand(5.0), CornerRadius::same(8), pal.row_hover);
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                    let pic = Rect::from_min_size(r.min, vec2(THUMB, THUMB));
                                    let shape = egui::epaint::RectShape::filled(pic, CornerRadius::same(6), Color32::WHITE)
                                        .with_texture(tile.tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                                    p.add(shape);
                                    let rel = &tile.release;
                                    let line = |text: &str, font: egui::FontId, color: Color32, y: f32| {
                                        let g = elided(ui, text, font, color, THUMB);
                                        p.galley(pos2(r.left(), pic.bottom() + y - g.size().y / 2.0), g, color);
                                    };
                                    line(&rel.title, look::title(13.0), pal.text, 12.0);
                                    line(&rel.artist, look::text(11.5), pal.secondary, 28.0);
                                    line(&coverart::release_facts(rel), look::text(11.5), pal.secondary, 43.0);
                                    if resp.clicked() {
                                        choose = Some(i);
                                    }
                                }
                            });
                        });
                    });
                }
                if large {
                    ui.label(RichText::new(tr_text(&self.details)).color(pal.text));
                    ui.label(RichText::new(self.caption()).color(pal.secondary));
                    ui.add_space(4.0);
                    if n > 1 {
                        ui.radio_value(all_songs, true, tr!("All {n} songs of this album", n = thousands(n as i64)));
                        ui.radio_value(all_songs, false, tr!("This song only"));
                    }
                    ui.checkbox(also_folder, tr!("Also save it as cover.jpg in the album's folder"));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if large && ui.button(tr!("Back to the covers")).clicked() {
                        back = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(tr!("Close")).clicked() {
                            close = true;
                        }
                        if large {
                            let can = self.picture.is_some() && !self.loading && idle;
                            if ui.add_enabled(can, egui::Button::new(RichText::new(tr!("Use this cover")).strong())).clicked() {
                                use_it = true;
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(RichText::new(tr_text(coverart::CREDIT)).small().color(pal.secondary));
                if esc && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    if self.chosen.is_some() {
                        back = true;
                    } else {
                        close = true;
                    }
                }
            });
        self.rect = resp.map(|r| r.response.rect);
        if search {
            self.search();
        }
        if let Some(i) = choose {
            self.choose(i);
        }
        if back {
            self.back();
        }
        if use_it {
            if let (Some(data), Some(r)) = (self.picture.clone(), self.chosen.clone()) {
                result = Outcome::Use(data, tr!("The cover of “{album}” from the Cover Art Archive", album = r.title));
            }
        }
        if close || !open {
            result = Outcome::Closed;
        }
        result
    }
}

impl Drop for CoverSearch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.jobs.1.notify_all();
    }
}
