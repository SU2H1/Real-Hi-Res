//! Moods, tempo and playlists in the egui player (2.4, `docs/player-spec.md` "4. Moods,
//! tempo and playlists"), as the Tk player's `player_moods.py` does them: the moods of
//! `library.sqlite` read at start, the background analysis after every library update
//! (one song at a time, `moods::run_analysis()`, when `analyse_moods` is on) with its
//! progress in the sidebar's status, the Moods page (a card per mood and per tempo with
//! its number of songs), a card's page (its songs, Play, Shuffle, Create Playlist), the
//! BPM and Mood columns and Now Playing's Details rows.  New results are shown at most
//! every `REDRAW` while the job runs; the song lists' cells find them through a
//! thread-local copy (`shown_mood()`), since `cell_text()` has no player.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Rect, RichText, Sense, Ui};
use realhires::moods::{self, Mood, MoodStore};
use realhires::{i18n, n_, tr};

use crate::icons::{self, Icon};
use crate::library_pages::LibraryEntry;
use crate::look::{self, Palette};
use crate::player::{plural, Page, Player};

/// The pages show new results at most this often while the job runs.
pub const REDRAW: Duration = Duration::from_secs(2);
/// How long the summary of a finished run stays in the sidebar.
pub const STATUS_FOR: Duration = Duration::from_secs(8);
const CARD_W: f32 = 190.0;
const CARD_H: f32 = 104.0;
const GAP: f32 = 14.0;
/// The two parts of the Moods page: its heading and the cards' kind.
pub const SECTIONS: [(&str, &str); 2] = [(n_!("Moods"), "mood"), (n_!("Tempo"), "tempo")];

/// The colour of each card (white text stays readable on all of them; as the Tk app's).
pub fn card_color(label: &str) -> Color32 {
    look::parse_hex(match label {
        "Happy" => "#e8650a",
        "Sad" => "#1a6fd8",
        "Chill" => "#0f8f8c",
        "Intense" => "#e0245e",
        "Up-tempo" => "#8e44ad",
        "Mid-tempo" => "#5856d6",
        _ => "#3f51b5",
    })
}

/// The icon of a card: a smiling or a frowning face, or a metronome for the tempos.
pub fn card_icon(kind: &str, label: &str) -> Icon {
    match (kind, label) {
        ("tempo", _) => Icon::Tempo,
        (_, "Sad" | "Intense") => Icon::MoodSad,
        _ => Icon::Mood,
    }
}

thread_local! {
    static SHOWN: RefCell<Arc<HashMap<String, Mood>>> = RefCell::new(Arc::default());
}

/// The analysis of a song as the window shows it now (for the song lists' cells).
pub fn shown_mood<R>(path: &str, f: impl FnOnce(Option<&Mood>) -> R) -> R {
    SHOWN.with(|s| f(s.borrow().get(path)))
}

fn publish(map: Arc<HashMap<String, Mood>>) {
    SHOWN.with(|s| *s.borrow_mut() = map);
}

/// The BPM and Mood columns of the song lists.
pub fn cell(path: &str, mood_col: bool) -> String {
    shown_mood(path, |m| {
        if mood_col {
            m.filter(|m| !m.mood.is_empty()).map(|m| i18n::tr(&m.mood).to_string()).unwrap_or_default()
        } else {
            moods::bpm_text(m)
        }
    })
}

/// What a click on the BPM heading sorts by (songs not analysed yet first).
pub fn bpm_sort(path: &str) -> String {
    format!("{:06}", shown_mood(path, moods::sort_bpm) + 1)
}

/// The tooltip of the BPM and Mood columns: everything the analysis found, as an estimate.
pub fn mood_tip(path: &str) -> String {
    shown_mood(path, |m| match m {
        None => tr!("Not analysed yet").to_string(),
        Some(m) if m.mood.is_empty() => tr!("No mood: the song is silent or too short.").to_string(),
        Some(m) => {
            let mut parts = vec![i18n::tr(&m.mood).to_string(), i18n::tr(&m.energy_label).to_string()];
            if m.bpm > 0 {
                parts.push(tr!("{bpm} BPM, {tempo}", bpm = m.bpm, tempo = i18n::tr(&m.tempo)));
            }
            if !m.key.is_empty() {
                parts.push(i18n::tr(&moods::key_name(&m.key, &m.mode)).to_string());
            }
            tr!("Estimated from the audio: {what}", what = parts.join(" · "))
        }
    })
}

