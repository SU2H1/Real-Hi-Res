//! The music player window (2.0), as `docs/player-spec.md` describes it: a
//! sidebar with the library, a playback bar with the "LCD", the pages
//! (`pages.rs`), the Up Next panel and the Playback info and Equalizer windows.
//! The 1.x scanner (`app::App`) lives on as the Quality check page.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, Sense, Stroke, Ui};
use realhires::musiclib::{self, Album, Group, Library, Track};
use realhires::playerstate::{self, PlayerState};
use realhires::playqueue::{repeat_label, shuffle_label, PlayQueue, Song, REPEAT_MODES, SHUFFLE_MODES};
use realhires::pyunicode::casefold;
use realhires::settings::Settings;

use crate::app::App;
use crate::audio::{Engine, Event};
use crate::cover_cache::Covers;
use crate::icons::{self, Icon};
use crate::look::{self, Palette};

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    /// the start page with the fourteen entries
    Library,
    AllSongs,
    /// a library folder or one of its subfolders (None: the library folders)
    Folders(Option<String>),
    Albums,
    Album(String),
    Artists,
    AlbumArtists,
    Genres,
    Years,
    Composers,
    /// one artist, album artist, genre, year or composer (a card of a grouping page)
    Group(&'static str, String),
    Playlists,
    Playlist(String),
    Liked,
    Disliked,
    RecentlyPlayed,
    Recent,
    Stats,
    StatsDetails,
    Search,
    NowPlaying,
    Settings,
    Quality,
}

impl Page {
    pub fn title(&self) -> &'static str {
        match self {
            Page::Library => "Library",
            Page::AllSongs => "All Songs",
            Page::Folders(_) => "Folders",
            Page::Albums | Page::Album(_) => "Albums",
            Page::Artists => "Artists",
            Page::AlbumArtists => "Album Artists",
            Page::Genres => "Genres",
            Page::Years => "Years",
            Page::Composers => "Composers",
            Page::Group(kind, _) => GROUPINGS.iter().find(|g| g.0 == *kind).map(|g| g.1).unwrap_or("Artists"),
            Page::Playlists | Page::Playlist(_) => "Playlists",
            Page::Liked => "Liked Songs",
            Page::Disliked => "Disliked Songs",
            Page::RecentlyPlayed => "Recently Played",
            Page::Recent => "Recently Added",
            Page::Stats | Page::StatsDetails => "Your Stats",
            Page::Search => "Search",
            Page::NowPlaying => "Now Playing",
            Page::Settings => "Settings",
            Page::Quality => "Quality check",
        }
    }
}

