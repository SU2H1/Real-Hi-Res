//! For You in the egui player (2.5, `docs/player-spec.md` "1. For You: playlists made
//! from likes, retention and listening time"), as the Tk player's `player_foryou.py`
//! does it: the mixes of `realhires::recommend` worked out once per library, play
//! history, likes and day (`foryou_mixes()`), the For You page (a card per mix: the
//! mosaic of its first songs, its name, number of songs and description) and a mix's
//! page (Play, Shuffle, Save as Playlist and the songs in the mix's order).

use std::sync::Arc;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Rect, RichText, Sense, Ui};
use realhires::recommend::{self, Mix};
use realhires::{i18n, tr};

use crate::icons::Icon;
use crate::library_pages::{back_link, LibraryEntry};
use crate::look::{self, Palette};
use crate::moodpages::pill;
use crate::pages::elided;
use crate::player::{plural, Page, Player};

const GAP: f32 = 22.0;
/// under a card's mosaic: the name, the number of songs and up to three lines of the description
const TEXT_H: f32 = 92.0;

/// The player's mixes: what was worked out last and for what, and what the pages drew.
#[derive(Default)]
pub struct ForYouState {
    /// (library, likes, history, day) the mixes were made for
    cache: Option<((u64, u64, u64, String), Arc<Vec<Mix>>)>,
    /// what Save as Playlist made last, and on which mix's page
    pub message: String,
    message_mix: String,
    /// where the For You page drew each card (by the mix's id), and the mix page's Save
    /// as Playlist (for the tests)
    pub card_rects: Vec<(String, Rect)>,
    pub save_rect: Option<Rect>,
}

fn clock() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A mix's name in English by its id ("For You" for an id that is none).
pub fn mix_name(id: &str) -> &'static str {
    recommend::MIXES.iter().find(|m| m.0 == id).map_or("For You", |m| m.1)
}

fn mix_note(id: &str) -> &'static str {
    recommend::MIXES.iter().find(|m| m.0 == id).map_or("", |m| m.2)
}

impl Player {
    /// The mixes now (`recommend::mixes()`), made again only after the library, the play
    /// history, a like or a dislike changed, or on another day.
    pub fn foryou_mixes(&mut self) -> Arc<Vec<Mix>> {
        let now = clock();
        let key = (self.lib.gen, self.likes_gen, self.history_gen, recommend::day_of(now));
        if let Some((k, v)) = &self.foryou.cache {
            if *k == key {
                return v.clone();
            }
        }
        let plays = self.history.as_ref().map(|h| h.plays(0.0, None)).unwrap_or_default();
        let mixes = Arc::new(recommend::mixes(&self.lib.tracks, &plays, &self.state.liked, &self.state.disliked, now, &key.3));
        self.foryou.cache = Some((key, mixes.clone()));
        mixes
    }

    /// The songs of a mix in its order (empty when the mix is not there now).
    pub fn mix_songs(&mut self, id: &str) -> Vec<String> {
        self.foryou_mixes().iter().find(|m| m.id == id).map(|m| m.paths.clone()).unwrap_or_default()
    }

    /// What Play of the bar plays on these pages with nothing playing: the mix shown, or
    /// the first one (from the mixes worked out last, which the page shown has asked for).
    pub fn foryou_page_songs(&self, id: Option<&str>) -> Vec<String> {
        let Some((_, mixes)) = &self.foryou.cache else { return vec![] };
        match id {
            Some(id) => mixes.iter().find(|m| m.id == id),
            None => mixes.first(),
        }
        .map(|m| m.paths.clone())
        .unwrap_or_default()
    }

    /// The Library page's For You entry.
    pub fn foryou_library_entry(&mut self) -> LibraryEntry {
        let count = match self.foryou_mixes().len() {
            0 => tr!("No mixes yet").to_string(),
            1 => tr!("1 mix").to_string(),
            n => tr!("{n} mixes", n = n),
        };
        LibraryEntry { icon: Icon::Sparkle, name: tr!("For You").to_string(), count, page: Page::ForYou }
    }