/// Now Playing's Details rows for a song's mood, tempo and key (labels English, shown
/// with `i18n::tr()`, as `pages::song_details()` gives them).
pub fn detail_rows(m: Option<&Mood>) -> Vec<(&'static str, String)> {
    let dash = || "—".to_string();
    match m {
        None => vec![(n_!("Mood (estimated)"), tr!("Not analysed yet").into())],
        Some(m) if m.mood.is_empty() => vec![(n_!("Mood (estimated)"), tr!("No mood: the song is silent or too short.").into())],
        Some(m) => vec![
            (n_!("Mood (estimated)"), tr!("{a}, {b}", a = i18n::tr(&m.mood), b = i18n::tr(&m.energy_label))),
            (
                n_!("Tempo (estimated)"),
                if m.bpm > 0 { tr!("{bpm} BPM, {tempo}", bpm = m.bpm, tempo = i18n::tr(&m.tempo)) } else { dash() },
            ),
            (n_!("Key (estimated)"), if m.key.is_empty() { dash() } else { i18n::tr(&moods::key_name(&m.key, &m.mode)).to_string() }),
        ],
    }
}

pub enum MoodMsg {
    Progress(usize, usize, String, Option<Mood>),
    Done(Option<(usize, usize)>),
}

/// The player's moods: the results, the background job and what the pages drew last.
pub struct MoodState {
    pub map: Arc<HashMap<String, Mood>>,
    rx: Option<Receiver<MoodMsg>>,
    stop: Arc<AtomicBool>,
    pub progress: Option<(usize, usize)>,
    /// (analysed, failed) of the last run
    pub last: Option<(usize, usize)>,
    again: bool,
    pending: bool,
    shown_at: Instant,
    /// changes with the results
    pub gen: u64,
    /// what Create Playlist made last
    pub message: String,
    /// the summary of a finished run, until it leaves the sidebar
    pub status: Option<(String, Instant)>,
    /// where the Moods page drew each card, and the card page's Create Playlist (for the tests)
    pub card_rects: Vec<(String, Rect)>,
    pub create_rect: Option<Rect>,
    counts: Option<((u64, u64), Vec<(&'static str, usize)>)>,
}

impl Default for MoodState {
    fn default() -> Self {
        MoodState {
            map: Arc::default(),
            rx: None,
            stop: Arc::new(AtomicBool::new(false)),
            progress: None,
            last: None,
            again: false,
            pending: false,
            shown_at: Instant::now(),
            gen: 0,
            message: String::new(),
            status: None,
            card_rects: vec![],
            create_rect: None,
            counts: None,
        }
    }
}

impl Player {
    // ------------------------------------------------------------- the job ---

    /// The moods analysed before (library.sqlite), read when the player starts.
    pub fn load_moods(&mut self) {
        if let Ok(store) = MoodStore::open(None) {
            self.moods.map = Arc::new(store.all());
        }
        publish(self.moods.map.clone());
    }

    pub fn moods_busy(&self) -> bool {
        self.moods.rx.is_some()
    }

    /// Analyse the songs that need it in a thread (after each library update when
    /// `analyse_moods` is on, or by hand with `force`).  False when it did not start.
    pub fn analyse_moods(&mut self, force: bool) -> bool {
        if !force && !self.scanner.cfg.analyse_moods {
            return false;
        }
        if self.moods_busy() {
            self.moods.again = true;
            return false;
        }
        if self.lib.tracks.is_empty() || self.lib_progress.is_some() {
            return false;
        }
        let tracks = self.lib.tracks.clone();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.moods.stop = stop.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new().name("moods".into()).spawn(move || {
            let res = MoodStore::open(None).ok().map(|store| {
                let p = tx.clone();
                let c = ctx.clone();
                moods::run_analysis(
                    &tracks,
                    &store,
                    Some(&stop),
                    &mut |i, n, path, m| {
                        let _ = p.send(MoodMsg::Progress(i, n, path.to_string(), m.cloned()));
                        c.request_repaint();
                    },
                    moods::PAUSE,
                    &moods::analyse_file,
                )
            });
            let _ = tx.send(MoodMsg::Done(res));
            ctx.request_repaint();
        });
        if spawned.is_err() {
            return false;
        }
        self.moods.again = false;
        self.moods.rx = Some(rx);
        self.moods.progress = Some((0, 0));
        true
    }

    pub fn stop_moods(&mut self) {
        self.moods.stop.store(true, Ordering::Relaxed);
    }

    /// `analyse_moods` was turned on or off in Settings.
    pub fn moods_setting_changed(&mut self, on: bool) {
        if on {
            self.analyse_moods(false);
        } else {
            self.stop_moods();
        }
    }

    /// Every frame: the job's results and progress.
    pub fn poll_moods(&mut self) {
        let mut done = None;
        if let Some(rx) = &self.moods.rx {
            let mut fresh: Vec<(String, Mood)> = vec![];
            while let Ok(m) = rx.try_recv() {
                match m {
                    MoodMsg::Progress(i, n, path, mood) => {
                        self.moods.progress = Some((i, n));
                        if let Some(mood) = mood {
                            fresh.push((path, mood));
                        }
                    }
                    MoodMsg::Done(res) => done = Some(res),
                }
            }
            if !fresh.is_empty() {
                let map = Arc::make_mut(&mut self.moods.map);
                for (p, m) in fresh {
                    map.insert(p, m);
                }
                self.moods.pending = true;
            }
        }
        if self.moods.pending && (done.is_some() || self.moods.shown_at.elapsed() >= REDRAW) {
            self.moods.pending = false;
            self.moods.shown_at = Instant::now();
            self.moods_changed();
        } else if self.moods.pending {
            self.ctx.request_repaint_after(REDRAW);
        }
        if let Some(res) = done {
            self.moods.rx = None;
            self.moods.progress = None;
            self.moods.last = res;
            self.moods.status = match res {
                Some((n, _)) if n > 0 => {
                    let text = if n == 1 {
                        tr!("Moods analysed for 1 song.").to_string()
                    } else {
                        tr!("Moods analysed for {n} songs.", n = realhires::stats::thousands(n as i64))
                    };
                    self.ctx.request_repaint_after(STATUS_FOR);
                    Some((text, Instant::now() + STATUS_FOR))
                }
                _ => None,
            };
            if std::mem::take(&mut self.moods.again) {
                self.analyse_moods(false);
            }
        }
    }

    /// New results: the pages, the song lists, the tabs and Details show them.
    fn moods_changed(&mut self) {
        self.moods.gen += 1;
        publish(self.moods.map.clone());
        self.forget_lists();
    }

    /// The sidebar's status while the job runs, then its summary for a while.
    pub fn mood_status_line(&self) -> Option<String> {
        match self.moods.progress {
            Some((i, n)) if i < n => {
                return Some(tr!(
                    "Analysing moods: {done} of {total} songs",
                    done = realhires::stats::thousands(i as i64),
                    total = realhires::stats::thousands(n as i64)
                ))
            }
            Some(_) => return None,
            None => {}
        }
        self.moods.status.as_ref().filter(|(_, until)| Instant::now() < *until).map(|(t, _)| t.clone())
    }

    // -------------------------------------------------------- what is shown ---

    /// The number of songs of every card, worked out once per library and results.
    pub fn mood_counts(&mut self) -> Vec<(&'static str, usize)> {
        let key = (self.lib.gen, self.moods.gen);
        if let Some((k, v)) = &self.moods.counts {
            if *k == key {
                return v.clone();
            }
        }
        let v = moods::card_counts(&self.lib.tracks, &self.moods.map);
        self.moods.counts = Some((key, v.clone()));
        v
    }

    /// A card's songs in the sort chosen for song lists (worked out once per library,
    /// results and sort, never per frame).
    pub fn mood_list(&mut self, kind: &str, label: &str) -> Arc<Vec<String>> {
        let id = format!("mood:{}:{kind}:{label}", self.moods.gen);
        let sort = Some(self.song_sort);
        let (kind, label) = (kind.to_string(), label.to_string());
        self.cached_list(&id, sort, move |p| {
            let paths = moods::card_songs(&kind, &label, &p.lib.tracks, &p.moods.map);
            let songs: Vec<&realhires::musiclib::Track> = paths.iter().filter_map(|x| p.lib.track(x)).collect();
            p.sorted_songs(songs).iter().map(|t| t.path.clone()).collect()
        })
    }

    /// The Library page's Moods entry.
    pub fn moods_library_entry(&self) -> LibraryEntry {
        let n = moods::analysed_count(&self.lib.tracks, &self.moods.map);
        let count = match n {
            0 => tr!("Not analysed yet").to_string(),
            1 => tr!("1 song analysed").to_string(),
            n => tr!("{n} songs analysed", n = realhires::stats::thousands(n as i64)),
        };
        LibraryEntry { icon: Icon::Mood, name: tr!("Moods").to_string(), count, page: Page::Moods }
    }

    /// The Moods page's line under its title: how far the analysis is.
    pub fn moods_status_text(&self) -> String {
        let n = moods::analysed_count(&self.lib.tracks, &self.moods.map);
        let text = tr!(
            "{n} of {total} songs analysed",
            n = realhires::stats::thousands(n as i64),
            total = realhires::stats::thousands(self.lib.tracks.len() as i64)
        );
        if !self.scanner.cfg.analyse_moods {
            tr!("{text}. Analysing is turned off in Settings → Library.", text = text)
        } else if self.moods_busy() {
            tr!("{text}, analysing…", text = text)
        } else {
            text
        }
    }

    /// Create Playlist: an ordinary playlist of a card's songs ("Sad songs"; a taken name
    /// gets " 2"); says what was made.  Returns its id (None without songs).
    pub fn create_mood_playlist(&mut self, kind: &str, label: &str) -> Option<String> {
        let paths = moods::card_songs(kind, label, &self.lib.tracks, &self.moods.map);
        if paths.is_empty() {
            return None;
        }
        let id = self.new_playlist(i18n::tr(&moods::playlist_name(label)), &paths);
        let p = self.state.playlist(&id)?;
        let n = p.songs.len();
        self.moods.message = if n == 1 {
            tr!("Made the playlist “{name}” with 1 song.", name = p.name)
        } else {
            tr!("Made the playlist “{name}” with {n} songs.", name = p.name, n = realhires::stats::thousands(n as i64))
        };
        Some(id)
    }

    // ------------------------------------------------------------- the pages ---

    /// The Moods page: a card per mood and per tempo, each with its colour, number of
    /// songs and meaning; a click opens its songs.  It says the labels are estimates.
    pub fn moods_page(&mut self, ui: &mut Ui, pal: &Palette) {
        let counts = self.mood_counts();
        let status = self.moods_status_text();
        self.moods.card_rects.clear();
        let mut open = None;
        egui::ScrollArea::vertical().id_salt("moods-page").auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new(tr!("Moods")).font(look::title(28.0)).color(pal.text));
            ui.add_space(4.0);
            ui.label(RichText::new(status).color(pal.secondary));
            ui.add_space(2.0);
            ui.label(RichText::new(i18n::tr(moods::ESTIMATE_NOTE)).small().color(pal.secondary));
            for (title, kind) in SECTIONS {
                ui.add_space(14.0);
                ui.label(RichText::new(i18n::tr(title)).font(look::title(18.0)).color(pal.text));
                ui.add_space(8.0);
                let cards: Vec<&str> = moods::CARDS.iter().filter(|(k, _)| *k == kind).map(|(_, l)| *l).collect();
                let w = ui.available_width();
                let cols = (((w + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
                let cw = (w - (cols - 1) as f32 * GAP) / cols as f32;
                for row in cards.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        for label in row {
                            let n = counts.iter().find(|(l, _)| l == label).map_or(0, |(_, n)| *n);
                            let (rect, resp) = ui.allocate_exact_size(vec2(cw, CARD_H), Sense::click());
                            let color = card_color(label);
                            let fill = if resp.hovered() { color.gamma_multiply(0.85) } else { color };
                            let p = ui.painter();
                            p.rect_filled(rect, CornerRadius::same(14), fill);
                            icons::paint(p, Rect::from_center_size(pos2(rect.right() - 26.0, rect.top() + 26.0), vec2(22.0, 22.0)), card_icon(kind, label), Color32::WHITE);
                            let name = crate::pages::elided(ui, i18n::tr(label), look::title(18.0), Color32::WHITE, cw - 56.0);
                            ui.painter().galley(pos2(rect.left() + 16.0, rect.top() + 26.0 - name.size().y / 2.0), name, Color32::WHITE);
                            ui.painter().text(pos2(rect.left() + 16.0, rect.top() + 54.0), Align2::LEFT_CENTER, plural(n, "song"), look::title(13.0), Color32::WHITE);
                            let note = crate::pages::elided(ui, i18n::tr(moods::card_note(label)), look::text(12.0), Color32::WHITE, cw - 28.0);
                            ui.painter().galley(pos2(rect.left() + 16.0, rect.top() + 78.0 - note.size().y / 2.0), note, Color32::WHITE);
                            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                            if resp.clicked() {
                                open = Some((kind, label.to_string()));
                            }
                            self.moods.card_rects.push((label.to_string(), rect));
                        }
                    });
                    ui.add_space(GAP - ui.spacing().item_spacing.y);
                }
            }
            ui.add_space(20.0);
        });
        if let Some((kind, label)) = open {
            self.go(Page::Mood(kind, label));
        }
    }

    /// A card's page: its name, "N songs, M minutes", Play, Shuffle, Create Playlist, a
    /// line saying the labels are estimates, and the songs.
    pub fn mood_page(&mut self, ui: &mut Ui, pal: &Palette, kind: &'static str, label: &str) {
        let list = self.mood_list(kind, label);
        let shown = i18n::tr(label).to_string();
        ui.label(RichText::new(&shown).font(look::title(28.0)).color(pal.text));
        let secs: f64 = list.iter().filter_map(|p| self.lib.track(p)).map(|t| t.duration.max(0.0)).sum();
        let minutes = realhires::pyfmt::round_int(secs / 60.0).max(0) as usize;
        ui.label(RichText::new(tr!("{a}, {b}", a = plural(list.len(), "song"), b = plural(minutes, "minute"))).color(pal.secondary));
        ui.add_space(10.0);
        let mut create = false;
        ui.horizontal_wrapped(|ui| {
            if !list.is_empty() {
                if pill(ui, pal, Icon::Play, tr!("Play"), true).clicked() {
                    self.play_list(list.to_vec(), Some(&list[0]), &shown);
                }
                if pill(ui, pal, Icon::Shuffle, tr!("Shuffle"), true).clicked() {
                    self.shuffle_list(list.to_vec(), &shown);
                }
                let r = pill(ui, pal, Icon::Playlist, tr!("Create Playlist"), false);
                self.moods.create_rect = Some(r.rect);
                if r.clicked() {
                    create = true;
                }
            } else {
                self.moods.create_rect = None;
            }
        });
        if create {
            self.create_mood_playlist(kind, label);
        }
        ui.add_space(6.0);
        ui.label(RichText::new(i18n::tr(moods::ESTIMATE_NOTE)).small().color(pal.secondary));
        if !self.moods.message.is_empty() {
            ui.label(RichText::new(&self.moods.message).small().color(pal.accent));
        }
        ui.add_space(8.0);
        if list.is_empty() {
            ui.label(RichText::new(tr!("No song has this label yet. Songs get their labels while they are analysed.")).color(pal.secondary));
            return;
        }
        self.song_table(ui, pal, &list, &shown, true);
    }
}

/// A rounded button with an icon: filled with the accent colour (Play, Shuffle) or grey.
fn pill(ui: &mut Ui, pal: &Palette, icon: Icon, label: &str, accent: bool) -> egui::Response {
    let font = look::title(13.0);
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x) + 46.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(110.0), 32.0), Sense::click());
    let (fill, fg) = if accent { (pal.accent, pal.on_accent) } else { (pal.lcd, pal.text) };
    let fill = if resp.hovered() { fill.gamma_multiply(0.85) } else { fill };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(18.0, 18.0)), icon, fg);
    ui.painter().text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, font, fg);
    resp
}
