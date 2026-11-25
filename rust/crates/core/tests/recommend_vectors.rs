//! 2.5: For You (`recommend.rs`) against Python's answers
//! (`rust/tools/gen_recommend_vectors.py` writes `python_recommend.json`): the
//! statistics of every song of the invented histories (numbers within rounding), the
//! candidates of every mix and the mixes on several dates (orders exactly), the limits
//! per artist, the spreading of artists, the day's order and the keys.

use std::collections::HashMap;

use realhires::json::{self, Value};
use realhires::musiclib::Track;
use realhires::recommend::{self, Candidates, Mix};
use realhires::stats::Play;

fn load() -> Value {
    let path = format!("{}/tests/python_recommend.json", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_array().unwrap()
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_f64().unwrap()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_str().unwrap().to_string()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0), "{what}: {a} != {b}");
}

fn track(v: &Value) -> Track {
    Track {
        path: text(v, "path"),
        title: text(v, "title"),
        artist: text(v, "artist"),
        album: text(v, "album"),
        album_artist: text(v, "album_artist"),
        genre: text(v, "genre"),
        duration: num(v, "duration"),
        ..Default::default()
    }
}

fn play(v: &Value) -> Play {
    Play { path: text(v, "path"), started: num(v, "started"), seconds: num(v, "seconds"), duration: num(v, "duration"), ..Default::default() }
}

fn mixes_json(mixes: &[Mix]) -> Vec<(String, String, String, Vec<String>)> {
    mixes.iter().map(|m| (m.id.to_string(), m.name.to_string(), m.description.to_string(), m.paths.clone())).collect()
}

#[test]
fn constants_and_texts_equal_python() {
    let data = load();
    let c = data.get("constants").unwrap();
    let pairs: [(&str, f64); 24] = [
        ("HALF_LIFE_DAYS", recommend::HALF_LIFE_DAYS),
        ("PRIOR_R", recommend::PRIOR_R),
        ("PRIOR_N", recommend::PRIOR_N),
        ("NO_DURATION_SECONDS", recommend::NO_DURATION_SECONDS),
        ("LIKE_POINTS", recommend::LIKE_POINTS),
        ("RETENTION_POINTS", recommend::RETENTION_POINTS),
        ("TIME_UNIT", recommend::TIME_UNIT),
        ("MIN_SONGS", recommend::MIN_SONGS as f64),
        ("MAX_PER_ARTIST", recommend::MAX_PER_ARTIST as f64),
        ("DAILY_PER_ARTIST", recommend::DAILY_PER_ARTIST as f64),
        ("MIN_FILL", recommend::MIN_FILL as f64),
        ("FAVOURITES_SIZE", recommend::FAVOURITES_SIZE as f64),
        ("ON_REPEAT_SIZE", recommend::ON_REPEAT_SIZE as f64),
        ("ON_REPEAT_DAYS", recommend::ON_REPEAT_DAYS),
        ("REDISCOVER_SIZE", recommend::REDISCOVER_SIZE as f64),
        ("REDISCOVER_DAYS", recommend::REDISCOVER_DAYS),
        ("REDISCOVER_PLAYS", recommend::REDISCOVER_PLAYS as f64),
        ("REDISCOVER_RETENTION", recommend::REDISCOVER_RETENTION),
        ("DISCOVER_SIZE", recommend::DISCOVER_SIZE as f64),
        ("DISCOVER_SKIPS", recommend::DISCOVER_SKIPS as f64),
        ("DAILY_SIZE", recommend::DAILY_SIZE as f64),
        ("DAILY_REDISCOVER", recommend::DAILY_REDISCOVER as f64),
        ("DAILY_DISCOVER", recommend::DAILY_DISCOVER as f64),
        ("DAILY_POOL", recommend::DAILY_POOL as f64),
    ];
    assert_eq!(c.as_object().unwrap().len(), pairs.len());
    for (name, value) in pairs {
        assert_eq!(num(c, name), value, "{name}");
    }
    let table: Vec<Vec<String>> = arr(&data, "mixes").iter().map(strings).collect();
    let ours: Vec<Vec<String>> = recommend::MIXES.iter().map(|(a, b, c)| vec![a.to_string(), b.to_string(), c.to_string()]).collect();
    assert_eq!(table, ours);
    assert_eq!(text(&data, "how_note"), recommend::HOW_NOTE);
    assert_eq!(text(&data, "empty_note"), recommend::EMPTY_NOTE);
}