/// A small window that asks for a name (new playlist, preset, theme...).
pub struct NameDialog {
    pub title: String,
    pub text: String,
    pub action: NameAction,
    pub error: String,
    /// what the field is for ("Name" when empty)
    pub prompt: String,
    /// names to choose from (Merge into…); the field can still be typed into
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NameAction {
    NewPlaylist(Vec<String>),
    RenamePlaylist(String),
    SaveEqPreset,
    RenameEqPreset(String),
    NewTheme(String),
    RenameTheme(String),
    /// rename a genre, composer or year (the kind, its name)
    RenameGroup(&'static str, String),
    /// merge genres into the name typed or chosen (the kind, their names)
    MergeGroups(&'static str, Vec<String>),
}

/// A question before something is deleted.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmAction {
    DeletePlaylist(String),
    DeleteEqPreset(String),
    DeleteTheme(String),
    ClearHistory,
    /// write a new genre, composer or year into these files (the question, the kind, the
    /// paths, the new name, the groups it came from)
    RetagGroup { title: String, text: String, kind: &'static str, paths: Vec<String>, name: String, from: Vec<String> },
}

/// The song being listened to, for the play history.
struct Listening {
    token: u64,
    path: String,
    started: f64,
    seconds: f64,
    last_pos: f64,
}

fn wall_clock() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

enum LibMsg {
    Progress(usize, usize),
    Done(Vec<Track>),
}

/// The library, grouped for the pages.
#[derive(Default)]
pub struct LibView {
    pub tracks: Vec<Track>,
    pub by_path: HashMap<String, usize>,
    pub albums: Vec<Album>,
    pub album_index: HashMap<String, usize>,
    pub artists: Vec<Group>,
    pub album_artists: Vec<Group>,
    pub genres: Vec<Group>,
    pub years: Vec<Group>,
    pub composers: Vec<Group>,
    /// album indices, newest first
    pub recent: Vec<usize>,
    pub songs: Vec<Song>,
    /// the first four songs of each album (what its cover is made from), made once
    pub album_paths: Vec<Vec<String>>,
    /// per grouping ("artists"...), each group's albums (indices, in album order), made once
    pub group_albums: HashMap<&'static str, Vec<Vec<usize>>>,
    /// the artists that stand on their own (`musiclib::known_artists`), for split artist fields
    pub known: std::collections::HashSet<String>,
    /// a new number for each library read, so that the pages' caches know when to start again
    pub gen: u64,
}

static LIB_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The groupings of the pages: (page kind, title, noun).
pub const GROUPINGS: [(&str, &str, &str); 5] = [
    ("artists", "Artists", "artist"),
    ("album_artists", "Album Artists", "album artist"),
    ("genres", "Genres", "genre"),
    ("years", "Years", "year"),
    ("composers", "Composers", "composer"),
];

impl LibView {
    pub fn new(mut tracks: Vec<Track>) -> LibView {
        musiclib::sort_songs(&mut tracks);
        let by_path = tracks.iter().enumerate().map(|(i, t)| (t.path.clone(), i)).collect();
        let albums = musiclib::albums(&tracks);
        let album_index: HashMap<String, usize> = albums.iter().enumerate().map(|(i, a)| (a.key.clone(), i)).collect();
        let mut recent: Vec<usize> = (0..albums.len()).collect();
        let newest = |a: &Album| a.tracks.iter().map(|t| t.mtime_ns).max().unwrap_or(0);
        recent.sort_by(|a, b| newest(&albums[*b]).cmp(&newest(&albums[*a])));
        let album_paths = albums.iter().map(|a| a.tracks.iter().take(4).map(|t| t.path.clone()).collect()).collect();
        let mut lib = LibView {
            artists: musiclib::artists(&tracks),
            album_artists: musiclib::album_artists(&tracks),
            genres: musiclib::genres(&tracks),
            years: musiclib::years(&tracks),
            composers: musiclib::composers(&tracks),
            songs: tracks.iter().map(Track::song).collect(),
            known: musiclib::known_artists(&tracks),
            tracks,
            by_path,
            albums,
            album_index,
            recent,
            album_paths,
            group_albums: HashMap::new(),
            gen: LIB_GEN.fetch_add(1, Ordering::Relaxed),
        };
        let album_of_path: HashMap<&str, usize> =
            lib.albums.iter().enumerate().flat_map(|(i, a)| a.tracks.iter().map(move |t| (t.path.as_str(), i))).collect();
        for (kind, _, _) in GROUPINGS {
            let per: Vec<Vec<usize>> = lib
                .groups(kind)
                .iter()
                .map(|g| {
                    let mut idx: Vec<usize> = g.tracks.iter().filter_map(|t| album_of_path.get(t.path.as_str()).copied()).collect();
                    idx.sort_unstable();
                    idx.dedup();
                    idx
                })
                .collect();
            lib.group_albums.insert(kind, per);
        }
        drop(album_of_path);
        lib
    }

    pub fn track(&self, path: &str) -> Option<&Track> {
        self.by_path.get(path).map(|i| &self.tracks[*i])
    }

    pub fn album_of(&self, path: &str) -> Option<&Album> {
        let key = self.track(path)?.song().album_key();
        self.album_index.get(&key).map(|i| &self.albums[*i])
    }

    /// The artists of a song's artist field (`musiclib::split_artists()`), each a link.
    pub fn artists_of(&self, t: &Track) -> Vec<String> {
        let names = musiclib::split_artists(&t.display_artist(), &self.known);
        if names.is_empty() {
            vec![t.display_artist()]
        } else {
            names
        }
    }

    /// The groups of a grouping page ("artists", "album_artists", "genres", "years", "composers").
    pub fn groups(&self, kind: &str) -> &Vec<Group> {
        match kind {
            "artists" => &self.artists,
            "album_artists" => &self.album_artists,
            "genres" => &self.genres,
            "years" => &self.years,
            _ => &self.composers,
        }
    }

    /// The index of a group by its name (in any case).
    pub fn find_group(&self, kind: &str, name: &str) -> Option<usize> {
        let key = casefold(name.trim());
        self.groups(kind).iter().position(|g| casefold(g.name.trim()) == key)
    }
}

/// Where Now Playing drew what can be clicked, and what it showed (for the tests).
#[derive(Default)]
pub struct NpHits {
    pub cover: Option<Rect>,
    /// the artist and album name links under the cover
    pub links: Vec<(Rect, crate::pages::Link)>,
    pub get_lyrics: Option<Rect>,
    /// A− and A+
    pub size_buttons: Vec<Rect>,
    /// "Back to the current line"
    pub back: Option<Rect>,
    pub up_next: Vec<String>,
    /// Details as "Label: value" lines
    pub details: Vec<String>,
    pub detail_links: Vec<(Rect, String)>,
}

/// Now Playing's right half (2.1): the switch Lyrics | Up Next | Details.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NpTab {
    Lyrics,
    UpNext,
    Details,
}

impl NpTab {
    pub const ALL: [(NpTab, &'static str); 3] =
        [(NpTab::Lyrics, "Lyrics"), (NpTab::UpNext, "Up Next"), (NpTab::Details, "Details")];
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShownLyrics {
    Synced(Vec<(u32, String)>),
    Plain(String),
    Instrumental,
}

/// What the Playback info window shows of the saved Real-Hi-Res verdict.
pub struct InfoWindow {
    pub path: String,
    pub verdict: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SongCol {
    Num,
    Title,
    Artist,
    Album,
    AlbumArtist,
    Composer,
    Genre,
    Year,
    Disc,
    Time,
    Quality,
    Format,
    BitRate,
    Added,
    File,
    /// the heart (not sortable)
    Like,
    /// Recently Played's "Played" column
    Played,
}

impl SongCol {
    /// The column of a name in player.json's "columns" (`playerstate::SONG_COLUMNS`).
    pub fn from_name(name: &str) -> Option<SongCol> {
        Some(match name {
            "num" => SongCol::Num,
            "title" => SongCol::Title,
            "artist" => SongCol::Artist,
            "album" => SongCol::Album,
            "album_artist" => SongCol::AlbumArtist,
            "composer" => SongCol::Composer,
            "genre" => SongCol::Genre,
            "year" => SongCol::Year,
            "disc" => SongCol::Disc,
            "time" => SongCol::Time,
            "quality" => SongCol::Quality,
            "format" => SongCol::Format,
            "bit_rate" => SongCol::BitRate,
            "added" => SongCol::Added,
            "file" => SongCol::File,
            "like" => SongCol::Like,
            _ => return None,
        })
    }

    /// (heading, menu label, width weight, right-aligned)
    pub fn info(self) -> (&'static str, &'static str, f32, bool) {
        match self {
            SongCol::Num => ("#", "# (track number)", 0.35, true),
            SongCol::Title => ("Title", "Title", 3.0, false),
            SongCol::Artist => ("Artist", "Artist", 2.0, false),
            SongCol::Album => ("Album", "Album", 2.2, false),
            SongCol::AlbumArtist => ("Album Artist", "Album Artist", 1.8, false),
            SongCol::Composer => ("Composer", "Composer", 1.6, false),
            SongCol::Genre => ("Genre", "Genre", 1.1, false),
            SongCol::Year => ("Year", "Year", 0.55, false),
            SongCol::Disc => ("Disc", "Disc", 0.45, true),
            SongCol::Time => ("Time", "Time", 0.6, true),
            SongCol::Quality => ("Quality", "Quality", 1.15, false),
            SongCol::Format => ("Format", "Format", 0.7, false),
            SongCol::BitRate => ("Bit rate", "Bit rate", 0.9, true),
            SongCol::Added => ("Date added", "Date added", 1.0, false),
            SongCol::File => ("File", "File", 1.8, false),
            SongCol::Like => ("", "Heart (like)", 0.0, false),
            SongCol::Played => ("Played", "Played", 1.1, false),
        }
    }
}

/// The Format column: the codec ("FLAC", "MP3"; PCM for WAV and AIFF samples).
pub fn format_label(t: &Track) -> String {
    let c = t.codec.to_lowercase();
    if c.starts_with("pcm_") {
        "PCM".into()
    } else {
        c.to_uppercase()
    }
}

/// A song table's cell (Python `track_value`).
pub fn cell_text(t: &Track, col: SongCol) -> String {
    match col {
        SongCol::Num => if t.track > 0 { t.track.to_string() } else { String::new() },
        SongCol::Title => t.display_title(),
        SongCol::Artist => t.display_artist(),
        SongCol::Album => t.display_album(),
        SongCol::AlbumArtist => t.album_artist.clone(),
        SongCol::Composer => t.composer.clone(),
        SongCol::Genre => t.genre.clone(),
        SongCol::Year => t.year.clone(),
        SongCol::Disc => if t.disc > 0 { t.disc.to_string() } else { String::new() },
        SongCol::Time => fmt_time(t.duration),
        SongCol::Quality => musiclib::quality(t),
        SongCol::Format => format_label(t),
        SongCol::BitRate => {
            if t.bit_rate > 0 {
                format!("{} kbps", realhires::stats::thousands(realhires::pyfmt::round_int(t.bit_rate as f64 / 1000.0)))
            } else {
                String::new()
            }
        }
        SongCol::Added => {
            if t.mtime_ns <= 0 {
                return String::new();
            }
            let p = realhires::stats::local_parts(t.mtime_ns as f64 / 1e9);
            realhires::stats::day_label(&format!("{:04}-{:02}-{:02}", p.year, p.month, p.day))
        }
        SongCol::File => realhires::paths::file_name(&t.path),
        SongCol::Like | SongCol::Played => String::new(),
    }
}

/// What a click on a column heading sorts by (Python `track_sort`; numbers padded).
pub fn sort_key(t: &Track, col: SongCol) -> (String, String, String) {
    let title = casefold(&t.display_title());
    let n = |x: i64| if x == 0 { 9999 } else { x };
    match col {
        SongCol::Artist => (casefold(&t.display_artist()), casefold(&t.display_album()), format!("{:04}{:04}", t.disc, t.track)),
        SongCol::Num | SongCol::Disc => (format!("{:05}{:05}", n(t.disc), n(t.track)), title, t.path.clone()),
        SongCol::Time => (format!("{:012.3}", t.duration), title, t.path.clone()),
        SongCol::Quality => {
            let (bits, rate, float, kbps) = musiclib::quality_sort(t);
            (format!("{bits:04}{rate:09}{}{kbps:012}", float as u8), title, t.path.clone())
        }
        SongCol::BitRate => (format!("{:012}", t.bit_rate), title, t.path.clone()),
        SongCol::Added => (format!("{:020}", t.mtime_ns), title, t.path.clone()),
        SongCol::Title => (title, casefold(&t.display_artist()), t.path.clone()),
        other => (casefold(&cell_text(t, other)), title, t.path.clone()),
    }
}

pub struct NowPlaying {
    pub token: u64,
    pub path: String,
    /// false once the queue ended or playback was stopped
    pub active: bool,
}

pub struct Player {
    pub scanner: App,
    pub ctx: egui::Context,
    pub state: PlayerState,
    state_dirty: bool,
    state_saved: Instant,
    pub lib: LibView,
    lib_rx: Option<Receiver<LibMsg>>,
    pub lib_progress: Option<(usize, usize)>,
    lib_stop: Arc<AtomicBool>,
    pub page: Page,
    pub back: Vec<Page>,
    /// the pages left with Back (the mouse's back button, Alt+Left), for Forward (2.1)
    pub forward: Vec<Page>,
    pub search: String,
    search_due: Option<Instant>,
    pub search_query: String,
    search_from: Option<Page>,
    pub engine: Engine,
    pub queue: Option<PlayQueue>,
    pub queue_name: String,
    pub now: Option<NowPlaying>,
    /// a message for the LCD's second line, and when playback moves on after it
    pub message: Option<(String, Option<Instant>)>,
    fail_streak: usize,
    /// the lyrics of the song playing: (path, lyrics_gen, lyrics, (label, source))
    lyrics: Option<(String, u64, Option<ShownLyrics>, (String, String))>,
    pub covers: Covers,
    pub show_up_next: bool,
    /// Now Playing without its lyrics column (the Lyrics button on that page, 2.1)
    pub lyrics_hidden: bool,
    /// Now Playing's right half: Lyrics, Up Next or Details (for the session)
    pub np_tab: NpTab,
    /// Details: per song its comment tag (None while ffprobe reads it) and saved verdict
    details_extra: HashMap<String, (Option<String>, Option<(String, String)>)>,
    details_song: String,
    details_tx: Sender<(String, String)>,
    details_rx: Receiver<(String, String)>,
    pub eq_open: bool,
    pub eq_tab: usize,
    pub info: Option<InfoWindow>,
    pub settings_group: usize,
    settings_draft: Option<Settings>,
    settings_due: Option<Instant>,
    /// the user scrolled the current line of synced lyrics out of view: no following, and
    /// the "Back to the current line" button shows (2.1)
    pub lyrics_away: bool,
    /// the Back button was pressed: the current line is scrolled to at once
    pub lyrics_back: bool,
    /// the user scrolled the lyrics: whether the current line is in view is checked again
    pub lyrics_check: bool,
    pub np_hits: NpHits,
    /// the LCD's links: the title and the album name (the album), each artist
    pub lcd_links: Vec<(Rect, crate::pages::Link)>,
    /// the line followed last
    pub lyrics_line: Option<usize>,
    pub song_sort: (SongCol, bool),
    applied_look: Option<(String, String, u64)>,
    pub font_used: String,
    pub selected_song: Option<String>,
    /// "Check new songs when the app starts": scan once the library is read
    check_after_update: bool,
    /// the genres selected (Ctrl/Cmd-click) on the Genres page, to be merged
    pub group_selected: Vec<String>,
    /// the sort of album pages (None: track order) and the album it is for
    pub album_sort: Option<(SongCol, bool)>,
    album_sort_for: String,
    /// sorted song lists, made again only when the library, the sort or the likes change
    list_cache: HashMap<String, ((u64, Option<(SongCol, bool)>, u64), Arc<Vec<String>>)>,
    /// counts likes and dislikes, for the lists made of them
    likes_gen: u64,
    /// renaming a genre, composer or year, or merging genres (one at a time)
    pub tag_job: Option<crate::editinfo::RenameJob>,
    seed: u64,
    // 2.0 additions
    pub custom_themes: Vec<realhires::themes::Theme>,
    /// the themes of the files in the themes folder (read again every 2 s on the Settings page)
    pub file_themes: Vec<realhires::themes::FileTheme>,
    pub file_themes_read: Instant,
    themes_gen: u64,
    /// the Edit Info window (one at a time)
    pub edit_info: Option<crate::editinfo::EditInfo>,
    /// the Cover window (one at a time)
    pub cover_win: Option<crate::coverwin::CoverWin>,
    /// the play history (`library.sqlite`)
    pub history: Option<Library>,
    pub history_gen: u64,
    listening: Option<Listening>,
    pub stats_period: String,
    stats_cache: Option<(String, u64, realhires::stats::Stats)>,
    /// the calendar of Your Stats (2.1)
    pub calendar: crate::library_pages::Calendar,
    recent_cache: Option<(u64, Vec<(String, f64)>)>,
    pub name_dialog: Option<NameDialog>,
    pub confirm: Option<ConfirmAction>,
    /// what the last EQ import said
    pub eq_notes: Vec<String>,
    /// the playlist whose page is shown (its songs get Remove/Move)
    pub table_playlist: Option<String>,
}

fn seed_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

pub fn gain_of(state: &PlayerState) -> f32 {
    if state.muted {
        0.0
    } else {
        (state.volume * state.volume) as f32
    }
}

/// The gain the engine gets: exactly 1 (0 when muted) with bit-perfect playback,
/// which keeps the saved volume for when it is turned off.
pub fn engine_gain(state: &PlayerState, bit_perfect: bool) -> f32 {
    if bit_perfect && !state.muted {
        1.0
    } else {
        gain_of(state)
    }
}

/// What bit-perfect playback needs to know about a library song (rate 0: the engine reads the file).
pub fn song_of(t: Option<&Track>) -> realhires::bitperfect::Song {
    match t {
        Some(t) => realhires::bitperfect::Song {
            rate: t.sample_rate.max(0) as u32,
            channels: t.channels.max(0) as u32,
            bits: t.bits.max(0) as u32,
            is_float: t.is_float,
            is_lossy: t.is_lossy,
        },
        None => Default::default(),
    }
}

impl Player {
    pub fn new(ctx: &egui::Context, cfg: Settings, initial: Vec<String>) -> Player {
        let quality_first = !initial.is_empty();
        let scanner = App::new(ctx, cfg, initial);
        let state = playerstate::load(None);
        let tracks = Library::open(None).map(|l| l.tracks()).unwrap_or_default();
        let mut engine = Engine::new();
        let (details_tx, details_rx) = mpsc::channel();
        engine.set_gain(engine_gain(&state, scanner.cfg.bit_perfect));
        engine.set_eq(&state.eq);
        engine.set_gapless(scanner.cfg.gapless);
        engine.set_bit_perfect(scanner.cfg.bit_perfect);
        let mut p = Player {
            covers: Covers::new(ctx),
            ctx: ctx.clone(),
            lib: LibView::new(tracks),
            lib_rx: None,
            lib_progress: None,
            lib_stop: Arc::new(AtomicBool::new(false)),
            page: if quality_first { Page::Quality } else { Page::Library },
            back: vec![],
            forward: vec![],
            search: String::new(),
            search_due: None,
            search_query: String::new(),
            search_from: None,
            engine,
            queue: None,
            queue_name: String::new(),
            now: None,
            message: None,
            fail_streak: 0,
            lyrics: None,
            show_up_next: false,
            lyrics_hidden: false,
            np_tab: NpTab::Lyrics,
            details_extra: HashMap::new(),
            details_song: String::new(),
            details_tx,
            details_rx,
            eq_open: false,
            eq_tab: 0,
            info: None,
            settings_group: 0,
            settings_draft: None,
            settings_due: None,
            lyrics_away: false,
            lyrics_back: false,
            lyrics_check: false,
            np_hits: NpHits::default(),
            lcd_links: vec![],
            lyrics_line: None,
            song_sort: (SongCol::Artist, false),
            applied_look: None,
            font_used: String::new(),
            selected_song: None,
            check_after_update: false,
            group_selected: vec![],
            album_sort: None,
            album_sort_for: String::new(),
            list_cache: HashMap::new(),
            likes_gen: 0,
            tag_job: None,
            custom_themes: realhires::themes::load_custom(None),
            file_themes: vec![],
            file_themes_read: Instant::now(),
            themes_gen: 0,
            edit_info: None,
            cover_win: None,
            history: Library::open(None).ok(),
            history_gen: 0,
            listening: None,
            stats_period: realhires::stats::DEFAULT_PERIOD.to_string(),
            stats_cache: None,
            calendar: Default::default(),
            recent_cache: None,
            name_dialog: None,
            confirm: None,
            eq_notes: vec![],
            table_playlist: None,
            seed: seed_now(),
            state,
            state_dirty: false,
            state_saved: Instant::now(),
            scanner,
        };
        p.file_themes = realhires::themes::load_folder(&p.custom_themes, None);
        p.covers.library_changed(&p.lib.albums);
        p.fill_choices();
        p.engine.set_device(&p.scanner.cfg.output_device.clone());
        p.update_library();
        p.check_after_update = p.scanner.cfg.scan_library_on_start && p.lib_rx.is_some();
        // the song of last time, paused where it was
        let last = p.state.last_path.clone();
        if !last.is_empty() && p.lib.by_path.contains_key(&last) && std::path::Path::new(&last).is_file() {
            let pos = p.state.last_position;
            // go on with the song's album, as after playing it from there
            let (list, name) = match p.lib.album_of(&last) {
                Some(a) => (a.tracks.iter().map(|t| t.path.clone()).collect(), a.title.clone()),
                None => (vec![last.clone()], "Songs".to_string()),
            };
            p.make_queue(list, &name);
            if let Some(q) = p.queue.as_mut() {
                q.start(&last);
            }
            let song = song_of(p.lib.track(&last));
            let token = p.engine.cue(&last, pos, song);
            p.now = Some(NowPlaying { token, path: last, active: true });
            p.update_next();
        }
        p
    }

    // ------------------------------------------------------------ library ---

    /// Read the tags of new and changed files in the library folders.
    pub fn update_library(&mut self) {
        if self.lib_rx.is_some() {
            return;
        }
        let cfg = self.scanner.cfg.clone();
        if cfg.library_folders.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.lib_stop = stop.clone();
        let ctx = self.ctx.clone();
        std::thread::Builder::new()
            .name("library-update".into())
            .spawn(move || {
                let files = musiclib::library_files(&cfg);
                let _ = tx.send(LibMsg::Progress(0, files.len()));
                let Ok(mut lib) = Library::open(None) else {
                    let _ = tx.send(LibMsg::Done(vec![]));
                    return;
                };
                let jobs = realhires::effective_jobs(cfg.jobs).min(8);
                let mut last = Instant::now();
                let tracks = lib.refresh(
                    &files,
                    jobs,
                    &mut |done, total| {
                        if last.elapsed() > Duration::from_millis(200) || done == total {
                            last = Instant::now();
                            let _ = tx.send(LibMsg::Progress(done, total));
                            ctx.request_repaint();
                        }
                    },
                    Some(&stop),
                );
                let _ = tx.send(LibMsg::Done(tracks));
                ctx.request_repaint();
            })
            .expect("library thread");
        self.lib_rx = Some(rx);
        self.lib_progress = Some((0, 0));
    }

    fn poll_library(&mut self) {
        let Some(rx) = &self.lib_rx else { return };
        let mut done = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                LibMsg::Progress(d, t) => self.lib_progress = Some((d, t)),
                LibMsg::Done(tracks) => done = Some(tracks),
            }
        }
        if let Some(tracks) = done {
            self.lib_rx = None;
            self.lib_progress = None;
            self.lib = LibView::new(tracks);
            // covers stay unless an album's songs changed; then the grid covers not on disk yet
            self.covers.library_changed(&self.lib.albums);
            self.covers.prefetch(&self.lib.albums);
            if let Some(q) = self.queue.as_mut() {
                q.library = self.lib.songs.clone();
            }
            if std::mem::take(&mut self.check_after_update) && !self.scanner.scanning {
                let page = self.page.clone();
                self.check_library();
                self.scanner.quiet_scan = true;
                self.page = page;
            }
        }
    }

    pub fn library_status(&self) -> Option<String> {
        let (done, total) = self.lib_progress?;
        Some(if total == 0 {
            "Updating the library: looking for songs…".into()
        } else {
            format!("Updating the library: {done} of {total} songs")
        })
    }

    // ---------------------------------------------------------- playback ---

    fn make_queue(&mut self, list: Vec<String>, name: &str) {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut q = PlayQueue::new(self.lib.songs.clone(), list, &self.state.shuffle, &self.state.repeat, self.seed);
        q.disliked = self.state.disliked.iter().cloned().collect();
        self.queue = Some(q);
        self.queue_name = name.to_string();
    }

    /// Play `list` (paths) from `start` (None: from the first song, or a random
    /// one when shuffle is on); `name` is shown as "Playing from …".
    pub fn play_list(&mut self, list: Vec<String>, start: Option<&str>, name: &str) {
        // songs whose files are gone (in a playlist) are skipped
        let list: Vec<String> = list.into_iter().filter(|p| self.lib.by_path.contains_key(p)).collect();
        if list.is_empty() {
            return;
        }
        self.make_queue(list.clone(), name);
        let first = match start {
            Some(p) => p.to_string(),
            None if self.state.shuffle != "off" => {
                let i = (self.seed >> 33) as usize % list.len();
                list[i].clone()
            }
            None => list[0].clone(),
        };
        let path = self.queue.as_mut().unwrap().start(&first);
        self.fail_streak = 0;
        self.play_path(&path);
    }

    /// The Shuffle button of a page: shuffle on (songs of the list) and play.
    pub fn shuffle_list(&mut self, list: Vec<String>, name: &str) {
        if self.state.shuffle == "off" {
            self.state.shuffle = "songs".into();
            self.state_dirty = true;
        }
        self.play_list(list, None, name);
    }

    fn play_path(&mut self, path: &str) {
        let song = song_of(self.lib.track(path));
        let token = self.engine.play(path, 0.0, song);
        self.now = Some(NowPlaying { token, path: path.to_string(), active: true });
        self.message = None;
        self.update_next();
        self.state.last_path = path.to_string();
        self.state.last_position = 0.0;
        self.state_dirty = true;
    }

    /// Play Next (`first`) / Add to Queue (2.1): the songs come after the current one (Play
    /// Next before songs queued earlier, Add to Queue after them); with nothing playing they
    /// start playing.
    pub fn queue_songs(&mut self, paths: &[String], first: bool) {
        let paths: Vec<String> = paths.iter().filter(|p| self.lib.track(p).is_some()).cloned().collect();
        if paths.is_empty() {
            return;
        }
        let playing = self.now.as_ref().is_some_and(|n| n.active);
        match self.queue.as_mut() {
            Some(q) if playing && q.current().is_some() => {
                if first {
                    q.play_next(&paths);
                } else {
                    q.add_to_queue(&paths);
                }
                self.update_next();
            }
            _ => self.play_list(paths, None, "Queue"),
        }
    }

    /// Tell the engine which song follows gaplessly.
    pub fn update_next(&mut self) {
        let Some(q) = self.queue.as_ref() else {
            self.engine.set_next(None);
            return;
        };
        // what the queue plays when the current song ends by itself, asked of a copy (as the
        // Python app does), so repeat, the album rules and queued songs all count
        let next = q.clone().next(true);
        match next {
            Some(p) => {
                let song = song_of(self.lib.track(&p));
                self.engine.set_next(Some((&p, song)));
            }
            None => self.engine.set_next(None),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.now.as_ref().is_some_and(|n| n.active) && !self.engine.is_paused()
    }

    pub fn toggle_play(&mut self) {
        match &self.now {
            None => {
                let list = self.page_songs();
                let name = self.page.title().to_string();
                self.play_list(list, None, &name);
            }
            Some(n) if !n.active => {
                let p = n.path.clone();
                self.play_path(&p);
            }
            Some(_) => {
                if self.engine.is_paused() {
                    self.engine.resume();
                } else {
                    self.engine.pause();
                    self.remember_position();
                }
            }
        }
    }

    pub fn next(&mut self) {
        let n = self.queue.as_mut().and_then(|q| q.next(false));
        match n {
            Some(p) => self.play_path(&p),
            None => self.stop_at_end(),
        }
    }

    pub fn previous(&mut self) {
        if self.engine.position().unwrap_or(0.0) > 3.0 {
            self.engine.seek(0.0);
            return;
        }
        match self.queue.as_mut().and_then(|q| q.previous()) {
            Some(p) => self.play_path(&p),
            None => self.engine.seek(0.0),
        }
    }

    fn stop_at_end(&mut self) {
        self.engine.stop();
        if let Some(n) = self.now.as_mut() {
            n.active = false;
        }
        self.state.last_position = 0.0;
        self.state_dirty = true;
    }

    pub fn seek(&mut self, seconds: f64) {
        if self.now.as_ref().is_some_and(|n| !n.active) {
            let p = self.now.as_ref().unwrap().path.clone();
            self.play_path(&p);
        }
        self.engine.seek(seconds);
    }

    pub fn set_shuffle(&mut self, mode: &str) {
        self.state.shuffle = mode.to_string();
        if let Some(q) = self.queue.as_mut() {
            q.set_shuffle(mode);
        }
        self.update_next();
        self.state_dirty = true;
    }

    pub fn set_repeat(&mut self, mode: &str) {
        self.state.repeat = mode.to_string();
        if let Some(q) = self.queue.as_mut() {
            q.set_repeat(mode);
        }
        self.update_next();
        self.state_dirty = true;
    }

    pub fn toggle_like(&mut self, path: &str) {
        let liked = self.state.is_liked(path);
        self.state.set_liked(path, !liked);
        self.likes_gen += 1;
        self.save_state();
    }

    pub fn set_volume(&mut self, v: f64) {
        if self.scanner.cfg.bit_perfect {
            return; // the volume is not used then: it is set on the amplifier or DAC
        }
        self.state.volume = v.clamp(0.0, 1.0);
        self.engine.set_gain(engine_gain(&self.state, false));
        self.state_dirty = true;
    }

    pub fn toggle_mute(&mut self) {
        self.state.muted = !self.state.muted;
        self.engine.set_gain(engine_gain(&self.state, self.scanner.cfg.bit_perfect));
        self.state_dirty = true;
    }

    pub fn set_eq(&mut self) {
        self.engine.set_eq(&self.state.eq);
        self.state_dirty = true;
    }

    fn remember_position(&mut self) {
        if let (Some(n), Some(pos)) = (&self.now, self.engine.position()) {
            self.state.last_path = n.path.clone();
            self.state.last_position = if n.active { pos } else { 0.0 };
            self.state_dirty = true;
        }
    }

    pub fn save_state(&mut self) {
        self.remember_position();
        if let Err(e) = playerstate::save(&self.state, None) {
            eprintln!("Real-Hi-Res: cannot save the player state: {e}");
        }
        self.state_dirty = false;
        self.state_saved = Instant::now();
    }

    fn poll_engine(&mut self) {
        // bit-perfect playback may also be switched in the Quality check page's Settings window
        if self.engine.bit_perfect() != self.scanner.cfg.bit_perfect {
            let on = self.scanner.cfg.bit_perfect;
            self.engine.set_gain(engine_gain(&self.state, on));
            self.engine.set_bit_perfect(on);
        }
        for ev in self.engine.take_events() {
            match ev {
                Event::Started { token, path } => {
                    if self.now.as_ref().is_some_and(|n| n.token == token) {
                        continue;
                    }
                    // a song that followed gaplessly
                    let want = self.queue.as_mut().and_then(|q| q.next(true));
                    if want.as_deref() == Some(path.as_str()) {
                        self.now = Some(NowPlaying { token, path: path.clone(), active: true });
                        self.fail_streak = 0;
                        self.state.last_path = path;
                        self.state_dirty = true;
                        self.update_next();
                    } else if let Some(p) = want {
                        self.play_path(&p);
                    } else {
                        self.stop_at_end();
                    }
                }
                Event::Ended { token, chained, .. } => {
                    if chained || !self.now.as_ref().is_some_and(|n| n.token == token) {
                        continue;
                    }
                    self.fail_streak = 0;
                    match self.queue.as_mut().and_then(|q| q.next(true)) {
                        Some(p) => self.play_path(&p),
                        None => self.stop_at_end(),
                    }
                }
                Event::Failed { token, message, .. } => {
                    if !self.now.as_ref().is_some_and(|n| n.token == token) {
                        continue;
                    }
                    self.fail_streak += 1;
                    let total = self.queue.as_ref().map(|q| q.order.len()).unwrap_or(1).max(1);
                    let go_on = self.fail_streak < total.min(50);
                    self.message = Some((message, go_on.then(|| Instant::now() + Duration::from_secs(2))));
                    if !go_on {
                        self.stop_at_end();
                    }
                }
                Event::OutputError(m) => {
                    self.message = Some((m, None));
                    self.stop_at_end();
                }
            }
        }
        if let Some((_, Some(at))) = &self.message {
            if Instant::now() >= *at {
                self.message.as_mut().unwrap().1 = None;
                // a manual next, so "repeat song" does not loop over a file that fails
                match self.queue.as_mut().and_then(|q| q.next(false)) {
                    Some(p) => {
                        let keep = self.message.take();
                        self.play_path(&p);
                        // the failure stays visible until the next song plays
                        if self.engine.last_error.is_empty() {
                            let _ = keep;
                        }
                    }
                    None => self.stop_at_end(),
                }
            }
        }
    }

    /// The songs of the page shown (for Play with nothing playing).
    pub fn page_songs(&self) -> Vec<String> {
        let all = || self.lib.tracks.iter().map(|t| t.path.clone()).collect::<Vec<_>>();
        match &self.page {
            Page::Album(key) => self
                .lib
                .album_index
                .get(key)
                .map(|i| self.lib.albums[*i].tracks.iter().map(|t| t.path.clone()).collect())
                .unwrap_or_default(),
            Page::Liked => self.liked_songs().iter().map(|t| t.path.clone()).collect(),
            Page::AllSongs => self.sorted_songs(self.lib.tracks.iter().collect()).iter().map(|t| t.path.clone()).collect(),
            Page::Playlist(id) => self.playlist_songs(id),
            Page::Group(kind, name) => self
                .lib
                .find_group(kind, name)
                .map(|i| self.lib.groups(kind)[i].tracks.iter().map(|t| t.path.clone()).collect())
                .unwrap_or_default(),
            Page::Disliked => self.lib.tracks.iter().filter(|t| self.state.is_disliked(&t.path)).map(|t| t.path.clone()).collect(),
            _ => all(),
        }
    }

    pub fn liked_songs(&self) -> Vec<&Track> {
        self.lib.tracks.iter().filter(|t| self.state.is_liked(&t.path)).collect()
    }

    pub fn sorted_songs<'a>(&self, v: Vec<&'a Track>) -> Vec<&'a Track> {
        sort_tracks(v, Some(self.song_sort))
    }

    /// A song list of a page, sorted, made again only when the library, the sort or the
    /// likes changed: sorting thousands of songs every frame made scrolling lag.
    pub fn cached_list(
        &mut self,
        id: &str,
        sort: Option<(SongCol, bool)>,
        make: impl FnOnce(&Player) -> Vec<String>,
    ) -> Arc<Vec<String>> {
        let key = (self.lib.gen, sort, self.likes_gen);
        if let Some((k, v)) = self.list_cache.get(id) {
            if *k == key {
                return v.clone();
            }
        }
        let v = Arc::new(make(self));
        if self.list_cache.len() > 64 {
            self.list_cache.clear();
        }
        self.list_cache.insert(id.to_string(), (key, v.clone()));
        v
    }

    pub fn album_sort_for(&self) -> &str {
        &self.album_sort_for
    }

    /// Another album page is shown: in track order again.
    pub fn set_album_sort_for(&mut self, key: &str) {
        self.album_sort_for = key.to_string();
        self.album_sort = None;
    }

    /// The sort of a kind of song list: album pages have their own (track order at first).
    pub fn sort_of(&self, kind: &str) -> Option<(SongCol, bool)> {
        if kind == "album" {
            self.album_sort
        } else {
            Some(self.song_sort)
        }
    }

    pub fn set_sort(&mut self, kind: &str, col: SongCol) {
        let cur = self.sort_of(kind);
        let next = match cur {
            Some((c, rev)) if c == col => (col, !rev),
            _ => (col, false),
        };
        if kind == "album" {
            self.album_sort = Some(next);
        } else {
            self.song_sort = next;
        }
    }

    /// The page of an artist (Artists, else Album Artists).  Each artist of a field such
    /// as "Mariah Carey, Michael Bublé" has a page of its own; a name that is not an
    /// artist of the library opens the first artist `split_artists()` gives.
    pub fn open_artist(&mut self, name: &str) {
        let mut names = vec![name.to_string()];
        names.extend(musiclib::split_artists(name, &self.lib.known).into_iter().take(1));
        for n in &names {
            for kind in ["artists", "album_artists"] {
                if let Some(i) = self.lib.find_group(kind, n) {
                    let name = self.lib.groups(kind)[i].name.clone();
                    self.go(Page::Group(kind, name));
                    return;
                }
            }
        }
        self.go(Page::Artists);
    }

    // --------------------------------------------- renaming and merging ---

    /// Rename… a genre, composer or year: asks for the new name.
    pub fn ask_rename_group(&mut self, kind: &'static str, name: &str) {
        let noun = realhires::tagedit::group_noun(kind);
        let unknown = name == unknown_name(kind);
        let prompt = if unknown {
            format!("Give the songs without a {noun} this {noun}:")
        } else if kind == "years" {
            format!("Change the year {name} of these songs to:")
        } else {
            format!("New name of the {noun} “{name}”:")
        };
        let initial = if unknown { "" } else { name };
        self.ask_name_with(&format!("Rename {noun}"), &prompt, initial, NameAction::RenameGroup(kind, name.to_string()), vec![]);
    }

    /// Merge into… (one genre) or Merge N genres… (several): asks for the genre they become.
    pub fn ask_merge_groups(&mut self, kind: &'static str, names: Vec<String>) {
        let groups: Vec<&Group> = names.iter().filter_map(|n| self.lib.find_group(kind, n)).map(|i| &self.lib.groups(kind)[i]).collect();
        if groups.is_empty() {
            return;
        }
        if groups.len() == 1 {
            let mine = casefold(groups[0].name.trim());
            let others: Vec<String> = self
                .lib
                .groups(kind)
                .iter()
                .filter(|g| casefold(g.name.trim()) != mine && g.name != unknown_name(kind))
                .map(|g| g.name.clone())
                .collect();
            let prompt = format!("Merge the genre “{}” into which genre? Choose one, or type a new name.", groups[0].name);
            self.ask_name_with("Merge into", &prompt, "", NameAction::MergeGroups(kind, names), others);
        } else {
            let mut biggest = groups.clone();
            biggest.sort_by_key(|g| std::cmp::Reverse(g.tracks.len()));
            let choices: Vec<String> = biggest.iter().filter(|g| g.name != unknown_name(kind)).map(|g| g.name.clone()).collect();
            let first = choices.first().cloned().unwrap_or_default();
            let k = groups.len();
            self.ask_name_with(
                &format!("Merge {k} genres"),
                &format!("Merge {k} genres into one. Its name:"),
                &first,
                NameAction::MergeGroups(kind, names),
                choices,
            );
        }
    }

    /// Check the new name, find the songs that do not have it yet and ask before
    /// changing their files (`ConfirmAction::RetagGroup`); Err keeps the name dialog open.
    pub fn retag_group(&mut self, kind: &'static str, names: &[String], text: &str, merge: bool) -> Result<(), String> {
        let new = realhires::tagedit::check_group_name(kind, text).map_err(|e| e.0)?;
        if self.tag_job.as_ref().is_some_and(|j| !j.done()) {
            return Err("Songs are still being changed; try again when that has finished.".into());
        }
        let noun = realhires::tagedit::group_noun(kind);
        let groups: Vec<usize> = names.iter().filter_map(|n| self.lib.find_group(kind, n)).collect();
        let mut seen = HashSet::new();
        let mut paths = vec![];
        for &g in &groups {
            for t in &self.lib.groups(kind)[g].tracks {
                if group_value(kind, t) != new && seen.insert(t.path.clone()) {
                    paths.push(t.path.clone());
                }
            }
        }
        if paths.is_empty() {
            return Err(format!("Every song already has the {noun} “{new}”: nothing to change."));
        }
        let from: Vec<String> = groups.iter().map(|&g| self.lib.groups(kind)[g].name.clone()).collect();
        let n = crate::style::plural(paths.len(), "song");
        let old = from.first().cloned().unwrap_or_default();
        let (title, what) = if merge && from.len() > 1 {
            let listed: Vec<String> = from.iter().map(|f| format!("“{f}”")).collect();
            (
                "Merge genres".to_string(),
                format!("Merge {} genres ({}) into “{new}”? The genre of {n} becomes “{new}”.", from.len(), listed.join(", ")),
            )
        } else if merge {
            ("Merge genres".to_string(), format!("Merge the genre “{old}” into “{new}”? The genre of {n} becomes “{new}”."))
        } else if old == unknown_name(kind) {
            (format!("Rename {noun}"), format!("Give {n} without a {noun} the {noun} “{new}”?"))
        } else if kind == "years" {
            (format!("Rename {noun}"), format!("Change the year {old} to {new}? The release date of {n} changes (only its year)."))
        } else {
            (format!("Rename {noun}"), format!("Rename the {noun} “{old}” to “{new}”? The {noun} of {n} changes."))
        };
        self.confirm = Some(ConfirmAction::RetagGroup {
            title,
            text: format!("{what} Only the information in the files is changed; the music itself is not touched."),
            kind,
            paths,
            name: new,
            from,
        });
        Ok(())
    }

    /// The Equalizer's Delete…: the preset named (chosen from its menu), else the one the
    /// equalizer is set to; asks first (`ConfirmAction::DeleteEqPreset`).
    pub fn eq_delete(&mut self, name: Option<String>) -> bool {
        let name = name.or_else(|| Some(self.state.matching_eq_preset()).filter(|n| !n.is_empty()));
        match name {
            Some(n) if self.state.eq_preset(&n).is_some() => {
                self.confirm = Some(ConfirmAction::DeleteEqPreset(n));
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------ lyrics ---

    /// The lyrics of the song playing, read again when lyrics were saved.
    pub fn lyrics_of_now(&mut self) -> Option<ShownLyrics> {
        use realhires::lyrics::Status;
        let path = self.now.as_ref()?.path.clone();
        let gen = self.scanner.lyrics_gen;
        if let Some((p, g, l, _)) = &self.lyrics {
            if *p == path && *g == gen {
                return l.clone();
            }
        }
        let mut file = String::new();
        let shown = realhires::lyrics::read_lyrics_file(&path).and_then(|(kind, text, f)| {
            file = f;
            match kind {
                Status::Instrumental => Some(ShownLyrics::Instrumental),
                Status::Synced => {
                    let lines = realhires::embed::lrc_entries(&text);
                    if lines.is_empty() {
                        let plain = realhires::lyrics::strip_timestamps(&text);
                        (!plain.trim().is_empty()).then_some(ShownLyrics::Plain(plain))
                    } else {
                        Some(ShownLyrics::Synced(lines))
                    }
                }
                _ => (!text.trim().is_empty()).then(|| ShownLyrics::Plain(text)),
            }
        });
        // the label above them says what is shown (synced lyrics without a timed line show as plain)
        let label = match &shown {
            Some(l) => {
                let kind = match l {
                    ShownLyrics::Synced(_) => Status::Synced,
                    ShownLyrics::Plain(_) => Status::Plain,
                    ShownLyrics::Instrumental => Status::Instrumental,
                };
                let (a, b) = realhires::lyrics::shown_label(kind, &path, &file);
                (a.to_string(), b)
            }
            None => (String::new(), String::new()),
        };
        self.lyrics = Some((path, gen, shown.clone(), label));
        self.lyrics_line = None;
        self.lyrics_away = false;
        shown
    }

    /// What Now Playing writes above the lyrics: ("Synced lyrics", ".lrc file"), or empty.
    pub fn lyrics_label_of_now(&mut self) -> (String, String) {
        self.lyrics_of_now();
        self.lyrics.as_ref().map(|l| l.3.clone()).unwrap_or_default()
    }

    /// A+ (+1) / A− (-1) on Now Playing: the lyrics larger or smaller, saved at once.
    pub fn lyrics_size_step(&mut self, steps: i64) {
        let before = self.state.lyrics_size;
        if self.state.step_lyrics_size(steps) != before {
            self.save_state();
        }
    }

    /// Get lyrics (Now Playing, a song's menu): the lyrics window for these songs, which
    /// starts downloading at once; Now Playing shows the lyrics when they are saved.
    pub fn get_lyrics(&mut self, paths: Vec<String>) {
        self.scanner.download_lyrics_now(paths);
    }

    /// Now Playing's Details for the song playing: the comment tag (read by ffprobe in a
    /// thread) and the saved Quality check verdict, read again when the song changes.
    pub fn details_extra(&mut self, path: &str) -> (String, Option<(String, String)>) {
        while let Ok((p, comment)) = self.details_rx.try_recv() {
            if let Some(e) = self.details_extra.get_mut(&p) {
                e.0 = Some(comment);
            }
        }
        if self.details_song != path {
            self.details_song = path.to_string();
            self.details_extra.retain(|_, e| e.0.is_none()); // still being read: the answer is on its way
            self.details_extra.remove(path);
        }
        if !self.details_extra.contains_key(path) {
            let skey = realhires::store::settings_key(&self.scanner.cfg);
            let verdict = self
                .scanner
                .results
                .get(path)
                .cloned()
                .or_else(|| self.scanner.store.as_ref().and_then(|s| s.lookup(path, &skey, None)))
                .map(|r| (crate::style::verdict_style(r.verdict).0.to_string(), r.summary.clone()));
            self.details_extra.insert(path.to_string(), (None, verdict));
            let (tx, p, ctx) = (self.details_tx.clone(), path.to_string(), self.ctx.clone());
            std::thread::spawn(move || {
                let comment = read_comment(&p);
                let _ = tx.send((p, comment));
                ctx.request_repaint();
            });
        }
        let e = &self.details_extra[path];
        (e.0.clone().unwrap_or_default(), e.1.clone())
    }

    // ------------------------------------------------------------ settings ---

    fn apply_look(&mut self, ctx: &egui::Context) {
        let want = (self.scanner.cfg.font_family.clone(), self.scanner.cfg.theme.clone(), self.themes_gen);
        if self.applied_look.as_ref() != Some(&want) {
            if self.applied_look.as_ref().is_none_or(|a| a.0 != want.0) {
                self.font_used = look::install_fonts(ctx, &want.0);
            }
            look::install_theme(ctx, &want.1, &self.all_themes());
            self.applied_look = Some(want);
        }
    }

    /// The user's themes changed: save them and draw with them.
    pub fn themes_changed(&mut self) {
        if let Err(e) = realhires::themes::save_custom(&self.custom_themes, None) {
            self.scanner.info = Some(("Themes".into(), format!("The themes could not be saved: {e}.")));
        }
        self.file_themes = realhires::themes::load_folder(&self.custom_themes, None);
        self.themes_gen += 1;
        self.fill_choices();
    }

    /// The user's own themes, then those of the themes folder.
    pub fn all_themes(&self) -> Vec<realhires::themes::Theme> {
        let mut all = self.custom_themes.clone();
        all.extend(self.file_themes.iter().map(|f| f.theme.clone()));
        all
    }

    /// The file a theme of the themes folder comes from.
    pub fn theme_file(&self, name: &str) -> Option<&std::path::Path> {
        self.file_themes.iter().find(|f| f.theme.name == name).map(|f| f.file.as_path())
    }

    /// Read the themes folder again; true when its themes changed.
    pub fn reload_file_themes(&mut self) -> bool {
        self.file_themes_read = Instant::now();
        let now = realhires::themes::load_folder(&self.custom_themes, None);
        if now == self.file_themes {
            return false;
        }
        self.file_themes = now;
        self.themes_gen += 1;
        self.fill_choices();
        true
    }

    /// Import a theme file into the user's own themes and use it (Import theme…,
    /// or a .json dropped onto the window).
    pub fn import_theme_path(&mut self, path: &std::path::Path) {
        let all = self.all_themes();
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| {
            realhires::themes::import_theme(&String::from_utf8_lossy(&b), &all).map_err(|e| e.0)
        }) {
            Ok(t) => {
                let name = t.name.clone();
                self.custom_themes.push(t);
                self.themes_changed();
                self.set_theme(&name);
            }
            Err(e) => self.scanner.info = Some(("Import theme".into(), e)),
        }
    }

    /// Open the Edit Info window for one song (one at a time).
    pub fn edit_song_info(&mut self, path: &str) {
        let (heading, sub) = match self.lib.track(path) {
            Some(t) => (
                t.display_title(),
                [t.artist.as_str(), t.album.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · "),
            ),
            None => (realhires::paths::file_name(path), String::new()),
        };
        self.edit_info(vec![path.to_string()], heading, sub);
    }

    /// Open the Edit Info window for every song of an album.
    pub fn edit_album_info(&mut self, album: &Album) {
        let n = album.tracks.len();
        let count = format!("{} song{}", realhires::stats::thousands(n as i64), if n == 1 { "" } else { "s" });
        let sub = [album.artist.as_str(), count.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
        self.edit_info(album.tracks.iter().map(|t| t.path.clone()).collect(), album.title.clone(), sub);
    }

    /// Open the Edit Info window for these songs; while one is open nothing happens.
    pub fn edit_info(&mut self, paths: Vec<String>, heading: String, subheading: String) {
        let paths: Vec<String> = paths.into_iter().filter(|p| std::path::Path::new(p).is_file()).collect();
        if self.edit_info.is_some() || paths.is_empty() {
            return;
        }
        self.edit_info = Some(crate::editinfo::EditInfo::new(&self.ctx, paths, heading, subheading));
    }

    /// Open the Cover window of this song's cover (a click on Now Playing's
    /// cover, Cover… in the song menus); one at a time, and not while it works.
    pub fn open_cover_window(&mut self, path: &str) {
        if self.cover_win.as_ref().is_some_and(|w| w.busy || w.path == path) {
            return;
        }
        if !std::path::Path::new(path).is_file() {
            return;
        }
        let album = self.lib.album_of(path).cloned();
        let track = self.lib.track(path).cloned();
        self.cover_win = Some(crate::coverwin::CoverWin::new(&self.ctx, path, album.as_ref(), track.as_ref(), &self.scanner.cfg));
    }

    /// Use the theme called `name` (saved in the settings at once).
    pub fn set_theme(&mut self, name: &str) {
        let mut cfg = self.scanner.cfg.clone();
        cfg.theme = name.to_string();
        self.scanner.apply_settings(cfg.clone());
        if let Some(d) = self.settings_draft.as_mut() {
            d.theme = name.to_string();
        }
    }

    // ----------------------------------------------- history and stats ---

    /// Follow what is heard, and log a song to the play history when it stops
    /// being the current one.
    fn track_listening(&mut self) {
        let cur = self.now.as_ref().filter(|n| n.active).map(|n| (n.token, n.path.clone()));
        let pos = self.engine.position().unwrap_or(0.0);
        let same = match (&self.listening, &cur) {
            (Some(l), Some((token, _))) => l.token == *token,
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.flush_listening();
            if let Some((token, path)) = cur {
                self.listening = Some(Listening { token, path, started: wall_clock(), seconds: 0.0, last_pos: pos });
            }
            return;
        }
        let playing = self.is_playing();
        if let Some(l) = self.listening.as_mut() {
            if playing {
                let d = pos - l.last_pos;
                if d > 0.0 && d < 2.0 {
                    l.seconds += d;
                }
            }
            l.last_pos = pos;
        }
    }

    fn flush_listening(&mut self) {
        let Some(l) = self.listening.take() else { return };
        let track = self.lib.track(&l.path).cloned().unwrap_or_else(|| Track { path: l.path.clone(), ..Default::default() });
        if let Some(h) = &self.history {
            if h.add_play(&track, l.started, l.seconds) {
                self.history_gen += 1;
            }
        }
    }

    /// (stats of the chosen period), computed again when there are new plays.
    pub fn stats(&mut self) -> realhires::stats::Stats {
        let key = (self.stats_period.clone(), self.history_gen);
        if let Some((p, g, st)) = &self.stats_cache {
            if *p == key.0 && *g == key.1 {
                return st.clone();
            }
        }
        let (since, until) = realhires::stats::period_range(&self.stats_period, wall_clock());
        let plays = self.history.as_ref().map(|h| h.plays(since, until)).unwrap_or_default();
        let st = realhires::stats::compute(&plays, &realhires::stats::local_parts);
        self.stats_cache = Some((key.0, key.1, st.clone()));
        st
    }

    /// The songs played last, newest first, with when.
    pub fn recently_played(&mut self) -> Vec<(String, f64)> {
        if let Some((g, v)) = &self.recent_cache {
            if *g == self.history_gen {
                return v.clone();
            }
        }
        let mut v = vec![];
        if let Some(h) = &self.history {
            let paths = h.recently_played(200);
            let plays = h.plays(0.0, None);
            let mut last: HashMap<&str, f64> = HashMap::new();
            for p in &plays {
                last.insert(p.path.as_str(), p.started);
            }
            v = paths.into_iter().map(|p| {
                let t = last.get(p.as_str()).copied().unwrap_or(0.0);
                (p, t)
            }).collect();
        }
        self.recent_cache = Some((self.history_gen, v.clone()));
        v
    }

    pub fn clear_history(&mut self) {
        if let Some(h) = &self.history {
            h.clear_history();
        }
        self.history_gen += 1;
    }

    // ------------------------------------------- playlists and dislikes ---

    pub fn playlist_songs(&self, id: &str) -> Vec<String> {
        self.state.playlist(id).map(|p| p.songs.clone()).unwrap_or_default()
    }

    pub fn new_playlist(&mut self, name: &str, songs: &[String]) -> String {
        let id = self.state.create_playlist(name, songs, wall_clock());
        self.save_state();
        id
    }

    pub fn add_to_playlist(&mut self, id: &str, songs: &[String]) {
        let n = self.state.add_to_playlist(id, songs);
        self.save_state();
        if let Some(p) = self.state.playlist(id) {
            self.message_info(&format!(
                "{} added to “{}”.",
                crate::style::plural(n, "song"),
                p.name
            ));
        }
    }

    fn message_info(&mut self, text: &str) {
        self.scanner.status = text.to_string();
    }

    pub fn toggle_dislike(&mut self, path: &str) {
        let d = self.state.is_disliked(path);
        self.state.set_disliked(path, !d);
        self.likes_gen += 1;
        self.save_state();
        if let Some(q) = self.queue.as_mut() {
            q.disliked = self.state.disliked.iter().cloned().collect();
        }
        self.update_next();
    }

    /// Run what a name dialog was opened for; Err keeps it open with the message.
    pub fn apply_name(&mut self, action: NameAction, text: &str) -> Result<(), String> {
        match action {
            NameAction::NewPlaylist(songs) => {
                if realhires::playerstate::clean_name(text).is_empty() {
                    return Err("Give the playlist a name.".into());
                }
                let id = self.new_playlist(text, &songs);
                if songs.is_empty() {
                    self.go(Page::Playlist(id));
                }
            }
            NameAction::RenamePlaylist(id) => {
                self.state.rename_playlist(&id, text).map_err(|e| e.0)?;
                self.save_state();
            }
            NameAction::SaveEqPreset => {
                let eq = self.state.eq.clone();
                self.state.save_eq_preset(text, &eq, false).map_err(|e| e.0)?;
                self.save_state();
            }
            NameAction::RenameEqPreset(old) => {
                self.state.rename_eq_preset(&old, text).map_err(|e| e.0)?;
                self.save_state();
            }
            NameAction::NewTheme(base) => {
                let all = self.all_themes();
                let name = realhires::themes::check_name(text, &all, "").map_err(|e| e.0)?;
                let mut t = realhires::themes::resolve(&base, self.ctx.global_style().visuals.dark_mode, &all);
                t.name = name.clone();
                self.custom_themes.push(t);
                self.themes_changed();
                self.set_theme(&name);
            }
            NameAction::RenameGroup(kind, name) => self.retag_group(kind, &[name], text, false)?,
            NameAction::MergeGroups(kind, names) => self.retag_group(kind, &names, text, true)?,
            NameAction::RenameTheme(old) => {
                let name = realhires::themes::check_name(text, &self.all_themes(), &old).map_err(|e| e.0)?;
                if let Some(t) = self.custom_themes.iter_mut().find(|t| t.name == old) {
                    t.name = name.clone();
                }
                self.themes_changed();
                if self.scanner.cfg.theme == old {
                    self.set_theme(&name);
                }
            }
        }
        Ok(())
    }

    pub fn confirmed(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeletePlaylist(id) => {
                self.state.delete_playlist(&id);
                self.save_state();
                let gone = Page::Playlist(id);
                self.back.retain(|p| *p != gone);
                self.forward.retain(|p| *p != gone);
                if self.page == gone {
                    self.page = Page::Playlists;
                }
            }
            ConfirmAction::DeleteEqPreset(name) => {
                self.state.delete_eq_preset(&name);
                self.save_state();
            }
            ConfirmAction::DeleteTheme(name) => {
                self.custom_themes.retain(|t| t.name != name);
                self.themes_changed();
                if self.scanner.cfg.theme == name {
                    self.set_theme(realhires::themes::SYSTEM);
                }
            }
            ConfirmAction::ClearHistory => self.clear_history(),
            ConfirmAction::RetagGroup { title, kind, paths, name, from, .. } => {
                if self.tag_job.as_ref().is_none_or(|j| j.done()) {
                    // the page of the group follows it to its new name
                    if let Page::Group(k, n) = &self.page {
                        if *k == kind && from.iter().any(|f| casefold(f.trim()) == casefold(n.trim())) {
                            self.page = Page::Group(kind, name.clone());
                        }
                    }
                    self.group_selected.clear();
                    let cfg = self.scanner.cfg.clone();
                    self.tag_job = Some(crate::editinfo::RenameJob::new(&self.ctx, title, kind, paths, name, cfg));
                }
            }
        }
    }

    pub fn ask_name(&mut self, title: &str, text: &str, action: NameAction) {
        self.ask_name_with(title, "", text, action, vec![]);
    }

    pub fn ask_name_with(&mut self, title: &str, prompt: &str, text: &str, action: NameAction, choices: Vec<String>) {
        self.name_dialog = Some(NameDialog {
            title: title.into(),
            text: text.into(),
            action,
            error: String::new(),
            prompt: prompt.into(),
            choices,
        });
    }

    /// Import an equalizer preset file's text: applied and kept under its name.
    pub fn import_eq_text(&mut self, text: &str, file_name: &str) -> Result<String, String> {
        let (name, st, notes) = realhires::eq::import_eq(text, file_name).map_err(|e| e.0)?;
        let mut candidate = name.clone();
        let mut n = 2;
        while let Err(e) = self.state.check_preset_name(&candidate, "") {
            if candidate.is_empty() || e.0.contains("built-in") {
                candidate = format!("{name} (imported)");
                if self.state.check_preset_name(&candidate, "").is_ok() {
                    break;
                }
            }
            candidate = format!("{name} {n}");
            n += 1;
            if n > 999 {
                return Err(e.0);
            }
        }
        let saved = self.state.save_eq_preset(&candidate, &st, false).map_err(|e| e.0)?;
        self.state.eq = st;
        self.set_eq();
        self.save_state();
        self.eq_notes = notes;
        Ok(saved)
    }

    /// The Settings page edits a draft that is applied after a short pause.
    pub fn settings_draft(&mut self) -> &mut Settings {
        if self.settings_draft.is_none() {
            self.settings_draft = Some(self.scanner.cfg.clone());
        }
        self.settings_draft.as_mut().unwrap()
    }

    pub fn settings_changed(&mut self) {
        self.settings_due = Some(Instant::now() + Duration::from_millis(400));
    }

    fn apply_settings_due(&mut self, ctx: &egui::Context) {
        let Some(at) = self.settings_due else { return };
        if Instant::now() < at || ctx.input(|i| i.pointer.any_down()) {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        self.settings_due = None;
        let Some(new) = self.settings_draft.clone() else { return };
        let old = self.scanner.cfg.clone();
        if new == old {
            return;
        }
        self.scanner.apply_settings(new.clone());
        if new.gapless != old.gapless {
            self.engine.set_gapless(new.gapless);
        }
        if new.output_device != old.output_device {
            self.engine.set_device(&new.output_device);
        }
        if new.bit_perfect != old.bit_perfect {
            // the song playing restarts where it is, the device opened the new way
            self.engine.set_gain(engine_gain(&self.state, new.bit_perfect));
            self.engine.set_bit_perfect(new.bit_perfect);
        }
        if new.library_folders != old.library_folders || new.extension_set() != old.extension_set() {
            self.lib_stop.store(true, Ordering::Relaxed);
            self.lib_rx = None;
            self.update_library();
        }
    }

    pub fn fill_choices(&mut self) {
        if !self.scanner.choices.contains_key("font_family") {
            let mut fonts = vec![String::new()];
            fonts.extend(look::installed_families());
            self.scanner.choices.insert("font_family", fonts);
        }
        let mut devices = vec![String::new()];
        devices.extend(crate::audio::output_devices());
        self.scanner.choices.insert("output_device", devices);
        self.scanner.choices.insert("theme", realhires::themes::names(&self.all_themes()));
    }

    pub fn add_library_folder(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            let d = dir.to_string_lossy().into_owned();
            let draft = self.settings_draft();
            if !draft.library_folders.contains(&d) {
                draft.library_folders.push(d);
            }
            self.settings_due = Some(Instant::now());
        }
    }

    /// Settings → Library → Check the library for fake files.
    pub fn check_library(&mut self) {
        let folders = self.scanner.cfg.library_folders.clone();
        if folders.is_empty() {
            self.scanner.info =
                Some(("Check the library".into(), "Add your music folder under Settings → Library first.".into()));
            return;
        }
        self.scanner.clear();
        for f in folders {
            self.scanner.add_source(&f);
        }
        self.scanner.start_scan(None, false);
        self.go(Page::Quality);
    }

    pub fn lyrics_for_library(&mut self) {
        let paths: Vec<String> = self.lib.tracks.iter().map(|t| t.path.clone()).collect();
        if paths.is_empty() {
            self.scanner.info = Some((
                "Download synced lyrics".into(),
                "The library is empty; add your music folder under Settings → Library first.".into(),
            ));
            return;
        }
        self.scanner.download_lyrics(paths);
    }

    // -------------------------------------------------------------- pages ---

    /// Open a page (a new page clears the Forward list, as in a browser).
    pub fn go(&mut self, page: Page) {
        if self.page != page {
            self.forward.clear();
            self.enter(page);
        }
    }

    /// Show `page`, remembering the page left for Back (not Now Playing, Search,
    /// the Quality check or Settings).
    fn enter(&mut self, page: Page) {
        let old = std::mem::replace(&mut self.page, page);
        if !matches!(old, Page::NowPlaying | Page::Search | Page::Quality | Page::Settings) {
            self.back.push(old);
            if self.back.len() > 20 {
                self.back.remove(0);
            }
        }
        if self.page == Page::Settings {
            self.settings_draft = None;
            self.fill_choices();
        }
    }

    pub fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            let left = std::mem::replace(&mut self.page, p);
            self.forward.push(left);
        }
    }

    /// Forward (the mouse's forward button, Alt+Right, Cmd+] on macOS): the page
    /// left last with Back (2.1).
    pub fn go_forward(&mut self) {
        if let Some(p) = self.forward.pop() {
            self.enter(p);
        }
    }

    pub fn open_info(&mut self) {
        let Some(n) = &self.now else { return };
        let path = n.path.clone();
        let cfg = &self.scanner.cfg;
        let skey = realhires::store::settings_key(cfg);
        let verdict = self
            .scanner
            .results
            .get(&path)
            .cloned()
            .or_else(|| self.scanner.store.as_ref().and_then(|s| s.lookup(&path, &skey, None)))
            .map(|r| (crate::style::verdict_style(r.verdict).0.to_string(), r.summary.clone()));
        self.info = Some(InfoWindow { path, verdict });
    }

    // ----------------------------------------------------------------- UI ---

    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| crate::app::is_theme_file(p)).collect()
        });
        for p in dropped {
            self.import_theme_path(&p);
        }
        self.apply_look(&ctx);
        self.scanner.background(&ctx);
        self.covers.poll(&ctx);
        self.poll_library();
        self.poll_engine();
        self.track_listening();
        self.apply_settings_due(&ctx);
        if let Some(at) = self.search_due {
            if Instant::now() >= at {
                self.search_due = None;
                self.search_query = self.search.trim().to_string();
                if self.search_query.is_empty() {
                    if self.page == Page::Search {
                        let back = self.search_from.take().unwrap_or(Page::Albums);
                        self.page = back;
                    }
                } else if self.page != Page::Search {
                    self.search_from = Some(self.page.clone());
                    self.page = Page::Search;
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        if self.state_dirty && self.state_saved.elapsed() > Duration::from_secs(10) {
            self.save_state();
        }
        self.keys(&ctx);
        let pal = look::current(&ctx);

        egui::Panel::left("player-sidebar")
            .exact_size(224.0)
            .resizable(false)
            .frame(Frame::new().fill(pal.sidebar).inner_margin(Margin::symmetric(12, 14)))
            .show(ui, |ui| self.sidebar(ui, &pal));
        egui::Panel::top("player-bar")
            .exact_size(70.0)
            .resizable(false)
            .frame(Frame::new().fill(pal.bg).inner_margin(Margin::symmetric(14, 8)))
            .show(ui, |ui| self.playback_bar(ui, &pal));
        if self.show_up_next {
            egui::Panel::right("player-up-next")
                .default_size(300.0)
                .resizable(true)
                .frame(Frame::new().fill(pal.sidebar).inner_margin(Margin::same(14)))
                .show(ui, |ui| self.up_next(ui, &pal));
        }
        egui::CentralPanel::default().frame(Frame::new().fill(pal.bg)).show(ui, |ui| {
            if self.page == Page::Quality {
                self.scanner.page(ui);
            } else {
                Frame::new().inner_margin(Margin::symmetric(28, 18)).show(ui, |ui| self.content(ui, &pal));
            }
        });
        self.windows(&ctx, &pal);
        if self.is_playing() {
            ctx.request_repaint_after(Duration::from_millis(if self.page == Page::NowPlaying { 50 } else { 200 }));
        } else if self.message.as_ref().is_some_and(|m| m.1.is_some()) || self.lib_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        // the mouse's back and forward buttons, on every page; Alt+Left / Alt+Right
        // (Cmd+[ / Cmd+] on macOS) when no text field has the keyboard (2.1)
        let typing = ctx.egui_wants_keyboard_input();
        let (back, forward) = ctx.input(|i| {
            let mac = cfg!(target_os = "macos");
            let key = |arrow, bracket| {
                if mac {
                    i.modifiers.mac_cmd && i.key_pressed(bracket)
                } else {
                    i.modifiers.alt && !i.modifiers.ctrl && i.key_pressed(arrow)
                }
            };
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1)
                    || (!typing && key(egui::Key::ArrowLeft, egui::Key::OpenBracket)),
                i.pointer.button_pressed(egui::PointerButton::Extra2)
                    || (!typing && key(egui::Key::ArrowRight, egui::Key::CloseBracket)),
            )
        });
        if back {
            self.go_back();
        } else if forward {
            self.go_forward();
        }
        if self.page == Page::Quality || typing {
            return;
        }
        let (space, right, left, cmd_f, cmd_l) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.modifiers.command && i.key_pressed(egui::Key::ArrowRight),
                i.modifiers.command && i.key_pressed(egui::Key::ArrowLeft),
                i.modifiers.command && i.key_pressed(egui::Key::F),
                i.modifiers.command && i.key_pressed(egui::Key::L),
            )
        });
        if cmd_l {
            self.lyrics_button();
        }
        if space {
            self.toggle_play();
        }
        if right && self.now.is_some() {
            self.next();
        }
        if left && self.now.is_some() {
            self.previous();
        }
        if cmd_f {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("player-search")));
        }
    }

    fn sidebar(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new(realhires::APP_NAME).font(look::title(17.0)).color(pal.text));
        });
        ui.add_space(10.0);
        let search = egui::TextEdit::singleline(&mut self.search)
            .id(egui::Id::new("player-search"))
            .hint_text("Search")
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(8, 5));
        if ui.add(search).changed() {
            self.search_due = Some(Instant::now() + Duration::from_millis(200));
        }
        ui.add_space(10.0);
        let section = |ui: &mut Ui, text: &str| {
            ui.add_space(8.0);
            ui.label(RichText::new(text).font(look::title(11.5)).color(pal.secondary));
            ui.add_space(2.0);
        };
        // room at the bottom for Quality Check, Settings and the library status
        let height = ui.available_height() - 96.0;
        egui::ScrollArea::vertical().id_salt("sidebar").max_height(height.max(100.0)).auto_shrink([false, true]).show(ui, |ui| {
            if sidebar_item(ui, pal, Icon::Library, "Library", self.page == Page::Library).clicked() {
                self.back.clear();
                self.go(Page::Library);
            }
            section(ui, "Library");
            let items: [(Icon, &str, Page); 8] = [
                (Icon::Note, "All Songs", Page::AllSongs),
                (Icon::Folder, "Folders", Page::Folders(None)),
                (Icon::Albums, "Albums", Page::Albums),
                (Icon::Person, "Artists", Page::Artists),
                (Icon::People, "Album Artists", Page::AlbumArtists),
                (Icon::Genres, "Genres", Page::Genres),
                (Icon::Calendar, "Years", Page::Years),
                (Icon::Composer, "Composers", Page::Composers),
            ];
            for (icon, label, page) in items {
                let selected = self.page == page
                    || matches!((&self.page, &page), (Page::Album(_), Page::Albums) | (Page::Folders(Some(_)), Page::Folders(None)));
                if sidebar_item(ui, pal, icon, label, selected).clicked() {
                    self.back.clear();
                    self.go(page);
                }
            }
            section(ui, "Playlists");
            if sidebar_item(ui, pal, Icon::Playlist, "Playlists", self.page == Page::Playlists).clicked() {
                self.go(Page::Playlists);
            }
            if sidebar_item(ui, pal, Icon::Heart, "Liked Songs", self.page == Page::Liked).clicked() {
                self.go(Page::Liked);
            }
            if sidebar_item(ui, pal, Icon::ThumbDown, "Disliked Songs", self.page == Page::Disliked).clicked() {
                self.go(Page::Disliked);
            }
            let lists: Vec<(String, String)> = self.state.playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
            for (id, name) in lists {
                let page = Page::Playlist(id);
                if sidebar_item(ui, pal, Icon::Note, &name, self.page == page).clicked() {
                    self.go(page);
                }
            }
            if sidebar_item(ui, pal, Icon::Playlist, "New Playlist…", false).clicked() {
                self.ask_name("New Playlist", "New Playlist", NameAction::NewPlaylist(vec![]));
            }
            section(ui, "History");
            let items: [(Icon, &str, Page); 3] = [
                (Icon::Clock, "Recently Played", Page::RecentlyPlayed),
                (Icon::Albums, "Recently Added", Page::Recent),
                (Icon::Chart, "Your Stats", Page::Stats),
            ];
            for (icon, label, page) in items {
                let selected = self.page == page || (page == Page::Stats && self.page == Page::StatsDetails);
                if sidebar_item(ui, pal, icon, label, selected).clicked() {
                    self.go(page);
                }
            }
        });
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            if let Some(s) = self.library_status() {
                ui.label(RichText::new(s).small().color(pal.secondary));
                ui.add_space(4.0);
            }
            if sidebar_item(ui, pal, Icon::Gear, "Settings", self.page == Page::Settings).clicked() {
                self.go(Page::Settings);
            }
            // always there (2.1: the menu bars are gone), just above Settings
            if sidebar_item(ui, pal, Icon::Check, "Quality Check", self.page == Page::Quality).clicked() {
                self.go(Page::Quality);
            }
        });
    }

    fn playback_bar(&mut self, ui: &mut Ui, pal: &Palette) {
        let has = self.now.is_some();
        let full = ui.available_rect_before_wrap();
        ui.allocate_ui_with_layout(full.size(), Layout::left_to_right(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            // left: transport
            let shuffle_on = self.state.shuffle != "off";
            let r = icons::button(ui, Icon::Shuffle, 30.0, shuffle_on, true, &format!("Shuffle: {}", shuffle_label(&self.state.shuffle)));
            egui::Popup::menu(&r).show(|ui| {
                ui.label(RichText::new("Shuffle").strong());
                for m in SHUFFLE_MODES {
                    if ui.radio(self.state.shuffle == m, shuffle_label(m)).clicked() {
                        self.set_shuffle(m);
                        ui.close();
                    }
                }
            });
            if icons::button(ui, Icon::Previous, 34.0, false, has, "Previous").clicked() {
                self.previous();
            }
            let play_icon = if self.is_playing() { Icon::Pause } else { Icon::Play };
            if icons::button(ui, play_icon, 42.0, false, true, if self.is_playing() { "Pause" } else { "Play" }).clicked() {
                self.toggle_play();
            }
            if icons::button(ui, Icon::Next, 34.0, false, has, "Next").clicked() {
                self.next();
            }
            let rep = self.state.repeat.as_str();
            let rep_icon = match rep {
                "song" => Icon::RepeatOne,
                "stop" => Icon::RepeatStop,
                _ => Icon::Repeat,
            };
            let r = icons::button(ui, rep_icon, 30.0, rep != "off", true, &format!("Repeat: {}", repeat_label(rep)));
            egui::Popup::menu(&r).show(|ui| {
                ui.label(RichText::new("Repeat").strong());
                for m in REPEAT_MODES {
                    if ui.radio(self.state.repeat == m, repeat_label(m)).clicked() {
                        self.set_repeat(m);
                        ui.close();
                    }
                }
            });
            // right part is laid out from the right edge
            let right_w = 360.0;
            let lcd_w = (ui.available_width() - right_w - 16.0).clamp(280.0, 720.0);
            let gap = ((ui.available_width() - right_w - lcd_w) / 2.0).max(8.0);
            ui.add_space(gap);
            self.lcd(ui, pal, lcd_w);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                // bit-perfect playback: the gain is exactly 1, so the slider shows full volume and cannot move
                let bit_perfect = self.scanner.cfg.bit_perfect;
                let mut v = if bit_perfect { 1.0 } else { self.state.volume as f32 };
                let slider = ui
                    .add_enabled_ui(!bit_perfect, |ui| {
                        ui.add_sized([96.0, 20.0], egui::Slider::new(&mut v, 0.0..=1.0).show_value(false))
                    })
                    .inner;
                let slider = if bit_perfect {
                    slider.on_disabled_hover_text("Bit-perfect playback is on: set the volume on your amplifier or DAC")
                } else {
                    slider
                };
                if slider.changed() {
                    self.set_volume(v as f64);
                }
                let vol_icon = if self.state.muted || self.state.volume <= 0.0 { Icon::SpeakerMuted } else { Icon::Speaker };
                if icons::button(ui, vol_icon, 28.0, false, true, if self.state.muted { "Unmute" } else { "Mute" }).clicked() {
                    self.toggle_mute();
                }
                ui.add_space(6.0);
                if icons::button(ui, Icon::Equalizer, 28.0, self.state.eq.enabled && !bit_perfect, true, "Equalizer").clicked() {
                    self.eq_open = !self.eq_open;
                }
                if icons::button(ui, Icon::UpNext, 28.0, self.show_up_next, true, "Up Next").clicked() {
                    self.show_up_next = !self.show_up_next;
                }
                let lyrics_on = self.page == Page::NowPlaying && !self.lyrics_hidden && self.np_tab == NpTab::Lyrics;
                if icons::button(ui, Icon::Lyrics, 28.0, lyrics_on, true, "Lyrics").clicked() {
                    self.lyrics_button();
                }
                let now_path = self.now.as_ref().map(|n| n.path.clone());
                let r = icons::button(ui, Icon::More, 28.0, false, has, "More");
                if let Some(p) = now_path.clone() {
                    egui::Popup::menu(&r).show(|ui| self.song_menu(ui, &p));
                }
                let disliked = now_path.as_ref().is_some_and(|p| self.state.is_disliked(p));
                if icons::button(ui, Icon::ThumbDown, 28.0, disliked, has, if disliked { "Remove dislike" } else { "Dislike" }).clicked() {
                    if let Some(p) = now_path.clone() {
                        self.toggle_dislike(&p);
                    }
                }
                let liked = now_path.as_ref().is_some_and(|p| self.state.is_liked(p));
                let heart = if liked { Icon::HeartFilled } else { Icon::Heart };
                if icons::button(ui, heart, 28.0, liked, has, if liked { "Unlike" } else { "Like" }).clicked() {
                    if let Some(p) = now_path {
                        self.toggle_like(&p);
                    }
                }
            });
        });
    }

    fn go_back_from_now(&mut self) {
        let back = self.back.pop().unwrap_or(Page::Albums);
        self.page = back;
    }

    /// The Lyrics button and Ctrl/Cmd+L: elsewhere they open Now Playing with the lyrics; on
    /// Now Playing they hide or show the lyrics (hidden, the cover is in the middle) and the
    /// page stays (2.1, as the user asked).
    pub fn lyrics_button(&mut self) {
        if self.page == Page::NowPlaying {
            if self.np_tab != NpTab::Lyrics {
                self.np_tab = NpTab::Lyrics;
                self.lyrics_hidden = false;
            } else {
                self.lyrics_hidden = !self.lyrics_hidden;
            }
        } else {
            self.lyrics_hidden = false;
            self.np_tab = NpTab::Lyrics;
            self.go(Page::NowPlaying);
        }
    }

    /// Now Playing's switch: Lyrics, Up Next or Details (kept for the session).
    pub fn set_np_tab(&mut self, tab: NpTab) {
        self.np_tab = tab;
        self.lyrics_hidden = false;
    }

    /// The album page of a song's album.
    pub fn open_album_of(&mut self, path: &str) {
        if let Some(key) = self.lib.album_of(path).map(|a| a.key.clone()) {
            self.go(Page::Album(key));
        }
    }

    /// Up Next (Now Playing): play that song now; the queue continues after it.
    pub fn jump_up_next(&mut self, i: usize) {
        // PlayQueue::jump() also handles the songs queued with Play Next / Add to Queue
        if let Some(p) = self.queue.as_mut().and_then(|q| q.jump(i)) {
            self.play_path(&p);
        }
    }

    /// A click on the LCD's cover opens Now Playing, or leaves it when it is shown.
    pub fn cover_clicked(&mut self) {
        if self.page == Page::NowPlaying {
            self.go_back_from_now();
        } else {
            self.go(Page::NowPlaying);
        }
    }

    /// The panel in the middle of the playback bar.
    fn lcd(&mut self, ui: &mut Ui, pal: &Palette, width: f32) {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 54.0), Sense::hover());
        let p = ui.painter_at(rect.expand(2.0));
        p.rect(rect, CornerRadius::same(8), pal.lcd, Stroke::new(1.0, pal.separator), egui::StrokeKind::Inside);
        let Some((now_path, now_active)) = self.now.as_ref().map(|n| (n.path.clone(), n.active)) else {
            p.text(rect.center(), Align2::CENTER_CENTER, realhires::APP_NAME, look::title(15.0), pal.secondary);
            return;
        };
        let path = now_path;
        let track = self.lib.track(&path).cloned().unwrap_or_else(|| Track { path: path.clone(), ..Default::default() });
        // cover
        let cover_rect = Rect::from_min_size(rect.min + vec2(5.0, 5.0), vec2(44.0, 44.0));
        let key = track.song().album_key();
        let paths: Vec<String> = self.lib.album_of(&path).map(|a| a.tracks.iter().map(|t| t.path.clone()).collect()).unwrap_or_else(|| vec![path.clone()]);
        let tex = self.covers.get(&key, &paths);
        cover(&p, cover_rect, tex.as_ref(), &track.display_album(), 5);
        let cover_resp = ui.interact(cover_rect, ui.id().with("lcd-cover"), Sense::click());
        if cover_resp.on_hover_text("Now Playing").clicked() {
            self.cover_clicked();
        }
        // texts
        let badge = musiclib::quality(&track);
        let badge_font = look::title(10.0);
        let badge_w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(badge.clone(), badge_font.clone(), pal.text).size().x) + 12.0;
        let text_left = cover_rect.right() + 10.0;
        let text_right = rect.right() - badge_w - 14.0;
        let mid = (text_left + text_right) / 2.0;
        let clip = Rect::from_x_y_ranges(text_left..=text_right, rect.y_range());
        let tp = ui.painter_at(clip);
        // the title and the album name open the album, each artist name that artist (2.1)
        let has_album = self.lib.album_of(&path).is_some();
        let mut open = None;
        let title = crate::pages::elided(ui, &track.display_title(), look::title(13.0), pal.text, clip.width());
        let trect = Rect::from_center_size(pos2(mid, rect.top() + 13.0), title.size());
        tp.galley(trect.min, title, pal.text);
        self.lcd_links.clear();
        if has_album {
            let r = ui.interact(trect, ui.scope_id().with("lcd-title"), Sense::click());
            if r.hovered() {
                tp.line_segment([pos2(trect.left(), trect.bottom() - 1.0), pos2(trect.right(), trect.bottom() - 1.0)], Stroke::new(1.0, pal.text));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() {
                open = Some(crate::pages::Link::Album);
            }
            self.lcd_links.push((trect, crate::pages::Link::Album));
        }
        match &self.message {
            Some((m, _)) => {
                tp.text(pos2(mid, rect.top() + 29.0), Align2::CENTER_CENTER, m.clone(), look::text(11.5), pal.accent);
            }
            None => {
                let artists = self.lib.artists_of(&track);
                let pieces = crate::pages::artist_line(ui, look::text(11.5), pal.secondary, &artists, &track.display_album(), has_album, clip.width());
                let (rects, clicked) = crate::pages::paint_links(ui, &tp, pieces, pos2(mid, rect.top() + 29.0), pal.secondary, ui.scope_id().with("lcd-links"));
                self.lcd_links.extend(rects);
                open = open.or(clicked);
            }
        }
        // seek bar
        let dur = track.duration.max(0.0);
        let pos = if now_active { self.engine.position().unwrap_or(0.0) } else { 0.0 };
        let bar = Rect::from_x_y_ranges(text_left + 34.0..=text_right - 38.0, rect.top() + 41.0..=rect.top() + 45.0);
        let resp = ui.interact(bar.expand2(vec2(0.0, 5.0)), ui.id().with("seek"), Sense::click_and_drag());
        let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
        let shown = if resp.dragged() || resp.clicked() {
            resp.interact_pointer_pos().map(|q| ((q.x - bar.left()) / bar.width()).clamp(0.0, 1.0)).unwrap_or(frac)
        } else {
            frac
        };
        p.rect_filled(bar, CornerRadius::same(2), pal.separator);
        p.rect_filled(Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * shown, bar.bottom())), CornerRadius::same(2), pal.secondary);
        if resp.hovered() || resp.dragged() {
            p.circle_filled(pos2(bar.left() + bar.width() * shown, bar.center().y), 5.0, pal.text);
        }
        if (resp.drag_stopped() || resp.clicked()) && dur > 0.0 {
            self.seek(shown as f64 * dur);
        }
        let t_font = look::text(10.0);
        p.text(pos2(bar.left() - 6.0, bar.center().y), Align2::RIGHT_CENTER, fmt_time(shown as f64 * dur), t_font.clone(), pal.secondary);
        p.text(pos2(bar.right() + 6.0, bar.center().y), Align2::LEFT_CENTER, format!("-{}", fmt_time((dur - shown as f64 * dur).max(0.0))), t_font, pal.secondary);
        // quality badge: a click opens the Playback info
        let brect = Rect::from_center_size(pos2(rect.right() - badge_w / 2.0 - 8.0, rect.center().y), vec2(badge_w, 18.0));
        let bresp = ui.interact(brect, ui.id().with("quality-badge"), Sense::click());
        let bcol = if bresp.hovered() { pal.text } else { pal.secondary };
        p.rect_stroke(brect, CornerRadius::same(4), Stroke::new(1.0, bcol), egui::StrokeKind::Inside);
        p.text(brect.center(), Align2::CENTER_CENTER, badge, badge_font, bcol);
        if bresp.on_hover_text("Playback info").clicked() {
            self.open_info();
        }
        match open {
            Some(crate::pages::Link::Artist(name)) => self.open_artist(&name),
            Some(crate::pages::Link::Album) => self.open_album_of(&path),
            None => {}
        }
    }

    fn up_next(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.label(RichText::new("Up Next").font(look::title(18.0)).color(pal.text));
        if !self.queue_name.is_empty() {
            ui.label(RichText::new(format!("Playing from {}", self.queue_name)).color(pal.secondary));
        }
        ui.add_space(8.0);
        let Some(q) = &self.queue else {
            ui.label(RichText::new("Nothing is queued.").color(pal.secondary));
            return;
        };
        let upcoming = q.up_next(50);
        if upcoming.is_empty() {
            ui.label(RichText::new("Nothing comes after this song.").color(pal.secondary));
        }
        let mut jump = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, path) in upcoming.iter().enumerate() {
                let t = self.lib.track(path);
                let title = t.map(|t| t.display_title()).unwrap_or_else(|| path.clone());
                let artist = t.map(|t| t.display_artist()).unwrap_or_default();
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
                }
                let p = ui.painter_at(rect);
                p.text(rect.left_top() + vec2(8.0, 4.0), Align2::LEFT_TOP, title, look::text(13.0), pal.text);
                p.text(rect.left_top() + vec2(8.0, 21.0), Align2::LEFT_TOP, artist, look::text(11.0), pal.secondary);
                if resp.double_clicked() {
                    jump = Some(i);
                }
            }
        });
        if let Some(i) = jump {
            if let Some(p) = self.queue.as_mut().and_then(|q| q.jump(i)) {
                self.play_path(&p);
            }
        }
    }

    fn windows(&mut self, ctx: &egui::Context, pal: &Palette) {
        self.dialogs(ctx, pal);
        let cover_open = crate::coverwin::cover_window(self, ctx, pal);
        if let Some(mut w) = self.edit_info.take() {
            let cfg = self.scanner.cfg.clone();
            match w.show(ctx, pal, &cfg) {
                crate::editinfo::Outcome::Open => self.edit_info = Some(w),
                crate::editinfo::Outcome::Closed => {}
                crate::editinfo::Outcome::Saved { close } => {
                    self.update_library();
                    if !close {
                        self.edit_info = Some(w);
                    }
                }
            }
            if self.edit_info.is_some() {
                return; // Esc belongs to the Edit Info window while it is open
            }
        }
        if let Some(mut job) = self.tag_job.take() {
            let (open, finished) = job.show(ctx, pal);
            if finished {
                self.update_library();
            }
            if open {
                self.tag_job = Some(job);
            }
        }
        // Esc closes the topmost of the player's windows
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() && !cover_open {
            if self.eq_open {
                self.eq_open = false;
                self.save_state();
            } else if self.info.is_some() {
                self.info = None;
            }
        }
        if self.info.is_some() {
            crate::eqwin::info_window(self, ctx, pal);
        }
        if self.eq_open {
            crate::eqwin::eq_window(self, ctx, pal);
        }
    }

    /// The name and confirmation dialogs.
    fn dialogs(&mut self, ctx: &egui::Context, pal: &Palette) {
        if let Some(mut d) = self.name_dialog.take() {
            let mut open = true;
            let mut done: Option<bool> = None;
            egui::Window::new(d.title.clone())
                .id(egui::Id::new("player-name-dialog"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .pivot(Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(if d.prompt.is_empty() { "Name" } else { d.prompt.as_str() });
                    let r = ui.add(egui::TextEdit::singleline(&mut d.text).desired_width(320.0));
                    r.request_focus();
                    if !d.choices.is_empty() {
                        egui::ScrollArea::vertical().max_height(160.0).id_salt("name-choices").show(ui, |ui| {
                            for c in d.choices.clone() {
                                if ui.selectable_label(d.text == c, &c).clicked() {
                                    d.text = c;
                                }
                            }
                        });
                    }
                    if !d.error.is_empty() {
                        ui.label(RichText::new(&d.error).color(pal.accent));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("OK").strong()).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            done = Some(true);
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            done = Some(false);
                        }
                    });
                });
            match (open, done) {
                (false, _) | (_, Some(false)) => {}
                (_, Some(true)) => {
                    if let Err(e) = self.apply_name(d.action.clone(), &d.text) {
                        d.error = e;
                        self.name_dialog = Some(d);
                    }
                }
                _ => self.name_dialog = Some(d),
            }
        }
        if let Some(action) = self.confirm.clone() {
            let (title, text, button) = match &action {
                ConfirmAction::DeletePlaylist(id) => (
                    "Delete playlist",
                    format!("Delete the playlist “{}”? Its songs stay in your library.", self.state.playlist(id).map(|p| p.name.as_str()).unwrap_or("")),
                    "Delete",
                ),
                ConfirmAction::DeleteEqPreset(n) => ("Delete preset", format!("Delete your preset “{n}”? The equalizer keeps its current settings."), "Delete"),
                ConfirmAction::DeleteTheme(n) => ("Delete theme", format!("Delete the theme “{n}”?"), "Delete"),
                ConfirmAction::RetagGroup { title, text, .. } => (title.as_str(), text.clone(), "Change"),
                ConfirmAction::ClearHistory => (
                    "Clear the play history",
                    "Forget every song played so far? Recently Played and Your Stats start again from nothing.".to_string(),
                    "Clear",
                ),
            };
            let mut open = true;
            let mut done: Option<bool> = None;
            egui::Window::new(title)
                .id(egui::Id::new("player-confirm"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .pivot(Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                    ui.set_max_width(380.0);
                    ui.label(text);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(button).strong().color(pal.accent)).clicked() {
                            done = Some(true);
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            done = Some(false);
                        }
                    });
                });
            if !open || done.is_some() {
                self.confirm = None;
            }
            if done == Some(true) {
                self.confirmed(action);
            }
        }
    }

    pub fn on_exit(&mut self) {
        self.flush_listening();
        self.save_state();
        self.lib_stop.store(true, Ordering::Relaxed);
        self.engine.stop();
    }
}

