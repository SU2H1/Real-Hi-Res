//! The 2.5 jacket work against Python's answers (`rust/tools/gen_jacket_vectors.py` →
//! `python_jacket.json`): the turn's timing and geometry and the measures of the back
//! (`jacket`), the back of the jacket (`covers::back_picture()`, the same pixels, from
//! pictures made here with the same formula) and the cover search (`coverart`: the
//! archive's addresses, what MusicBrainz is asked, the outcome of every answer).
//! The last test downloads from a small server on 127.0.0.1: redirects are followed,
//! the size limit holds and one download runs at a time.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use realhires::coverart::{self, CoverArtClient, CoverArtError};
use realhires::json::{self, Value};
use realhires::lyrics::TransportError;
use realhires::musicbrainz::MusicBrainzClient;
use realhires::{covers, jacket};

fn load() -> Value {
    let path = format!("{}/tests/python_jacket.json", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_array().unwrap()
}

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_f64().unwrap()
}

fn int(v: &Value, key: &str) -> i64 {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_i64().unwrap()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_str().unwrap()
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).unwrap_or_else(|| panic!("no {key}")).as_bool().unwrap()
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0), "{what}: {a} != {b}");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The generator's `picture(w, h, seed)`: (h, w, 3) bytes.
fn picture(w: usize, h: usize, seed: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            for c in 0..3 {
                out.push(((x * 7 + y * 13 + c * 29 + ((x * y + seed) % 17) * 5 + seed * 3) % 256) as u8);
            }
        }
    }
    out
}

#[test]
fn the_turn_and_the_back_measure_as_in_python() {
    let all = load();
    let v = all.get("jacket").unwrap();
    let c = v.get("constants").unwrap();
    assert_eq!(int(c, "FLIP_MS"), jacket::FLIP_MS);
    assert_eq!(int(c, "FADE_MS"), jacket::FADE_MS);
    assert_eq!(int(c, "SWIPE_PX"), jacket::SWIPE_PX);
    assert_eq!(int(c, "SCROLL_PX"), jacket::SCROLL_PX);
    close(num(c, "DARKEN"), jacket::DARKEN, "DARKEN");
    close(num(c, "PERSPECTIVE"), jacket::PERSPECTIVE, "PERSPECTIVE");
    close(num(c, "HOVER_TINT"), jacket::HOVER_TINT, "HOVER_TINT");
    close(num(c, "SECONDARY"), jacket::SECONDARY, "SECONDARY");
    assert_eq!(text(c, "NO_ANIMATION_ENV"), jacket::NO_ANIMATION_ENV);
    let sides: Vec<&str> = arr(c, "SIDES").iter().map(|s| s.as_str().unwrap()).collect();
    assert_eq!(sides, jacket::SIDES);
    for e in arr(v, "ease") {
        close(jacket::ease(num(e, "t")), num(e, "out"), "ease");
    }
    for f in arr(v, "frames") {
        let got = jacket::turn_frame(num(f, "p"), int(f, "dir") as i32);
        let what = format!("frame {} {}", num(f, "p"), int(f, "dir"));
        assert_eq!(got.other, flag(f, "other"), "{what}");
        assert_eq!(got.far_right, flag(f, "far_right"), "{what}");
        close(got.width, num(f, "width"), &what);
        close(got.far, num(f, "far"), &what);
        close(got.dark, num(f, "dark"), &what);
    }
    for d in arr(v, "drags") {
        let (dx, w) = (num(d, "dx"), num(d, "width"));
        close(jacket::drag_progress(dx, w), num(d, "progress"), "drag");
        assert_eq!(jacket::drag_finishes(dx, w), flag(d, "finishes"), "drag {dx} of {w}");
    }
    for r in arr(v, "rests") {
        assert_eq!(jacket::rest_ms(num(r, "p"), flag(r, "finish"), int(r, "whole")), int(r, "ms"), "{r:?}");
    }
    for m in arr(v, "moves") {
        close(jacket::progress_at(num(m, "start"), num(m, "target"), num(m, "elapsed"), num(m, "total")), num(m, "p"), "move");
    }
    for l in arr(v, "layouts") {
        let got = jacket::back_layout(int(l, "size"));
        let want = jacket::BackLayout {
            size: int(l, "size"),
            pad: int(l, "pad"),
            title_px: int(l, "title_px"),
            text_px: int(l, "text_px"),
            small_px: int(l, "small_px"),
            title_y: int(l, "title_y"),
            artist_y: int(l, "artist_y"),
            list_top: int(l, "list_top"),
            row_h: int(l, "row_h"),
            rows: int(l, "rows"),
            num_right: int(l, "num_right"),
            title_x: int(l, "title_x"),
            time_right: int(l, "time_right"),
            bar_w: int(l, "bar_w"),
            bar_right: int(l, "bar_right"),
        };
        assert_eq!(got, want);
        // the rows end inside the square
        assert!(got.size < 60 || got.list_top + got.rows * got.row_h <= got.size, "{got:?}");
    }
    for s in arr(v, "scrolls") {
        let (first, total, rows) = (int(s, "first"), int(s, "total"), int(s, "rows"));
        assert_eq!(jacket::clamp_first(first, total, rows), int(s, "clamp"), "{s:?}");
        assert_eq!(jacket::first_for(int(s, "current"), first, total, rows), int(s, "first_for"), "{s:?}");
        let bar = jacket::scroll_bar(first, total, rows, int(s, "length"));
        let want: Vec<i64> = arr(s, "bar").iter().map(|x| x.as_i64().unwrap()).collect();
        assert_eq!(vec![bar.0, bar.1], want, "{s:?}");
    }
    let rgb = |m: &Value, k: &str| -> [u8; 3] {
        let a = arr(m, k);
        [a[0].as_i64().unwrap() as u8, a[1].as_i64().unwrap() as u8, a[2].as_i64().unwrap() as u8]
    };
    for m in arr(v, "mixes") {
        assert_eq!(jacket::mix(rgb(m, "a"), rgb(m, "b"), num(m, "amount")), rgb(m, "out"), "{m:?}");
    }
    let other = v.get("other_side").unwrap();
    for side in ["cover", "songs", ""] {
        assert_eq!(jacket::other_side(side), text(other, side));
    }
}

