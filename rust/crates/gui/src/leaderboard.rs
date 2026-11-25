//! 2.4 (`docs/player-spec.md`, "2.4" item 2) in the Rust player: the names on Your
//! Stats as links, and the leaderboard: a section of Your Stats and the "Most Played"
//! page (`Page::MostPlayed`), as the Python app's `realhires/player_stats.py`.
//!
//! Every song, artist, album and genre named on Your Stats (the cards, the top lists of
//! Details, the day beside the calendar, the leaderboard) is a link (`Link`): a song
//! plays it with the songs of that list as the queue, an artist opens the artist's page,
//! an album its page, a genre its page.  The pointer is a hand over a link, and a right
//! click on a song opens the song menu.
//!
//! The leaderboard ranks the most played songs, artists or albums of the period chosen
//! on Your Stats (`stats::leaderboard()`): rank (the top three on a gold, silver and
//! bronze badge), cover, title, artist, plays and minutes listened, up to 100 rows, ties
//! sharing a rank.  Its switch is `Player::leaderboard_kind`, kept for the session and
//! shared by the section and the page; Play / Shuffle play the ranked songs in rank
//! order (`stats::ranked_songs()`).
//!
//! 2.5 (item 4): every card of Your Stats is one click target (`Player::card_click()`:
//! the top song, artist, album and genre open Most Played with that kind, the busiest
//! day is chosen in the calendar, the others open Details).  The leaderboard has a fourth
//! kind, Genres, and a "Sort by" switch (Times played / Time listened: `MostState::by`,
//! shared by the section and the page; the number sorted by is drawn in the text
//! colour).  Most Played has a period control of its own (`MostState::span`, a unit of
//! `stats::SPAN_UNITS` and an offset: Day, Week, Month, Year, All time, with ‹ and ›),
//! which starts from the period of Your Stats when the page is opened from there
//! (`Player::open_most_played()`).  The switches wrap onto more lines on a narrow page.
//! Item 5: `quality_tip()` is the quality tooltip of the playback bar's badge and of the
//! Format row of Now Playing's Details (`format_row_tip()`).

use std::sync::Arc;

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Ui};
use realhires::i18n::{self, tr_name, tr_text};
use realhires::pyunicode::casefold;
use realhires::stats::{self, LeaderRow};
use realhires::{musiclib, n_, tr};

use crate::look::{self, Palette};
use crate::pages::{elided, pill_button};
use crate::player::{count, cover, Page, Player};
use crate::icons::Icon;

/// The rank badges of the top three: gold, silver, bronze.
pub const MEDALS: [Color32; 3] = [Color32::from_rgb(0xd4, 0xa0, 0x17), Color32::from_rgb(0x9a, 0xa0, 0xa6), Color32::from_rgb(0xc2, 0x7a, 0x3a)];
/// The rows of the leaderboard section of Your Stats (the Most Played page shows up to `stats::TOP`).
pub const SECTION_ROWS: usize = 10;
/// A leaderboard row and its cover.
pub const ROW_H: f32 = 52.0;
const COVER: f32 = 40.0;
const KIND_LABELS: [(&str, &str); 4] = [("songs", n_!("Songs")), ("artists", n_!("Artists")), ("albums", n_!("Albums")), ("genres", n_!("Genres"))];
/// 2.5: "Sort by", and the units of Most Played's period control.
const SORT_LABELS: [(&str, &str); 2] = [("plays", n_!("Times played")), ("time", n_!("Time listened"))];
const UNIT_LABELS: [(&str, &str); 5] = [("day", n_!("Day")), ("week", n_!("Week")), ("month", n_!("Month")), ("year", n_!("Year")), ("all", n_!("All time"))];
/// A switch's height.
const SEG_H: f32 = 30.0;
/// How far a card of Your Stats rises under the pointer.
pub const CARD_LIFT: f32 = 3.0;