    /// Save as Playlist: an ordinary playlist with the mix's name and songs in its order
    /// (a taken name gets " 2"); says what was made.  Returns its id (None without the
    /// mix).
    pub fn save_mix_playlist(&mut self, id: &str) -> Option<String> {
        let paths = self.mix_songs(id);
        if paths.is_empty() {
            return None;
        }
        let pid = self.new_playlist(i18n::tr(mix_name(id)), &paths);
        let p = self.state.playlist(&pid)?;
        let n = p.songs.len();
        self.foryou.message = if n == 1 {
            tr!("Made the playlist “{name}” with 1 song.", name = p.name)
        } else {
            tr!("Made the playlist “{name}” with {n} songs.", name = p.name, n = realhires::stats::thousands(n as i64))
        };
        self.foryou.message_mix = id.to_string();
        Some(pid)
    }

    // ------------------------------------------------------------- the pages ---

    /// The For You page: one line on how the mixes are made, then a card per mix; a
    /// click opens it.  Without mixes: how to get them.
    pub fn foryou_page(&mut self, ui: &mut Ui, pal: &Palette) {
        let mixes = self.foryou_mixes();
        self.foryou.card_rects.clear();
        let mut open = None;
        egui::ScrollArea::vertical().id_salt("foryou-page").auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new(tr!("For You")).font(look::title(28.0)).color(pal.text));
            ui.add_space(4.0);
            ui.label(RichText::new(i18n::tr(recommend::HOW_NOTE)).color(pal.secondary));
            ui.add_space(18.0);
            if mixes.is_empty() {
                ui.label(RichText::new(i18n::tr(recommend::EMPTY_NOTE)).font(look::title(17.0)).color(pal.text));
                return;
            }
            let width = ui.available_width();
            let cols = ((width + GAP) / (180.0 + GAP)).floor().max(1.0) as usize;
            let size = ((width + GAP) / cols as f32 - GAP).min(220.0);
            for row in mixes.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for mix in row {
                        let (rect, resp) = ui.allocate_exact_size(vec2(size, size + TEXT_H), Sense::click());
                        let crect = Rect::from_min_size(rect.min, vec2(size, size));
                        let p = ui.painter().clone();
                        let name = i18n::tr(mix.name);
                        self.mosaic(&p, crect, &mix.paths, name);
                        if resp.hovered() {
                            p.rect_filled(crect, CornerRadius::same(8), Color32::from_black_alpha(30));
                        }
                        let g = elided(ui, name, look::title(13.0), pal.text, size);
                        p.galley(pos2(rect.left(), crect.bottom() + 6.0), g, pal.text);
                        p.text(pos2(rect.left(), crect.bottom() + 24.0), Align2::LEFT_TOP, plural(mix.paths.len(), "song"), look::text(12.0), pal.secondary);
                        let note = p.layout(i18n::tr(mix.description).to_string(), look::text(11.5), pal.secondary, size);
                        p.with_clip_rect(Rect::from_min_max(pos2(rect.left(), crect.bottom() + 42.0), rect.max))
                            .galley(pos2(rect.left(), crect.bottom() + 42.0), note, pal.secondary);
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            open = Some(mix.id.to_string());
                        }
                        self.foryou.card_rects.push((mix.id.to_string(), rect));
                    }
                });
                ui.add_space(GAP - ui.spacing().item_spacing.y);
            }
        });
        if let Some(id) = open {
            self.go(Page::Mix(id));
        }
    }

    /// A mix's page: the mosaic, its name and description, "N songs, M minutes", Play,
    /// Shuffle and Save as Playlist (the page says what it made), then the songs in the
    /// mix's order with the usual song menu.
    pub fn mix_page(&mut self, ui: &mut Ui, pal: &Palette, id: &str) {
        if back_link(ui, pal, tr!("For You")).clicked() {
            self.go(Page::ForYou);
            return;
        }
        let list = self.mix_songs(id);
        let shown = i18n::tr(mix_name(id)).to_string();
        let mut save = false;
        ui.horizontal(|ui| {
            let (crect, _) = ui.allocate_exact_size(vec2(180.0, 180.0), Sense::hover());
            let p = ui.painter().clone();
            self.mosaic(&p, crect, &list, &shown);
            ui.add_space(22.0);
            ui.vertical(|ui| {
                ui.add_space(26.0);
                ui.label(RichText::new(&shown).font(look::title(26.0)).color(pal.text));
                ui.label(RichText::new(i18n::tr(mix_note(id))).color(pal.secondary));
                let secs: f64 = list.iter().filter_map(|p| self.lib.track(p)).map(|t| t.duration.max(0.0)).sum();
                let minutes = realhires::pyfmt::round_int(secs / 60.0).max(0) as usize;
                ui.label(RichText::new(tr!("{a}, {b}", a = plural(list.len(), "song"), b = plural(minutes, "minute"))).color(pal.secondary));
                ui.add_space(12.0);
                self.foryou.save_rect = None;
                if !list.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        if pill(ui, pal, Icon::Play, tr!("Play"), true).clicked() {
                            self.play_list(list.clone(), Some(&list[0]), &shown);
                        }
                        if pill(ui, pal, Icon::Shuffle, tr!("Shuffle"), true).clicked() {
                            self.shuffle_list(list.clone(), &shown);
                        }
                        let r = pill(ui, pal, Icon::Playlist, tr!("Save as Playlist"), false);
                        self.foryou.save_rect = Some(r.rect);
                        save = r.clicked();
                    });
                }
                if !self.foryou.message.is_empty() && self.foryou.message_mix == id {
                    ui.add_space(4.0);
                    ui.label(RichText::new(&self.foryou.message).small().color(pal.accent));
                }
            });
        });
        if save {
            self.save_mix_playlist(id);
        }
        ui.add_space(16.0);
        if list.is_empty() {
            ui.label(RichText::new(tr!("This mix has no songs at the moment. It comes back as you play and like songs.")).color(pal.secondary));
            return;
        }
        self.song_table(ui, pal, &list, &shown, false);
    }
}