#[test]
fn the_back_of_the_jacket_has_pythons_pixels() {
    let all = load();
    let v = all.get("covers").unwrap();
    let c = v.get("constants").unwrap();
    assert_eq!(int(c, "BACK_SMALL") as usize, covers::BACK_SMALL);
    assert_eq!(int(c, "BACK_BLUR_RADIUS") as usize, covers::BACK_BLUR_RADIUS);
    assert_eq!(int(c, "BACK_BLUR_PASSES") as usize, covers::BACK_BLUR_PASSES);
    close(num(c, "BACK_SCRIM"), covers::BACK_SCRIM, "BACK_SCRIM");
    assert_eq!(hex(&picture(4, 3, 1)), text(v, "picture_check"), "the test picture");
    for b in arr(v, "backs") {
        let (w, h, seed) = (int(b, "w") as usize, int(b, "h") as usize, int(b, "seed") as usize);
        let size = int(b, "size") as usize;
        let out = covers::back_picture(&picture(w, h, seed), w, h, size, num(b, "scrim"));
        let side = int(b, "out_size") as usize;
        assert_eq!(out.len(), side * side * 3, "{w}x{h} to {size}");
        assert_eq!(hex(&out), text(b, "hex"), "{w}x{h} to {size}");
    }
    for d in arr(v, "darken") {
        assert_eq!(hex(&covers::darken(&picture(6, 4, 9), num(d, "amount"))), text(d, "hex"));
    }
    assert!(covers::back_picture(&[], 0, 0, 16, 0.0).is_empty());
}

#[test]
fn the_back_is_the_cover_mirrored_and_blurred() {
    // red on the left, blue on the right, with a fine black and white pattern on top
    let (w, h) = (128usize, 128usize);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) * 3;
            let fine = if (x + y) % 2 == 0 { 40 } else { 0 };
            rgb[at] = if x < w / 2 { 200 } else { 0 } + fine;
            rgb[at + 2] = if x < w / 2 { 0 } else { 200 } + fine;
        }
    }
    let out = covers::back_picture(&rgb, w, h, 0, 0.0);
    assert_eq!(out.len(), w * h * 3, "the same size");
    let px = |x: usize, y: usize| -> [u8; 3] { [out[(y * w + x) * 3], out[(y * w + x) * 3 + 1], out[(y * w + x) * 3 + 2]] };
    // mirrored: the red is on the right now
    assert!(px(110, 64)[0] > 150 && px(110, 64)[2] < 60, "{:?}", px(110, 64));
    assert!(px(16, 64)[2] > 150 && px(16, 64)[0] < 60, "{:?}", px(16, 64));
    // blurred: the fine pattern is gone, and the edge between the halves is many pixels wide
    let step = |a: [u8; 3], b: [u8; 3]| (a[0] as i32 - b[0] as i32).abs();
    assert!((0..w - 1).all(|x| step(px(x, 64), px(x + 1, 64)) <= 12), "no sharp edge is left");
    assert!(step(px(20, 30), px(21, 30)) <= 2 && step(px(20, 30), px(20, 31)) <= 2);
    let wide = (0..w).filter(|&x| (40..160).contains(&px(x, 64)[0])).count();
    assert!(wide >= 12, "the edge is {wide} pixels wide");
    // under the scrim it is half as bright
    let dark = covers::back_picture(&rgb, w, h, 0, covers::BACK_SCRIM);
    assert!(dark.iter().zip(&out).all(|(d, o)| *d == o / 2));
}