/// The period's leaderboards, by kind.
pub type Boards = Arc<Vec<(&'static str, Vec<LeaderRow>)>>;

/// 2.5: the leaderboard's "Sort by" and Most Played's period (kept for the session), the
/// rows of the period shown, and where a card of Your Stats asked a page to scroll.
pub struct MostState {
    /// "plays" or "time" (`stats::LEADERBOARD_SORTS`), shared by the section and the page
    pub by: &'static str,
    /// the period Most Played shows: a unit of `stats::SPAN_UNITS` and an offset (0 = the present)
    pub span: (&'static str, i32),
    /// (kind, sort, since, until, history generation) of the rows read, the rows, and
    /// whether the history holds plays before the period (‹ leads somewhere)
    cache: Option<(MostKey, Vec<LeaderRow>, bool)>,
    /// Details scrolls to this section when it is drawn next ("top", "hour", "weekday")
    pub details_goto: Option<&'static str>,
    /// Your Stats scrolls to its top (the calendar) when it is drawn next
    pub stats_top: bool,
    /// where ‹ and › of the period control were drawn last (None when it has none)
    pub nav: Option<(Rect, Rect)>,
}

type MostKey = (&'static str, &'static str, u64, u64, u64);

impl Default for MostState {
    fn default() -> Self {
        MostState { by: "plays", span: ("all", 0), cache: None, details_goto: None, stats_top: false, nav: None }
    }
}

/// What a click on a card of Your Stats does (2.5).
#[derive(Clone, Debug, PartialEq)]
pub enum CardTarget {
    /// Most Played with this kind
    Most(&'static str),
    /// Details, scrolled to this section ("top", "hour", "weekday")
    Details(&'static str),
    /// this day chosen in the calendar
    Day(i32, u32, u32),
}

/// Where a card of Your Stats leads (`label` is its English label): the top song, artist,
/// album and genre to Most Played with that kind, the biggest day to the calendar, the
/// others to Details (the listening hour and day at their charts).
pub fn card_target(label: &str, st: &stats::Stats) -> CardTarget {
    match label {
        "Your top song" => CardTarget::Most("songs"),
        "Your top artist" => CardTarget::Most("artists"),
        "Your top album" => CardTarget::Most("albums"),
        "Your top genre" => CardTarget::Most("genres"),
        "Your listening hour" => CardTarget::Details("hour"),
        "Your listening day" => CardTarget::Details("weekday"),
        "Your biggest day" => {
            let day: Vec<u32> = st.busiest_day.as_ref().map(|(d, _)| d.split('-').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
            match day[..] {
                [y, m, d] if y > 0 => CardTarget::Day(y as i32, m, d),
                _ => CardTarget::Details("top"),
            }
        }
        _ => CardTarget::Details("top"),
    }
}

/// The name of a span in the language in use: "25 November 2026", "23 – 29 November
/// 2026", "November 2026", "2026", "All time".
pub fn span_text(unit: &str, offset: i32) -> String {
    let title = stats::span_title(unit, offset, now());
    match unit {
        "year" => tr!("{year}", year = title),
        "all" => i18n::tr(&title).to_string(),
        _ => tr_text(&title),
    }
}

/// The quality tooltip of a song (2.5): its resolution and bit rate, in the language in use.
pub fn quality_tip(t: &musiclib::Track) -> String {
    tr_text(&musiclib::quality_detail(t))
}

/// Now Playing's Details: the Format row (at `rect`) shows the song's quality tooltip.
pub fn format_row_tip(ui: &mut Ui, rect: Rect, t: &musiclib::Track) {
    ui.interact(rect, ui.scope_id().with("details-format-tip"), Sense::hover()).on_hover_text_at_pointer(quality_tip(t));
}

fn text_w(ui: &Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.ctx().fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE).size().x)
}

/// A segmented switch, with `label` before it: a rounded panel, the chosen one of
/// `options` ((key, shown text)) raised in the page's colour.  It is one item of the
/// layout, so a wrapping row keeps it together.  Returns the option clicked.
fn segments(ui: &mut Ui, pal: &Palette, id: &str, label: Option<&str>, options: &[(&'static str, &str)], value: &str) -> Option<&'static str> {
    let (font, lfont) = (look::title(13.0), look::text(13.0));
    let lw = label.map(|l| text_w(ui, l, &lfont) + 10.0).unwrap_or(0.0);
    let widths: Vec<f32> = options.iter().map(|(_, t)| text_w(ui, i18n::tr(t), &font) + 24.0).collect();
    let (rect, _) = ui.allocate_exact_size(vec2(lw + widths.iter().sum::<f32>() + 4.0, SEG_H), Sense::hover());
    let p = ui.painter().clone();
    if let Some(l) = label {
        p.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, l, lfont, pal.secondary);
    }
    let panel = Rect::from_min_max(pos2(rect.left() + lw, rect.top()), rect.max);
    p.rect_filled(panel, CornerRadius::same(9), pal.lcd);
    let mut x = panel.left() + 2.0;
    let mut chosen = None;
    for ((key, text), sw) in options.iter().zip(&widths) {
        let r = Rect::from_min_size(pos2(x, panel.top() + 2.0), vec2(*sw, SEG_H - 4.0));
        let on = *key == value;
        let resp = ui.interact(r, ui.scope_id().with((id, *key)), if on { Sense::hover() } else { Sense::click() });
        if on {
            p.rect_filled(r, CornerRadius::same(7), pal.bg);
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(7), pal.hover);
        }
        p.text(r.center(), Align2::CENTER_CENTER, i18n::tr(text), font.clone(), if on { pal.text } else { pal.secondary });
        if !on && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            chosen = Some(*key);
        }
        x += sw;
    }
    chosen
}

/// ‹ name › of Most Played's period: the step asked for (-1, 0 or +1).  The name gets the
/// room of a long name of its unit, so the buttons stay where they are from one period to
/// the next.
fn span_nav(ui: &mut Ui, pal: &Palette, unit: &str, title: &str, back: bool, next: bool) -> (i32, (Rect, Rect)) {
    let font = look::title(13.0);
    let sample = match unit {
        "day" => tr_text("28 September 2026"),
        "week" => tr_text("28 September – 4 October 2026"),
        "month" => tr_text("September 2026"),
        _ => tr!("{year}", year = "2026"),
    };
    let tw = text_w(ui, &sample, &font).max(text_w(ui, title, &font)) + 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(30.0 + tw + 30.0, SEG_H), Sense::hover());
    let p = ui.painter().clone();
    let mut step = 0;
    for (cx, delta, on, tip) in [(rect.left() + 15.0, -1, back, tr!("Previous period")), (rect.right() - 15.0, 1, next, tr!("Next period"))] {
        let r = Rect::from_center_size(pos2(cx, rect.center().y), vec2(30.0, 30.0));
        let resp = ui.interact(r, ui.scope_id().with(("span-step", delta)), if on { Sense::click() } else { Sense::hover() });
        p.rect_filled(r, CornerRadius::same(8), if on && resp.hovered() { pal.hover } else { pal.lcd });
        crate::library_pages::chevron(&p, r.center(), 18.0, delta > 0, if on { pal.accent } else { pal.disabled });
        if on && resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            step = delta;
        }
    }
    p.text(rect.center(), Align2::CENTER_CENTER, title, font, pal.text);
    let button = |cx: f32| Rect::from_center_size(pos2(cx, rect.center().y), vec2(30.0, 30.0));
    (step, (button(rect.left() + 15.0), button(rect.right() - 15.0)))
}

/// What a click on a name of Your Stats does.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// play `path`, with `list` as the queue named `name`
    Play { list: Vec<String>, path: String, name: &'static str },
    /// open a page (an artist, album or genre)
    Open(Page),
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

impl Player {
    // ------------------------------------------------------------- data ---

    /// The leaderboards of the period chosen on Your Stats (sorted as "Sort by" says), made
    /// again when the period, the sort or the plays change.
    pub fn boards(&mut self) -> Boards {
        let by = self.most.by;
        let key = format!("{}\u{0}{by}", self.stats_period);
        if let Some((p, g, b)) = &self.boards_cache {
            if *p == key && *g == self.history_gen {
                return b.clone();
            }
        }
        let (since, until) = stats::period_range(&self.stats_period, now());
        let plays = self.history.as_ref().map(|h| h.plays(since, until)).unwrap_or_default();
        let b: Boards = Arc::new(stats::LEADERBOARD_KINDS.iter().map(|k| (*k, stats::leaderboard_by(&plays, k, stats::TOP, by))).collect());
        self.boards_cache = Some((key, self.history_gen, b.clone()));
        b
    }

    /// Most Played's period (2.5): a unit of `stats::SPAN_UNITS` and an offset, never past the present.
    pub fn most_span(&self) -> (&'static str, i32) {
        match stats::SPAN_UNITS.iter().copied().find(|u| *u == self.most.span.0) {
            Some(unit) if unit != "all" => (unit, self.most.span.1.min(0)),
            _ => ("all", 0),
        }
    }

    /// The leaderboard of Most Played's own period (of the kind and sort chosen), and
    /// whether the history holds plays before that period; read again when any of them or
    /// the plays change.
    pub fn most_rows(&mut self) -> (Vec<LeaderRow>, bool) {
        let (unit, offset) = self.most_span();
        let (since, until) = stats::span_range(unit, offset, now());
        let key: MostKey = (self.leaderboard_kind, self.most.by, since.to_bits(), until.unwrap_or(f64::INFINITY).to_bits(), self.history_gen);
        if let Some((k, rows, older)) = &self.most.cache {
            if *k == key {
                return (rows.clone(), *older);
            }
        }
        let plays = self.history.as_ref().map(|h| h.plays(since, until)).unwrap_or_default();
        let rows = stats::leaderboard_by(&plays, self.leaderboard_kind, stats::TOP, self.most.by);
        let first = if unit == "all" { None } else { self.history.as_ref().and_then(|h| h.first_play()) };
        let older = first.is_some_and(|t| t < since);
        self.most.cache = Some((key, rows.clone(), older));
        (rows, older)
    }

    /// Most Played, opened from Your Stats (See All, a card): with this kind, starting
    /// from the period chosen there (`stats::period_span()`).
    pub fn open_most_played(&mut self, kind: Option<&'static str>) {
        if let Some(k) = kind.and_then(|k| stats::LEADERBOARD_KINDS.iter().copied().find(|x| *x == k)) {
            self.leaderboard_kind = k;
        }
        self.most.span = stats::period_span(&self.stats_period);
        self.go(Page::MostPlayed);
    }

    /// A click on a card of Your Stats (2.5).
    pub fn card_click(&mut self, target: CardTarget) {
        match target {
            CardTarget::Most(kind) => self.open_most_played(Some(kind)),
            CardTarget::Details(section) => {
                self.most.details_goto = Some(section);
                self.go(Page::StatsDetails);
            }
            CardTarget::Day(y, m, d) => {
                self.calendar.month = (y, m);
                self.calendar.day = Some((y, m, d));
                self.most.stats_top = true;
            }
        }
    }

    /// The period's leaderboard of `kind`.
    pub fn board(&mut self, kind: &str) -> Vec<LeaderRow> {
        self.boards().iter().find(|(k, _)| *k == kind).map(|(_, rows)| rows.clone()).unwrap_or_default()
    }

    // ------------------------------------------------------------ links ---

    /// The page an artist's name opens (Artists, else Album Artists), as `open_artist()`
    /// finds it; None when the library has no such artist.
    pub fn artist_link(&self, name: &str) -> Option<Link> {
        let mut names = vec![name.to_string()];
        names.extend(musiclib::split_artists(name, &self.lib.known).into_iter().take(1));
        for n in &names {
            for kind in ["artists", "album_artists"] {
                if let Some(i) = self.lib.find_group(kind, n) {
                    return Some(Link::Open(Page::Group(kind, self.lib.groups(kind)[i].name.clone())));
                }
            }
        }
        None
    }

    pub fn genre_link(&self, name: &str) -> Option<Link> {
        self.lib.find_group("genres", name).map(|i| Link::Open(Page::Group("genres", self.lib.genres[i].name.clone())))
    }

    /// The album page of a row of Your Stats: the album of one of its songs, else the
    /// album of that name (and artist).
    pub fn album_link(&self, name: &str, artist: &str, paths: &[String]) -> Option<Link> {
        if let Some(a) = paths.iter().find_map(|p| self.lib.album_of(p)) {
            return Some(Link::Open(Page::Album(a.key.clone())));
        }
        let (n, a) = (casefold(name.trim()), casefold(artist.trim()));
        let named: Vec<&musiclib::Album> = self.lib.albums.iter().filter(|x| casefold(x.title.trim()) == n).collect();
        named.iter().find(|x| casefold(x.artist.trim()) == a).or(named.first()).map(|x| Link::Open(Page::Album(x.key.clone())))
    }

    /// A song as a link: it plays with `list` as the queue (None when its file is gone).
    pub fn song_link(&self, list: &[String], path: &str, name: &'static str) -> Option<Link> {
        self.lib.by_path.contains_key(path).then(|| Link::Play { list: list.to_vec(), path: path.to_string(), name })
    }

    /// Do what a link says.
    pub fn follow(&mut self, link: Link) {
        match link {
            Link::Play { mut list, path, name } => {
                if !list.contains(&path) {
                    list.push(path.clone());
                }
                let mut seen = std::collections::HashSet::new();
                list.retain(|p| seen.insert(p.clone()));
                self.play_list(list, Some(&path), name);
            }
            Link::Open(page) => self.go(page),
        }
    }

    /// `rect` (a name drawn there) as a link: the hand pointer, a click returns the link,
    /// a right click on a song (`song`) opens the song menu.
    pub fn link_rect(&mut self, ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, link: Option<Link>, song: Option<&str>) -> Option<Link> {
        if link.is_none() && song.is_none() {
            return None;
        }
        let resp = ui.interact(rect, ui.scope_id().with(("stats-link", id)), Sense::click());
        let resp = if link.is_some() { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
        if let Some(path) = song.filter(|p| self.lib.by_path.contains_key(*p)) {
            let path = path.to_string();
            resp.context_menu(|ui| self.song_menu(ui, &path));
        }
        if resp.clicked() {
            link
        } else {
            None
        }
    }

    /// A name drawn as a link where the layout puts it (elided to `width`).
    pub fn link_label(
        &mut self,
        ui: &mut Ui,
        text: &str,
        font: egui::FontId,
        color: Color32,
        width: f32,
        link: Option<Link>,
        song: Option<&str>,
    ) -> Option<Link> {
        let g = elided(ui, text, font, color, width);
        let (rect, _) = ui.allocate_exact_size(g.size(), Sense::hover());
        ui.painter().galley(rect.min, g, color);
        self.link_rect(ui, rect, (text, rect.min.x as i32, rect.min.y as i32), link, song)
    }

    // ------------------------------------------------------- leaderboard ---

    /// The leaderboard's controls, wrapped onto more lines when the page is narrow: the
    /// switch (Songs, Artists, Albums, Genres), "Sort by" (Times played / Time listened)
    /// and, with `span` (Most Played), the period's units (Day … All time) and ‹ name ›
    /// for the first four (‹ only with `can_go_back`, › not at the present).
    fn board_controls(&mut self, ui: &mut Ui, pal: &Palette, span: bool, can_go_back: bool) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(18.0, 10.0);
            if let Some(kind) = segments(ui, pal, "board-kind", None, &KIND_LABELS, self.leaderboard_kind) {
                self.leaderboard_kind = kind;
            }
            if let Some(by) = segments(ui, pal, "board-sort", Some(tr!("Sort by")), &SORT_LABELS, self.most.by) {
                self.most.by = by;
            }
            if span {
                let (unit, offset) = self.most_span();
                self.most.nav = None;
                if let Some(u) = segments(ui, pal, "board-unit", None, &UNIT_LABELS, unit) {
                    self.most.span = (u, 0);
                } else if unit != "all" {
                    let (step, buttons) = span_nav(ui, pal, unit, &span_text(unit, offset), can_go_back, offset < 0);
                    self.most.nav = Some(buttons);
                    if step != 0 {
                        self.most.span = (unit, offset + step);
                    }
                }
            }
        });
    }

    /// The cover of a leaderboard row: the album of its (most played) song.
    fn row_cover(&mut self, kind: &str, r: &LeaderRow) -> Option<egui::TextureHandle> {
        let album = if kind == "albums" {
            match self.album_link(&r.name, &r.artist, &r.paths) {
                Some(Link::Open(Page::Album(key))) => self.lib.album_index.get(&key).copied(),
                _ => None,
            }
        } else {
            None
        }
        .or_else(|| r.paths.iter().find_map(|p| self.lib.album_of(p).and_then(|a| self.lib.album_index.get(&a.key).copied())));
        let i = album?;
        let key = self.lib.albums[i].key.clone();
        let paths = self.lib.album_paths[i].clone();
        self.covers.get(&key, &paths)
    }

    /// One leaderboard row, `w` wide: rank (the top three on a medal), cover, name and
    /// artist (links), plays and minutes.
    fn board_row(&mut self, ui: &mut Ui, pal: &Palette, kind: &str, r: &LeaderRow, i: usize, queue: &[String], name: &'static str) -> Option<Link> {
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
        // the whole row of a song opens its menu on a right click (the links come after it, on top)
        let row = (kind == "songs").then(|| ui.interact(rect, ui.scope_id().with(("board-row", kind, i)), Sense::click()));
        let p = ui.painter_at(rect);
        if i % 2 == 1 {
            p.rect_filled(rect.shrink2(vec2(0.0, 1.0)), CornerRadius::same(8), pal.zebra);
        }
        let mid = rect.center().y;
        let badge = pos2(rect.left() + 19.0, mid);
        if r.rank <= 3 {
            p.circle_filled(badge, 13.0, MEDALS[r.rank - 1]);
            p.text(badge, Align2::CENTER_CENTER, r.rank.to_string(), look::title(13.0), Color32::WHITE);
        } else {
            p.text(badge, Align2::CENTER_CENTER, r.rank.to_string(), look::title(13.0), pal.secondary);
        }
        let tex = self.row_cover(kind, r);
        cover(&p, Rect::from_min_size(pos2(rect.left() + 42.0, mid - COVER / 2.0), vec2(COVER, COVER)), tex.as_ref(), &r.name, 5);
        // 2.5: the number the leaderboard is sorted by in the text colour, the other in secondary text
        let plays = count(r.plays.max(0) as usize, n_!("1 play"), n_!("{n} plays"));
        let minutes = stats::thousands(r.minutes);
        let filled = |s: &str| s.replace("{plays}", &plays).replace("{minutes}", &minutes);
        let template = tr!("{plays} · {minutes} min");
        let parts: Vec<&str> = template.split(" · ").collect();
        let mut left = rect.right() - 12.0;
        if parts.len() == 2 {
            for i in [1, 0] {
                let strong = parts[i].contains(if self.most.by == "time" { "{minutes}" } else { "{plays}" });
                let color = if strong { pal.text } else { pal.secondary };
                left = p.text(pos2(left, mid), Align2::RIGHT_CENTER, filled(parts[i]), look::text(12.0), color).left();
                if i == 1 {
                    left = p.text(pos2(left, mid), Align2::RIGHT_CENTER, " · ", look::text(12.0), pal.secondary).left();
                }
            }
        } else {
            left = p.text(pos2(left, mid), Align2::RIGHT_CENTER, filled(template), look::text(12.0), pal.secondary).left();
        }
        let room = (left - (rect.left() + 94.0) - 16.0).max(20.0);
        let playing = kind == "songs" && self.now.as_ref().is_some_and(|n| n.path == r.key);
        let color = if playing { pal.accent } else { pal.text };
        let (shown, link, song) = match kind {
            "songs" => (r.name.clone(), self.song_link(queue, &r.key, name), Some(r.key.as_str())),
            "artists" => (tr_name(&r.name).to_string(), self.artist_link(&r.name), None),
            "genres" => (tr_name(&r.name).to_string(), self.genre_link(&r.name), None),
            _ => (tr_name(&r.name).to_string(), self.album_link(&r.name, &r.artist, &r.paths), None),
        };
        let one_line = matches!(kind, "artists" | "genres");
        let mut out = None;
        let g = elided(ui, &shown, look::title(13.0), color, room);
        let y = if one_line { mid - g.size().y / 2.0 } else { mid - g.size().y + 1.0 };
        let nrect = Rect::from_min_size(pos2(rect.left() + 94.0, y), g.size());
        ui.painter().galley(nrect.min, g, color);
        if let Some(l) = self.link_rect(ui, nrect, ("board-name", kind, i), link, song) {
            out = Some(l);
        }
        if !one_line {
            let g = elided(ui, tr_name(&r.artist), look::text(12.0), pal.secondary, room);
            let arect = Rect::from_min_size(pos2(rect.left() + 94.0, mid + 2.0), g.size());
            ui.painter().galley(arect.min, g, pal.secondary);
            let link = self.artist_link(&r.artist);
            if let Some(l) = self.link_rect(ui, arect, ("board-artist", kind, i), link, None) {
                out = Some(l);
            }
        }
        if let (Some(resp), Some(path)) = (row, song.filter(|p| self.lib.by_path.contains_key(*p))) {
            let path = path.to_string();
            resp.context_menu(|ui| self.song_menu(ui, &path));
        }
        out
    }

    /// Leaderboard rows (up to `limit`), or "No plays yet in this period.".
    fn board_rows(&mut self, ui: &mut Ui, pal: &Palette, rows: &[LeaderRow], limit: usize, name: &'static str) -> Option<Link> {
        if rows.is_empty() {
            ui.label(egui::RichText::new(tr!("No plays yet in this period.")).color(pal.secondary));
            return None;
        }
        let kind = self.leaderboard_kind;
        let queue = stats::ranked_songs(rows);
        let mut out = None;
        for (i, r) in rows.iter().take(limit).enumerate() {
            if let Some(l) = self.board_row(ui, pal, kind, r, i, &queue, name) {
                out = Some(l);
            }
        }
        out
    }

    /// The leaderboard of Your Stats: "Leaderboard", See All (the Most Played page), the
    /// switch, Sort by and the top rows of the period.
    pub fn leaderboard_section(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr!("Leaderboard")).font(look::title(17.0)).color(pal.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let see = ui.add(egui::Label::new(egui::RichText::new(tr!("See All")).font(look::title(13.0)).color(pal.accent)).sense(Sense::click()));
                if see.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.open_most_played(None);
                }
            });
        });
        ui.add_space(6.0);
        self.board_controls(ui, pal, false, true);
        ui.add_space(8.0);
        let rows = self.board(self.leaderboard_kind);
        if let Some(l) = self.board_rows(ui, pal, &rows, SECTION_ROWS, "Your Stats") {
            self.follow(l);
        }
        ui.add_space(12.0);
    }

    /// The leaderboard as a page of its own ("Most Played"): Back, Play and Shuffle (the
    /// ranked songs in rank order), the switch, Sort by, the period control (2.5) and up
    /// to 100 rows of that period.
    pub fn most_played_page(&mut self, ui: &mut Ui, pal: &Palette) {
        if crate::library_pages::back_link(ui, pal, tr!("Your Stats")).clicked() {
            self.go(Page::Stats);
            return;
        }
        let (rows, older) = self.most_rows();
        let ranked = stats::ranked_songs(&rows);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(tr!("Most Played")).font(look::title(28.0)).color(pal.text));
            ui.add_space(16.0);
            if !ranked.is_empty() {
                if pill_button(ui, pal, Some(Icon::Play), tr!("Play"), true, true).clicked() {
                    self.play_list(ranked.clone(), Some(&ranked[0]), "Most Played");
                }
                if pill_button(ui, pal, Some(Icon::Shuffle), tr!("Shuffle"), true, true).clicked() {
                    self.shuffle_list(ranked.clone(), "Most Played");
                }
            }
        });
        ui.add_space(10.0);
        self.board_controls(ui, pal, true, older);
        ui.add_space(8.0);
        // the controls may have changed the kind, the sort or the period
        let (rows, _) = self.most_rows();
        if rows.is_empty() {
            ui.label(egui::RichText::new(tr!("Nothing was played in this period.")).color(pal.secondary));
            return;
        }
        let kind = self.leaderboard_kind;
        let queue = stats::ranked_songs(&rows);
        let mut out = None;
        egui::ScrollArea::vertical().id_salt("most-played").auto_shrink([false, false]).show_rows(ui, ROW_H, rows.len().min(stats::TOP), |ui, range| {
            for i in range {
                if let Some(l) = self.board_row(ui, pal, kind, &rows[i], i, &queue, "Most Played") {
                    out = Some(l);
                }
            }
        });
        if let Some(l) = out {
            self.follow(l);
        }
    }
}