#[test]
fn stats_candidates_and_mixes_equal_python() {
    let data = load();
    let libraries: HashMap<String, Vec<Track>> =
        data.get("libraries").unwrap().as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(track).collect())).collect();
    let histories: HashMap<String, Vec<Play>> =
        data.get("histories").unwrap().as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_array().unwrap().iter().map(play).collect())).collect();
    let (mut seen_mixes, mut seen_days) = (0, 0);
    for sc in arr(&data, "scenarios") {
        let name = text(sc, "name");
        let tracks = &libraries[&text(sc, "library")];
        let plays = &histories[&text(sc, "history")];
        let (liked, disliked, now) = (strings(sc.get("liked").unwrap()), strings(sc.get("disliked").unwrap()), num(sc, "now"));

        // ---- every song's statistics
        let stats = recommend::song_stats(plays, &liked, &disliked, now);
        let want = sc.get("stats").unwrap().as_object().unwrap();
        assert_eq!(stats.len(), want.len(), "{name}: songs with statistics");
        for (path, w) in want {
            let s = stats.get(path).unwrap_or_else(|| panic!("{name}: no statistics of {path}"));
            let got = s.to_json();
            assert_eq!(got.as_object().unwrap().len(), w.as_object().unwrap().len(), "{name}: fields");
            for (k, v) in got.as_object().unwrap() {
                let x = w.get(k).unwrap_or_else(|| panic!("{name}: no {k}"));
                match v {
                    Value::Float(f) => close(*f, x.as_f64().unwrap(), &format!("{name}: {path} {k}")),
                    _ => assert_eq!(v, x, "{name}: {path} {k}"),
                }
            }
            // the plain retention is made of exact operations only: equal to the last bit
            assert_eq!(s.plain_retention, num(w, "plain_retention"), "{name}: {path}");
            assert_eq!(s.seconds, num(w, "seconds"), "{name}: {path}");
        }

        // ---- the candidates of every mix, best first
        let c = recommend::candidates(tracks, plays, &liked, &disliked, now);
        let w = sc.get("candidates").unwrap();
        let want = Candidates {
            favourites: strings(w.get("favourites").unwrap()),
            on_repeat: strings(w.get("on_repeat").unwrap()),
            rediscover: strings(w.get("rediscover").unwrap()),
            discover: strings(w.get("discover").unwrap()),
        };
        assert_eq!(c, want, "{name}: candidates");

        // ---- the mixes of each day
        for (day, ms) in sc.get("days").unwrap().as_object().unwrap() {
            let got = mixes_json(&recommend::mixes(tracks, plays, &liked, &disliked, now, day));
            let want: Vec<(String, String, String, Vec<String>)> = ms
                .as_array()
                .unwrap()
                .iter()
                .map(|m| (text(m, "id"), text(m, "name"), text(m, "description"), strings(m.get("paths").unwrap())))
                .collect();
            assert_eq!(got, want, "{name} on {day}");
            seen_mixes += got.len();
            seen_days += 1;
            // what the mixes promise, whatever Python says
            let artists: HashMap<&str, String> = tracks.iter().map(|t| (t.path.as_str(), recommend::artist_key(t))).collect();
            for (id, _, _, paths) in &got {
                assert!(paths.len() >= recommend::MIN_SONGS, "{name} {id}");
                let mut once = paths.clone();
                once.sort();
                once.dedup();
                assert_eq!(once.len(), paths.len(), "{name} {id}: a song twice");
                assert!(paths.iter().all(|p| artists.contains_key(p.as_str()) && !disliked.contains(p)), "{name} {id}: gone or disliked");
            }
        }
    }
    assert!(seen_mixes > 40 && seen_days > 20, "{seen_mixes} mixes on {seen_days} days");
}

