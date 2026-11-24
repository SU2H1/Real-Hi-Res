//! Changing and removing covers against Python's answers
//! (`rust/tools/gen_player_vectors.py` writes `python_covers_embed.json`): the
//! image headers, the bytes of FLAC, MP3 and Ogg files with a new cover or
//! without their covers, the picture types read back, the FFmpeg arguments for
//! M4A and AIFF, the formats that cannot hold a picture, and the texts.

use realhires::covers;
use realhires::json::{self, Value};
use realhires::tagedit::{self, TagOutcome};

fn load() -> Value {
    let path = format!("{}/tests/python_covers_embed.json", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn list(v: &Value) -> &Vec<Value> {
    v.as_array().expect("a list")
}

fn text(v: &Value) -> String {
    v.as_str().unwrap().to_string()
}

fn object(v: &Value) -> &Vec<(String, Value)> {
    v.as_object().expect("an object")
}

fn sha1(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    Sha1::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn image_headers_are_read_as_in_python() {
    let d = load();
    let images = d.get("images").unwrap();
    for (name, want) in object(d.get("infos").unwrap()) {
        let data = hex(images.get(name).unwrap().as_str().unwrap());
        assert_eq!(covers::image_format(&data), text(want.get("format").unwrap()), "{name}");
        let got = covers::image_info(&data);
        match (want.get("info").unwrap(), got) {
            (Value::Null, None) => {}
            (Value::Array(w), Some(i)) => {
                assert_eq!(i.mime, text(&w[0]), "{name}");
                let nums: Vec<i64> = w[1..].iter().map(|v| v.as_i64().unwrap()).collect();
                assert_eq!(nums, vec![i.width as i64, i.height as i64, i.depth as i64, i.colors as i64], "{name}");
            }
            (w, g) => panic!("{name}: Python {w:?}, Rust {g:?}"),
        }
    }
    for case in list(d.get("checks").unwrap()) {
        let data = hex(images.get(&text(case.get("image").unwrap())).unwrap().as_str().unwrap());
        let got = tagedit::check_picture(&data, &text(case.get("mime").unwrap())).err().map(|e| e.0).unwrap_or_default();
        assert_eq!(got, text(case.get("error").unwrap()), "{:?}", case.get("image"));
    }
    for (name, want) in object(d.get("pictures").unwrap()) {
        let data = hex(images.get(name).unwrap().as_str().unwrap());
        assert_eq!(tagedit::flac_picture(&data).unwrap(), hex(want.as_str().unwrap()), "{name}");
    }
    for (name, want) in object(d.get("apic_bodies").unwrap()) {
        let data = hex(images.get(name).unwrap().as_str().unwrap());
        assert_eq!(tagedit::apic_body(&data).unwrap(), hex(want.as_str().unwrap()), "{name}");
    }
}

#[test]
fn files_get_the_same_bytes_as_from_python() {
    let d = load();
    let images = d.get("images").unwrap();
    for case in list(d.get("cases").unwrap()) {
        let (kind, name, pick) = (text(case.get("kind").unwrap()), text(case.get("name").unwrap()), text(case.get("image").unwrap()));
        let input = hex(d.get(&format!("{kind}_inputs")).unwrap().get(&name).unwrap().as_str().unwrap());
        let image = (pick != "remove").then(|| hex(images.get(&pick).unwrap().as_str().unwrap()));
        let got = match kind.as_str() {
            "flac" => tagedit::flac_with_cover(&input, image.as_deref()),
            "mp3" => tagedit::mp3_with_cover(&input, image.as_deref()),
            _ => tagedit::ogg_with_cover(&input, image.as_deref()),
        };
        let what = format!("{kind} {name} {pick}");
        match (case.get("sha1"), got) {
            (Some(sha), Ok((out, removed))) => {
                if let Some(full) = case.get("output") {
                    assert_eq!(out, hex(full.as_str().unwrap()), "{what}");
                }
                assert_eq!(sha1(&out), text(sha), "{what}");
                assert_eq!(removed as i64, case.get("removed").unwrap().as_i64().unwrap(), "{what}");
            }
            (None, Err(e)) => assert_eq!(e.0, text(case.get("error").unwrap()), "{what}"),
            (s, g) => panic!("{what}: Python {s:?}, Rust {:?}", g.map(|o| o.1)),
        }
    }
}

#[test]
fn picture_types_streams_and_commands_as_in_python() {
    let d = load();
    let types = d.get("types").unwrap();
    for (comment, want) in object(types.get("mbp").unwrap()) {
        assert_eq!(tagedit::mbp_type(comment.as_bytes()), want.as_i64().unwrap(), "{comment}");
    }
    for case in list(types.get("apic").unwrap()) {
        let c = list(case);
        let got = tagedit::apic_type(c[0].as_i64().unwrap() as u8, &hex(c[1].as_str().unwrap()), &hex(c[2].as_str().unwrap()));
        assert_eq!(got, c[3].as_i64().unwrap(), "{case:?}");
    }
    for case in list(types.get("streams").unwrap()) {
        let c = list(case);
        assert_eq!(tagedit::cover_stream(c[0].truthy(), c[1].as_str().unwrap()), c[2].truthy(), "{case:?}");
    }
    for case in list(d.get("commands").unwrap()) {
        let streams: Vec<(i64, bool)> = list(case.get("streams").unwrap())
            .iter()
            .map(|s| (list(s)[0].as_i64().unwrap(), list(s)[1].truthy()))
            .collect();
        let image = case.get("image").unwrap().as_str().map(str::to_string);
        let args = tagedit::ffmpeg_cover_args(&text(case.get("src").unwrap()), image.as_deref(), "/tmp/out", &streams);
        let want: Vec<String> = list(case.get("args").unwrap()).iter().map(text).collect();
        assert_eq!(args, want, "{case:?}");
    }
    for (path, why) in object(d.get("cannot").unwrap()) {
        assert_eq!(tagedit::cannot_hold_cover(path), text(why), "{path}");
    }
}

#[test]
fn summaries_and_texts_read_the_same() {
    let d = load();
    for case in list(d.get("summaries").unwrap()) {
        let outcomes: Vec<TagOutcome> = list(case.get("outcomes").unwrap())
            .iter()
            .map(|o| {
                let o = list(o);
                TagOutcome { path: text(&o[0]), ok: o[1].truthy(), message: text(&o[2]), tags: vec![] }
            })
            .collect();
        let notes: Vec<String> = list(case.get("notes").unwrap()).iter().map(text).collect();
        let (head, lines) = tagedit::cover_summary(&outcomes, case.get("removing").unwrap().truthy(), &notes);
        let want = list(case.get("summary").unwrap());
        assert_eq!(head, text(&want[0]));
        assert_eq!(lines, list(&want[1]).iter().map(text).collect::<Vec<_>>());
    }
    let texts = d.get("texts").unwrap();
    for case in list(texts.get("where").unwrap()) {
        let c = list(case);
        assert_eq!(covers::where_text(c[0].as_str().unwrap(), c[1].as_str().unwrap()), text(&c[2]));
    }
    for case in list(texts.get("details").unwrap()) {
        let c = list(case);
        let got = covers::details_text(
            c[0].as_i64().unwrap() as u32,
            c[1].as_i64().unwrap() as u32,
            c[2].as_str().unwrap(),
            c[3].as_i64().unwrap() as usize,
        );
        assert_eq!(got, text(&c[4]));
    }
}

#[test]
fn a_removed_big_cover_does_not_stay_as_padding() {
    let mut picture = 3u32.to_be_bytes().to_vec();
    picture.extend(std::iter::repeat_n(b'x', 1_200_000));
    let blocks: Vec<(u8, Vec<u8>)> = vec![(0, vec![0; 34]), (4, vec![0; 8]), (6, picture), (1, vec![0; 10])];
    let mut data = b"fLaC".to_vec();
    for (i, (k, b)) in blocks.iter().enumerate() {
        data.push(k | if i == blocks.len() - 1 { 0x80 } else { 0 });
        data.extend_from_slice(&(b.len() as u32).to_be_bytes()[1..]);
        data.extend_from_slice(b);
    }
    data.extend_from_slice(b"AUDIO");
    let (out, n) = tagedit::flac_with_cover(&data, None).unwrap();
    assert_eq!(n, 1);
    assert_eq!(out.len(), 4 + 38 + 12 + 4 + realhires::embed::NEW_PADDING + 5);
    assert!(out.ends_with(b"AUDIO"));
    assert_eq!(tagedit::b64encode(b""), "");
    assert_eq!(tagedit::b64encode(b"f"), "Zg==");
    assert_eq!(tagedit::b64encode(b"fo"), "Zm8=");
    assert_eq!(tagedit::b64encode(b"foobar"), "Zm9vYmFy");
}
