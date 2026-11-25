//! For You: playlists made from likes, retention and listening time (2.5), as
//! `realhires/recommend.py` makes them; its docstring explains the formula and the
//! mixes, and this module repeats every step in the same order
//! (`tests/recommend_vectors.rs` against `tests/python_recommend.json`).
//!
//! Per song (`song_stats()`): `r` of a play is the part of the song heard, `w` its
//! recency weight (`0.5 ** (age in days / 60)`); the retention is
//! `(Σ w·r + 1) / (Σ w + 2)`, the time `Σ w·seconds·r`, and the score
//! ±2 for a like or a dislike + 4 × (retention − 0.5) + log2(1 + time / 180).
//! The mixes (`mixes()`): Favourites Mix, On Repeat, Rediscover, New For You and the
//! Daily Mix (in the order of SHA-1(date + path)), each with at most four songs of an
//! artist (the Daily Mix three), a mix under five songs left out.
//!
//! The same answers in both languages: sums run over the plays as given (oldest first)
//! and over the songs sorted by path, every tie is broken by the path, and nothing is
//! ordered or compared on a value that went through `powf` or `log2` except as whole
//! millionths (`key()`), because those two may differ in the last bit between
//! platforms.

use std::collections::{BTreeMap, HashMap, HashSet};

use sha1::{Digest, Sha1};

use crate::json::{Obj, Value};
use crate::musiclib::{artist_pieces, Track};
use crate::pyunicode::{casefold, strip};
use crate::stats::{counted, local_parts, Play};

/// a play's weight halves every 60 days
pub const HALF_LIFE_DAYS: f64 = 60.0;
/// the retention of a song nothing is known about
pub const PRIOR_R: f64 = 0.5;
/// ... worth two plays
pub const PRIOR_N: f64 = 2.0;
/// a play of a song without a known duration is whole after this
pub const NO_DURATION_SECONDS: f64 = 30.0;
pub const LIKE_POINTS: f64 = 2.0;
pub const RETENTION_POINTS: f64 = 4.0;
/// seconds of recent listening to the end that are worth one point
pub const TIME_UNIT: f64 = 180.0;

/// a mix with fewer songs is left out
pub const MIN_SONGS: usize = 5;
pub const MAX_PER_ARTIST: usize = 4;
pub const DAILY_PER_ARTIST: usize = 3;
/// the artist limit gives way until a mix has this many songs
pub const MIN_FILL: usize = 12;
pub const FAVOURITES_SIZE: usize = 50;
pub const ON_REPEAT_SIZE: usize = 30;
pub const ON_REPEAT_DAYS: f64 = 30.0;
pub const REDISCOVER_SIZE: usize = 30;
pub const REDISCOVER_DAYS: f64 = 60.0;
pub const REDISCOVER_PLAYS: i64 = 3;
pub const REDISCOVER_RETENTION: f64 = 0.6;
pub const DISCOVER_SIZE: usize = 30;
pub const DAILY_SIZE: usize = 40;
pub const DAILY_REDISCOVER: usize = 8;
pub const DAILY_DISCOVER: usize = 8;
/// each part of the Daily Mix is drawn from this many times its songs
pub const DAILY_POOL: usize = 3;
const DAY: f64 = 86400.0;

/// id, name, description (English; the apps show them through `i18n::tr()`)
pub const MIXES: [(&str, &str, &str); 5] = [
    ("favourites", "Favourites Mix", "The songs you like and listen to the most."),
    ("on_repeat", "On Repeat", "What you played most in the last 30 days."),
    ("rediscover", "Rediscover", "Songs you loved and have not played for a while."),
    ("discover", "New For You", "Songs you have not played yet, close to what you like."),
    ("daily", "Daily Mix", "A different mix every day: favourites, old loves and new songs."),
];
pub const HOW_NOTE: &str =
    "Made on this computer from your likes, how much of each song you listen to and the time you spend with it.";
pub const EMPTY_NOTE: &str = "Play and like some songs, and your mixes will appear here.";

