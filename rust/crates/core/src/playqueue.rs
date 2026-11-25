//! The play queue (2.0), the counterpart of `realhires/playqueue.py`: the
//! shuffle modes (off, songs of the list, the whole library, same genre, same
//! artist, same album) and the repeat modes (off, song, album, stop after this
//! song).  The shuffled order comes from SplitMix64 and a Fisher-Yates shuffle,
//! so the same seed gives Python's order; `tests/python_queue.json` holds it.
//! Up Next can be edited (`move_upcoming()`, `remove_upcoming()`); removed songs
//! stay out when the order is built again, until the next `start()`.

use std::collections::{HashMap, HashSet};

use crate::pyunicode::{casefold, strip};

pub const SHUFFLE_MODES: [&str; 6] = ["off", "songs", "library", "genre", "artist", "album"];
pub const REPEAT_MODES: [&str; 4] = ["off", "song", "album", "stop"];

pub fn shuffle_label(mode: &str) -> &'static str {
    match mode {
        "songs" => "Songs in this list",
        "library" => "All songs in the library",
        "genre" => "Same genre",
        "artist" => "Same artist",
        "album" => "Same album",
        _ => "Off",
    }
}

pub fn repeat_label(mode: &str) -> &'static str {
    match mode {
        "song" => "Repeat song",
        "album" => "Repeat album",
        "stop" => "Stop after this song",
        _ => "Off",
    }
}

pub struct SplitMix64 {
    pub state: u64,
}

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    pub fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

pub fn shuffled(items: &[String], rng: &mut SplitMix64) -> Vec<String> {
    let mut out = items.to_vec();
    for i in (1..out.len()).rev() {
        let j = rng.below(i + 1);
        out.swap(i, j);
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub disc: i64,
    pub track: i64,
}

impl Song {
    pub fn artist_key(&self) -> String {
        let a = if self.album_artist.is_empty() { &self.artist } else { &self.album_artist };
        casefold(strip(a))
    }

    pub fn album_key(&self) -> String {
        format!("{}\u{0}{}", casefold(strip(&self.album)), self.artist_key())
    }

    pub fn genre_key(&self) -> String {
        casefold(strip(&self.genre))
    }
}

/// Disc, then track number, then title and path (songs without numbers last).
pub fn album_order<'a>(songs: impl IntoIterator<Item = &'a Song>) -> Vec<&'a Song> {
    let mut v: Vec<&Song> = songs.into_iter().collect();
    let n = |x: i64| if x == 0 { 9999 } else { x };
    v.sort_by(|a, b| {
        (n(a.disc), n(a.track), casefold(&a.title), &a.path).cmp(&(n(b.disc), n(b.track), casefold(&b.title), &b.path))
    });
    v
}

/// "3/12" -> 3; anything unreadable -> 0.
pub fn number(value: &str) -> i64 {
    let text = strip(strip(value).split('/').next().unwrap_or(""));
    if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().unwrap_or(0)
    } else {
        0
    }
}

pub struct PlayQueue {
    pub library: Vec<Song>,
    by_path: HashMap<String, usize>,
    pub context: Vec<String>,
    pub shuffle: String,
    pub repeat: String,
    rng: SplitMix64,
    pub order: Vec<String>,
    pub pos: usize,
    /// left out whenever the order is shuffled (unless chosen)
    pub disliked: HashSet<String>,
    /// taken out of Up Next by the user; left out until the next `start()`
    pub removed: HashSet<String>,
}

impl PlayQueue {
    pub fn new(library: Vec<Song>, context: Vec<String>, shuffle: &str, repeat: &str, seed: u64) -> PlayQueue {
        let by_path = library.iter().enumerate().map(|(i, s)| (s.path.clone(), i)).collect();
        PlayQueue {
            library,
            by_path,
            context,
            shuffle: if SHUFFLE_MODES.contains(&shuffle) { shuffle } else { "off" }.to_string(),
            repeat: if REPEAT_MODES.contains(&repeat) { repeat } else { "off" }.to_string(),
            rng: SplitMix64::new(seed),
            order: vec![],
            pos: 0,
            disliked: HashSet::new(),
            removed: HashSet::new(),
        }
    }

    fn song(&self, path: &str) -> Option<&Song> {
        self.by_path.get(path).map(|i| &self.library[*i])
    }

    pub fn start(&mut self, path: &str) -> String {
        self.removed.clear();
        let up = self.upcoming(path);
        self.order = std::iter::once(path.to_string()).chain(up).collect();
        self.pos = 0;
        path.to_string()
    }

    pub fn current(&self) -> Option<&str> {
        self.order.get(self.pos).map(String::as_str)
    }

    fn album_paths(&self, key: &str) -> Vec<String> {
        album_order(self.library.iter().filter(|s| s.album_key() == key)).into_iter().map(|s| s.path.clone()).collect()
    }