#[cfg(test)]
mod tests {
    //! 2.4 items 1 and 2 in the Rust player, headlessly in a 1024 x 768 window (the CI
    //! runners' screens): the Folders page scrolls as one list with the mouse wheel, the
    //! trackpad and the scroll bar; the names on Your Stats are links (cards, Details,
    //! the day beside the calendar) with the hand pointer and the song menu; the
    //! leaderboard section and the Most Played page (switch, ranks, Play, the page's name
    //! on Now Playing's back link), also in Japanese.
    use super::*;
    use egui::{Event, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, TouchPhase};
    use realhires::settings::Settings;
    use realhires::{musiclib, playerstate};
    use std::process::Command;
    use std::time::{Duration, Instant};

    const W: f32 = 1024.0;
    const H: f32 = 768.0;

    fn song(dir: &std::path::Path, album: &str, title: &str, artist: &str, genre: &str, n: u32) -> String {
        let d = dir.join(album);
        std::fs::create_dir_all(&d).unwrap();
        let out = d.join(format!("{n:02} {title}.flac")).to_string_lossy().into_owned();
        let src = format!("sine=frequency={}:sample_rate=44100:duration=2", 300 + 40 * n);
        let meta = [format!("title={title}"), format!("artist={artist}"), format!("album={album}"), format!("track={n}"), format!("genre={genre}")];
        let mut args: Vec<&str> = vec!["-v", "error", "-y", "-f", "lavfi", "-i", &src];
        for m in &meta {
            args.extend(["-metadata", m.as_str()]);
        }
        args.push(&out);
        assert!(Command::new(realhires::probe::tool("ffmpeg")).args(&args).status().unwrap().success());
        realhires::paths::abspath(&out)
    }