/// What the history says about one song.
#[derive(Clone, Debug, PartialEq)]
pub struct SongStats {
    pub path: String,
    /// counted plays (`stats::counted()`)
    pub plays: i64,
    /// logged plays that are not counted
    pub skips: i64,
    /// every logged second
    pub seconds: f64,
    /// Unix time of the newest play, 0.0 = never
    pub last_played: f64,
    /// recency-weighted, smoothed part of the song heard
    pub retention: f64,
    /// recency-weighted seconds, each play's by the part heard
    pub time: f64,
    pub score: f64,
    /// the same without the recency weight
    pub plain_retention: f64,
    pub plain_score: f64,
    /// counted plays of the last `ON_REPEAT_DAYS` days
    pub recent_plays: i64,
    /// seconds of the last `ON_REPEAT_DAYS` days
    pub recent_seconds: f64,
    pub liked: bool,
    pub disliked: bool,
}

impl SongStats {
    pub fn new(path: &str) -> SongStats {
        SongStats {
            path: path.to_string(),
            plays: 0,
            skips: 0,
            seconds: 0.0,
            last_played: 0.0,
            retention: PRIOR_R,
            time: 0.0,
            score: 0.0,
            plain_retention: PRIOR_R,
            plain_score: 0.0,
            recent_plays: 0,
            recent_seconds: 0.0,
            liked: false,
            disliked: false,
        }
    }

    pub fn to_json(&self) -> Value {
        Obj::new()
            .put("path", self.path.as_str())
            .put("plays", self.plays)
            .put("skips", self.skips)
            .put("seconds", self.seconds)
            .put("last_played", self.last_played)
            .put("retention", self.retention)
            .put("time", self.time)
            .put("score", self.score)
            .put("plain_retention", self.plain_retention)
            .put("plain_score", self.plain_score)
            .put("recent_plays", self.recent_plays)
            .put("recent_seconds", self.recent_seconds)
            .put("liked", self.liked)
            .put("disliked", self.disliked)
            .build()
    }
}

/// `x` in whole millionths: what scores and seconds are ordered and compared by.
pub fn key(x: f64) -> i64 {
    (x * 1e6 + 0.5).floor() as i64
}

/// The part of a song that one play heard, 0..1.
pub fn heard(seconds: f64, duration: f64) -> f64 {
    let seconds = if seconds > 0.0 { seconds } else { 0.0 };
    let part = if duration > 0.0 { seconds / duration } else { seconds / NO_DURATION_SECONDS };
    if part < 1.0 {
        part
    } else {
        1.0
    }
}

/// The recency weight of a play that old (a play "in the future" counts as now).
pub fn weight(age_seconds: f64) -> f64 {
    let age = if age_seconds > 0.0 { age_seconds } else { 0.0 };
    0.5f64.powf(age / DAY / HALF_LIFE_DAYS)
}

pub fn score_of(liked: bool, disliked: bool, retention: f64, seconds: f64) -> f64 {
    let like = (if liked { LIKE_POINTS } else { 0.0 }) - (if disliked { LIKE_POINTS } else { 0.0 });
    like + RETENTION_POINTS * (retention - PRIOR_R) + (1.0 + seconds / TIME_UNIT).log2()
}

