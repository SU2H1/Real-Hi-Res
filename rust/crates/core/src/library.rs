//! Group scan results by album or by artist (the counterpart of
//! `realhires/library.py`).  Untagged files fall back to their folder: the
//! folder name is the album and its parent folder the artist.

use std::collections::{HashMap, HashSet};

use crate::paths::{file_name, parent};
use crate::verdict::{FileResult, Verdict};

pub const UNKNOWN_ARTIST: &str = "Unknown artist";

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn artist_of(r: &FileResult) -> String {
    let name = clean(r.tag("album_artist").filter(|s| !s.is_empty()).or_else(|| r.tag("artist")).unwrap_or(""));
    if !name.is_empty() {
        return name;
    }
    let p = file_name(&parent(&parent(&r.path)));
    if p.is_empty() {
        UNKNOWN_ARTIST.into()
    } else {
        p
    }
}

pub fn album_of(r: &FileResult) -> String {
    let a = clean(r.tag("album").unwrap_or(""));
    if !a.is_empty() {
        return a;
    }
    let d = file_name(&parent(&r.path));
    if d.is_empty() {
        "Unknown album".into()
    } else {
        d
    }
}

/// The leading number of a tag such as "3/12" (`missing` when there is none).
pub fn leading_number(v: Option<&str>, missing: u64) -> u64 {
    let s = v.unwrap_or("").trim_start();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(missing)
}

pub fn track_key(r: &FileResult) -> (u64, u64, String) {
    (leading_number(r.tag("disc"), 9999), leading_number(r.tag("track"), 9999), file_name(&r.path).to_lowercase())
}

#[derive(Clone, Debug)]
pub struct Group {
    /// unique id ("album\0artist" / "artist")
    pub key: String,
    pub title: String,
    /// artist (album view) or "N albums" (artist view)
    pub subtitle: String,
    pub paths: Vec<String>,
    pub fake: usize,
    pub suspicious: usize,
    pub total: usize,
    pub albums: usize,
}

impl Group {
    pub fn problems(&self) -> usize {
        self.fake + self.suspicious
    }
    pub fn problem_pct(&self) -> f64 {
        if self.total > 0 {
            100.0 * self.problems() as f64 / self.total as f64
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GroupMode {
    Album,
    Artist,
}

/// Tracks inside a group are sorted by disc and track number.
pub fn group_results<'a>(results: impl IntoIterator<Item = &'a FileResult>, mode: GroupMode) -> Vec<Group> {
    let mut order: Vec<String> = vec![];
    let mut groups: HashMap<String, Group> = HashMap::new();
    let mut album_sets: HashMap<String, HashSet<String>> = HashMap::new();
    let mut keys: HashMap<String, (u64, u64, String)> = HashMap::new();
    for r in results {
        let artist = artist_of(r);
        let (key, title, subtitle) = match mode {
            GroupMode::Album => {
                let album = album_of(r);
                (format!("{}\0{}", album.to_lowercase(), artist.to_lowercase()), album, artist)
            }
            GroupMode::Artist => {
                album_sets.entry(artist.to_lowercase()).or_default().insert(album_of(r).to_lowercase());
                (artist.to_lowercase(), artist, String::new())
            }
        };
        let g = groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            Group { key: key.clone(), title, subtitle, paths: vec![], fake: 0, suspicious: 0, total: 0, albums: 0 }
        });
        g.paths.push(r.path.clone());
        keys.insert(r.path.clone(), track_key(r));
        g.total += 1;
        match r.verdict {
            Verdict::Fake | Verdict::FakeLossless => g.fake += 1,
            Verdict::Suspect => g.suspicious += 1,
            _ => {}
        }
    }
    order
        .into_iter()
        .map(|k| {
            let mut g = groups.remove(&k).unwrap();
            g.paths.sort_by(|a, b| keys[a].cmp(&keys[b]));
            if mode == GroupMode::Artist {
                g.albums = album_sets[&k].len();
                g.subtitle = format!("{} album{}", g.albums, if g.albums != 1 { "s" } else { "" });
            }
            g
        })
        .collect()
}

pub fn sort_groups(groups: &mut [Group], by_name: bool) {
    if by_name {
        groups.sort_by_key(|g| (g.title.to_lowercase(), g.subtitle.to_lowercase()));
    } else {
        groups.sort_by(|a, b| {
            b.problem_pct()
                .total_cmp(&a.problem_pct())
                .then(b.problems().cmp(&a.problems()))
                .then(a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
    }
}