#[test]
fn the_cover_search_asks_and_answers_as_in_python() {
    let all = load();
    let v = all.get("coverart").unwrap();
    let c = v.get("constants").unwrap();
    assert_eq!(text(c, "DEFAULT_SERVER"), coverart::DEFAULT_SERVER);
    assert_eq!(text(c, "SERVER_ENV"), coverart::SERVER_ENV);
    assert_eq!(int(c, "MAX_BYTES") as usize, coverart::MAX_BYTES);
    assert_eq!(int(c, "MAX_RELEASES") as usize, coverart::MAX_RELEASES);
    assert_eq!(int(c, "THUMB_PX") as u32, coverart::THUMB_PX);
    close(num(c, "TIMEOUT_S"), coverart::TIMEOUT_S, "TIMEOUT_S");
    assert_eq!(text(c, "CREDIT"), coverart::CREDIT);
    assert_eq!(text(c, "ABOUT_CREDIT"), coverart::ABOUT_CREDIT);
    for s in arr(v, "servers") {
        assert_eq!(coverart::normalize_server(text(s, "in")), text(s, "out"));
    }
    for u in arr(v, "urls") {
        assert_eq!(coverart::front_url(text(u, "mbid"), flag(u, "thumb"), Some(text(u, "base"))), text(u, "url"));
    }
    for t in arr(v, "terms") {
        let i: Vec<&str> = arr(t, "in").iter().map(|x| x.as_str().unwrap()).collect();
        let o: Vec<&str> = arr(t, "out").iter().map(|x| x.as_str().unwrap()).collect();
        let got = coverart::search_terms(i[0], i[1], i[2], i[3]);
        assert_eq!(vec![got.0.as_str(), got.1.as_str()], o, "{i:?}");
    }
    for m in arr(v, "megabytes") {
        assert_eq!(coverart::megabytes(int(m, "n") as usize), text(m, "out"));
    }
    // what MusicBrainz is asked, and the releases in its order (`answer`: what
    // tests/fake_musicbrainz.py says to that request)
    for f in arr(v, "found") {
        let url = text(f, "url").to_string();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let (seen2, body) = (seen.clone(), text(f, "answer").to_string());
        let mut client = MusicBrainzClient::new("https://musicbrainz.org")
            .with_transport(Box::new(move |u, _, _| {
                seen2.lock().unwrap().push(u.to_string());
                Ok((200, vec![], body.clone().into_bytes()))
            }))
            .with_clock(|| 0.0, |_| {})
            .with_min_interval(0.0);
        let got = coverart::find_releases(&mut client, text(f, "album"), text(f, "artist")).unwrap();
        assert_eq!(seen.lock().unwrap().as_slice(), &[url.clone()]);
        let want = arr(f, "releases");
        assert_eq!(got.len(), want.len(), "{url}");
        for (g, w) in got.iter().zip(want) {
            assert_eq!((g.id.as_str(), g.title.as_str(), g.artist.as_str()), (text(w, "id"), text(w, "title"), text(w, "artist")));
            assert_eq!(coverart::release_facts(g), text(w, "facts"));
        }
    }
    // every outcome of a download, with its text
    let bodies = |kind: &str| -> Vec<u8> {
        match kind {
            "png" => b"\x89PNG\r\n\x1a\nrest".to_vec(),
            "jpeg" => b"\xff\xd8\xff\xe0rest".to_vec(),
            "text" => b"<html>busy</html>".to_vec(),
            "big" => [b"\x89PNG\r\n\x1a\nrest".as_slice(), &[0u8; 40]].concat(),
            _ => vec![],
        }
    };
    for o in arr(v, "outcomes") {
        let kind = text(o, "kind").to_string();
        let status = o.get("status").unwrap().clone();
        let body = bodies(&kind);
        let agents = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let agents2 = agents.clone();
        let mut client = CoverArtClient::new(Some(text(o, "server"))).with_max_bytes(32).with_transport(Box::new(move |url, headers, _| {
            let ua = headers.iter().find(|(k, _)| *k == "User-Agent").map(|(_, v)| v.to_string()).unwrap_or_default();
            agents2.lock().unwrap().push((url.to_string(), ua));
            match status.as_str() {
                Some("network") => Err(TransportError::Network(kind.clone())),
                Some("timeout") => Err(TransportError::Network("no answer in time".into())),
                Some("invalid") => Err(TransportError::Invalid(kind.clone())),
                _ => Ok((status.as_i64().unwrap() as u16, vec![], body.iter().take(33).cloned().collect())),
            }
        }));
        let got = client.front("rel-1", true);
        let what = format!("{} {:?} {}", text(o, "server"), o.get("status"), text(o, "kind"));
        match text(o, "result") {
            "picture" => assert_eq!(got.unwrap().unwrap().len() as i64, int(o, "bytes"), "{what}"),
            "none" => assert_eq!(got.unwrap(), None, "{what}"),
            "busy" => assert_eq!(got.unwrap_err(), CoverArtError::Busy(text(o, "message").into()), "{what}"),
            _ => assert_eq!(got.unwrap_err(), CoverArtError::Failed(text(o, "message").into()), "{what}"),
        }
        // asked once, never again, and by name
        let seen = agents.lock().unwrap();
        assert_eq!(seen.len(), 1, "{what}");
        assert_eq!(client.requests, 1);
        assert!(seen[0].0.ends_with("/release/rel-1/front-250"), "{}", seen[0].0);
        assert_eq!(seen[0].1, realhires::musicbrainz::user_agent());
    }
}