/// The statistics of every song that was played, liked or disliked, by its path
/// (plays oldest first, as `Library::plays()` gives them).
pub fn song_stats(plays: &[Play], liked: &[String], disliked: &[String], now: f64) -> HashMap<String, SongStats> {
    let liked: HashSet<&str> = liked.iter().map(String::as_str).collect();
    let disliked: HashSet<&str> = disliked.iter().map(String::as_str).collect();
    let mut out: HashMap<String, SongStats> = HashMap::new();
    // Σ w·r, Σ w, Σ w·seconds·r, Σ r, n, Σ seconds·r
    let mut sums: HashMap<String, [f64; 6]> = HashMap::new();
    let recent = now - ON_REPEAT_DAYS * DAY;
    for p in plays {
        let s = out.entry(p.path.clone()).or_insert_with(|| SongStats::new(&p.path));
        let a = sums.entry(p.path.clone()).or_insert([0.0; 6]);
        let seconds = if p.seconds > 0.0 { p.seconds } else { 0.0 };
        let c = counted(p);
        if c {
            s.plays += 1;
        } else {
            s.skips += 1;
        }
        s.seconds += seconds;
        if p.started > s.last_played {
            s.last_played = p.started;
        }
        if p.started >= recent {
            s.recent_seconds += seconds;
            if c {
                s.recent_plays += 1;
            }
        }
        let r = heard(seconds, p.duration);
        let w = weight(now - p.started);
        a[0] += w * r;
        a[1] += w;
        a[2] += w * seconds * r;
        a[3] += r;
        a[4] += 1.0;
        a[5] += seconds * r;
    }
    for path in liked.iter().chain(disliked.iter()) {
        if !out.contains_key(*path) {
            out.insert(path.to_string(), SongStats::new(path));
        }
    }
    for (path, s) in out.iter_mut() {
        let a = sums.get(path).copied().unwrap_or([0.0; 6]);
        s.liked = liked.contains(path.as_str());
        s.disliked = disliked.contains(path.as_str());
        s.retention = (a[0] + PRIOR_R * PRIOR_N) / (a[1] + PRIOR_N);
        s.time = a[2];
        s.score = score_of(s.liked, s.disliked, s.retention, s.time);
        s.plain_retention = (a[3] + PRIOR_R * PRIOR_N) / (a[4] + PRIOR_N);
        s.plain_score = score_of(s.liked, s.disliked, s.plain_retention, a[5]);
    }
    out
}

// --------------------------------------------------------------- artists ---

/// The artist a song counts for in the mixes' limits: the first name of its artist
/// field (else of its album artist), in any case; "" without one.
pub fn artist_key(t: &Track) -> String {
    let mut pieces = artist_pieces(&t.artist);
    if pieces.is_empty() {
        pieces = artist_pieces(&t.album_artist);
    }
    pieces.first().map(|p| casefold(p)).unwrap_or_default()
}

/// The album a song belongs to for New For You ("" without an album tag).
pub fn album_key(t: &Track) -> String {
    let album = casefold(strip(&t.album));
    if album.is_empty() {
        return album;
    }
    let artist = if t.album_artist.is_empty() { &t.artist } else { &t.album_artist };
    format!("{album}\u{0}{}", casefold(strip(artist)))
}

pub fn genre_key(t: &Track) -> String {
    casefold(strip(&t.genre))
}

/// The first `size` of `paths` with at most `per_artist` songs of an artist (songs
/// without an artist, "", are not limited), in their order.  While that leaves fewer
/// than `MIN_FILL` songs and some were refused, the limit rises by one.
pub fn limit_artists(paths: &[String], artist_of: &dyn Fn(&str) -> String, size: usize, per_artist: usize) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut taken: Vec<usize> = vec![];
    let mut rest: Vec<usize> = (0..paths.len()).collect();
    let (mut limit, mut target) = (per_artist, size);
    while !rest.is_empty() && taken.len() < target {
        let mut left = vec![];
        for i in rest {
            let k = artist_of(&paths[i]);
            if taken.len() < target && (k.is_empty() || counts.get(&k).copied().unwrap_or(0) < limit) {
                taken.push(i);
                if !k.is_empty() {
                    *counts.entry(k).or_insert(0) += 1;
                }
            } else {
                left.push(i);
            }
        }
        rest = left;
        limit += 1;
        target = size.min(MIN_FILL);
    }
    taken.sort();
    taken.into_iter().map(|i| paths[i].clone()).collect()
}

/// `paths` with two songs of one artist not following each other where that can be
/// avoided, otherwise in their order: each place gets the first song left whose artist
/// is not the one before it; an artist who has half of the songs left (or more) goes
/// first, or their songs would meet at the end.
pub fn spread_artists(paths: &[String], artist_of: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut left: Vec<(String, String)> = paths.iter().map(|p| (p.clone(), artist_of(p))).collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, k) in &left {
        if !k.is_empty() {
            *counts.entry(k.clone()).or_insert(0) += 1;
        }
    }
    let mut out = Vec::with_capacity(left.len());
    let mut prev = String::new();
    while !left.is_empty() {
        let n = left.len();
        // an artist who needs every other place from here on
        let pick = left
            .iter()
            .position(|(_, k)| !k.is_empty() && *k != prev && 2 * counts[k] >= n)
            .or_else(|| left.iter().position(|(_, k)| k.is_empty() || *k != prev))
            .unwrap_or(0);
        let (path, k) = left.remove(pick);
        if !k.is_empty() {
            *counts.get_mut(&k).unwrap() -= 1;
        }
        prev = k;
        out.push(path);
    }
    out
}