#[cfg(test)]
mod tests {
    //! For You in the egui player, headlessly in a 1024 x 768 window, as
    //! `tests/test_foryou_25.py` checks it in the Tk one: an invented library and
    //! listening history (written straight into the library file), the entries of the
    //! sidebar and the Library page, the For You page's cards, a mix's page with Save as
    //! Playlist (the playlist's songs and order, a taken name gets " 2"), a song played
    //! and logged, a like and a dislike, the history cleared, and Japanese.
    use super::*;
    use egui::{Event, PointerButton, Pos2};
    use realhires::settings::Settings;
    use realhires::{musiclib, playerstate};
    use std::collections::HashMap;
    use std::process::Command;
    use std::time::{Duration, Instant};

    const W: f32 = 1024.0;
    const H: f32 = 768.0;
    /// artist, album, genre, the songs (as `tests/player_library.py`'s library)
    const ALBUMS: [(&str, &str, &str, &[&str]); 4] = [
        ("Aurora Fields", "Northern Lights", "Ambient", &["First Light", "Glacier Song", "Midnight Sun", "Polar Drift"]),
        ("The Paper Boats", "City Rain", "Indie Pop", &["Umbrella Weather", "Taxi Lights", "Last Train Home"]),
        ("June Harbor", "Small Hours", "Jazz", &["Blue Kettle", "Porch Lamp"]),
        ("Velvet Static", "Signal & Noise", "Electronic", &["Carrier Wave", "Low Pass"]),
    ];