    fn pool(&self, path: &str) -> Vec<String> {
        let cur = self.song(path);
        let mut mode = self.shuffle.as_str();
        if self.repeat == "album" && !matches!(mode, "off" | "songs" | "album") {
            mode = "album";
        }
        if mode == "off" || mode == "songs" {
            if let (true, Some(c)) = (self.repeat == "album", cur) {
                return self.album_paths(&c.album_key());
            }
            return self.context.clone();
        }
        let Some(cur) = cur else {
            return self.library.iter().map(|s| s.path.clone()).collect();
        };
        if mode == "library" {
            return self.library.iter().map(|s| s.path.clone()).collect();
        }
        let key = |s: &Song| match mode {
            "genre" => s.genre_key(),
            "artist" => s.artist_key(),
            _ => s.album_key(),
        };
        let want = key(cur);
        if mode == "album" {
            return self.album_paths(&want);
        }
        self.library.iter().filter(|s| key(s) == want).map(|s| s.path.clone()).collect()
    }

    fn upcoming(&mut self, path: &str) -> Vec<String> {
        let pool = self.pool(path);
        if self.shuffle == "off" {
            let from = pool.iter().position(|p| p == path).map_or(0, |i| i + 1);
            return pool[from..].iter().filter(|p| !self.removed.contains(*p)).cloned().collect();
        }
        let rest: Vec<String> = pool
            .into_iter()
            .filter(|p| p != path && !self.disliked.contains(p) && !self.removed.contains(p))
            .collect();
        shuffled(&rest, &mut self.rng)
    }

    pub fn set_shuffle(&mut self, mode: &str) {
        self.shuffle = if SHUFFLE_MODES.contains(&mode) { mode } else { "off" }.to_string();
        self.rebuild();
    }