// ------------------------------------------------------------------ the day ---

/// The local date of `now` as the Daily Mix uses it: "2026-11-25".
pub fn day_of(now: f64) -> String {
    let t = local_parts(now);
    format!("{:04}-{:02}-{:02}", t.year, t.month, t.day)
}

pub fn day_hash(day: &str, path: &str) -> String {
    let mut h = Sha1::new();
    h.update(day.as_bytes());
    h.update(path.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `paths` in the day's order: by SHA-1(day + path), then the path.
pub fn day_order(day: &str, paths: &[String]) -> Vec<String> {
    let mut keyed: Vec<(String, &String)> = paths.iter().map(|p| (day_hash(day, p), p)).collect();
    keyed.sort();
    keyed.into_iter().map(|(_, p)| p.clone()).collect()
}

// ---------------------------------------------------------------- the mixes ---

/// The songs each mix chooses from, best first, before the limit per artist.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Candidates {
    pub favourites: Vec<String>,
    pub on_repeat: Vec<String>,
    pub rediscover: Vec<String>,
    pub discover: Vec<String>,
}

/// One mix: its id, name and description in English (`MIXES`) and its songs in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub paths: Vec<String>,
}

fn by_key(mut rows: Vec<(i64, i64, &str)>) -> Vec<String> {
    rows.sort();
    rows.into_iter().map(|(_, _, p)| p.to_string()).collect()
}

pub fn candidates(tracks: &[Track], plays: &[Play], liked: &[String], disliked: &[String], now: f64) -> Candidates {
    let stats = song_stats(plays, liked, disliked, now);
    let dis: HashSet<&str> = disliked.iter().map(String::as_str).collect();
    // each path once (the last track of a path), by path
    let library: Vec<&Track> = tracks.iter().map(|t| (t.path.as_str(), t)).collect::<BTreeMap<_, _>>().into_values().collect();
    let songs: Vec<&Track> = library.iter().copied().filter(|t| !dis.contains(t.path.as_str())).collect();
    let blank = SongStats::new("");
    let st = |t: &Track| stats.get(&t.path).unwrap_or(&blank);

    let favourites = by_key(songs.iter().filter(|t| key(st(t).score) > 0).map(|t| (-key(st(t).score), 0, t.path.as_str())).collect());
    let on_repeat = by_key(
        songs
            .iter()
            .filter(|t| st(t).recent_plays > 0 && key(st(t).score) > 0)
            .map(|t| (-st(t).recent_plays, -key(st(t).recent_seconds), t.path.as_str()))
            .collect(),
    );
    let before = now - REDISCOVER_DAYS * DAY;
    let rediscover = by_key(
        songs
            .iter()
            .filter(|t| {
                let s = st(t);
                0.0 < s.last_played
                    && s.last_played < before
                    && key(s.plain_score) > 0
                    && (s.liked || (s.plays >= REDISCOVER_PLAYS && s.plain_retention >= REDISCOVER_RETENTION))
            })
            .map(|t| (-key(st(t).plain_score), 0, t.path.as_str()))
            .collect(),
    );
    // New For You: the mean score of the songs the user knows, per artist, album and genre
    let keys_of: [fn(&Track) -> String; 3] = [artist_key, album_key, genre_key];
    let mut means: [HashMap<String, (f64, f64)>; 3] = Default::default();
    for t in &library {
        let Some(s) = stats.get(&t.path) else { continue };
        if !(s.plays > 0 || s.liked || s.disliked) {
            continue;
        }
        for (table, key_of) in means.iter_mut().zip(keys_of) {
            let k = key_of(t);
            if !k.is_empty() {
                let a = table.entry(k).or_insert((0.0, 0.0));
                a.0 += s.score;
                a.1 += 1.0;
            }
        }
    }
    let mean = |i: usize, t: &Track| means[i].get(&keys_of[i](t)).map_or(0.0, |a| a.0 / a.1);
    let mut ranks = vec![];
    for t in &songs {
        let s = st(t);
        if s.plays > 0 || s.liked {
            continue;
        }
        let rank = key((2.0 * mean(0, t) + mean(1, t) + mean(2, t)) / 4.0 + s.score);
        if rank > 0 {
            ranks.push((-rank, 0, t.path.as_str()));
        }
    }
    Candidates { favourites, on_repeat, rediscover, discover: by_key(ranks) }
}

