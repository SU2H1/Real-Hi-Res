//! The music library of the player (2.0), the counterpart of
//! `realhires/musiclib.py`: the same `library.sqlite` (path, size, mtime_ns,
//! the track as JSON), so both versions share the index.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection};

use crate::json::{self, Obj as ObjectBuilder, Value};
use crate::paths::{abspath, file_name, parent};
use crate::playqueue::{number, Song};
use crate::probe::{probe, tag, AudioInfo};
use crate::pyunicode::casefold;
use crate::settings::Settings;

pub const LIBRARY_ENV: &str = "REALHIRES_LIBRARY";
pub const DB_NAME: &str = "library.sqlite";
pub const LIBRARY_VERSION: i64 = 1;

pub fn default_path() -> PathBuf {
    std::env::var_os(LIBRARY_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::store::data_dir().join(DB_NAME))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub disc: i64,
    pub track: i64,
    pub year: String,
    pub duration: f64,
    pub codec: String,
    pub sample_rate: i64,
    pub bits: i64,
    pub is_float: bool,
    pub is_lossy: bool,
    pub channels: i64,
    pub bit_rate: i64,
    pub size: i64,
    pub mtime_ns: i64,
}

impl Track {
    pub fn display_title(&self) -> String {
        if self.title.is_empty() {
            crate::paths::stem(&self.path)
        } else {
            self.title.clone()
        }
    }

    pub fn display_artist(&self) -> String {
        if !self.artist.is_empty() {
            self.artist.clone()
        } else if !self.album_artist.is_empty() {
            self.album_artist.clone()
        } else {
            "Unknown Artist".into()
        }
    }

    pub fn display_album(&self) -> String {
        if !self.album.is_empty() {
            return self.album.clone();
        }
        let folder = file_name(&parent(&self.path));
        if folder.is_empty() {
            "Unknown Album".into()
        } else {
            folder
        }
    }

    pub fn song(&self) -> Song {
        Song {
            path: self.path.clone(),
            title: self.display_title(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
            genre: self.genre.clone(),
            disc: self.disc,
            track: self.track,
        }
    }

    pub fn to_json(&self) -> Value {
        ObjectBuilder::new()
            .put("album", self.album.as_str())
            .put("album_artist", self.album_artist.as_str())
            .put("artist", self.artist.as_str())
            .put("bit_rate", self.bit_rate)
            .put("bits", self.bits)
            .put("channels", self.channels)
            .put("codec", self.codec.as_str())
            .put("disc", self.disc)
            .put("duration", self.duration)
            .put("genre", self.genre.as_str())
            .put("is_float", self.is_float)
            .put("is_lossy", self.is_lossy)
            .put("mtime_ns", self.mtime_ns)
            .put("path", self.path.as_str())
            .put("sample_rate", self.sample_rate)
            .put("size", self.size)
            .put("title", self.title.as_str())
            .put("track", self.track)
            .put("year", self.year.as_str())
            .build()
    }

    pub fn from_json(v: &Value) -> Option<Track> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let i = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
        let b = |k: &str| v.get(k).is_some_and(Value::truthy);
        Some(Track {
            path: v.get("path")?.as_str()?.to_string(),
            title: s("title"),
            artist: s("artist"),
            album: s("album"),
            album_artist: s("album_artist"),
            genre: s("genre"),
            disc: i("disc"),
            track: i("track"),
            year: s("year"),
            duration: v.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
            codec: s("codec"),
            sample_rate: i("sample_rate"),
            bits: i("bits"),
            is_float: b("is_float"),
            is_lossy: b("is_lossy"),
            channels: i("channels"),
            bit_rate: i("bit_rate"),
            size: i("size"),
            mtime_ns: i("mtime_ns"),
        })
    }
}

/// "Hi-Res Lossless", "Lossless" or the codec with its bit rate.
pub fn quality(t: &Track) -> String {
    if t.is_lossy {
        let kbps = if t.bit_rate > 0 {
            format!(" {} kbps", crate::pyfmt::round_int(t.bit_rate as f64 / 1000.0))
        } else {
            String::new()
        };
        return format!("{}{kbps}", t.codec.to_uppercase());
    }
    let bits = if t.bits == 0 { 16 } else { t.bits };
    if t.sample_rate > 48000 || bits > 16 || t.is_float {
        "Hi-Res Lossless".into()
    } else {
        "Lossless".into()
    }
}

