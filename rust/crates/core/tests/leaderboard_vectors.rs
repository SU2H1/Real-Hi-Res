//! 2.4: Your Stats' leaderboard ("Most Played") against Python's answers
//! (`rust/tools/gen_player_vectors.py` writes `python_leaderboard.json`): the ranking of
//! songs, artists and albums by plays and minutes, shared ranks, the cut at `top` rows
//! and the songs Play and Shuffle take; 2.5: the same sorted by the time listened, the
//! genres, and the spans of Most Played's period control (their days and names).

use realhires::json::{self, Value};
use realhires::stats::{self, Play};

fn load() -> Value {
    let path = format!("{}/tests/python_leaderboard.json", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Equal JSON, numbers within rounding.
fn same(a: &Value, b: &Value, at: &str) {
    match (a, b) {
        (Value::Object(x), Value::Object(_)) => {
            let mut kx: Vec<&String> = x.iter().map(|(k, _)| k).collect();
            let mut ky: Vec<&String> = b.as_object().unwrap().iter().map(|(k, _)| k).collect();
            kx.sort();
            ky.sort();
            assert_eq!(kx, ky, "keys at {at}");
            for (k, v) in x {
                same(v, b.get(k).unwrap(), &format!("{at}.{k}"));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            assert_eq!(x.len(), y.len(), "length at {at}");
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                same(p, q, &format!("{at}[{i}]"));
            }
        }
        _ if a.as_f64().is_some() && b.as_f64().is_some() => {
            let (p, q) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            assert!((p - q).abs() <= 1e-9 * p.abs().max(q.abs()).max(1.0), "{p} != {q} at {at}");
        }
        _ => assert_eq!(a, b, "at {at}"),
    }
}

fn plays_of(v: &Value) -> Vec<Play> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|p| Play {
            path: s(p, "path"),
            started: p.get("started").unwrap().as_f64().unwrap(),
            seconds: p.get("seconds").unwrap().as_f64().unwrap(),
            duration: p.get("duration").unwrap().as_f64().unwrap(),
            title: s(p, "title"),
            artist: s(p, "artist"),
            album: s(p, "album"),
            album_artist: s(p, "album_artist"),
            genre: s(p, "genre"),
            year: s(p, "year"),
            composer: s(p, "composer"),
            // 2.4: plays carry the place they were played at (none in these vectors)
            place: p.get("place").filter(|v| !v.is_null()).map(|v| realhires::places::Place::from_json(v).unwrap()),
        })
        .collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect()
}

#[test]
fn leaderboards_equal_python() {
    let data = load();
    let plays = plays_of(data.get("plays").unwrap());
    assert_eq!(strings(data.get("kinds").unwrap()), stats::LEADERBOARD_KINDS);
    let boards = data.get("boards").unwrap();
    let mut shared = 0;
    for kind in stats::LEADERBOARD_KINDS {
        for top in [stats::TOP, 5, 3, 0] {
            let rows = stats::leaderboard(&plays, kind, top);
            let got = Value::Array(rows.iter().map(|r| r.to_json()).collect());
            same(&got, boards.get(kind).unwrap().get(&top.to_string()).unwrap(), &format!("{kind} top {top}"));
            shared += rows.windows(2).filter(|w| w[0].rank == w[1].rank).count();
        }
        let rows = stats::leaderboard(&plays, kind, stats::TOP);
        let ranked = Value::Array(stats::ranked_songs(&rows).into_iter().map(Value::Str).collect());
        same(&ranked, data.get("ranked").unwrap().get(kind).unwrap(), &format!("ranked {kind}"));
    }
    assert!(shared > 0, "the vectors hold ties");
    assert!(stats::leaderboard(&[], "songs", stats::TOP).is_empty());
    assert_eq!(data.get("empty").unwrap().as_array().unwrap().len(), 0);
    assert!(stats::leaderboard(&plays, "moods", stats::TOP).is_empty(), "not a leaderboard");
    assert!(!stats::leaderboard(&plays, "genres", stats::TOP).is_empty(), "2.5: genres");
}