pub fn fmt_time(sec: f64) -> String {
    crate::style::fmt_duration(Some(sec.floor()))
}

fn sidebar_item(ui: &mut Ui, pal: &Palette, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover.gamma_multiply(0.6));
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(18.0, 18.0)), icon, pal.accent);
    ui.painter().text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, label, look::text(13.0), pal.text);
    resp
}

/// The name of the group of songs without a genre, composer or year.
pub fn unknown_name(kind: &str) -> &'static str {
    match kind {
        "genres" => "Unknown Genre",
        "composers" => "Unknown Composer",
        "years" => "Unknown Year",
        _ => "",
    }
}

/// A song's genre, composer or year, as a rename compares it with the new name.
pub fn group_value(kind: &str, t: &Track) -> String {
    match kind {
        "years" => t.year.clone(),
        "composers" => realhires::tagedit::clean_value(&t.composer),
        _ => realhires::tagedit::clean_value(&t.genre),
    }
}

/// Songs sorted by a column (None: as they are).
pub fn sort_tracks(mut v: Vec<&Track>, sort: Option<(SongCol, bool)>) -> Vec<&Track> {
    let Some((col, rev)) = sort else { return v };
    v.sort_by_cached_key(|t| sort_key(t, col));
    if rev {
        v.reverse();
    }
    v
}