/// "24-bit/96 kHz"
pub fn resolution(t: &Track) -> String {
    if t.sample_rate == 0 {
        return String::new();
    }
    let khz = format!("{} kHz", crate::pyfmt::g(t.sample_rate as f64 / 1000.0));
    if t.is_lossy || t.bits == 0 {
        return khz;
    }
    format!("{}-bit{}/{khz}", t.bits, if t.is_float { " float" } else { "" })
}

pub fn track_from_info(info: &AudioInfo, size: i64, mtime_ns: i64) -> Track {
    let t = |k: &str| tag(&info.tags, k).unwrap_or("").to_string();
    let bits =
        if info.is_lossy { 0 } else { info.claimed_bits.map(i64::from).unwrap_or(if info.is_float { 32 } else { 0 }) };
    Track {
        path: info.path.clone(),
        title: t("title"),
        artist: t("artist"),
        album: t("album"),
        album_artist: t("album_artist"),
        genre: t("genre"),
        disc: number(&t("disc")),
        track: number(&t("track")),
        year: t("date").chars().take(4).collect(),
        duration: info.duration.unwrap_or(0.0),
        codec: info.codec.clone(),
        sample_rate: info.sample_rate as i64,
        bits,
        is_float: info.is_float,
        is_lossy: info.is_lossy,
        channels: info.channels as i64,
        bit_rate: info.bit_rate.unwrap_or(0),
        size,
        mtime_ns,
    }
}

fn stat(path: &str) -> Option<(i64, i64)> {
    crate::store::stat(path).ok()
}

pub struct Library {
    pub path: PathBuf,
    db: Connection,
}

impl Library {
    pub fn open(path: Option<&Path>) -> rusqlite::Result<Library> {
        let path = path.map(Path::to_path_buf).unwrap_or_else(default_path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let db = Connection::open(&path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (path TEXT PRIMARY KEY, size INTEGER, mtime_ns INTEGER, data TEXT);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);",
        )?;
        db.execute("INSERT OR IGNORE INTO meta VALUES ('version', ?1)", params![LIBRARY_VERSION.to_string()])?;
        Ok(Library { path, db })
    }

    pub fn tracks(&self) -> Vec<Track> {
        let Ok(mut st) = self.db.prepare("SELECT data FROM tracks ORDER BY path") else { return vec![] };
        let rows = st.query_map([], |r| r.get::<_, String>(0));
        let Ok(rows) = rows else { return vec![] };
        rows.filter_map(|r| r.ok()).filter_map(|d| json::parse(&d).ok()).filter_map(|v| Track::from_json(&v)).collect()
    }

    pub fn put(&self, t: &Track) {
        let style = json::Style { indent: None, compact: false, ensure_ascii: false, sort_keys: true };
        let _ = self.db.execute(
            "INSERT OR REPLACE INTO tracks VALUES (?1, ?2, ?3, ?4)",
            params![t.path, t.size, t.mtime_ns, json::dumps(&t.to_json(), style)],
        );
    }

    /// Bring the library up to date with `files` (Python `Library.refresh`).
    pub fn refresh(
        &mut self,
        files: &[String],
        jobs: usize,
        progress: &mut dyn FnMut(usize, usize),
        stop: Option<&AtomicBool>,
    ) -> Vec<Track> {
        let files: Vec<String> = files.iter().map(|f| abspath(f)).collect();
        let wanted: std::collections::HashSet<&String> = files.iter().collect();
        let mut known: HashMap<String, (i64, i64)> = HashMap::new();
        if let Ok(mut st) = self.db.prepare("SELECT path, size, mtime_ns FROM tracks") {
            if let Ok(rows) =
                st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
            {
                for (p, s, m) in rows.flatten() {
                    known.insert(p, (s, m));
                }
            }
        }
        let tx = self.db.unchecked_transaction().ok();
        for p in known.keys().filter(|p| !wanted.contains(p)) {
            let _ = self.db.execute("DELETE FROM tracks WHERE path = ?1", params![p]);
        }
        let todo: Vec<(String, i64, i64)> = files
            .iter()
            .filter_map(|f| stat(f).map(|(s, m)| (f.clone(), s, m)))
            .filter(|(f, s, m)| known.get(f) != Some(&(*s, *m)))
            .collect();
        let results = crate::fix::run_parallel(&todo, jobs.max(1), |(path, size, mtime)| {
            if stop.is_some_and(|s| s.load(Ordering::Relaxed)) {
                return None;
            }
            probe(path).ok().map(|info| track_from_info(&info, *size, *mtime))
        });
        for (i, t) in results.into_iter().enumerate() {
            if let Some(t) = t {
                self.put(&t);
            }
            progress(i + 1, todo.len());
        }
        if let Some(tx) = tx {
            let _ = tx.commit();
        }
        self.tracks()
    }