    /// One frame of the 1024 x 768 window with these events: every text drawn (and where),
    /// the narrow filled rectangles (a scroll bar's handle) and the pointer's shape.
    struct Drawn {
        texts: Vec<(String, Rect)>,
        /// the colour of each text (of its first section), in the order of `texts`
        inks: Vec<Color32>,
        bars: Vec<Rect>,
        cursor: egui::CursorIcon,
    }

    fn run(ctx: &egui::Context, p: &mut Player, events: Vec<Event>) -> Drawn {
        run_sized(ctx, p, events, W, H)
    }

    /// One frame of a window of this size.
    fn run_sized(ctx: &egui::Context, p: &mut Player, events: Vec<Event>, w: f32, h: f32) -> Drawn {
        fn walk(shape: &egui::Shape, d: &mut Drawn) {
            match shape {
                egui::Shape::Text(t) => {
                    d.texts.push((t.galley.text().to_string(), t.visual_bounding_rect()));
                    let ink = t.galley.job.sections.first().map(|s| s.format.color).filter(|c| *c != Color32::PLACEHOLDER);
                    d.inks.push(ink.unwrap_or(t.fallback_color));
                }
                egui::Shape::Rect(r) if r.rect.width() <= 12.0 && r.rect.height() >= 16.0 && r.rect.left() > 600.0 => {
                    d.bars.push(r.rect)
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, d)),
                _ => {}
            }
        }
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h))), events, ..Default::default() };
        let out = ctx.run_ui(raw, |ui| p.ui(ui));
        let mut d = Drawn { texts: vec![], inks: vec![], bars: vec![], cursor: out.platform_output.cursor_icon };
        out.shapes.iter().for_each(|s| walk(&s.shape, &mut d));
        out.drop_without_applying_deltas();
        d
    }

    fn frames(ctx: &egui::Context, p: &mut Player) -> Drawn {
        run(ctx, p, vec![]);
        run(ctx, p, vec![]);
        run(ctx, p, vec![])
    }

    fn shown(d: &Drawn, text: &str) -> bool {
        d.texts.iter().any(|(t, r)| t == text && r.bottom() > 0.0 && r.top() < H)
    }

    /// Where the text `text` was drawn on the visible page (the first such text when `first`).
    fn at(d: &Drawn, text: &str, pick: impl Fn(&Rect) -> bool) -> Pos2 {
        let names: Vec<&str> = d.texts.iter().map(|(t, _)| t.as_str()).collect();
        d.texts
            .iter()
            .find(|(t, r)| t == text && r.bottom() > 70.0 && r.top() < H && pick(r))
            .unwrap_or_else(|| panic!("no {text:?} in {names:?}"))
            .1
            .center()
    }

    /// Scroll the page (the wheel over its middle) until `text` (one that `pick` accepts) is
    /// well inside the window: down first, then up.
    fn reveal(ctx: &egui::Context, p: &mut Player, text: &str, pick: impl Fn(&Rect) -> bool) -> Drawn {
        let inside = |d: &Drawn| d.texts.iter().any(|(t, r)| t == text && r.top() > 200.0 && r.bottom() < H - 60.0 && pick(r));
        let mid = pos2(620.0, 420.0);
        // the page scrolls on for a few frames after a wheel step: wait until it stands still
        let settled = |ctx: &egui::Context, p: &mut Player| {
            let mut last = frames(ctx, p);
            for _ in 0..120 {
                let d = run(ctx, p, vec![]);
                let same = d.texts.len() == last.texts.len() && d.texts.iter().zip(&last.texts).all(|(a, b)| a.1 == b.1);
                last = d;
                if same {
                    break;
                }
            }
            last
        };
        for (dy, steps) in [(-150.0, 14), (150.0, 30)] {
            let d = settled(ctx, p);
            if inside(&d) {
                return d;
            }
            for _ in 0..steps {
                run(ctx, p, vec![Event::PointerMoved(mid), wheel(mid, dy, MouseWheelSource::Wheel, TouchPhase::Move)]);
                let d = settled(ctx, p);
                if inside(&d) {
                    return d;
                }
            }
        }
        panic!("{text:?} never came into view");
    }

    fn press(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos, button, pressed, modifiers: Default::default() }
    }

    fn click(ctx: &egui::Context, p: &mut Player, pos: Pos2, button: PointerButton) {
        run(ctx, p, vec![Event::PointerMoved(pos)]);
        run(ctx, p, vec![press(pos, button, true)]);
        run(ctx, p, vec![press(pos, button, false)]);
    }

    fn wheel(_pos: Pos2, dy: f32, source: MouseWheelSource, phase: TouchPhase) -> Event {
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, dy), phase, source, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn folders_scroll_stats_links_and_most_played() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        realhires::i18n::set_thread_language("English");
        let tmp = std::env::temp_dir().join(format!("rhr-stats-24-{}", std::process::id()));
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
        let a1 = song(&music, "Paper Lanterns", "Lanterns Over Water", "Mira Calloway", "Pop", 1);
        let a2 = song(&music, "Paper Lanterns", "Paper in the Rain", "Mira Calloway", "Pop", 2);
        let b1 = song(&music, "Harbour Lights", "Tidewater", "The Northern Quay", "Jazz", 1);
        // a folder of 25 subfolders (a song each) and 30 songs of its own
        let many = music.join("Many");
        for i in 0..25 {
            let d = many.join(format!("Sub {i:02}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::copy(&a2, d.join("song.flac")).unwrap();
        }
        for i in 0..30 {
            std::fs::copy(&b1, many.join(format!("{i:02} direct.flac"))).unwrap();
        }
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        cfg.theme = "Light".into();
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        let t0 = Instant::now();
        while !(p.lib.tracks.len() == 58 && p.library_status().is_none()) {
            assert!(t0.elapsed() < Duration::from_secs(120), "the library index");
            run(&ctx, &mut p, vec![]);
            std::thread::sleep(Duration::from_millis(20));
        }

        // ---- 1. Folders: the subfolders and the songs are one list that scrolls
        let many_dir = realhires::paths::abspath(&many.to_string_lossy());
        p.go(Page::Folders(Some(many_dir.clone())));
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "Sub 00") && !shown(&d, "Sub 24"), "the first subfolders, not the last");
        assert!(shown(&d, "Title"), "the songs' headings above the list");
        let list = at(&d, "Sub 03", |_| true);
        // the mouse wheel: the subfolders scroll away and the songs come up in the same list
        run(&ctx, &mut p, vec![Event::PointerMoved(list), wheel(list, -500.0, MouseWheelSource::Wheel, TouchPhase::Move)]);
        let d = frames(&ctx, &mut p);
        assert!(!shown(&d, "Sub 00") && shown(&d, "Sub 24") && shown(&d, "Tidewater"), "scrolled by the wheel");
        let tide = at(&d, "Tidewater", |_| true).y;
        let sub24 = at(&d, "Sub 24", |_| true).y;
        assert!(tide > sub24, "the songs below the subfolders");
        // the trackpad (smooth scrolling) moves the same list on
        run(&ctx, &mut p, vec![wheel(list, 0.0, MouseWheelSource::Trackpad, TouchPhase::Start)]);
        for _ in 0..5 {
            run(&ctx, &mut p, vec![wheel(list, -60.0, MouseWheelSource::Trackpad, TouchPhase::Move)]);
        }
        run(&ctx, &mut p, vec![wheel(list, 0.0, MouseWheelSource::Trackpad, TouchPhase::End)]);
        let d = frames(&ctx, &mut p);
        let moved = d.texts.iter().filter(|(t, _)| t == "Sub 24").map(|(_, r)| r.center().y).next();
        assert!(moved.is_none_or(|y| y < sub24 - 200.0), "scrolled by the trackpad: {sub24} -> {moved:?}");
        // the scroll bar: its handle dragged to the bottom shows the last songs
        let d = run(&ctx, &mut p, vec![Event::PointerMoved(pos2(W - 5.0, 400.0))]);
        let d2 = frames(&ctx, &mut p);
        let bar = d.bars.iter().chain(d2.bars.iter()).copied().max_by(|a, b| a.height().total_cmp(&b.height())).expect("a scroll bar");
        run(&ctx, &mut p, vec![Event::PointerMoved(bar.center())]);
        run(&ctx, &mut p, vec![press(bar.center(), PointerButton::Primary, true)]);
        for k in 1..=10 {
            run(&ctx, &mut p, vec![Event::PointerMoved(pos2(bar.center().x, bar.center().y + 60.0 * k as f32))]);
        }
        run(&ctx, &mut p, vec![press(pos2(bar.center().x, H - 2.0), PointerButton::Primary, false)]);
        let d = frames(&ctx, &mut p);
        let tides: Vec<f32> = d.texts.iter().filter(|(t, r)| t == "Tidewater" && r.top() < H).map(|(_, r)| r.bottom()).collect();
        assert!(d.texts.iter().all(|(t, _)| !t.starts_with("Sub ")) && tides.iter().any(|y| *y > H - 80.0), "dragged to the end: {tides:?}");
        // the root folder: only subfolders (no headings); a click opens one
        p.go(Page::Folders(Some(realhires::paths::abspath(&music.to_string_lossy()))));
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "Many") && !shown(&d, "Title"));
        click(&ctx, &mut p, at(&d, "Many", |r| r.top() > 150.0), PointerButton::Primary);
        assert_eq!(p.page, Page::Folders(Some(many_dir.clone())));

        // ---- 2. Your Stats: plays of three songs, all time
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        for (i, (path, secs)) in [(&a1, 60.0), (&a1, 60.0), (&a1, 60.0), (&a1, 60.0), (&b1, 60.0), (&b1, 60.0), (&a2, 40.0)].iter().enumerate() {
            let track = p.lib.track(path).unwrap().clone();
            assert!(p.history.as_ref().unwrap().add_play(&track, now - 600.0 * (i as f64 + 1.0), *secs));
        }
        p.history_gen += 1;
        p.stats_period = "All time".into();
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "Leaderboard", |_| true);
        assert!(shown(&d, "See All") && shown(&d, "Songs") && shown(&d, "Artists"));
        let ranks = p.board("songs");
        assert_eq!(ranks.iter().map(|r| (r.rank, r.key.clone())).collect::<Vec<_>>(), vec![(1, a1.clone()), (2, b1.clone()), (3, a2.clone())]);
        // the top song's card: the hand pointer.  Since 2.5 the whole card is one link that
        // opens Most Played with Songs (the 2.5 test below); the name is not a link of its own
        let d = reveal(&ctx, &mut p, "Lanterns Over Water", |r| r.height() > 16.0);
        let card_song = at(&d, "Lanterns Over Water", |r| r.height() > 16.0);
        let d = run(&ctx, &mut p, vec![Event::PointerMoved(card_song)]);
        let d = if d.cursor == egui::CursorIcon::PointingHand { d } else { run(&ctx, &mut p, vec![Event::PointerMoved(card_song)]) };
        assert_eq!(d.cursor, egui::CursorIcon::PointingHand, "the hand over a card");
        click(&ctx, &mut p, card_song, PointerButton::Primary);
        assert!(p.now.is_none(), "the card does not play the song");
        assert_eq!((p.page.clone(), p.leaderboard_kind), (Page::MostPlayed, "songs"));
        p.go_back();
        // the top artist's card opens Most Played with Artists
        let d = reveal(&ctx, &mut p, "Mira Calloway", |r| r.height() > 16.0);
        click(&ctx, &mut p, at(&d, "Mira Calloway", |r| r.height() > 16.0), PointerButton::Primary);
        assert_eq!((p.page.clone(), p.leaderboard_kind), (Page::MostPlayed, "artists"));
        p.go_back();
        p.leaderboard_kind = "songs";
        // the leaderboard: a right click on a song opens its menu
        let d = reveal(&ctx, &mut p, "Tidewater", |r| r.height() < 16.0 && r.left() > 300.0);
        let row = at(&d, "Tidewater", |r| r.height() < 16.0 && r.left() > 300.0);
        click(&ctx, &mut p, row, PointerButton::Secondary);
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "Play Next") && shown(&d, "Add to Queue"), "the song menu");
        run(&ctx, &mut p, vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        let d = frames(&ctx, &mut p);
        assert!(!shown(&d, "Play Next"), "Esc closed the menu");
        // the switch: Albums, a click on an album opens it; an artist name under it opens the artist
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "Albums", |r| r.left() > 240.0);
        click(&ctx, &mut p, at(&d, "Albums", |r| r.left() > 240.0), PointerButton::Primary);
        assert_eq!(p.leaderboard_kind, "albums");
        let d = reveal(&ctx, &mut p, "Paper Lanterns", |r| r.height() < 16.0);
        let album_key = p.lib.album_of(&a1).unwrap().key.clone();
        click(&ctx, &mut p, at(&d, "Paper Lanterns", |r| r.height() < 16.0), PointerButton::Primary);
        assert_eq!(p.page, Page::Album(album_key.clone()));
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "The Northern Quay", |r| r.height() < 16.0);
        click(&ctx, &mut p, at(&d, "The Northern Quay", |r| r.height() < 16.0), PointerButton::Primary);
        assert_eq!(p.page, Page::Group("artists", "The Northern Quay".into()));
        // See All: the Most Played page, its name on Now Playing's back link
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "See All", |_| true);
        click(&ctx, &mut p, at(&d, "See All", |_| true), PointerButton::Primary);
        assert_eq!(p.page, Page::MostPlayed);
        assert_eq!(p.page_label(&Page::MostPlayed), "Most Played");
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "Most Played") && shown(&d, "Paper Lanterns") && shown(&d, "Harbour Lights"));
        click(&ctx, &mut p, at(&d, "Songs", |r| r.left() > 240.0), PointerButton::Primary);
        assert_eq!(p.leaderboard_kind, "songs");
        let d = frames(&ctx, &mut p);
        for (n, t) in [("1", "Lanterns Over Water"), ("2", "Tidewater"), ("3", "Paper in the Rain")] {
            let (rank, row) = (at(&d, n, |r| r.left() < 400.0 && r.top() > 150.0), at(&d, t, |_| true));
            assert!((rank.y - row.y).abs() < 14.0, "rank {n} beside {t}");
        }
        click(&ctx, &mut p, at(&d, "Play", |r| r.top() < 200.0 && r.left() > 240.0), PointerButton::Primary);
        assert_eq!(p.queue_name, "Most Played");
        assert_eq!(p.queue.as_ref().unwrap().context, vec![a1.clone(), b1.clone(), a2.clone()]);
        assert_eq!(p.now.as_ref().map(|n| n.path.clone()), Some(a1.clone()), "rank order");
        assert_eq!(p.queue_name, "Most Played");
        p.engine.pause();
        p.go(Page::NowPlaying);
        assert_eq!(p.back_label().as_deref(), Some("Most Played"));
        p.go_back();

        // ---- Details: the top lists' names are links
        p.go(Page::StatsDetails);
        let d = reveal(&ctx, &mut p, "Jazz", |_| true);
        click(&ctx, &mut p, at(&d, "Jazz", |_| true), PointerButton::Primary);
        assert_eq!(p.page, Page::Group("genres", "Jazz".into()));
        p.go(Page::StatsDetails);
        let d = reveal(&ctx, &mut p, "Harbour Lights", |_| true);
        click(&ctx, &mut p, at(&d, "Harbour Lights", |_| true), PointerButton::Primary);
        assert_eq!(p.page, Page::Album(p.lib.album_of(&b1).unwrap().key.clone()));

        // ---- the day beside the calendar: the artist is a link
        let today = realhires::stats::local_parts(now - 600.0);
        p.calendar.month = (today.year, today.month);
        p.calendar.day = Some((today.year, today.month, today.day));
        p.go(Page::Stats);
        let title = realhires::stats::day_title(today.year, today.month, today.day);
        let d = reveal(&ctx, &mut p, &title, |_| true);
        let below = at(&d, &title, |_| true).y;
        let day_artist = d
            .texts
            .iter()
            .filter(|(t, r)| t == "The Northern Quay" && r.top() > below && r.top() < below + 300.0 && r.bottom() < H - 30.0)
            .map(|(_, r)| r.center())
            .next()
            .expect("the artist in the day's list");
        click(&ctx, &mut p, day_artist, PointerButton::Primary);
        assert_eq!(p.page, Page::Group("artists", "The Northern Quay".into()));

        // ---- in Japanese
        realhires::i18n::set_thread_language("日本語");
        p.go(Page::MostPlayed);
        let d = frames(&ctx, &mut p);
        let ja = realhires::i18n::tr("Most Played");
        assert!(ja != "Most Played" && shown(&d, ja), "{ja}");
        assert!(shown(&d, realhires::i18n::tr("Songs")) && shown(&d, realhires::i18n::tr("Albums")));
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, realhires::i18n::tr("Leaderboard"), |_| true);
        assert!(shown(&d, realhires::i18n::tr("See All")) && realhires::i18n::tr("Leaderboard") != "Leaderboard");
        realhires::i18n::set_thread_language("English");
        p.on_exit();
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ------------------------------------------------------------------ 2.5 ---

    /// A library of five songs (four albums, four artists and genres) in a temp folder,
    /// indexed: (the folder, the context, the player, the songs' paths).
    fn library(tag: &str) -> (std::path::PathBuf, egui::Context, Player, Vec<String>) {
        let tmp = std::env::temp_dir().join(format!("rhr-stats-25-{tag}-{}", std::process::id()));
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
        let paths = vec![
            song(&music, "Paper Lanterns", "Lanterns Over Water", "Mira Calloway", "Pop", 1),
            song(&music, "Paper Lanterns", "Paper in the Rain", "Mira Calloway", "Pop", 2),
            song(&music, "Harbour Lights", "Tidewater", "The Northern Quay", "Jazz", 1),
            song(&music, "Static Bloom", "Carrier", "Juniper Vale", "Rock", 1),
            song(&music, "Old Roads", "Milestone", "Wren Hollis", "Folk", 1),
        ];
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        cfg.theme = "Light".into();
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        let t0 = Instant::now();
        while !(p.lib.tracks.len() == paths.len() && p.library_status().is_none()) {
            assert!(t0.elapsed() < Duration::from_secs(120), "the library index");
            run(&ctx, &mut p, vec![]);
            std::thread::sleep(Duration::from_millis(20));
        }
        (tmp, ctx, p, paths)
    }

    fn ink(d: &Drawn, text: &str) -> Color32 {
        let i = d.texts.iter().position(|(t, r)| t == text && r.bottom() > 70.0 && r.top() < H).unwrap_or_else(|| panic!("no {text:?}"));
        d.inks[i]
    }

    fn names(p: &mut Player) -> Vec<String> {
        p.most_rows().0.into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn cards_lead_on_and_most_played_sorts_and_filters() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        realhires::i18n::set_thread_language("English");
        let (tmp, ctx, mut p, s) = library("cards");
        let (a1, a2, b1, c1, d1) = (s[0].clone(), s[1].clone(), s[2].clone(), s[3].clone(), s[4].clone());
        // today: a1 three times (2 minutes in all) and b1 once for 10 minutes; yesterday: a2
        // twice; 40 days ago: c1; June of last year: d1
        let now = super::now();
        let day0 = stats::span_range("day", 0, now).0;
        let today = stats::local_parts(now - 6.0);
        let last_year = stats::local_midnight(today.year - 1, 6, 15) + 43_200.0;
        for (path, started, secs) in [
            (&a1, now - 9.0, 40.0),
            (&a1, now - 8.0, 40.0),
            (&a1, now - 7.0, 40.0),
            (&b1, now - 6.0, 600.0),
            (&a2, day0 - 3600.0, 60.0),
            (&a2, day0 - 3500.0, 60.0),
            (&c1, now - 40.0 * 86_400.0, 120.0),
            (&d1, last_year, 200.0),
        ] {
            let track = p.lib.track(path).unwrap().clone();
            assert!(p.history.as_ref().unwrap().add_play(&track, started, secs));
        }
        p.history_gen += 1;
        assert!(p.history.as_ref().unwrap().first_play().unwrap() < now - 300.0 * 86_400.0);
        p.stats_period = "All time".into();
        p.go(Page::Stats);

        // ---- the cards: one click target each (the hand), leading on
        let big = |r: &Rect| r.height() > 16.0;
        for (name, kind) in [("Lanterns Over Water", "songs"), ("Mira Calloway", "artists"), ("Paper Lanterns", "albums"), ("Pop", "genres")] {
            let d = reveal(&ctx, &mut p, name, big);
            let at_name = at(&d, name, big);
            let d = run(&ctx, &mut p, vec![Event::PointerMoved(at_name)]);
            let d = if d.cursor == egui::CursorIcon::PointingHand { d } else { run(&ctx, &mut p, vec![Event::PointerMoved(at_name)]) };
            assert_eq!(d.cursor, egui::CursorIcon::PointingHand, "the hand over the card of {name}");
            // under the pointer the card lifts
            let lifted = run(&ctx, &mut p, vec![]);
            assert!((at(&lifted, name, big).y - (at_name.y - CARD_LIFT)).abs() < 0.6, "{name}: the card lifts");
            click(&ctx, &mut p, at_name, PointerButton::Primary);
            assert!(p.now.is_none(), "a card plays nothing");
            assert_eq!((p.page.clone(), p.leaderboard_kind, p.most_span()), (Page::MostPlayed, kind, ("all", 0)), "{name}");
            p.go_back();
            assert_eq!(p.page, Page::Stats);
        }
        // Genres: the genres ranked, each a link to its page
        p.open_most_played(Some("genres"));
        assert_eq!(names(&mut p), ["Pop", "Jazz", "Folk", "Rock"]);
        let d = frames(&ctx, &mut p);
        click(&ctx, &mut p, at(&d, "Jazz", |r| r.left() > 300.0 && r.top() > 150.0), PointerButton::Primary);
        assert_eq!(p.page, Page::Group("genres", "Jazz".into()));
        // the other cards open Details (the listening hour at its chart)
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "Minutes listened", |_| true);
        click(&ctx, &mut p, at(&d, "Minutes listened", |_| true), PointerButton::Primary);
        assert_eq!(p.page, Page::StatsDetails);
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "Totals"), "Details from its top");
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "Your listening hour", |_| true);
        click(&ctx, &mut p, at(&d, "Your listening hour", |_| true), PointerButton::Primary);
        assert_eq!(p.page, Page::StatsDetails);
        let mut d = frames(&ctx, &mut p);
        for _ in 0..60 {
            d = run(&ctx, &mut p, vec![]); // the page scrolls there over a few frames
        }
        let hours = at(&d, "By hour of the day (minutes)", |_| true);
        assert!(hours.y < 260.0 && !shown(&d, "Totals"), "Details at the hours: {hours:?}");
        // the biggest day: chosen in the calendar, which is scrolled to
        p.go(Page::Stats);
        p.calendar.month = (2020, 1);
        p.calendar.day = None;
        let d = reveal(&ctx, &mut p, "Your biggest day", |_| true);
        assert!(!shown(&d, &stats::month_title(today.year, today.month)));
        click(&ctx, &mut p, at(&d, "Your biggest day", |_| true), PointerButton::Primary);
        assert_eq!(p.page, Page::Stats);
        assert_eq!((p.calendar.month, p.calendar.day), ((today.year, today.month), Some((today.year, today.month, today.day))));
        let mut d = frames(&ctx, &mut p);
        for _ in 0..60 {
            d = run(&ctx, &mut p, vec![]);
        }
        assert!(shown(&d, &stats::month_title(today.year, today.month)), "the calendar is in view");
        assert!(shown(&d, &stats::day_title(today.year, today.month, today.day)), "the day's plays");

        // ---- opened from Your Stats, Most Played starts from that page's period
        for (period, span) in [("Last 30 days", ("month", 0)), ("This year", ("year", 0)), ("Last year", ("year", -1)), ("All time", ("all", 0))] {
            p.stats_period = period.into();
            p.go(Page::Stats);
            let d = reveal(&ctx, &mut p, "See All", |_| true);
            click(&ctx, &mut p, at(&d, "See All", |_| true), PointerButton::Primary);
            assert_eq!((p.page.clone(), p.most_span()), (Page::MostPlayed, span), "{period}");
            if period == "Last year" {
                assert_eq!(names(&mut p), ["Folk"], "last year's genres");
            }
        }

        // ---- Sort by: the number sorted by in the text colour, the other in secondary text
        let pal = look::current(&ctx);
        let d = frames(&ctx, &mut p);
        click(&ctx, &mut p, at(&d, "Songs", |r| r.left() > 240.0), PointerButton::Primary);
        assert_eq!((p.leaderboard_kind, p.most.by), ("songs", "plays"));
        assert_eq!(names(&mut p), ["Lanterns Over Water", "Paper in the Rain", "Tidewater", "Milestone", "Carrier"]);
        let d = frames(&ctx, &mut p);
        for t in ["Sort by", "Times played", "Time listened", "Genres", "Day", "Week", "Month", "Year", "All time"] {
            assert!(shown(&d, t), "{t}");
        }
        assert_eq!((ink(&d, "3 plays"), ink(&d, "10 min")), (pal.text, pal.secondary));
        click(&ctx, &mut p, at(&d, "Time listened", |_| true), PointerButton::Primary);
        assert_eq!(p.most.by, "time");
        assert_eq!(names(&mut p), ["Tidewater", "Milestone", "Lanterns Over Water", "Paper in the Rain", "Carrier"]);
        assert_eq!(p.most_rows().0.iter().map(|r| r.rank).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        let d = frames(&ctx, &mut p);
        assert_eq!((ink(&d, "3 plays"), ink(&d, "10 min")), (pal.secondary, pal.text));
        click(&ctx, &mut p, at(&d, "Play", |r| r.top() < 200.0 && r.left() > 240.0), PointerButton::Primary);
        assert_eq!(p.queue_name, "Most Played");
        assert_eq!(p.queue.as_ref().unwrap().context[..3], [b1.clone(), d1.clone(), a1.clone()]);
        assert_eq!(p.now.as_ref().map(|n| n.path.clone()), Some(b1.clone()), "the longest listened first");
        p.engine.pause();
        // the section of Your Stats shares the switch
        p.stats_period = "All time".into();
        assert_eq!(p.board("songs")[0].key, b1);
        p.go(Page::Stats);
        let d = reveal(&ctx, &mut p, "Times played", |_| true);
        assert!(shown(&d, "Sort by") && shown(&d, "Time listened") && !shown(&d, "Day"), "no period control in the section");
        click(&ctx, &mut p, at(&d, "Times played", |_| true), PointerButton::Primary);
        assert_eq!(p.most.by, "plays");
        assert_eq!(p.board("songs")[0].key, a1);

        // ---- the period control: Day, Week, Month, Year, All time, with ‹ and ›
        p.go(Page::MostPlayed);
        let d = frames(&ctx, &mut p);
        click(&ctx, &mut p, at(&d, "Day", |_| true), PointerButton::Primary);
        assert_eq!(p.most_span(), ("day", 0));
        assert_eq!(names(&mut p), ["Lanterns Over Water", "Tidewater"]);
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, &stats::span_title("day", 0, now)), "today's name");
        let step = |p: &mut Player, delta: i32| {
            frames(&ctx, p);
            let (back, next) = p.most.nav.expect("the period control's buttons");
            click(&ctx, p, if delta < 0 { back.center() } else { next.center() }, PointerButton::Primary);
        };
        step(&mut p, 1); // › is off at the present
        assert_eq!(p.most_span(), ("day", 0));
        step(&mut p, -1);
        assert_eq!(p.most_span(), ("day", -1));
        assert_eq!(names(&mut p), ["Paper in the Rain"]);
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, &stats::span_title("day", -1, now)) && shown(&d, "2 plays"));
        step(&mut p, -1);
        assert_eq!(p.most_span(), ("day", -2));
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "Nothing was played in this period.") && names(&mut p).is_empty());
        step(&mut p, 1);
        step(&mut p, 1);
        assert_eq!(p.most_span(), ("day", 0));
        for unit in ["Week", "Month"] {
            let d = frames(&ctx, &mut p);
            click(&ctx, &mut p, at(&d, unit, |_| true), PointerButton::Primary);
            let key = if unit == "Week" { "week" } else { "month" };
            assert_eq!(p.most_span(), (key, 0));
            assert!(names(&mut p).contains(&"Lanterns Over Water".to_string()));
            let d = frames(&ctx, &mut p);
            assert!(shown(&d, &stats::span_title(key, 0, now)), "{unit}");
        }
        let d = frames(&ctx, &mut p);
        click(&ctx, &mut p, at(&d, "Year", |_| true), PointerButton::Primary);
        step(&mut p, -1);
        assert_eq!(p.most_span(), ("year", -1));
        let last = names(&mut p);
        assert!(last.contains(&"Milestone".to_string()) && !last.contains(&"Lanterns Over Water".to_string()), "{last:?}");
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, &(today.year - 1).to_string()));
        assert!(!p.most_rows().1, "nothing was played before that year");
        step(&mut p, -1); // ‹ is off
        assert_eq!(p.most_span(), ("year", -1));

        // ---- the controls fit the page, on more lines where it is narrow
        let lines = |d: &Drawn, w: f32, title: &str| -> Vec<f32> {
            let marks = ["Songs", "Sort by", "Day", title];
            let ys: Vec<f32> = marks.iter().map(|m| at(d, m, |r| r.left() > 230.0).y).collect();
            for (t, r) in d.texts.iter().filter(|(_, r)| r.top() > 70.0 && r.bottom() < ys[3] + 20.0) {
                assert!(r.right() <= w, "{t:?} is cut off at {w}: {r:?}");
            }
            ys
        };
        let title = (today.year - 1).to_string();
        let d = frames(&ctx, &mut p);
        let ys = lines(&d, W, &title);
        assert!(ys[0] + 20.0 < ys[2], "the period control under the switch at 1024 x 768: {ys:?}");
        let narrow = |ctx: &egui::Context, p: &mut Player| {
            run_sized(ctx, p, vec![], 760.0, 600.0);
            run_sized(ctx, p, vec![], 760.0, 600.0);
            run_sized(ctx, p, vec![], 760.0, 600.0)
        };
        let d = narrow(&ctx, &mut p);
        let ys = lines(&d, 760.0, &title);
        assert!(ys[0] + 20.0 < ys[1] && ys[1] + 20.0 < ys[2] && ys[2] + 20.0 < ys[3], "one group a line: {ys:?}");
        assert!(d.texts.iter().any(|(t, _)| t == "Milestone"), "the rows under the controls");
        frames(&ctx, &mut p);

        // ---- in Japanese
        realhires::i18n::set_thread_language("日本語");
        p.most.span = ("day", 0);
        let d = frames(&ctx, &mut p);
        for t in ["並べ替え", "再生回数", "再生時間", "日", "週", "月", "年", "ジャンル"] {
            assert!(shown(&d, t), "{t}");
        }
        let name = span_text("day", 0);
        assert_eq!(name, format!("{}年{}月{}日", today.year, today.month, today.day));
        assert!(shown(&d, &name));
        p.most.span = ("year", -1);
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, &format!("{}年", today.year - 1)));
        p.most.span = ("day", -2);
        let d = frames(&ctx, &mut p);
        assert!(shown(&d, "この期間には何も再生されていません。"));
        realhires::i18n::set_thread_language("English");
        p.on_exit();
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Rest the pointer at `pos` until a tooltip holding `needle` is drawn; its text.
    fn tooltip(ctx: &egui::Context, p: &mut Player, pos: Pos2, needle: &str) -> String {
        run(ctx, p, vec![Event::PointerMoved(pos2(pos.x - 30.0, pos.y))]);
        run(ctx, p, vec![Event::PointerMoved(pos)]);
        for _ in 0..240 {
            let d = run(ctx, p, vec![]);
            // a tooltip is drawn near the pointer, apart from the cell's own text
            if let Some((t, _)) = d.texts.iter().find(|(t, r)| t.contains(needle) && r.top() > pos.y - 60.0 && r.top() < pos.y + 80.0 && !r.contains(pos)) {
                return t.clone();
            }
        }
        panic!("no tooltip with {needle:?} at {pos:?}");
    }

    #[test]
    fn quality_tooltip_on_hover() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        realhires::i18n::set_thread_language("English");
        let (tmp, ctx, mut p, s) = library("quality");
        let tip_of = |p: &Player, path: &str| musiclib::quality_detail(p.lib.track(path).unwrap());
        let want = tip_of(&p, &s[0]);
        assert!(want.starts_with("16-bit/44.1 kHz · ") && want.ends_with(" kbps"), "{want}");
        let cell = |d: &Drawn| at(d, "Lossless", |r| r.top() > 150.0 && r.left() > 300.0);
        // All Songs
        p.go(Page::AllSongs);
        let d = frames(&ctx, &mut p);
        let pos = at(&d, "Lossless", |r| r.top() > 150.0 && r.left() > 300.0 && (r.center().y - at(&d, "Lanterns Over Water", |_| true).y).abs() < 10.0);
        assert_eq!(tooltip(&ctx, &mut p, pos, "kbps"), want, "All Songs");
        // no tooltip on another cell of the row
        let title = at(&d, "Lanterns Over Water", |_| true);
        run(&ctx, &mut p, vec![Event::PointerMoved(title)]);
        let mut d2 = frames(&ctx, &mut p);
        for _ in 0..60 {
            d2 = run(&ctx, &mut p, vec![]);
        }
        assert!(!d2.texts.iter().any(|(t, _)| t.contains("kbps")), "only the Quality cell has the tooltip");
        // an album's songs
        let album = p.lib.album_of(&s[2]).unwrap().key.clone();
        p.go(Page::Album(album));
        let d = frames(&ctx, &mut p);
        assert_eq!(tooltip(&ctx, &mut p, cell(&d), "kbps"), tip_of(&p, &s[2]), "an album's songs");
        // an artist's songs: the artist's page shows the albums, a click on one lists its songs
        p.go(Page::Group("artists", "Juniper Vale".into()));
        let d = frames(&ctx, &mut p);
        click(&ctx, &mut p, at(&d, "Static Bloom", |r| r.top() > 150.0), PointerButton::Primary);
        assert_eq!(p.page, Page::Album(p.lib.album_of(&s[3]).unwrap().key.clone()));
        let d = frames(&ctx, &mut p);
        assert_eq!(tooltip(&ctx, &mut p, cell(&d), "kbps"), tip_of(&p, &s[3]), "an artist's songs");
        // a genre's songs
        p.go(Page::Group("genres", "Folk".into()));
        let d = frames(&ctx, &mut p);
        assert_eq!(tooltip(&ctx, &mut p, cell(&d), "kbps"), tip_of(&p, &s[4]), "a genre's songs");
        // a folder's contents
        let folder = realhires::paths::abspath(&std::path::Path::new(&s[1]).parent().unwrap().to_string_lossy());
        p.go(Page::Folders(Some(folder)));
        let d = frames(&ctx, &mut p);
        let pos = at(&d, "Lossless", |r| r.top() > 150.0 && r.left() > 300.0 && (r.center().y - at(&d, "Paper in the Rain", |_| true).y).abs() < 10.0);
        assert_eq!(tooltip(&ctx, &mut p, pos, "kbps"), tip_of(&p, &s[1]), "a folder's contents");
        // the playback bar's quality badge
        p.play_list(vec![s[0].clone()], Some(&s[0]), "All Songs");
        p.engine.pause();
        p.go(Page::AllSongs);
        let d = frames(&ctx, &mut p);
        let badge = at2(&d, "Lossless", |r| r.top() < 70.0);
        let tip = tooltip(&ctx, &mut p, badge, "kbps");
        assert_eq!(tip, format!("{want}\nPlayback info"), "the badge");
        // Now Playing's Details: the Format row
        p.go(Page::NowPlaying);
        p.np_tab = crate::player::NpTab::Details;
        let d = frames(&ctx, &mut p);
        let format = d.texts.iter().find(|(t, _)| t.starts_with("FLAC · 16-bit/44.1 kHz")).map(|(_, r)| r.center()).expect("the Format row");
        assert_eq!(tooltip(&ctx, &mut p, format, "kbps"), want, "Details");
        // in Japanese
        realhires::i18n::set_thread_language("日本語");
        p.go(Page::AllSongs);
        let d = frames(&ctx, &mut p);
        let pos = at(&d, realhires::i18n::tr("Lossless"), |r| r.top() > 150.0 && r.left() > 300.0);
        let tip = tooltip(&ctx, &mut p, pos, "kbps");
        assert!(tip.starts_with("16ビット/44.1 kHz · "), "{tip}");
        realhires::i18n::set_thread_language("English");
        p.on_exit();
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Where a text was drawn, anywhere in the window (`at()` leaves the top bar out).
    fn at2(d: &Drawn, text: &str, pick: impl Fn(&Rect) -> bool) -> Pos2 {
        d.texts.iter().find(|(t, r)| t == text && pick(r)).unwrap_or_else(|| panic!("no {text:?}")).1.center()
    }
}