#[test]
fn the_daily_mix_changes_with_the_day() {
    let data = load();
    let big = arr(&data, "scenarios").iter().find(|s| text(s, "name") == "big").unwrap();
    let days = big.get("days").unwrap().as_object().unwrap();
    let daily: Vec<Vec<String>> = days
        .iter()
        .map(|(_, ms)| strings(ms.as_array().unwrap().iter().find(|m| text(m, "id") == "daily").unwrap().get("paths").unwrap()))
        .collect();
    assert!(daily.len() >= 4);
    for i in 1..daily.len() {
        assert_ne!(daily[0], daily[i], "another day, another order");
    }
    let other: Vec<Vec<String>> = days
        .iter()
        .map(|(_, ms)| strings(ms.as_array().unwrap().iter().find(|m| text(m, "id") == "favourites").unwrap().get("paths").unwrap()))
        .collect();
    assert!(other.iter().all(|m| *m == other[0]), "only the Daily Mix follows the date");
}

#[test]
fn limits_spreading_and_the_days_order_equal_python() {
    let data = load();
    let names: HashMap<String, String> = data.get("spread_artists").unwrap().as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
    for case in arr(&data, "spread") {
        let paths = strings(case.get("paths").unwrap());
        let got = recommend::spread_artists(&paths, &|p| names.get(p).cloned().unwrap_or_default());
        assert_eq!(got, strings(case.get("out").unwrap()), "{paths:?}");
    }
    for case in arr(&data, "limit") {
        let paths = strings(case.get("paths").unwrap());
        let artists: HashMap<String, String> = case.get("artists").unwrap().as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        let (size, per) = (num(case, "size") as usize, num(case, "per_artist") as usize);
        let got = recommend::limit_artists(&paths, &|p| artists.get(p).cloned().unwrap_or_default(), size, per);
        assert_eq!(got, strings(case.get("out").unwrap()), "{size} songs, {per} per artist of {paths:?}");
    }
    let paths = strings(data.get("day_paths").unwrap());
    for case in arr(&data, "day_order") {
        let day = text(case, "day");
        let hashes: Vec<String> = paths.iter().map(|p| recommend::day_hash(&day, p)).collect();
        assert_eq!(hashes, strings(case.get("hashes").unwrap()), "{day}");
        assert_eq!(recommend::day_order(&day, &paths), strings(case.get("out").unwrap()), "{day}");
    }
    for row in arr(&data, "track_keys") {
        let row = row.as_array().unwrap();
        let t = track(&row[0]);
        let got = [recommend::artist_key(&t), recommend::album_key(&t), recommend::genre_key(&t)];
        let want = [row[1].as_str().unwrap(), row[2].as_str().unwrap(), row[3].as_str().unwrap()];
        assert_eq!(got, want, "{t:?}");
    }
}

#[test]
fn the_small_parts_equal_python() {
    let data = load();
    for row in arr(&data, "keys") {
        let row = row.as_array().unwrap();
        assert_eq!(recommend::key(row[0].as_f64().unwrap()), row[1].as_i64().unwrap(), "{row:?}");
    }
    for row in arr(&data, "heard") {
        let row = row.as_array().unwrap();
        assert_eq!(recommend::heard(row[0].as_f64().unwrap(), row[1].as_f64().unwrap()), row[2].as_f64().unwrap(), "{row:?}");
    }
    for row in arr(&data, "weights") {
        let row = row.as_array().unwrap();
        close(recommend::weight(row[0].as_f64().unwrap()), row[1].as_f64().unwrap(), "weight");
    }
    for row in arr(&data, "scores") {
        let row = row.as_array().unwrap();
        let got = recommend::score_of(row[0].as_bool().unwrap(), row[1].as_bool().unwrap(), row[2].as_f64().unwrap(), row[3].as_f64().unwrap());
        close(got, row[4].as_f64().unwrap(), "score");
    }
    // a song nobody played or liked scores exactly 0
    assert_eq!(recommend::score_of(false, false, recommend::PRIOR_R, 0.0), 0.0);
    let day = recommend::day_of(1795600000.0);
    assert!(day.len() == 10 && day.starts_with("2026-11-2"), "{day}");
}