    /// A file was changed by Real-Hi-Res (lyrics embedded, fixed): read it again.
    pub fn update_file(&self, path: &str) {
        if let (Some((size, mtime)), Ok(info)) = (stat(path), probe(path)) {
            self.put(&track_from_info(&info, size, mtime));
        }
    }
}

/// Every audio file in the library folders.
pub fn library_files(cfg: &Settings) -> Vec<String> {
    let folders: Vec<String> =
        cfg.library_folders.iter().filter(|f| !f.is_empty() && Path::new(f).is_dir()).cloned().collect();
    if folders.is_empty() {
        return vec![];
    }
    crate::paths::collect(&folders, true, &cfg.extension_set(), &mut |_| {})
}

// ---------------------------------------------------------------- browsing ---

pub fn sort_songs(tracks: &mut [Track]) {
    tracks.sort_by(|a, b| {
        (casefold(&a.display_title()), casefold(&a.display_artist()), &a.path).cmp(&(
            casefold(&b.display_title()),
            casefold(&b.display_artist()),
            &b.path,
        ))
    });
}

#[derive(Clone, Debug)]
pub struct Album {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub year: String,
    pub genre: String,
    pub tracks: Vec<Track>,
}

pub fn albums(tracks: &[Track]) -> Vec<Album> {
    let mut groups: Vec<(String, Vec<Track>)> = vec![];
    let mut index: HashMap<String, usize> = HashMap::new();
    for t in tracks {
        let key = t.song().album_key();
        match index.get(&key) {
            Some(i) => groups[*i].1.push(t.clone()),
            None => {
                index.insert(key.clone(), groups.len());
                groups.push((key, vec![t.clone()]));
            }
        }
    }
    let n = |x: i64| if x == 0 { 9999 } else { x };
    let mut out: Vec<Album> = groups
        .into_iter()
        .map(|(key, mut ts)| {
            ts.sort_by(|a, b| {
                (n(a.disc), n(a.track), casefold(&a.display_title()), &a.path).cmp(&(
                    n(b.disc),
                    n(b.track),
                    casefold(&b.display_title()),
                    &b.path,
                ))
            });
            let first = &ts[0];
            Album {
                key,
                title: first.display_album(),
                artist: if first.album_artist.is_empty() { first.display_artist() } else { first.album_artist.clone() },
                year: ts.iter().find(|t| !t.year.is_empty()).map(|t| t.year.clone()).unwrap_or_default(),
                genre: ts.iter().find(|t| !t.genre.is_empty()).map(|t| t.genre.clone()).unwrap_or_default(),
                tracks: ts,
            }
        })
        .collect();
    out.sort_by(|a, b| (casefold(&a.title), casefold(&a.artist)).cmp(&(casefold(&b.title), casefold(&b.artist))));
    out
}

#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub tracks: Vec<Track>,
}

fn group_by(tracks: &[Track], name_of: impl Fn(&Track) -> String) -> Vec<Group> {
    let mut groups: Vec<Group> = vec![];
    let mut index: HashMap<String, usize> = HashMap::new();
    for t in tracks {
        let name = name_of(t);
        let key = casefold(crate::pyunicode::strip(&name));
        match index.get(&key) {
            Some(i) => groups[*i].tracks.push(t.clone()),
            None => {
                index.insert(key, groups.len());
                groups.push(Group { name, tracks: vec![t.clone()] });
            }
        }
    }
    for g in groups.iter_mut() {
        sort_songs(&mut g.tracks);
    }
    groups.sort_by_key(|g| casefold(&g.name));
    groups
}

pub fn artists(tracks: &[Track]) -> Vec<Group> {
    group_by(tracks, |t| if t.album_artist.is_empty() { t.display_artist() } else { t.album_artist.clone() })
}

pub fn genres(tracks: &[Track]) -> Vec<Group> {
    group_by(tracks, |t| {
        let g = crate::pyunicode::strip(&t.genre);
        if g.is_empty() {
            "Unknown Genre".into()
        } else {
            g.to_string()
        }
    })
}