/// A cover, or a coloured square with a note when the album has none.
/// The comment tag of a song ("" when it has none, or the file cannot be read); ffprobe,
/// so call it off the window's thread.
pub fn read_comment(path: &str) -> String {
    let out = realhires::probe::command(realhires::probe::tool("ffprobe"))
        .args(["-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "json", path])
        .stdin(std::process::Stdio::null())
        .output();
    let Ok(out) = out else { return String::new() };
    let Ok(data) = realhires::json::parse(&String::from_utf8_lossy(&out.stdout)) else { return String::new() };
    let mut sources: Vec<&realhires::json::Value> = data.get("format").into_iter().collect();
    if let Some(realhires::json::Value::Array(streams)) = data.get("streams") {
        sources.extend(streams.iter());
    }
    for src in sources {
        if let Some(realhires::json::Value::Object(tags)) = src.get("tags") {
            for (k, v) in tags {
                if casefold(k.trim()) == "comment" {
                    if let Some(v) = v.as_str().map(str::trim).filter(|v| !v.is_empty()) {
                        return v.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

pub fn cover(p: &egui::Painter, rect: Rect, tex: Option<&egui::TextureHandle>, title: &str, radius: u8) {
    match tex {
        Some(t) => {
            let shape = egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), Color32::WHITE)
                .with_texture(t.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
            p.add(shape);
        }
        None => {
            let base = crate::style::placeholder_color(title);
            p.rect_filled(rect, CornerRadius::same(radius), base);
            icons::paint(p, Rect::from_center_size(rect.center(), rect.size() * 0.45), Icon::Note, Color32::from_white_alpha(170));
        }
    }
}

#[cfg(test)]
mod tests {
    //! Drives the whole player headlessly with the null audio output: the
    //! library index, playback, a gapless song change with the queue, lyrics,
    //! likes, shuffle and repeat, the EQ and info windows, search, Settings,
    //! the Quality check page, and the state saved for the next start.
    use super::*;
    use std::process::Command;

    fn ffmpeg(args: &[&str]) {
        let st = Command::new(realhires::probe::tool("ffmpeg")).args(["-v", "error", "-y"]).args(args).status().unwrap();
        assert!(st.success(), "{args:?}");
    }

    fn song(dir: &std::path::Path, album: &str, title: &str, artist: &str, genre: &str, n: u32, rate: u32, secs: f64) -> String {
        let d = dir.join(album);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(format!("{n:02} {title}.flac"));
        let src = format!("sine=frequency={}:sample_rate={rate}:duration={secs}", 300 + 40 * n);
        let meta = [format!("title={title}"), format!("artist={artist}"), format!("album={album}"), format!("track={n}"), format!("genre={genre}")];
        let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src];
        for m in &meta {
            args.extend(["-metadata", m.as_str()]);
        }
        let out = f.to_string_lossy().into_owned();
        args.push(&out);
        ffmpeg(&args);
        realhires::paths::abspath(&out)
    }

    fn frame(ctx: &egui::Context, p: &mut Player) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 840.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| p.ui(ui)).drop_without_applying_deltas();
    }

    /// A frame with these input events; a key's modifiers are held during the frame
    /// and let go in the next one.
    fn frame_with(ctx: &egui::Context, p: &mut Player, events: Vec<egui::Event>) {
        let held = events.iter().find_map(|e| if let egui::Event::Key { modifiers, .. } = e { Some(*modifiers) } else { None });
        let run = |p: &mut Player, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 840.0))),
                events,
                ..Default::default()
            };
            ctx.run_ui(input, |ui| p.ui(ui)).drop_without_applying_deltas();
        };
        match held {
            Some(m) => {
                run(p, std::iter::once(egui::Event::ModifiersChanged(m)).chain(events).collect());
                run(p, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
            }
            None => run(p, events),
        }
    }

    fn run_until(ctx: &egui::Context, p: &mut Player, what: &str, done: impl Fn(&Player) -> bool) {
        let t0 = Instant::now();
        while !done(p) {
            assert!(t0.elapsed() < Duration::from_secs(90), "timed out waiting for {what}");
            frame(ctx, p);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn library_playback_lyrics_likes_modes_windows_and_state() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var(realhires::covers::COVERS_ENV, tmp.join("covers"));
        let a1 = song(&music, "Paper Lanterns", "Lanterns Over Water", "Mira Calloway", "Pop", 1, 44_100, 2.0);
        let a2 = song(&music, "Paper Lanterns", "Paper in the Rain", "Mira Calloway", "Pop", 2, 44_100, 1.5);
        let b1 = song(&music, "Harbour Lights", "Tidewater", "The Northern Quay", "Jazz", 1, 48_000, 1.5);
        std::fs::write(
            realhires::lyrics::sidecar(&a1, ".lrc"),
            "[00:00.20]Lanterns over water\n[00:01.00]Paper in the rain\n[00:01.60]Folded stars\n",
        )
        .unwrap();
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        realhires::settings::save(&cfg, None).unwrap();

        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg.clone(), vec![]);
        assert_eq!(p.page, Page::Library);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 3 && p.lib_rx.is_none());
        assert_eq!(p.lib.albums.len(), 2);
        assert_eq!((p.lib.artists.len(), p.lib.genres.len()), (2, 2));
        // the Library page and every one of its entries
        let music_dir = realhires::paths::abspath(&music.to_string_lossy());
        for page in [
            Page::Library,
            Page::AllSongs,
            Page::Folders(None),
            Page::Folders(Some(music_dir.clone())),
            Page::Folders(Some(format!("{music_dir}{}Paper Lanterns", std::path::MAIN_SEPARATOR))),
            Page::Albums,
            Page::Artists,
            Page::AlbumArtists,
            Page::Genres,
            Page::Years,
            Page::Composers,
            Page::Playlists,
            Page::Liked,
            Page::Disliked,
            Page::RecentlyPlayed,
            Page::Recent,
            Page::Stats,
            Page::StatsDetails,
        ] {
            p.go(page);
            frame(&ctx, &mut p);
        }
        assert_eq!(p.lib.album_artists.len(), 2);
        let (subs, songs) = musiclib::folder_view(&p.lib.tracks, &music_dir);
        assert_eq!((subs.len(), songs.len()), (2, 0));

        // Back and Forward between pages (2.1): the mouse's side buttons and Alt+Left/Right
        // (Cmd+[ / Cmd+] on macOS); a new page clears the Forward list
        p.go(Page::Albums);
        p.go(Page::Genres);
        // a click of a side button: pressed and let go
        let side = |button| {
            [true, false]
                .map(|pressed| egui::Event::PointerButton { pos: pos2(700.0, 500.0), button, pressed, modifiers: egui::Modifiers::NONE })
                .to_vec()
        };
        frame_with(&ctx, &mut p, side(egui::PointerButton::Extra1));
        assert_eq!((&p.page, &p.forward), (&Page::Albums, &vec![Page::Genres]));
        frame_with(&ctx, &mut p, side(egui::PointerButton::Extra2));
        assert_eq!(p.page, Page::Genres);
        assert!(p.forward.is_empty() && p.back.last() == Some(&Page::Albums));
        let key = |key, mac_key| {
            let (key, modifiers) =
                if cfg!(target_os = "macos") { (mac_key, egui::Modifiers::MAC_CMD) } else { (key, egui::Modifiers::ALT) };
            egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
        };
        frame_with(&ctx, &mut p, vec![key(egui::Key::ArrowLeft, egui::Key::OpenBracket)]);
        assert_eq!((&p.page, &p.forward), (&Page::Albums, &vec![Page::Genres]));
        p.go(Page::Years);
        assert!(p.forward.is_empty(), "a new page: nothing to go forward to");
        p.go_back();
        frame_with(&ctx, &mut p, vec![key(egui::Key::ArrowRight, egui::Key::CloseBracket)]);
        assert_eq!(p.page, Page::Years);

        // play the album from its first song; the next song follows gaplessly
        let key = p.lib.track(&a1).unwrap().song().album_key();
        p.go(Page::Album(key.clone()));
        frame(&ctx, &mut p);
        let album: Vec<String> = p.page_songs();
        assert_eq!(album, vec![a1.clone(), a2.clone()]);
        p.play_list(album.clone(), Some(&a1), "Paper Lanterns");
        assert_eq!(p.now.as_ref().unwrap().path, a1);
        run_until(&ctx, &mut p, "playback", |p| p.engine.position().unwrap_or(0.0) > 0.3);
        assert!(p.is_playing());
        assert_eq!(p.engine.info().unwrap().device, "Null output (no sound)");

        // Now Playing reads the synced lyrics
        p.go(Page::NowPlaying);
        frame(&ctx, &mut p);
        match p.lyrics_of_now() {
            Some(ShownLyrics::Synced(lines)) => assert_eq!(lines.len(), 3),
            other => panic!("no synced lyrics: {other:?}"),
        }
        // 2.1: on Now Playing the Lyrics button hides the lyrics and the page stays; pressed
        // again it shows them; the LCD's cover leaves the page and opens it again
        p.lyrics_button();
        frame(&ctx, &mut p);
        assert!(p.page == Page::NowPlaying && p.lyrics_hidden);
        p.lyrics_button();
        frame(&ctx, &mut p);
        assert!(p.page == Page::NowPlaying && !p.lyrics_hidden);
        p.cover_clicked();
        assert!(p.page != Page::NowPlaying);
        p.cover_clicked();
        frame(&ctx, &mut p);
        assert_eq!(p.page, Page::NowPlaying);
        // 2.1: Play Next from a song's menu comes first; the gapless look-ahead leaves the queue alone
        p.queue_songs(&[b1.clone()], true);
        assert_eq!(p.queue.as_ref().unwrap().up_next(2), vec![b1.clone(), a2.clone()]);
        assert_eq!(p.queue.as_ref().unwrap().queued, vec![b1.clone()]);
        p.queue.as_mut().unwrap().queued.clear();
        p.update_next();

        // likes are saved at once
        p.toggle_like(&a1);
        assert!(playerstate::load(None).is_liked(&a1));
        p.go(Page::Liked);
        frame(&ctx, &mut p);
        assert_eq!(p.page_songs(), vec![a1.clone()]);

        // the song ends and the next one of the album plays, without a gap
        run_until(&ctx, &mut p, "the second song", |p| p.now.as_ref().is_some_and(|n| n.path == a2));
        assert_eq!(p.queue.as_ref().unwrap().current(), Some(a2.as_str()));
        // then the album ends: playback stops (repeat off)
        run_until(&ctx, &mut p, "the end of the album", |p| p.now.as_ref().is_some_and(|n| !n.active));
        frame(&ctx, &mut p);
        // both songs are in the play history, the newest first
        let recent: Vec<String> = p.recently_played().into_iter().map(|(x, _)| x).collect();
        assert_eq!(recent, vec![a2.clone(), a1.clone()]);
        let st = p.stats();
        assert_eq!((st.logged, st.songs), (2, 2), "{st:?}");
        assert!(!realhires::stats::cards(&st).is_empty());
        for page in [Page::Library, Page::RecentlyPlayed, Page::Stats, Page::StatsDetails] {
            p.go(page);
            frame(&ctx, &mut p);
        }
        // the calendar of Your Stats (2.1): the month of the plays, the day's plays newest
        // first, and a song of the list plays the day's songs from it
        p.go(Page::Stats);
        frame(&ctx, &mut p);
        let first = realhires::stats::local_parts(p.history.as_ref().unwrap().plays(0.0, None)[0].started);
        p.calendar.month = (first.year, first.month);
        frame(&ctx, &mut p);
        let today = first.day as usize - 1;
        assert!(p.calendar.minutes[today] > 0.0 && p.calendar.levels[today] >= 1, "{:?}", p.calendar.minutes);
        p.calendar_choose(first.day);
        frame(&ctx, &mut p);
        let rows: Vec<(&str, &str, &str)> =
            p.calendar.rows.iter().map(|r| (r.title.as_str(), r.artist.as_str(), r.minutes.as_str())).collect();
        assert_eq!(rows, vec![("Paper in the Rain", "Mira Calloway", "<1 min"), ("Lanterns Over Water", "Mira Calloway", "<1 min")]);
        let lp = realhires::stats::local_parts(p.calendar.rows[1].started);
        assert_eq!(p.calendar.rows[1].time, format!("{:02}:{:02}", lp.hour, lp.minute));
        p.calendar_play(1);
        assert_eq!(p.now.as_ref().unwrap().path, a1);
        assert_eq!(p.queue.as_ref().unwrap().context, vec![a2.clone(), a1.clone()]);
        p.engine.stop();
        p.calendar_choose(first.day);
        frame(&ctx, &mut p);
        assert!(p.calendar.day.is_none() && p.calendar.rows.is_empty(), "the chosen day again hides the list");

        // playlists: make one, add, play from it, rename, delete
        p.apply_name(NameAction::NewPlaylist(vec![b1.clone()]), "Road Trip").unwrap();
        let pid = p.state.playlists[0].id.clone();
        p.add_to_playlist(&pid, &[a2.clone(), "/gone/song.flac".to_string()]);
        assert_eq!(p.playlist_songs(&pid), vec![b1.clone(), a2.clone(), "/gone/song.flac".to_string()]);
        p.go(Page::Playlist(pid.clone()));
        frame(&ctx, &mut p);
        p.play_list(p.playlist_songs(&pid), None, "Road Trip");
        assert_eq!(p.queue.as_ref().unwrap().context, vec![b1.clone(), a2.clone()], "missing files are skipped");
        p.engine.stop();
        p.apply_name(NameAction::RenamePlaylist(pid.clone()), "Night Drive").unwrap();
        assert_eq!(playerstate::load(None).playlists[0].name, "Night Drive");
        assert!(p.apply_name(NameAction::NewPlaylist(vec![]), "   ").is_err());
        p.confirmed(ConfirmAction::DeletePlaylist(pid));
        assert!(playerstate::load(None).playlists.is_empty());

        // a disliked song is left out of a shuffle
        p.toggle_dislike(&a2);
        assert!(playerstate::load(None).is_disliked(&a2));
        p.set_shuffle("library");
        p.play_list(vec![a1.clone(), a2.clone(), b1.clone()], Some(&a1), "Shuffle");
        assert!(!p.queue.as_ref().unwrap().order.contains(&a2), "{:?}", p.queue.as_ref().unwrap().order);
        p.set_shuffle("off");
        p.toggle_dislike(&a2);
        p.engine.stop();

        // EQ presets: save, rename, import an AutoEQ file, export both ways
        p.state.eq.apply_preset("Jazz");
        p.apply_name(NameAction::SaveEqPreset, "My Jazz").unwrap();
        assert!(p.apply_name(NameAction::SaveEqPreset, "rock").is_err());
        p.apply_name(NameAction::RenameEqPreset("My Jazz".into()), "Late Jazz").unwrap();
        assert_eq!(p.state.matching_eq_preset(), "Late Jazz");
        let auto = "Preamp: -6.2 dB\nFilter 1: ON PK Fc 105 Hz Gain 3.5 dB Q 0.70\nFilter 2: ON LSC Fc 105 Hz Gain 4.1 dB Q 0.71\n";
        assert_eq!(p.import_eq_text(auto, "HD 600").unwrap(), "HD 600");
        assert_eq!(p.import_eq_text(auto, "HD 600").unwrap(), "HD 600 2");
        assert!(p.state.eq.parametric_enabled && p.state.eq.bands.len() == 2);
        assert!(realhires::eq::export_apo("HD 600", &p.state.eq).contains("Filter 2: ON LSC Fc 105.0 Hz Gain 4.1 dB Q 0.71"));
        assert!(realhires::eq::import_eq(&realhires::eq::export_json("x", &p.state.eq), "y").is_ok());
        assert_eq!(playerstate::load(None).eq_presets.len(), 3);
        p.eq_open = true;
        frame(&ctx, &mut p);
        p.eq_open = false;

        // a theme of one's own: made from Midnight, a colour changed, in use
        p.set_theme("Midnight");
        frame(&ctx, &mut p);
        assert!(ctx.global_style().visuals.dark_mode);
        p.apply_name(NameAction::NewTheme("Midnight".into()), "My Night").unwrap();
        assert_eq!(p.scanner.cfg.theme, "My Night");
        let mine = p.custom_themes.iter().position(|t| t.name == "My Night").unwrap();
        p.custom_themes[mine].colors[5] = "#00ff00".into();
        p.themes_changed();
        frame(&ctx, &mut p);
        assert_eq!(crate::look::current(&ctx).accent, crate::look::parse_hex("#00ff00"));
        assert_eq!(realhires::themes::load_custom(None)[mine].colors[5], "#00ff00");
        p.go(Page::Settings);
        p.settings_group = realhires::settings::GROUPS.iter().position(|g| *g == "Appearance").unwrap();
        frame(&ctx, &mut p);
        p.confirmed(ConfirmAction::DeleteTheme("My Night".into()));
        assert_eq!(p.scanner.cfg.theme, realhires::themes::SYSTEM);
        p.set_theme("Light");
        frame(&ctx, &mut p);

        // shuffle and repeat modes go to the queue and the saved state
        p.play_list(album.clone(), Some(&a1), "Paper Lanterns");
        let first = p.now.as_ref().unwrap().token;
        p.set_shuffle("library");
        p.set_repeat("song");
        assert_eq!((p.queue.as_ref().unwrap().shuffle.as_str(), p.queue.as_ref().unwrap().repeat.as_str()), ("library", "song"));
        // the same song again, gaplessly, as a new play of it
        run_until(&ctx, &mut p, "repeat song", |p| p.now.as_ref().is_some_and(|n| n.token != first));
        assert_eq!(p.now.as_ref().unwrap().path, a1);
        p.set_repeat("off");
        p.set_shuffle("off");
        // paused from here on: the song lasts 2 s, and the quality scan below took longer on
        // the Windows runner, so the next song had started before quitting
        p.toggle_play();
        assert!(!p.is_playing());

        // the windows
        p.open_info();
        p.eq_open = true;
        p.state.eq = realhires::eq::EqState::default();
        p.state.eq.enabled = true;
        p.state.eq.apply_preset("Rock");
        p.set_eq();
        frame(&ctx, &mut p);
        let t = p.lib.track(&a1).unwrap().clone();
        let secs = crate::eqwin::info_sections(&t, 2, p.engine.info().as_ref(), &p.state, true, false, None);
        let flat: Vec<String> = secs.iter().flat_map(|(_, r)| r.iter().map(|(k, v)| format!("{k}={v}"))).collect();
        assert!(flat.contains(&"Sample rate=44.1 kHz".to_string()), "{flat:?}");
        assert!(flat.contains(&"Track=1 of 2".to_string()), "{flat:?}");
        assert!(flat.iter().any(|x| x.starts_with("Equalizer=Graphic: Rock")), "{flat:?}");
        assert!(flat.contains(&"Verdict=Not checked yet".to_string()), "{flat:?}");

        // search
        p.search = "tidewater".into();
        p.search_due = Some(Instant::now());
        frame(&ctx, &mut p);
        assert_eq!(p.page, Page::Search);
        p.search.clear();
        p.search_due = Some(Instant::now());
        frame(&ctx, &mut p);
        assert_ne!(p.page, Page::Search);

        // Settings: the menus that depend on the computer are filled
        p.go(Page::Settings);
        frame(&ctx, &mut p);
        assert!(p.scanner.choices.contains_key("font_family") && p.scanner.choices.contains_key("output_device"));
        p.settings_draft().gapless = false;
        p.settings_due = Some(Instant::now());
        frame(&ctx, &mut p);
        assert!(!p.scanner.cfg.gapless && !realhires::settings::load(None).gapless);

        // Quality check: the 1.x scanner on the library folders
        p.check_library();
        assert_eq!(p.page, Page::Quality);
        run_until(&ctx, &mut p, "the quality scan", |p| !p.scanner.scanning && p.scanner.results.len() == 3);
        p.open_info();
        assert!(p.info.as_ref().unwrap().verdict.is_some());

        // quitting remembers the song; the next start shows it paused there
        p.on_exit();
        drop(p);
        let st = playerstate::load(None);
        assert_eq!(st.last_path, a1);
        assert_eq!((st.eq.preset.as_str(), st.eq.enabled), ("Rock", true));
        let mut again = Player::new(&ctx, realhires::settings::load(None), vec![]);
        frame(&ctx, &mut again);
        assert_eq!(again.now.as_ref().map(|n| n.path.clone()), Some(a1.clone()));
        assert!(!again.is_playing() && again.engine.is_paused());
        assert!(again.state.is_liked(&a1));
        drop(again);
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = b1;
    }

    #[test]
    fn edit_info_and_theme_files() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var(realhires::covers::COVERS_ENV, tmp.join("covers"));
        let a1 = song(&music, "Paper Lanterns", "Lanterns Over Water", "Mira Calloway", "Pop", 1, 44_100, 1.0);
        let a2 = song(&music, "Paper Lanterns", "Paper in the Rain", "Mira Calloway", "Pop", 2, 44_100, 1.0);
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 2 && p.lib_rx.is_none());

        // a theme file put into the themes folder appears in the menu, as it is
        let folder = realhires::themes::folder(None);
        assert_eq!(folder, tmp.join("themes"));
        std::fs::create_dir_all(&folder).unwrap();
        let neon = realhires::themes::normalize(&realhires::json::parse(
            r##"{"name": "Neon", "dark": true, "colors": {"accent": "#39ff14"}}"##).unwrap()).unwrap();
        std::fs::write(folder.join("neon.json"), realhires::themes::export_theme(&neon)).unwrap();
        std::fs::write(folder.join("broken.json"), "{").unwrap();
        assert!(p.reload_file_themes());
        assert!(p.scanner.choices["theme"].contains(&"Neon".to_string()));
        p.set_theme("Neon");
        p.go(Page::Settings);
        frame(&ctx, &mut p);
        assert_eq!(p.theme_file("Neon"), Some(folder.join("neon.json").as_path()));
        assert_eq!(look::current(&ctx).accent, look::parse_hex("#39ff14"));
        assert!(realhires::themes::load_custom(None).is_empty(), "a folder theme is not copied into themes.json");
        // a dropped (or imported) theme file becomes one of the user's own themes
        let sunset = tmp.join("sunset.json");
        let mut t = neon.clone();
        t.name = "Sunset".into();
        std::fs::write(&sunset, realhires::themes::export_theme(&t)).unwrap();
        assert!(crate::app::is_theme_file(&sunset) && !crate::app::is_theme_file(&tmp.join("missing.json")));
        p.import_theme_path(&sunset);
        assert_eq!(p.scanner.cfg.theme, "Sunset");
        assert_eq!(realhires::themes::load_custom(None)[0].name, "Sunset");
        // a second file called Neon gets a free name
        std::fs::write(folder.join("other.json"), realhires::themes::export_theme(&neon)).unwrap();
        p.reload_file_themes();
        assert!(p.scanner.choices["theme"].contains(&"Neon 2".to_string()));

        // Properties (2.0 "Edit Info") on one song: the composer and the release date
        p.go(Page::AllSongs);
        p.edit_song_info(&a1);
        run_until(&ctx, &mut p, "the tags", |p| !p.edit_info.as_ref().unwrap().reading());
        assert_eq!(p.edit_info.as_ref().unwrap().text[0], "Lanterns Over Water");
        // the window can be made bigger by its corner (2.1), and keeps that size for the next one
        frame(&ctx, &mut p);
        let window = || ctx.memory(|m| m.area_rect(egui::Id::new(crate::editinfo::WINDOW_ID))).unwrap();
        let before = window();
        assert!(before.width() >= 420.0, "{before:?}");
        let corner = before.max - vec2(3.0, 3.0);
        let wider = corner + vec2(200.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame_with(&ctx, &mut p, vec![egui::Event::PointerMoved(corner)]);
        frame_with(&ctx, &mut p, vec![button(corner, true)]);
        for i in 1..=4 {
            frame_with(&ctx, &mut p, vec![egui::Event::PointerMoved(corner + vec2(50.0 * i as f32, 0.0))]);
        }
        frame_with(&ctx, &mut p, vec![button(wider, false)]);
        frame(&ctx, &mut p);
        let after = window();
        assert!(after.width() >= before.width() + 150.0, "{before:?} → {after:?}");
        {
            let cfg = p.scanner.cfg.clone();
            let w = p.edit_info.as_mut().unwrap();
            w.set("composer", "Lena Frost");
            w.set("date", "2004-05-28");
            assert!(w.save(&ctx, &cfg));
        }
        run_until(&ctx, &mut p, "saving", |p| p.edit_info.is_none());
        run_until(&ctx, &mut p, "the library", |p| {
            p.lib_rx.is_none() && p.lib.track(&a1).is_some_and(|t| t.composer == "Lena Frost")
        });
        let tags = realhires::probe::probe(&a1).unwrap().tags;
        assert_eq!(realhires::probe::tag(&tags, "date"), Some("2004-05-28"));
        assert_eq!(realhires::probe::tag(&tags, "title"), Some("Lanterns Over Water"), "an unchanged field stays");

        // a bad track number is refused and nothing is written
        p.edit_song_info(&a2);
        run_until(&ctx, &mut p, "the tags", |p| !p.edit_info.as_ref().unwrap().reading());
        frame(&ctx, &mut p);
        assert_eq!(window().width(), after.width(), "the size is kept for the session");
        {
            let cfg = p.scanner.cfg.clone();
            let w = p.edit_info.as_mut().unwrap();
            w.set("track", "two");
            assert!(w.save(&ctx, &cfg));
            assert!(w.error.contains("must be a number"), "{}", w.error);
        }
        frame(&ctx, &mut p);
        assert!(p.edit_info.is_some());
        assert_eq!(realhires::probe::tag(&realhires::probe::probe(&a2).unwrap().tags, "track"), Some("2"));
        p.edit_info = None;

        // Edit album info: the album artist and the genre of every song of the album
        let key = p.lib.track(&a1).unwrap().song().album_key();
        p.go(Page::Album(key));
        frame(&ctx, &mut p);
        let album = p.lib.album_of(&a1).unwrap().clone();
        p.edit_album_info(&album);
        assert_eq!(p.edit_info.as_ref().unwrap().subheading, "Mira Calloway · 2 songs");
        run_until(&ctx, &mut p, "the tags", |p| !p.edit_info.as_ref().unwrap().reading());
        {
            let w = p.edit_info.as_ref().unwrap();
            assert_eq!(w.text[2], "Paper Lanterns");
            assert_eq!(w.text[0], "", "the titles differ, so the field starts empty");
            assert!(w.changes().is_empty());
        }
        frame(&ctx, &mut p);
        {
            let cfg = p.scanner.cfg.clone();
            let w = p.edit_info.as_mut().unwrap();
            w.set("genre", " Jazz ");
            w.set("album_artist", "Various Lanterns");
            assert_eq!(
                w.changes(),
                vec![("album_artist".to_string(), "Various Lanterns".to_string()), ("genre".to_string(), " Jazz ".to_string())]
            );
            assert!(w.save(&ctx, &cfg));
        }
        run_until(&ctx, &mut p, "saving the album", |p| p.edit_info.is_none());
        run_until(&ctx, &mut p, "the library", |p| {
            p.lib_rx.is_none()
                && [&a1, &a2].iter().all(|a| p.lib.track(a).is_some_and(|t| t.genre == "Jazz" && t.album_artist == "Various Lanterns"))
        });
        assert_eq!(p.lib.album_of(&a1).unwrap().tracks.len(), 2, "the album stays one album");
        assert_eq!(p.lib.track(&a2).unwrap().title, "Paper in the Rain");
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bit_perfect_playback_controls() {
        // Settings, Playback, Bit-perfect playback (2.1): the song restarts where it was in its own
        // format, the gain is exactly 1 (the saved volume is kept), the equalizer is bypassed, and
        // the Playback info has the Bit-perfect line and the device's real format
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-bitperfect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        let a = song(&music, "Night Train", "Coastline", "Ada Lux", "Pop", 1, 96_000, 3.0);
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg.clone(), vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 1 && p.lib_rx.is_none());
        p.set_volume(0.5);
        p.state.eq.enabled = true;
        p.state.eq.apply_preset("Rock");
        p.set_eq();
        p.play_list(vec![a.clone()], Some(&a), "Night Train");
        run_until(&ctx, &mut p, "playback", |p| p.engine.position().unwrap_or(0.0) > 0.5);
        assert_eq!(p.engine.gain(), 0.25);
        assert_eq!(p.engine.info().unwrap().bit_perfect, "Off");
        let before = p.engine.position().unwrap();
        p.settings_draft().bit_perfect = true;
        p.settings_due = Some(Instant::now());
        frame(&ctx, &mut p);
        assert!(p.scanner.cfg.bit_perfect && realhires::settings::load(None).bit_perfect);
        assert!(p.engine.bit_perfect() && p.engine.gain() == 1.0 && p.state.volume == 0.5);
        run_until(&ctx, &mut p, "the restart", |p| p.engine.position().unwrap_or(0.0) > before + 0.1);
        assert_eq!(p.now.as_ref().unwrap().path, a, "the same song goes on");
        let info = p.engine.info().unwrap();
        assert_eq!((info.rate, info.sample_format), (96_000, realhires::bitperfect::Format::I16));
        let t = p.lib.track(&a).unwrap().clone();
        let secs = crate::eqwin::info_sections(&t, 1, Some(&info), &p.state, true, true, None);
        let flat: Vec<String> = secs.iter().flat_map(|(_, r)| r.iter().map(|(k, v)| format!("{k}={v}"))).collect();
        for want in [
            "Output=96 kHz, 16-bit integer, stereo",
            "Bit-perfect=Yes: 16-bit, 96 kHz, Null output (no sound)",
            "Volume=0.0 dB",
            "Equalizer=Bypassed while bit-perfect playback is on.",
        ] {
            assert!(flat.contains(&want.to_string()), "{want}: {flat:?}");
        }
        p.eq_open = true;
        frame(&ctx, &mut p);
        p.set_volume(0.1); // the slider is disabled, and nothing else changes the volume either
        assert_eq!(p.state.volume, 0.5);
        p.settings_draft().bit_perfect = false;
        p.settings_due = Some(Instant::now());
        frame(&ctx, &mut p);
        assert!(!p.engine.bit_perfect() && p.engine.gain() == 0.25);
        run_until(&ctx, &mut p, "the normal path", |p| {
            p.engine.info().is_some_and(|i| i.sample_format == realhires::bitperfect::Format::F32)
        });
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Album covers are made ahead once the library is read, stay when the
    /// library is read again, and only a changed album's cover is made again.
    #[test]
    fn covers_are_made_ahead_and_kept() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-covers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var(realhires::covers::COVERS_ENV, tmp.join("covers"));
        let a1 = song(&music, "Paper Lanterns", "Lanterns Over Water", "Mira Calloway", "Pop", 1, 44_100, 0.5);
        song(&music, "Harbour Lights", "Tidewater", "The Northern Quay", "Jazz", 1, 48_000, 0.5);
        for (album, name, colour) in [("Paper Lanterns", "cover.png", "red"), ("Harbour Lights", "folder.png", "blue")] {
            let img = music.join(album).join(name).to_string_lossy().into_owned();
            ffmpeg(&["-f", "lavfi", "-i", &format!("color=c={colour}:s=96x96"), "-frames:v", "1", &img]);
        }
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 2 && p.lib_rx.is_none());
        let paths = |a: &Album| -> Vec<String> { a.tracks.iter().map(|t| t.path.clone()).collect() };
        assert_eq!(p.covers.grid_px(), crate::cover_cache::GRID_PX);
        run_until(&ctx, &mut p, "the covers made ahead", |p| {
            p.covers.prefetching() == 0
                && p.lib.albums.iter().all(|a| realhires::covers::is_cached(&paths(a), crate::cover_cache::GRID_PX))
        });
        p.go(Page::Albums);
        let shown = |p: &Player| -> Vec<(String, egui::TextureId)> {
            let mut v: Vec<(String, egui::TextureId)> = p
                .lib
                .albums
                .iter()
                .filter_map(|a| {
                    let t = p.covers.tex.get(&a.key)?.iter().find(|(s, _)| *s == crate::cover_cache::GRID_PX)?;
                    Some((a.key.clone(), t.1.as_ref()?.id()))
                })
                .collect();
            v.sort();
            v
        };
        run_until(&ctx, &mut p, "the covers on screen", |p| shown(p).len() == 2);
        let before = shown(&p);

        // the library is read again: the textures stay
        p.update_library();
        run_until(&ctx, &mut p, "the second update", |p| p.lib_rx.is_none());
        frame(&ctx, &mut p);
        assert_eq!(shown(&p), before);

        // a song changes: only its album's cover is made again
        let f = std::fs::File::options().write(true).open(&a1).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(86_400)).unwrap();
        drop(f);
        let key_a = p.lib.track(&a1).unwrap().song().album_key();
        p.update_library();
        run_until(&ctx, &mut p, "the third update", |p| p.lib_rx.is_none());
        let now = shown(&p);
        assert_eq!(now.len(), 1, "{now:?}");
        assert!(now[0].0 != key_a && before.contains(&now[0]));
        run_until(&ctx, &mut p, "the new cover", |p| shown(p).len() == 2);
        let after = shown(&p);
        assert!(after.iter().any(|(k, id)| *k == key_a && !before.contains(&(k.clone(), *id))));
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A click as a person makes it: the pointer moves there, presses, releases.
    fn click(ctx: &egui::Context, p: &mut Player, at: egui::Pos2) {
        frame_with(ctx, p, vec![egui::Event::PointerMoved(at)]);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame_with(ctx, p, vec![button(true)]);
        frame_with(ctx, p, vec![button(false)]);
        frame_with(ctx, p, vec![egui::Event::PointerMoved(pos2(5.0, 835.0))]);
    }

    /// The mouse wheel over `at` (a negative `dy` moves the content up: scrolls down).
    fn wheel(ctx: &egui::Context, p: &mut Player, at: egui::Pos2, dy: f32) {
        frame_with(ctx, p, vec![egui::Event::PointerMoved(at)]);
        frame_with(
            ctx,
            p,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Default::default(),
            }],
        );
        for _ in 0..30 {
            frame_with(ctx, p, vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn link_at(links: &[(Rect, crate::pages::Link)], want: &crate::pages::Link) -> egui::Pos2 {
        links.iter().find(|(_, l)| l == want).unwrap_or_else(|| panic!("no link {want:?} in {links:?}")).0.center()
    }

    /// One song on a fake LRCLIB: /api/get answers "Midnight Sun" with synced lyrics.
    fn fake_lrclib() -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = vec![];
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
                let (status, body) = if line.starts_with("GET /api/get?") && line.contains("track_name=Midnight") {
                    (
                        "200 OK",
                        r#"{"id": 7, "trackName": "Midnight Sun", "artistName": "Aurora Fields", "albumName": "Northern Lights", "duration": 20.0, "instrumental": false, "plainLyrics": "Midnight sun\nOver the bay", "syncedLyrics": "[00:00.50] Midnight sun\n[00:03.00] Over the bay"}"#,
                    )
                } else {
                    ("404 Not Found", r#"{"message": "Failed to find specified track"}"#)
                };
                let _ = write!(
                    s,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        url
    }

    /// Now Playing (2.1): the label above the lyrics, A− / A+ saved in player.json, "Back to
    /// the current line" while the current line is scrolled away, the switch Lyrics | Up Next
    /// | Details, one link per artist (also in the LCD and Details), the cover's hook, and Get
    /// lyrics, which downloads at once (the user's "Get lyrics button doesn't work": it opened
    /// the lyrics window, which waited for its own Download lyrics button).
    #[test]
    fn now_playing_switch_links_lyrics_size_and_get_lyrics() {
        use crate::pages::Link;
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-np-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var(realhires::lyrics::INTERVAL_ENV, "0");
        let glacier = song(&music, "Northern Lights", "Glacier Song", "Aurora Fields", "Ambient", 1, 44_100, 20.0);
        let sun = song(&music, "Northern Lights", "Midnight Sun", "Aurora Fields", "Ambient", 2, 44_100, 20.0);
        let kettle = song(&music, "Small Hours", "Blue Kettle", "June Harbor", "Jazz", 1, 44_100, 1.0);
        // a duet with a comment tag
        let duet_dir = music.join("Duets");
        std::fs::create_dir_all(&duet_dir).unwrap();
        let duet = realhires::paths::abspath(&duet_dir.join("01 Together.flac").to_string_lossy());
        ffmpeg(&["-f", "lavfi", "-i", "sine=frequency=500:sample_rate=44100:duration=2", "-metadata", "title=Together",
                 "-metadata", "artist=June Harbor, Aurora Fields", "-metadata", "album=Duets", "-metadata",
                 "comment=Recorded live", &duet]);
        let lines: Vec<String> = (0..40).map(|i| format!("[00:{:02}.{:02}]Line number {i}", i / 2, (i % 2) * 50)).collect();
        std::fs::write(realhires::lyrics::sidecar(&glacier, ".lrc"), lines.join("\n") + "\n").unwrap();
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        cfg.lyrics_server = fake_lrclib();
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 4 && p.lib_rx.is_none());
        p.play_list(vec![glacier.clone(), sun.clone()], Some(&glacier), "Northern Lights");
        run_until(&ctx, &mut p, "playback", |p| p.engine.position().unwrap_or(0.0) > 0.3);
        p.toggle_play(); // paused, so the current line stays
        p.lyrics_button();
        for _ in 0..3 {
            frame(&ctx, &mut p);
        }
        assert_eq!((p.page.clone(), p.np_tab), (Page::NowPlaying, NpTab::Lyrics));
        // the label says what the lyrics are and where they come from
        assert_eq!(p.lyrics_label_of_now(), ("Synced lyrics".to_string(), ".lrc file".to_string()));
        // the cover with the title and "Artist — Album": one block in the middle of the left half
        let cover = p.np_hits.cover.expect("the cover");
        let links = p.np_hits.links.clone();
        let line = links.iter().map(|(r, _)| r.center().x).fold(0.0, f32::max);
        assert!(cover.center().x < 640.0 && cover.left() < line && line < cover.right(), "{cover:?} {links:?}");
        // A− / A+: 26 px by default, 2 px a step, saved at once
        assert_eq!(p.state.lyrics_size, 26);
        let sizes = p.np_hits.size_buttons.clone();
        click(&ctx, &mut p, sizes[1].center());
        assert_eq!((p.state.lyrics_size, playerstate::load(None).lyrics_size), (28, 28));
        click(&ctx, &mut p, sizes[0].center());
        assert_eq!((p.state.lyrics_size, playerstate::load(None).lyrics_size), (26, 26));
        // scrolled away from the current line: following stops and the Back button shows
        let lyrics_at = pos2(1000.0, 600.0);
        wheel(&ctx, &mut p, lyrics_at, -3000.0);
        assert!(p.lyrics_away, "scrolled away");
        let back = p.np_hits.back.expect("the Back to the current line button");
        for _ in 0..5 {
            frame(&ctx, &mut p);
        }
        assert!(p.lyrics_away && p.np_hits.back.is_some(), "no automatic scrolling while it shows");
        click(&ctx, &mut p, back.center());
        for _ in 0..5 {
            frame(&ctx, &mut p);
        }
        assert!(!p.lyrics_away && p.np_hits.back.is_none());
        wheel(&ctx, &mut p, lyrics_at, -3000.0);
        assert!(p.lyrics_away);
        wheel(&ctx, &mut p, lyrics_at, 3000.0); // scrolled back into view: the button goes
        assert!(!p.lyrics_away && p.np_hits.back.is_none());
        // the switch: Up Next lists the rest of the album, Details describes the song
        p.set_np_tab(NpTab::UpNext);
        frame(&ctx, &mut p);
        assert_eq!(p.np_hits.up_next, vec![sun.clone()]);
        p.set_np_tab(NpTab::Details);
        frame(&ctx, &mut p);
        for needed in ["Title: Glacier Song", "Artist: Aurora Fields", "Track: 1 of 2", "Format: FLAC · 16-bit/44.1 kHz", "Quality check: Not checked yet"] {
            assert!(p.np_hits.details.iter().any(|l| l.starts_with(needed)), "{needed}: {:?}", p.np_hits.details);
        }
        p.lyrics_button(); // the Lyrics button chooses Lyrics again
        assert!(p.np_tab == NpTab::Lyrics && !p.lyrics_hidden);
        frame(&ctx, &mut p);
        // the album name opens the album, the artist the artist, the cover the Cover window hook
        let links = p.np_hits.links.clone();
        click(&ctx, &mut p, link_at(&links, &Link::Album));
        assert!(matches!(p.page, Page::Album(_)), "{:?}", p.page);
        p.go(Page::NowPlaying);
        frame(&ctx, &mut p);
        click(&ctx, &mut p, link_at(&links, &Link::Artist("Aurora Fields".into())));
        assert_eq!(p.page, Page::Group("artists", "Aurora Fields".into()));
        p.go(Page::NowPlaying);
        frame(&ctx, &mut p);
        click(&ctx, &mut p, cover.center());
        assert_eq!(p.page, Page::NowPlaying);
        assert!(p.cover_win.is_some(), "the Now Playing cover opens the Cover window");
        p.cover_win = None;
        frame(&ctx, &mut p);
        // the LCD: the title and the album name open the album, the artist the artist
        let lcd = p.lcd_links.clone();
        assert_eq!(lcd.iter().filter(|(_, l)| *l == Link::Album).count(), 2, "{lcd:?}");
        click(&ctx, &mut p, lcd[0].0.center());
        assert!(matches!(p.page, Page::Album(_)));
        click(&ctx, &mut p, link_at(&lcd, &Link::Artist("Aurora Fields".into())));
        assert_eq!(p.page, Page::Group("artists", "Aurora Fields".into()));

        // several artists in one field: each its own link, on Now Playing, in the LCD and in Details
        p.play_list(vec![duet.clone()], Some(&duet), "Duets");
        p.toggle_play();
        p.go(Page::NowPlaying);
        for _ in 0..3 {
            frame(&ctx, &mut p);
        }
        let artists: Vec<Link> = p.np_hits.links.iter().map(|(_, l)| l.clone()).filter(|l| *l != Link::Album).collect();
        assert_eq!(artists, vec![Link::Artist("June Harbor".into()), Link::Artist("Aurora Fields".into())]);
        let links = p.np_hits.links.clone();
        click(&ctx, &mut p, link_at(&links, &Link::Artist("June Harbor".into())));
        assert_eq!(p.page, Page::Group("artists", "June Harbor".into()));
        assert!(p.lcd_links.iter().any(|(_, l)| *l == Link::Artist("Aurora Fields".into())), "{:?}", p.lcd_links);
        p.go(Page::NowPlaying);
        p.set_np_tab(NpTab::Details);
        run_until(&ctx, &mut p, "the comment tag", |p| p.np_hits.details.iter().any(|l| l == "Comment: Recorded live"));
        assert!(p.np_hits.details.iter().any(|l| l == "Artist: June Harbor, Aurora Fields"), "{:?}", p.np_hits.details);
        for _ in 0..3 {
            frame(&ctx, &mut p); // the grid settles its column widths
        }
        let names: Vec<String> = p.np_hits.detail_links.iter().map(|(_, n)| n.clone()).collect();
        assert_eq!(names, vec!["June Harbor".to_string(), "Aurora Fields".to_string()]);
        let second = p.np_hits.detail_links[1].0.center();
        click(&ctx, &mut p, second);
        assert_eq!(p.page, Page::Group("artists", "Aurora Fields".into()));

        // Get lyrics: the download starts at once, and Now Playing shows the lyrics, staying
        p.play_list(vec![sun.clone()], Some(&sun), "Northern Lights");
        p.toggle_play();
        p.lyrics_button();
        for _ in 0..3 {
            frame(&ctx, &mut p);
        }
        assert!(p.lyrics_of_now().is_none());
        let get = p.np_hits.get_lyrics.expect("the Get lyrics button");
        click(&ctx, &mut p, get.center());
        assert!(p.scanner.lyrics_win.as_ref().is_some_and(|w| w.running), "the download started at once");
        run_until(&ctx, &mut p, "the lyrics", |p| p.scanner.lyrics_win.as_ref().is_some_and(|w| !w.running));
        assert_eq!(p.scanner.lyrics_win.as_ref().unwrap().status, "1 with synced lyrics.");
        frame(&ctx, &mut p);
        assert_eq!(p.page, Page::NowPlaying);
        match p.lyrics_of_now() {
            Some(ShownLyrics::Synced(lines)) => assert_eq!(lines[0].1, "Midnight sun"),
            other => panic!("no synced lyrics: {other:?}"),
        }
        assert!(std::path::Path::new(&realhires::lyrics::sidecar(&sun, ".lrc")).is_file());
        // a song's menu has Get lyrics… too (get_lyrics): this one is not on LRCLIB
        p.get_lyrics(vec![kettle.clone()]);
        assert!(p.scanner.lyrics_win.as_ref().is_some_and(|w| w.running));
        run_until(&ctx, &mut p, "the second lookup", |p| p.scanner.lyrics_win.as_ref().is_some_and(|w| !w.running));
        assert_eq!(p.scanner.lyrics_win.as_ref().unwrap().outcomes[0].status, realhires::lyrics::Status::NotFound);
        p.scanner.lyrics_win = None;
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// A song with a composer and a release date too.
    #[allow(clippy::too_many_arguments)]
    fn tagged(dir: &std::path::Path, album: &str, title: &str, artist: &str, genre: &str, composer: &str, date: &str, n: u32) -> String {
        let d = dir.join(album);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(format!("{n:02} {title}.flac"));
        let src = format!("sine=frequency={}:sample_rate=44100:duration=1", 300 + 40 * n);
        let meta = [
            format!("title={title}"),
            format!("artist={artist}"),
            format!("album_artist={artist}"),
            format!("album={album}"),
            format!("track={n}"),
            format!("genre={genre}"),
            format!("composer={composer}"),
            format!("date={date}"),
        ];
        let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src];
        for m in &meta {
            args.extend(["-metadata", m.as_str()]);
        }
        let out = f.to_string_lossy().into_owned();
        args.push(&out);
        ffmpeg(&args);
        realhires::paths::abspath(&out)
    }

    fn tag_of(path: &str, key: &str) -> String {
        realhires::probe::probe(path).unwrap().tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    /// Run a confirmed rename until its files are written and the library was read again.
    fn rename(ctx: &egui::Context, p: &mut Player, what: &str, check: impl Fn(&Player) -> bool) {
        let action = p.confirm.take().expect("a confirmation");
        p.confirmed(action);
        run_until(ctx, p, what, |p| p.tag_job.as_ref().is_some_and(|j| j.done()) && p.lib_rx.is_none() && check(p));
        assert!(p.tag_job.as_ref().unwrap().outcomes.as_ref().unwrap().iter().all(|o| o.ok));
    }

    #[test]
    fn grids_renames_columns_quality_and_preset_delete() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-grids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        let a1 = tagged(&music, "Paper Lanterns", "Lanterns Over Water", "Mira Calloway", "Pop", "Ann", "2001-05-03", 1);
        let a2 = tagged(&music, "Paper Lanterns", "Paper in the Rain", "Mira Calloway", "Pop", "Ann", "2001", 2);
        let b1 = tagged(&music, "Harbour Lights", "Tidewater", "The Northern Quay", "Jazz", "", "1999", 1);
        let c1 = tagged(&music, "Two Rivers", "Where the Rivers Meet", "Mira Calloway, The Northern Quay", "Folk", "Ann", "2005", 1);
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg.clone(), vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 4 && p.lib_rx.is_none());

        // the grouping pages are card grids: each group with its albums (a duet on both artists)
        let names: Vec<&str> = p.lib.artists.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Mira Calloway", "The Northern Quay"]);
        assert_eq!(p.lib.group_albums["artists"][0].len(), 2, "Paper Lanterns and Two Rivers");
        assert_eq!(p.lib.group_albums["composers"].len(), 2);
        for page in [Page::Artists, Page::AlbumArtists, Page::Genres, Page::Years, Page::Composers] {
            p.go(page);
            frame(&ctx, &mut p);
        }
        // open_artist(): an artist, or the first artist of a field that is not one
        p.open_artist("Mira Calloway & Somebody New");
        assert_eq!(p.page, Page::Group("artists", "Mira Calloway".into()));
        frame(&ctx, &mut p);
        p.open_artist("The Northern Quay");
        assert_eq!(p.page, Page::Group("artists", "The Northern Quay".into()));
        // the album page: track order at first, sortable, the album columns (# first)
        let key = p.lib.track(&a1).unwrap().song().album_key();
        p.go(Page::Album(key.clone()));
        frame(&ctx, &mut p);
        assert_eq!(p.album_sort, None);
        assert_eq!(p.state.columns_of("album")[0], "num");
        p.set_sort("album", SongCol::Title);
        p.set_sort("album", SongCol::Title);
        assert_eq!(p.album_sort, Some((SongCol::Title, true)));
        frame(&ctx, &mut p);
        let duet = p.lib.track(&c1).unwrap().song().album_key();
        p.go(Page::Album(duet));
        frame(&ctx, &mut p);
        assert_eq!(p.album_sort, None, "another album starts in track order");
        // the Quality column: quality() in the cell, the resolution as its tooltip; sorted by bits, then rate
        let t = p.lib.track(&a1).unwrap();
        assert_eq!((cell_text(t, SongCol::Quality), musiclib::quality_detail(t)), ("Lossless".to_string(), "16-bit/44.1 kHz".to_string()));
        p.song_sort = (SongCol::Quality, false);
        p.go(Page::AllSongs);
        frame(&ctx, &mut p);
        // the columns of song lists: chosen per kind, kept in player.json
        p.state.toggle_column("songs", "year");
        p.save_state();
        frame(&ctx, &mut p);
        assert!(playerstate::load(None).columns_of("songs").contains(&"year".to_string()));
        assert!(!p.state.columns_of("album").contains(&"year".to_string()));
        p.state.reset_columns("songs");

        // rename a genre from its page: the files change, the page follows the new name
        p.go(Page::Group("genres", "Jazz".into()));
        frame(&ctx, &mut p);
        p.ask_rename_group("genres", "Jazz");
        assert_eq!(p.name_dialog.as_ref().unwrap().text, "Jazz");
        assert!(p.apply_name(NameAction::RenameGroup("genres", "Jazz".into()), "  ").is_err());
        p.name_dialog = None;
        p.retag_group("genres", &["Jazz".into()], "Smooth Jazz", false).unwrap();
        match &p.confirm {
            Some(ConfirmAction::RetagGroup { text, paths, .. }) => {
                assert!(text.starts_with("Rename the genre “Jazz” to “Smooth Jazz”? The genre of 1 song changes."), "{text}");
                assert_eq!(paths, &vec![b1.clone()]);
            }
            other => panic!("{other:?}"),
        }
        rename(&ctx, &mut p, "the genre renamed", |p| p.lib.track(&b1).is_some_and(|t| t.genre == "Smooth Jazz"));
        assert_eq!(tag_of(&b1, "genre"), "Smooth Jazz");
        assert_eq!(p.page, Page::Group("genres", "Smooth Jazz".into()));
        // merge two genres: only the songs that do not have the name yet are written
        p.group_selected = vec!["Pop".into(), "Folk".into()];
        p.go(Page::Genres);
        frame(&ctx, &mut p);
        p.ask_merge_groups("genres", p.group_selected.clone());
        assert_eq!(p.name_dialog.as_ref().unwrap().choices, vec!["Pop".to_string(), "Folk".to_string()], "the biggest first");
        p.name_dialog = None;
        p.retag_group("genres", &["Pop".into(), "Folk".into()], "Pop", true).unwrap();
        rename(&ctx, &mut p, "the genres merged", |p| p.lib.genres.len() == 2);
        assert_eq!(tag_of(&c1, "genre"), "Pop");
        assert!(p.group_selected.is_empty());
        assert!(p.retag_group("genres", &["Pop".into()], "Pop", true).is_err(), "nothing to change");
        // a year: only the year of each release date changes
        p.retag_group("years", &["2001".into()], "2004", false).unwrap();
        rename(&ctx, &mut p, "the year renamed", |p| p.lib.track(&a2).is_some_and(|t| t.year == "2004"));
        assert_eq!((tag_of(&a1, "date"), tag_of(&a2, "date")), ("2004-05-03".to_string(), "2004".to_string()));
        assert!(p.retag_group("years", &["2004".into()], "04", false).is_err());
        // a composer
        p.retag_group("composers", &["Ann".into()], "Ann Lind", false).unwrap();
        rename(&ctx, &mut p, "the composer renamed", |p| p.lib.track(&c1).is_some_and(|t| t.composer == "Ann Lind"));
        p.tag_job = None;

        // the Equalizer: a preset saved while the equalizer was off, chosen with it on, deleted
        p.state.eq.apply_preset("Rock");
        p.state.eq.enabled = false;
        p.apply_name(NameAction::SaveEqPreset, "Quiet Rock").unwrap();
        p.state.eq.enabled = true;
        assert_eq!(p.state.matching_eq_preset(), "Quiet Rock");
        let before = p.state.eq.clone();
        p.eq_open = true;
        frame(&ctx, &mut p);
        assert!(p.eq_delete(None), "the Delete… button");
        assert_eq!(p.confirm, Some(ConfirmAction::DeleteEqPreset("Quiet Rock".into())));
        frame(&ctx, &mut p);
        let action = p.confirm.take().unwrap();
        p.confirmed(action);
        assert!(p.state.eq_presets.is_empty() && playerstate::load(None).eq_presets.is_empty());
        assert_eq!(p.state.eq, before, "the equalizer keeps its settings");
        assert_eq!(p.state.matching_eq_preset(), "");
        // set to none of the user's presets: Delete… lists them
        p.apply_name(NameAction::SaveEqPreset, "Mine").unwrap();
        p.state.eq.apply_preset("Jazz");
        assert!(!p.eq_delete(None));
        frame(&ctx, &mut p);
        assert!(p.eq_delete(Some("Mine".into())));
        let action = p.confirm.take().unwrap();
        p.confirmed(action);
        assert!(playerstate::load(None).eq_presets.is_empty());
        p.eq_open = false;
        let _ = a2;
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 5,000 invented songs on 500 albums (no files: the tracks are made directly).
    pub fn invented_tracks() -> Vec<Track> {
        let genres = ["Ambient", "Indie Pop", "Jazz", "Electronic", "Rock", "Classical", "Folk", "Soul", "Hip-Hop", "Metal"];
        let mut out = vec![];
        for a in 0..500usize {
            let artist = format!("Invented Artist {}", a % 120);
            let album = format!("Invented Album {a}");
            let lossy = a % 7 == 0;
            for n in 0..10usize {
                out.push(Track {
                    path: format!("/invented/Music/{artist}/{album}/{:02} Song {a}-{n}.{}", n + 1, if lossy { "mp3" } else { "flac" }),
                    title: format!("Song {a}-{n} of the invented library"),
                    artist: artist.clone(),
                    album: album.clone(),
                    album_artist: artist.clone(),
                    genre: genres[a % genres.len()].to_string(),
                    composer: format!("Composer {}", (a + n) % 60),
                    disc: 1,
                    track: n as i64 + 1,
                    year: (1970 + a % 45).to_string(),
                    duration: 120.0 + ((a * 7 + n * 13) % 200) as f64,
                    codec: if lossy { "mp3".into() } else { "flac".into() },
                    sample_rate: [44_100, 48_000, 96_000, 192_000][a % 4],
                    bits: if lossy { 0 } else { [16, 24, 24, 32][a % 4] },
                    is_lossy: lossy,
                    channels: 2,
                    bit_rate: if lossy { 320_000 } else { 900_000 },
                    size: 1000,
                    mtime_ns: (a * 1000 + n) as i64,
                    ..Default::default()
                });
            }
        }
        out
    }

    /// One frame with a mouse-wheel step over the page, tessellated as the window would.
    fn scroll_frame(ctx: &egui::Context, p: &mut Player, dy: f32) {
        let mut events = vec![egui::Event::PointerMoved(pos2(760.0, 520.0))];
        if dy != 0.0 {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 840.0))),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| p.ui(ui));
        let shapes = std::mem::take(&mut out.shapes);
        let ppp = out.pixels_per_point;
        out.drop_without_applying_deltas();
        let _ = ctx.tessellate(shapes, ppp);
    }

    /// Frame times while scrolling the pages of a big library (run with
    /// `cargo test --release -p realhires-gui profile_big_library -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn profile_big_library() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("rhr-player-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var("REALHIRES_COVERS", tmp.join("covers"));
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, Settings::default(), vec![]);
        let t0 = Instant::now();
        p.lib = LibView::new(invented_tracks());
        println!("LibView::new (5,000 songs, 500 albums): {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
        let pages = [
            ("Library", Page::Library),
            ("All Songs", Page::AllSongs),
            ("Albums", Page::Albums),
            ("Artists", Page::Artists),
            ("Genres", Page::Genres),
            ("Composers", Page::Composers),
            ("Years", Page::Years),
        ];
        for (name, page) in pages {
            p.go(page);
            for _ in 0..5 {
                scroll_frame(&ctx, &mut p, 0.0);
            }
            let mut ms = vec![];
            for i in 0..60 {
                let t = Instant::now();
                scroll_frame(&ctx, &mut p, if i % 20 < 10 { -60.0 } else { 60.0 });
                ms.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            let mean = ms.iter().sum::<f64>() / ms.len() as f64;
            let max = ms.iter().cloned().fold(0.0, f64::max);
            println!("{name:>10}: frame while scrolling: mean {mean:6.2} ms, max {max:6.2} ms");
        }
        drop(p);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