fn first(v: &[String], n: usize) -> &[String] {
    &v[..v.len().min(n)]
}

/// The Daily Mix of `day` from the mixes' candidates (see the Python module's text).
pub fn daily_mix(day: &str, pools: &Candidates, artist_of: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut chosen: Vec<String> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut take = |pool: &[String], quota: usize, limit: usize, chosen: &mut Vec<String>| {
        let mut got = 0;
        for p in pool {
            if got >= quota {
                break;
            }
            let k = artist_of(p);
            if seen.contains(p) || (!k.is_empty() && counts.get(&k).copied().unwrap_or(0) >= limit) {
                continue;
            }
            seen.insert(p.clone());
            chosen.push(p.clone());
            got += 1;
            if !k.is_empty() {
                *counts.entry(k).or_insert(0) += 1;
            }
        }
        got
    };
    take(&day_order(day, first(&pools.rediscover, DAILY_POOL * DAILY_REDISCOVER)), DAILY_REDISCOVER, DAILY_PER_ARTIST, &mut chosen);
    take(&day_order(day, first(&pools.discover, DAILY_POOL * DAILY_DISCOVER)), DAILY_DISCOVER, DAILY_PER_ARTIST, &mut chosen);
    let rest = DAILY_SIZE - DAILY_REDISCOVER - DAILY_DISCOVER;
    let favourites = day_order(day, first(&pools.favourites, DAILY_POOL * rest));
    let room = DAILY_SIZE - chosen.len();
    take(&favourites, room, DAILY_PER_ARTIST, &mut chosen);
    let mut limit = DAILY_PER_ARTIST;
    while chosen.len() < MIN_FILL {
        limit += 1;
        let room = MIN_FILL - chosen.len();
        // nothing taken: every favourite is in the mix
        if take(&favourites, room, limit, &mut chosen) == 0 {
            break;
        }
    }
    spread_artists(&day_order(day, &chosen), artist_of)
}

/// The mixes, in `MIXES`' order, those with fewer than `MIN_SONGS` songs left out.
/// `day` ("YYYY-MM-DD", `day_of(now)`) is the Daily Mix's day.
pub fn mixes(tracks: &[Track], plays: &[Play], liked: &[String], disliked: &[String], now: f64, day: &str) -> Vec<Mix> {
    let pools = candidates(tracks, plays, liked, disliked, now);
    let artists: HashMap<&str, String> = tracks.iter().map(|t| (t.path.as_str(), artist_key(t))).collect();
    let artist_of = |path: &str| artists.get(path).cloned().unwrap_or_default();
    let limited = |pool: &[String], size: usize| spread_artists(&limit_artists(pool, &artist_of, size, MAX_PER_ARTIST), &artist_of);
    let mut paths = [
        limited(&pools.favourites, FAVOURITES_SIZE),
        limited(&pools.on_repeat, ON_REPEAT_SIZE),
        limited(&pools.rediscover, REDISCOVER_SIZE),
        limited(&pools.discover, DISCOVER_SIZE),
        daily_mix(day, &pools, &artist_of),
    ];
    MIXES
        .iter()
        .zip(paths.iter_mut())
        .filter(|(_, p)| p.len() >= MIN_SONGS)
        .map(|((id, name, description), p)| Mix { id, name, description, paths: std::mem::take(p) })
        .collect()
}