    fn song(dir: &std::path::Path, artist: &str, album: &str, genre: &str, title: &str, n: usize) -> String {
        let d = dir.join(artist).join(album);
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join(format!("{n:02} {title}.flac")).to_string_lossy().into_owned();
        let src = format!("sine=frequency={}:sample_rate=44100:duration=3", 300 + 40 * n);
        let meta = [format!("title={title}"), format!("artist={artist}"), format!("album={album}"), format!("track={n}"), format!("genre={genre}")];
        let mut args: Vec<&str> = vec!["-v", "error", "-y", "-f", "lavfi", "-i", &src];
        for m in &meta {
            args.extend(["-metadata", m.as_str()]);
        }
        args.push(&out);
        assert!(Command::new(realhires::probe::tool("ffmpeg")).args(&args).status().unwrap().success());
        realhires::paths::abspath(&out)
    }

    fn run(ctx: &egui::Context, p: &mut Player, events: Vec<Event>) -> Vec<(String, Rect)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(W, H))), events, ..Default::default() };
        let output = ctx.run_ui(raw, |ui| p.ui(ui));
        let mut out = vec![];
        output.shapes.iter().for_each(|s| walk(&s.shape, &mut out));
        output.drop_without_applying_deltas();
        out
    }

    fn texts(ctx: &egui::Context, p: &mut Player) -> Vec<String> {
        for _ in 0..3 {
            run(ctx, p, vec![]);
        }
        run(ctx, p, vec![]).into_iter().map(|(t, _)| t).collect()
    }

    fn click(ctx: &egui::Context, p: &mut Player, at: Pos2) {
        run(ctx, p, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            run(ctx, p, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }]);
        }
        run(ctx, p, vec![Event::PointerMoved(pos2(5.0, H - 5.0))]);
    }

    fn run_until(ctx: &egui::Context, p: &mut Player, what: &str, done: impl Fn(&Player) -> bool) {
        let t0 = Instant::now();
        while !done(p) {
            assert!(t0.elapsed() < Duration::from_secs(120), "timed out waiting for {what}");
            run(ctx, p, vec![]);
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn has(t: &[String], s: &str) -> bool {
        t.iter().any(|x| x == s)
    }

    /// The invented listening history, written straight into the library file (the songs
    /// are three seconds long; the history says four minutes, as real songs are).
    fn invent_history(p: &mut Player, s: &HashMap<&str, String>) {
        let now = clock();
        let plays: [(&str, f64, f64, usize); 9] = [
            ("First Light", 1.0, 240.0, 5), // Aurora Fields: heard to the end this week
            ("Glacier Song", 1.0, 240.0, 5),
            ("Midnight Sun", 1.0, 240.0, 3),
            ("Polar Drift", 1.0, 240.0, 2),
            ("Umbrella Weather", 1.0, 10.0, 6), // skipped after ten seconds every time
            ("Taxi Lights", 2.0, 240.0, 2),
            ("Blue Kettle", 100.0, 240.0, 4), // heard to the end long ago
            ("Carrier Wave", 1.0, 240.0, 2),  // heard, and disliked
            ("Low Pass", 1.0, 240.0, 1),
        ]; // Last Train Home and Porch Lamp: never played
        for (title, days_ago, seconds, n) in plays {
            let track = musiclib::Track { duration: 240.0, ..p.lib.track(&s[title]).unwrap().clone() };
            for k in 0..n {
                assert!(p.history.as_ref().unwrap().add_play(&track, now - (days_ago + k as f64) * 86400.0, seconds));
            }
        }
        p.history_gen += 1;
    }

    #[test]
    fn for_you_pages_mixes_and_save_as_playlist() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        i18n::set_thread_language("en");
        let tmp = std::env::temp_dir().join(format!("rhr-foryou-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var(realhires::smarttabs::TABS_ENV, tmp.join("tabs"));
        std::env::set_var(realhires::covers::COVERS_ENV, tmp.join("covers"));
        let mut s: HashMap<&str, String> = HashMap::new();
        for (artist, album, genre, titles) in ALBUMS {
            for (i, title) in titles.iter().enumerate() {
                s.insert(title, song(&music, artist, album, genre, title, i + 1));
            }
        }
        let title_of: HashMap<String, &str> = s.iter().map(|(t, p)| (p.clone(), *t)).collect();
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        cfg.theme = "Light".into();
        cfg.analyse_moods = false; // not this test's business
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 11 && p.library_status().is_none());

        // ---- the Library page's entry after Playlists, the sidebar's above Liked Songs
        let entries = p.library_entries();
        let i = entries.iter().position(|e| e.page == Page::ForYou).unwrap();
        assert_eq!((entries[i - 1].page.clone(), entries[i + 1].page.clone()), (Page::Playlists, Page::Liked));
        assert_eq!((entries[i].name.as_str(), entries[i].count.as_str(), entries[i].icon), ("For You", "No mixes yet", Icon::Sparkle));
        p.go(Page::Library);
        for _ in 0..3 {
            run(&ctx, &mut p, vec![]);
        }
        let drawn = run(&ctx, &mut p, vec![]);
        let side = |text: &str| drawn.iter().find(|(t, r)| t == text && r.right() < 220.0).unwrap_or_else(|| panic!("no {text} in the sidebar")).1;
        let (fy, liked, lists) = (side("For You"), side("Liked Songs"), side("Playlists"));
        assert!(lists.top() < fy.top() && fy.top() < liked.top() && liked.top() - fy.top() < 40.0, "{lists:?} {fy:?} {liked:?}");
        click(&ctx, &mut p, fy.center());
        assert_eq!(p.page, Page::ForYou);
        let t = texts(&ctx, &mut p);
        assert!(has(&t, recommend::HOW_NOTE) && has(&t, recommend::EMPTY_NOTE), "{t:?}");
        assert!(p.foryou.card_rects.is_empty() && p.foryou_mixes().is_empty());

        // ---- an invented history, two likes and a dislike: three mixes
        invent_history(&mut p, &s);
        p.toggle_like(&s["First Light"]);
        p.toggle_like(&s["Porch Lamp"]);
        p.toggle_dislike(&s["Carrier Wave"]);
        let mixes = p.foryou_mixes();
        assert_eq!(mixes.iter().map(|m| m.id).collect::<Vec<_>>(), ["favourites", "on_repeat", "daily"]);
        let fav = mixes[0].paths.clone();
        let names: Vec<&str> = fav.iter().map(|x| title_of[x]).collect();
        assert_eq!(names[0], "First Light", "liked and heard to the end five times");
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, ["Blue Kettle", "First Light", "Glacier Song", "Low Pass", "Midnight Sun", "Polar Drift", "Porch Lamp", "Taxi Lights"]);
        let pos = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(pos("Glacier Song") < pos("Blue Kettle"), "this week before a hundred days ago");
        let artists: Vec<String> = fav.iter().map(|x| p.lib.track(x).unwrap().artist.clone()).collect();
        assert!(artists.windows(2).all(|w| w[0] != w[1]), "no artist twice in a row: {artists:?}");
        for m in mixes.iter() {
            assert!(!m.paths.contains(&s["Umbrella Weather"]) && !m.paths.contains(&s["Carrier Wave"]), "{}: skipped or disliked", m.id);
        }
        assert_eq!(mixes[1].paths.len(), 6);
        assert!(mixes[2].paths.contains(&s["Last Train Home"]), "never played, by an artist the user listens to");
        // the same mixes as the Tk player makes from this history (the core's vectors hold both to it)
        let plays = p.history.as_ref().unwrap().plays(0.0, None);
        let again = recommend::mixes(&p.lib.tracks, &plays, &p.state.liked, &p.state.disliked, clock(), &recommend::day_of(clock()));
        assert_eq!(*mixes, again);

        // ---- the For You page: a card per mix, inside the 1024 px window
        let t = texts(&ctx, &mut p);
        for m in mixes.iter() {
            assert!(has(&t, m.name) && has(&t, m.description), "{}: {t:?}", m.name);
        }
        assert!(has(&t, "8 songs") && has(&t, "6 songs") && !has(&t, recommend::EMPTY_NOTE), "{t:?}");
        let cards = p.foryou.card_rects.clone();
        assert_eq!(cards.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["favourites", "on_repeat", "daily"]);
        assert!(cards.iter().all(|(_, r)| r.left() > 220.0 && r.right() <= W && r.width() >= 150.0), "{cards:?}");
        assert!(cards[0].1.top() == cards[2].1.top() && cards[0].1.right() < cards[1].1.left(), "side by side: {cards:?}");
        assert_eq!(p.foryou_library_entry().count, "3 mixes");
        assert_eq!(p.page_songs(), fav, "Play of the bar: the first mix");

        // ---- a mix's page
        click(&ctx, &mut p, cards[0].1.center());
        assert_eq!(p.page, Page::Mix("favourites".into()));
        assert_eq!(p.page_label(&p.page.clone()), "Favourites Mix");
        let t = texts(&ctx, &mut p);
        for want in ["Favourites Mix", mixes[0].description, "8 songs, 0 minutes", "Play", "Shuffle", "Save as Playlist", "First Light", "Porch Lamp"] {
            assert!(has(&t, want), "{want}: {t:?}");
        }
        assert_eq!(p.page_songs(), fav);
        // the songs in the mix's order
        let drawn = run(&ctx, &mut p, vec![]);
        let row = |title: &str| drawn.iter().find(|(t, r)| t == title && r.left() > 220.0 && r.top() > 250.0).unwrap_or_else(|| panic!("no row {title}")).1.top();
        let tops: Vec<f32> = names.iter().map(|n| row(n)).collect();
        assert!(tops.windows(2).all(|w| w[0] < w[1]), "{names:?} at {tops:?}");

        // ---- Save as Playlist: the mix's name, songs and order; again: " 2"
        let save = p.foryou.save_rect.expect("Save as Playlist");
        assert!(save.right() <= W, "{save:?}");
        click(&ctx, &mut p, save.center());
        assert_eq!(p.state.playlists.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Favourites Mix"]);
        assert_eq!(p.state.playlists[0].songs, fav);
        assert_eq!(p.foryou.message, "Made the playlist “Favourites Mix” with 8 songs.");
        assert!(has(&texts(&ctx, &mut p), "Made the playlist “Favourites Mix” with 8 songs."));
        assert_eq!(p.page, Page::Mix("favourites".into()), "it stays on the mix");
        click(&ctx, &mut p, save.center());
        assert_eq!(p.state.playlists.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Favourites Mix", "Favourites Mix 2"]);
        assert_eq!(playerstate::load(None).playlists[0].songs, fav, "saved");

        // ---- Play: the mix from its first song; the next song logs the play, the mixes are made again
        let drawn = run(&ctx, &mut p, vec![]);
        let play = drawn.iter().find(|(t, r)| t == "Play" && r.left() > 220.0 && r.top() > 70.0).expect("Play").1;
        click(&ctx, &mut p, play.center());
        assert_eq!(p.now.as_ref().map(|n| n.path.clone()), Some(fav[0].clone()));
        assert_eq!(p.queue_name, "Favourites Mix");
        run_until(&ctx, &mut p, "a second of the first song", |p| p.engine.position().unwrap_or(0.0) >= 1.3);
        let (before, gen, logged) = (p.foryou_mixes(), p.history_gen, p.history.as_ref().unwrap().plays(0.0, None).len());
        p.next();
        run_until(&ctx, &mut p, "the play in the history", |p| p.history_gen > gen);
        assert_eq!(p.history.as_ref().unwrap().plays(0.0, None).len(), logged + 1);
        assert_eq!(p.now.as_ref().map(|n| n.path.clone()), Some(fav[1].clone()));
        let after = p.foryou_mixes();
        assert!(!Arc::ptr_eq(&before, &after), "made again after a play");
        assert_eq!(after.iter().map(|m| m.id).collect::<Vec<_>>(), ["favourites", "on_repeat", "daily"]);
        p.engine.pause();

        // ---- a dislike takes the song out at once, a like brings one in
        let gone = after[0].paths[2].clone();
        p.toggle_dislike(&gone);
        let t = texts(&ctx, &mut p);
        assert!(has(&t, "7 songs, 0 minutes") && !p.mix_songs("favourites").contains(&gone), "{t:?}");
        p.toggle_like(&s["Last Train Home"]);
        assert!(p.mix_songs("favourites").contains(&s["Last Train Home"]) && p.mix_songs("favourites").len() == 8);
        p.toggle_dislike(&gone);
        assert!(p.mix_songs("favourites").contains(&gone));

        // ---- Back returns to For You; the Daily Mix opens too
        p.go_back();
        assert_eq!(p.page, Page::ForYou);
        texts(&ctx, &mut p);
        let daily = p.foryou.card_rects.iter().find(|(id, _)| id == "daily").unwrap().1;
        click(&ctx, &mut p, daily.center());
        assert_eq!(p.page, Page::Mix("daily".into()));
        let t = texts(&ctx, &mut p);
        assert!(has(&t, "Daily Mix") && !t.iter().any(|x| x.starts_with("Made the playlist")), "{t:?}");
        assert!(p.mix_songs("daily").len() >= 5);

        // ---- the history cleared: the likes' artists are New For You; without the likes nothing is left
        p.clear_history();
        let left = p.foryou_mixes();
        assert_eq!(left.iter().map(|m| m.id).collect::<Vec<_>>(), ["discover", "daily"]);
        assert!(left[0].paths.contains(&s["Glacier Song"]) && !left[0].paths.contains(&s["First Light"]));
        for name in ["First Light", "Porch Lamp", "Last Train Home"] {
            p.toggle_like(&s[name]);
        }
        assert!(p.state.liked.is_empty() && p.foryou_mixes().is_empty());
        let t = texts(&ctx, &mut p);
        assert!(has(&t, "0 songs, 0 minutes") && !has(&t, "Save as Playlist") && p.foryou.save_rect.is_none(), "{t:?}");
        assert_eq!(p.save_mix_playlist("daily"), None);
        p.go(Page::ForYou);
        assert!(has(&texts(&ctx, &mut p), recommend::EMPTY_NOTE));

        // ---- Japanese, with the history again
        invent_history(&mut p, &s);
        p.toggle_like(&s["Porch Lamp"]);
        i18n::set_thread_language("ja");
        let t = texts(&ctx, &mut p);
        for want in ["おすすめ", "お気に入りミックス", "ヘビーローテーション", "デイリーミックス", "8曲"] {
            assert!(has(&t, want), "{want}: {t:?}");
        }
        assert!(!has(&t, recommend::HOW_NOTE) && !has(&t, "Favourites Mix") && !has(&t, "For You"), "{t:?}");
        assert_eq!(p.foryou_library_entry().count, "ミックス3個");
        p.go(Page::Mix("favourites".into()));
        let t = texts(&ctx, &mut p);
        assert!(has(&t, "お気に入りミックス") && has(&t, "プレイリストとして保存") && has(&t, "8曲、0分"), "{t:?}");
        assert!(p.foryou.save_rect.unwrap().right() <= W);
        let id = p.save_mix_playlist("favourites").unwrap();
        assert_eq!(p.state.playlist(&id).unwrap().name, "お気に入りミックス");
        assert_eq!(p.foryou.message, "8曲のプレイリスト「お気に入りミックス」を作成しました。");
        assert_eq!(p.page_label(&Page::Mix("daily".into())), "デイリーミックス");
        i18n::set_thread_language("en");
        p.on_exit();
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
