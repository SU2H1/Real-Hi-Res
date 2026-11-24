//! The tag editor and the themes folder against Python's answers
//! (`rust/tools/gen_player_vectors.py` writes `python_tagedit.json`): the
//! bytes of edited FLAC and MP3 files, the checks of what was typed, the
//! FFmpeg arguments for other formats, the notes and summaries after saving,
//! and the themes read from a themes folder.

use realhires::json::{self, Value};
use realhires::tagedit::{self, Changes, TagOutcome};
use realhires::themes;

fn load() -> Value {
    let path = format!("{}/tests/python_tagedit.json", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn pairs(v: &Value) -> Changes {
    match v {
        Value::Object(o) => o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect(),
        Value::Array(a) => a
            .iter()
            .map(|p| match p {
                Value::Array(kv) => (kv[0].as_str().unwrap().to_string(), kv[1].as_str().unwrap().to_string()),
                _ => panic!("pair"),
            })
            .collect(),
        _ => panic!("changes"),
    }
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn list(v: &Value) -> &Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => panic!("not a list"),
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap().to_string()
}

#[test]
fn flac_and_mp3_files_get_the_same_bytes_as_from_python() {
    let d = load();
    for case in list(d.get("cases").unwrap()) {
        let input = hex(case.get("input").unwrap().as_str().unwrap());
        let what = format!("{} {} {:?}", text(case.get("kind").unwrap()), text(case.get("name").unwrap()), case.get("changes"));
        let changes = tagedit::check_changes(&pairs(case.get("changes").unwrap())).unwrap();
        let got = if text(case.get("kind").unwrap()) == "flac" {
            tagedit::flac_with_tags(&input, &changes)
        } else {
            tagedit::mp3_with_tags(&input, &changes)
        };
        match (case.get("output"), got) {
            (Some(out), Ok(bytes)) => assert_eq!(bytes, hex(out.as_str().unwrap()), "{what}"),
            (None, Err(e)) => assert_eq!(e.0, text(case.get("error").unwrap()), "{what}"),
            (o, g) => panic!("{what}: Python {o:?}, Rust {g:?}"),
        }
    }
}

#[test]
fn typed_values_are_checked_as_in_python() {
    let d = load();
    for case in list(d.get("checks").unwrap()) {
        let got = tagedit::check_changes(&pairs(case.get("changes").unwrap()));
        match (case.get("ok"), got) {
            (Some(ok), Ok(c)) => assert_eq!(c, pairs(ok)),
            (None, Err(e)) => assert_eq!(e.0, text(case.get("error").unwrap())),
            (o, g) => panic!("{:?}: Python {o:?}, Rust {g:?}", case.get("changes")),
        }
    }
    for (path, why) in match d.get("cannot").unwrap() {
        Value::Object(o) => o.clone(),
        _ => panic!(),
    } {
        assert_eq!(tagedit::cannot_edit(&path), text(&why), "{path}");
    }
}

#[test]
fn ffmpeg_gets_the_same_arguments() {
    let d = load();
    for case in list(d.get("commands").unwrap()) {
        let changes = tagedit::check_changes(&pairs(case.get("changes").unwrap())).unwrap();
        let args = tagedit::ffmpeg_args(&text(case.get("src").unwrap()), &text(case.get("dst").unwrap()), &changes);
        let want: Vec<String> = list(case.get("args").unwrap()).iter().map(text).collect();
        assert_eq!(args, want);
    }
}

#[test]
fn notes_common_values_and_summaries_read_the_same() {
    let d = load();
    for case in list(d.get("notes").unwrap()) {
        let tags = pairs(case.get("tags").unwrap());
        let note = tagedit::kept_note("x.wav", &pairs(case.get("changes").unwrap()), &tags);
        assert_eq!(note, text(case.get("note").unwrap()));
    }
    for case in list(d.get("commons").unwrap()) {
        let tags: Vec<_> = list(case.get("tags").unwrap()).iter().map(pairs).collect();
        let got = tagedit::common_values(&tags);
        for (field, value) in got {
            let want = case.get("values").unwrap().get(&field).unwrap();
            match (want, value) {
                (Value::Null, None) => {}
                (w, Some(v)) => assert_eq!(text(w), v, "{field}"),
                (w, v) => panic!("{field}: Python {w:?}, Rust {v:?}"),
            }
        }
    }
    for case in list(d.get("summaries").unwrap()) {
        let outcomes: Vec<TagOutcome> = list(case.get("outcomes").unwrap())
            .iter()
            .map(|o| {
                let o = list(o);
                TagOutcome { path: text(&o[0]), ok: o[1].truthy(), message: text(&o[2]), tags: vec![] }
            })
            .collect();
        let (head, lines) = tagedit::save_summary(&outcomes);
        let want = list(case.get("summary").unwrap());
        assert_eq!(head, text(&want[0]));
        assert_eq!(lines, list(&want[1]).iter().map(text).collect::<Vec<_>>());
    }
}

#[test]
fn the_themes_folder_is_read_as_in_python() {
    let d = load();
    let f = d.get("folder").unwrap();
    let own: Vec<themes::Theme> = list(f.get("own").unwrap()).iter().filter_map(themes::normalize).collect();
    let dir = std::env::temp_dir().join(format!("realhires-themes-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if let Value::Object(files) = f.get("files").unwrap() {
        for (name, body) in files {
            std::fs::write(dir.join(name), body.as_str().unwrap()).unwrap();
        }
    }
    let got = themes::load_folder(&own, Some(&dir));
    let want = list(f.get("loaded").unwrap());
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert_eq!(Some(g.theme.clone()), themes::normalize(w.get("theme").unwrap()));
        assert_eq!(g.file.file_name().unwrap().to_string_lossy(), text(w.get("file").unwrap()));
    }
    assert_eq!(themes::folder(Some(&dir.join("themes.json"))), dir.join("themes"));
    let _ = std::fs::remove_dir_all(&dir);
}