/// 2.5, "Sort by": the time listened orders the rows by their whole minutes, then plays.
#[test]
fn leaderboards_by_time_equal_python() {
    let data = load();
    let plays = plays_of(data.get("plays").unwrap());
    assert_eq!(strings(data.get("sorts").unwrap()), stats::LEADERBOARD_SORTS);
    let boards = data.get("boards_time").unwrap();
    for kind in stats::LEADERBOARD_KINDS {
        for top in [stats::TOP, 5, 3, 0] {
            let rows = stats::leaderboard_by(&plays, kind, top, "time");
            let got = Value::Array(rows.iter().map(|r| r.to_json()).collect());
            same(&got, boards.get(kind).unwrap().get(&top.to_string()).unwrap(), &format!("{kind} by time, top {top}"));
            assert!(rows.windows(2).all(|w| w[0].minutes >= w[1].minutes), "{kind}: the longest first");
        }
        let rows = stats::leaderboard_by(&plays, kind, stats::TOP, "time");
        let ranked = Value::Array(stats::ranked_songs(&rows).into_iter().map(Value::Str).collect());
        same(&ranked, data.get("ranked_time").unwrap().get(kind).unwrap(), &format!("ranked {kind} by time"));
        // "plays" through leaderboard_by() is leaderboard()
        assert_eq!(stats::leaderboard_by(&plays, kind, stats::TOP, "plays"), stats::leaderboard(&plays, kind, stats::TOP));
    }
    assert!(stats::leaderboard_by(&plays, "songs", stats::TOP, "length").is_empty(), "not a way to sort");
    // the plays made for this order: both sorts of every kind
    let cases = data.get("time_cases").unwrap();
    let plays = plays_of(cases.get("plays").unwrap());
    let mut differ = 0;
    for by in stats::LEADERBOARD_SORTS {
        for kind in stats::LEADERBOARD_KINDS {
            let rows = stats::leaderboard_by(&plays, kind, stats::TOP, by);
            let got = Value::Array(rows.iter().map(|r| r.to_json()).collect());
            same(&got, cases.get("boards").unwrap().get(by).unwrap().get(kind).unwrap(), &format!("{kind} by {by}"));
            differ += usize::from(rows != stats::leaderboard(&plays, kind, stats::TOP));
        }
    }
    assert!(differ >= 3, "the two sorts differ in these vectors");
}

/// 2.5: the spans of Most Played's period control.
#[test]
fn spans_equal_python() {
    let data = load();
    assert_eq!(strings(data.get("span_units").unwrap()), stats::SPAN_UNITS);
    let day = |v: &Value| {
        let d = v.as_array().unwrap();
        (d[0].as_i64().unwrap() as i32, d[1].as_i64().unwrap() as u32, d[2].as_i64().unwrap() as u32)
    };
    let spans = data.get("spans").unwrap().as_array().unwrap();
    assert!(spans.len() > 200);
    for c in spans {
        let (unit, offset, today) = (s(c, "unit"), c.get("offset").unwrap().as_i64().unwrap() as i32, day(c.get("today").unwrap()));
        let want = c.get("days").filter(|v| !v.is_null()).map(|v| {
            let d = v.as_array().unwrap();
            (day(&d[0]), day(&d[1]))
        });
        assert_eq!(stats::span_days(&unit, offset, today), want, "{unit} {offset} {today:?}");
        assert_eq!(stats::span_name(&unit, offset, today), s(c, "name"), "{unit} {offset} {today:?}");
    }
    for c in data.get("period_spans").unwrap().as_array().unwrap() {
        let c = c.as_array().unwrap();
        let (unit, offset) = stats::period_span(c[0].as_str().unwrap());
        assert_eq!((unit, offset as i64), (c[1].as_str().unwrap(), c[2].as_i64().unwrap()), "{:?}", c[0]);
    }
    // the range of a span: local midnights, a day (or a week) apart; all time is open
    let now = 1_795_600_000.0;
    let (since, until) = stats::span_range("day", 0, now);
    let until = until.unwrap();
    assert!(since <= now && now < until && (82_800.0..=90_000.0).contains(&(until - since)), "{since} {until}");
    let (wsince, wuntil) = stats::span_range("week", -1, now);
    assert!(wuntil.unwrap() <= since + 1.0 && (wuntil.unwrap() - wsince - 7.0 * 86_400.0).abs() <= 3_600.0);
    assert_eq!(stats::span_range("all", 0, now), (0.0, None));
    assert_eq!(stats::span_range("year", 0, now).0, stats::period_range("This year", now).0);
    assert_eq!(stats::span_title("all", -3, now), "All time");
}
