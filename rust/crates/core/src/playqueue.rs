//! The play queue (2.0), the counterpart of `realhires/playqueue.py`: the
//! shuffle modes (off, songs of the list, the whole library, same genre, same
//! artist, same album) and the repeat modes (off, song, album, stop after this
//! song).  The shuffled order comes from SplitMix64 and a Fisher-Yates shuffle,
//! so the same seed gives Python's order; `tests/python_queue.json` holds it.

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

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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
    /// songs the user queued with Play Next / Add to Queue: they play before `order` goes on
    /// (each once, whatever the modes), and `up_next()` lists them first
    pub queued: Vec<String>,
    /// the furthest position played: after Previous, Next walks back up to it first
    pub top: usize,
    /// positions in `order` that hold queued songs: repeat album ignores them (a queued song
    /// from another album is an interruption, and the album being repeated goes on after it)
    pub inserted: HashSet<usize>,
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
            queued: vec![],
            top: 0,
            inserted: HashSet::new(),
        }
    }

    fn song(&self, path: &str) -> Option<&Song> {
        self.by_path.get(path).map(|i| &self.library[*i])
    }

    pub fn start(&mut self, path: &str) -> String {
        let up = self.upcoming(path);
        self.order = std::iter::once(path.to_string()).chain(up).collect();
        self.pos = 0;
        self.top = 0;
        self.inserted.clear();
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
            return match pool.iter().position(|p| p == path) {
                Some(i) => pool[i + 1..].to_vec(),
                None => pool,
            };
        }
        let rest: Vec<String> = pool.into_iter().filter(|p| p != path && !self.disliked.contains(p)).collect();
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
            self.top = self.pos;
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
        if !self.queued.is_empty() && self.pos >= self.top {
            let nxt = self.queued.remove(0);
            self.order.insert(self.pos + 1, nxt.clone());
            self.pos += 1;
            self.top = self.pos;
            self.inserted.insert(self.pos);
            return Some(nxt);
        }
        if self.pos + 1 < self.order.len() {
            let nxt = self.order[self.pos + 1].clone();
            if self.repeat == "album" && !self.inserted.contains(&(self.pos + 1)) {
                let base = self.album_song();
                if let (Some(a), Some(b)) = (self.song(&base), self.song(&nxt)) {
                    if a.album_key() != b.album_key() {
                        return self.restart_album(&base);
                    }
                }
            }
            self.pos += 1;
            self.top = self.top.max(self.pos);
            return Some(nxt);
        }
        if self.repeat == "album" {
            let base = self.album_song();
            return self.restart_album(&base);
        }
        None
    }

    /// The song whose album repeat album keeps to: the current one, or when that was queued,
    /// the last one played before it that was not.
    fn album_song(&self) -> String {
        let mut i = self.pos;
        while i > 0 && self.inserted.contains(&i) {
            i -= 1;
        }
        self.order[i].clone()
    }

    fn restart_album(&mut self, cur: &str) -> Option<String> {
        let key = self.song(cur)?.album_key();
        let mut tracks = self.album_paths(&key);
        if self.shuffle != "off" {
            tracks = shuffled(&tracks, &mut self.rng);
        }
        self.order.truncate(self.pos + 1);
        self.order.extend(tracks);
        self.pos += 1;
        self.top = self.pos;
        self.order.get(self.pos).cloned()
    }

    pub fn previous(&mut self) -> Option<String> {
        if self.pos > 0 {
            self.pos -= 1;
            return Some(self.order[self.pos].clone());
        }
        None
    }

    /// The songs to come: the queued ones first, then the rest of the order.
    pub fn up_next(&self, count: usize) -> Vec<String> {
        let top = self.top.max(self.pos);
        let back = self.order.get(self.pos + 1..top + 1).unwrap_or(&[]);
        back.iter().chain(&self.queued).chain(self.order.iter().skip(top + 1)).take(count).cloned().collect()
    }

    /// Play Next: these songs come right after the current one, before songs queued earlier.
    pub fn play_next(&mut self, paths: &[String]) {
        self.queued.splice(0..0, paths.iter().cloned());
    }

    /// Add to Queue: these songs come after the songs queued earlier, then the list goes on.
    pub fn add_to_queue(&mut self, paths: &[String]) {
        self.queued.extend(paths.iter().cloned());
    }

    /// Up Next: play the song at `index` of `up_next()` now.  The songs listed before it are
    /// skipped (queued ones are dropped); the list goes on after it.
    pub fn jump(&mut self, index: usize) -> Option<String> {
        let top = self.top.max(self.pos);
        let (back, n) = (top - self.pos, self.queued.len());
        if index < back {
            self.pos += index + 1; // a song stepped back over: the queue stays
            return Some(self.order[self.pos].clone());
        }
        if index < back + n {
            let k = index - back;
            let song = self.queued[k].clone();
            self.queued.drain(..=k);
            self.order.insert(top + 1, song.clone());
            self.pos = top + 1;
            self.top = self.pos;
            self.inserted.insert(self.pos);
            return Some(song);
        }
        let i = top + 1 + index - back - n;
        if i >= self.order.len() {
            return None;
        }
        self.queued.clear();
        self.pos = i;
        self.top = i;
        Some(self.order[i].clone())
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
        // the user's queue: Play Next / Add to Queue / Up Next jumps
        let everything: Vec<String> = songs.iter().map(|s| s.path.clone()).collect();
        let user_ops: Vec<String> = strs(data.get("user_ops").unwrap()).into_iter().flatten().collect();
        for run in data.get("user_runs").unwrap().as_array().unwrap() {
            let seed = run.get("seed").unwrap().as_i64().unwrap() as u64;
            let mut q = PlayQueue::new(songs.clone(), context.clone(), &s(run, "shuffle"), &s(run, "repeat"), seed);
            let mut got: Vec<Value> = vec![Value::Str(q.start(&start))];
            let opt = |v: Option<String>| v.map(Value::Str).unwrap_or(Value::Null);
            for op in &user_ops {
                let (name, arg) = op.split_once(':').unwrap_or((op.as_str(), ""));
                let picked: Vec<String> = if matches!(name, "play_next" | "add") {
                    arg.split(',').map(|i| everything[i.parse::<usize>().unwrap()].clone()).collect()
                } else {
                    vec![]
                };
                let step = match name {
                    "auto" => q.next(true),
                    "next" => q.next(false),
                    "prev" => q.previous(),
                    "play_next" => {
                        q.play_next(&picked);
                        q.current().map(str::to_string)
                    }
                    "add" => {
                        q.add_to_queue(&picked);
                        q.current().map(str::to_string)
                    }
                    "jump" => q.jump(arg.parse().unwrap()),
                    _ => {
                        q.set_shuffle(arg);
                        q.current().map(str::to_string)
                    }
                };
                got.push(opt(step));
                got.push(Value::Array(q.up_next(6).into_iter().map(Value::Str).collect()));
            }
            let want = run.get("steps").unwrap().as_array().unwrap();
            assert_eq!(got.len(), want.len());
            for (i, (g, w)) in got.iter().zip(want).enumerate() {
                assert_eq!(format!("{g:?}"), format!("{w:?}"), "{} {} step {i}", s(run, "shuffle"), s(run, "repeat"));
            }
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