/// A small Cover Art Archive on 127.0.0.1: `/release/<id>/front[-250]` redirects (307) to
/// `/img/<id>`, which is a PNG header and `size` more bytes; it counts how many requests
/// it answers at the same time.
fn fake_archive(sizes: Vec<(&'static str, usize)>) -> (String, Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let most = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let (seen2, most2) = (seen.clone(), most.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (seen, most, active, sizes) = (seen2.clone(), most2.clone(), active.clone(), sizes.clone());
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                seen.lock().unwrap().push(format!("{path} | {}", request.lines().find(|l| l.to_lowercase().starts_with("user-agent:")).unwrap_or("")));
                let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
                let known = |id: &str| sizes.iter().find(|(k, _)| *k == id).map(|(_, n)| *n);
                let answer: Vec<u8> = match parts.as_slice() {
                    ["release", id, "front" | "front-250"] if known(id).is_some() => {
                        format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: /img/{id}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
                    }
                    ["img", id] if known(id).is_some() => {
                        std::thread::sleep(std::time::Duration::from_millis(120)); // long enough to overlap, if they could
                        let mut body = b"\x89PNG\r\n\x1a\n".to_vec();
                        body.resize(8 + known(id).unwrap(), 7);
                        let mut out = format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        out.extend(body);
                        out
                    }
                    ["release", "busy", _] => b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                let _ = stream.write_all(&answer);
                let _ = stream.flush();
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (url, seen, most)
}

#[test]
fn downloads_follow_redirects_keep_the_limit_and_run_one_at_a_time() {
    let (url, seen, most) = fake_archive(vec![("small", 100), ("large", 5000)]);
    let mut client = CoverArtClient::new(Some(&url)).with_max_bytes(1000);
    let small = client.front("small", true).unwrap().expect("a picture");
    assert_eq!(small.len(), 108);
    assert!(small.starts_with(b"\x89PNG"));
    assert_eq!(client.front("none", true).unwrap(), None, "no cover: not an error");
    match client.front("large", false) {
        Err(CoverArtError::Failed(m)) => assert!(m.starts_with("The picture is larger than"), "{m}"),
        other => panic!("a picture over the limit was taken: {other:?}"),
    }
    assert!(matches!(client.front("busy", true), Err(CoverArtError::Busy(_))));
    assert_eq!(client.requests, 4, "each asked once");
    {
        let seen = seen.lock().unwrap();
        let paths: Vec<&str> = seen.iter().map(|s| s.split(" | ").next().unwrap()).collect();
        assert_eq!(
            paths,
            ["/release/small/front-250", "/img/small", "/release/none/front-250", "/release/large/front", "/img/large", "/release/busy/front-250"]
        );
        let ua = realhires::musicbrainz::user_agent();
        assert!(seen.iter().all(|s| s.to_lowercase().ends_with(&format!("user-agent: {}", ua.to_lowercase()))), "{seen:?}");
    }
    // two clients in two threads: still one download at a time
    most.store(0, Ordering::SeqCst);
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let url = url.clone();
            std::thread::spawn(move || {
                let mut c = CoverArtClient::new(Some(&url));
                for _ in 0..2 {
                    assert!(c.front("small", true).unwrap().is_some());
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(most.load(Ordering::SeqCst), 1, "downloads overlapped");
    // a stopped client asks for nothing
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let before = seen.lock().unwrap().len();
    assert_eq!(CoverArtClient::new(Some(&url)).with_stop(stop).front("small", true), Err(CoverArtError::Stopped));
    assert_eq!(seen.lock().unwrap().len(), before);
}