    pub fn set_repeat(&mut self, mode: &str) {
        self.repeat = if REPEAT_MODES.contains(&mode) { mode } else { "off" }.to_string();
        if self.repeat == "album" {
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        if let Some(cur) = self.current().map(str::to_string) {
            let up = self.upcoming(&cur);
            self.order.truncate(self.pos + 1);
            self.order.extend(up);
        }
    }

    /// The song to play next (`auto`: the current one ended by itself); None = stop.
    pub fn next(&mut self, auto: bool) -> Option<String> {
        let cur = self.current()?.to_string();
        if auto && self.repeat == "song" {
            return Some(cur);
        }
        if auto && self.repeat == "stop" {
            self.repeat = "off".into();
            return None;
        }
        if self.pos + 1 < self.order.len() {
            let nxt = self.order[self.pos + 1].clone();
            if self.repeat == "album" {
                if let (Some(a), Some(b)) = (self.song(&cur), self.song(&nxt)) {
                    if a.album_key() != b.album_key() {
                        return self.restart_album(&cur);
                    }
                }
            }
            self.pos += 1;
            return Some(nxt);
        }
        if self.repeat == "album" {
            return self.restart_album(&cur);
        }
        None
    }

    fn restart_album(&mut self, cur: &str) -> Option<String> {
        let key = self.song(cur)?.album_key();
        let mut tracks: Vec<String> =
            self.album_paths(&key).into_iter().filter(|p| p == cur || !self.removed.contains(p)).collect();
        if self.shuffle != "off" {
            tracks = shuffled(&tracks, &mut self.rng);
        }
        self.order.truncate(self.pos + 1);
        self.order.extend(tracks);
        self.pos += 1;
        self.order.get(self.pos).cloned()
    }

    pub fn previous(&mut self) -> Option<String> {
        if self.pos > 0 {
            self.pos -= 1;
            return Some(self.order[self.pos].clone());
        }
        None
    }

    pub fn up_next(&self, count: usize) -> Vec<String> {
        self.order.iter().skip(self.pos + 1).take(count).cloned().collect()
    }

    /// Move Up Next's song number `src` (0 = the next song) to place `dst`
    /// (counted in the list after the move). False when either is outside it.
    pub fn move_upcoming(&mut self, src: usize, dst: usize) -> bool {
        let (base, n) = (self.pos + 1, self.order.len().saturating_sub(self.pos + 1));
        if src >= n || dst >= n || src == dst {
            return false;
        }
        let song = self.order.remove(base + src);
        self.order.insert(base + dst, song);
        true
    }

    /// Take Up Next's song number `index` out of the queue; it stays out when the
    /// order is built again (unless it is the song playing). Returns its path.
    pub fn remove_upcoming(&mut self, index: usize) -> Option<String> {
        let base = self.pos + 1;
        if base + index >= self.order.len() {
            return None;
        }
        let path = self.order.remove(base + index);
        if self.current() != Some(path.as_str()) {
            self.removed.insert(path.clone());
        }
        Some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{self, Value};

    #[test]
    fn same_orders_as_python() {
        let data = json::parse(include_str!("../tests/python_queue.json")).unwrap();
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let songs: Vec<Song> = data
            .get("songs")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Song {
                path: s(v, "path"),
                title: s(v, "title"),
                artist: s(v, "artist"),
                album: s(v, "album"),
                album_artist: s(v, "album_artist"),
                genre: s(v, "genre"),
                disc: v.get("disc").unwrap().as_i64().unwrap(),
                track: v.get("track").unwrap().as_i64().unwrap(),
            })
            .collect();
        let strs = |v: &Value| -> Vec<Option<String>> {
            v.as_array().unwrap().iter().map(|x| x.as_str().map(str::to_string)).collect()
        };
        let context: Vec<String> = strs(data.get("context").unwrap()).into_iter().flatten().collect();
        let start = s(&data, "start");
        let ops: Vec<String> = strs(data.get("ops").unwrap()).into_iter().flatten().collect();
        for run in data.get("runs").unwrap().as_array().unwrap() {
            let seed = run.get("seed").unwrap().as_i64().unwrap() as u64;
            let mut q = PlayQueue::new(songs.clone(), context.clone(), &s(run, "shuffle"), &s(run, "repeat"), seed);
            if let Some(d) = run.get("disliked") {
                q.disliked = strs(d).into_iter().flatten().collect();
            }
            let mut steps = vec![Some(q.start(&start))];
            for op in &ops {
                steps.push(match op.as_str() {
                    "auto" => q.next(true),
                    "next" => q.next(false),
                    "prev" => q.previous(),
                    o if o.starts_with("set_shuffle:") => {
                        q.set_shuffle(&o[12..]);
                        q.current().map(str::to_string)
                    }
                    o => {
                        q.set_repeat(&o[11..]);
                        q.current().map(str::to_string)
                    }
                });
            }
            assert_eq!(steps, strs(run.get("steps").unwrap()), "{run:?}");
            let up: Vec<String> = strs(run.get("up_next").unwrap()).into_iter().flatten().collect();
            assert_eq!(q.up_next(30), up);
        }
        // Up Next edited by the user
        let edit_ops: Vec<String> = strs(data.get("edit_ops").unwrap()).into_iter().flatten().collect();
        for run in data.get("edit_runs").unwrap().as_array().unwrap() {
            let seed = run.get("seed").unwrap().as_i64().unwrap() as u64;
            let mut q = PlayQueue::new(songs.clone(), context.clone(), &s(run, "shuffle"), &s(run, "repeat"), seed);
            q.disliked = strs(run.get("disliked").unwrap()).into_iter().flatten().collect();
            q.start(&s(run, "start"));
            let steps = run.get("steps").unwrap().as_array().unwrap();
            assert_eq!(steps.len(), edit_ops.len());
            for (op, want) in edit_ops.iter().zip(steps) {
                let want_result = want.get("result").unwrap();
                let got: Option<String> = if let Some(arg) = op.strip_prefix("move:") {
                    let (a, b) = arg.split_once(':').unwrap();
                    let moved = q.move_upcoming(a.parse().unwrap(), b.parse().unwrap());
                    assert_eq!(Some(moved), want_result.as_bool(), "{op} {run:?}");
                    None
                } else if let Some(i) = op.strip_prefix("remove:") {
                    q.remove_upcoming(i.parse().unwrap())
                } else {
                    match op.as_str() {
                        "auto" => q.next(true),
                        "next" => q.next(false),
                        "prev" => q.previous(),
                        o if o.starts_with("set_shuffle:") => {
                            q.set_shuffle(&o[12..]);
                            q.current().map(str::to_string)
                        }
                        o => {
                            q.set_repeat(&o[11..]);
                            q.current().map(str::to_string)
                        }
                    }
                };
                if !op.starts_with("move:") {
                    assert_eq!(got, want_result.as_str().map(str::to_string), "{op} {run:?}");
                }
                assert_eq!(q.current().map(str::to_string), want.get("current").unwrap().as_str().map(str::to_string));
                let up: Vec<String> = strs(want.get("up_next").unwrap()).into_iter().flatten().collect();
                assert_eq!(q.up_next(30), up, "{op} {run:?}");
            }
            let mut removed: Vec<String> = q.removed.iter().cloned().collect();
            removed.sort();
            let want: Vec<String> = strs(run.get("removed").unwrap()).into_iter().flatten().collect();
            assert_eq!(removed, want);
        }
        let mut rng = SplitMix64::new(2024);
        for want in strs(data.get("splitmix_2024").unwrap()) {
            assert_eq!(rng.next().to_string(), want.unwrap());
        }
        let items: Vec<String> = (0..12).map(|i| i.to_string()).collect();
        let want: Vec<String> = strs(data.get("shuffled_9").unwrap()).into_iter().flatten().collect();
        assert_eq!(shuffled(&items, &mut SplitMix64::new(9)), want);
        assert_eq!((number("3/12"), number(" 07 "), number("x"), number("")), (3, 7, 0, 0));
    }
}
