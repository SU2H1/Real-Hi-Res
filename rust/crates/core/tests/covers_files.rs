//! Covers kept on disk against Python's answers (`rust/tools/gen_player_vectors.py`
//! writes `python_covers.json`): the key and file name of a cover, for made-up
//! paths and for real files with fixed sizes and times, so both apps find each
//! other's covers; and the cache itself on real files made with FFmpeg (skipped
//! without it): made once, then read from disk, the "no cover" mark, a changed
//! song, a damaged file.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

use realhires::covers;
use realhires::json::{self, Value};
use realhires::probe;

fn load() -> Value {
    let path = format!("{}/tests/python_covers.json", env!("CARGO_MANIFEST_DIR"));
    json::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn at<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or_else(|| panic!("no {key}"))
}

fn list(v: &Value) -> &Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => panic!("not a list"),
    }
}

fn strings(v: &Value) -> Vec<String> {
    list(v).iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        Value::Float(f) => *f as i64,
        _ => panic!("not a number: {v:?}"),
    }
}

/// Set a file's or a folder's modification time (a folder needs backup
/// semantics and the right to write its attributes on Windows).
fn set_mtime(path: &Path, ns: i64) {
    let t = SystemTime::UNIX_EPOCH + Duration::from_nanos(ns as u64);
    #[cfg(windows)]
    let f = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().access_mode(0x100).custom_flags(0x0200_0000).open(path).unwrap()
    };
    #[cfg(not(windows))]
    let f = std::fs::File::open(path).unwrap();
    f.set_modified(t).unwrap();
}

#[test]
fn names_of_made_up_paths_match_python() {
    let v = load();
    assert_eq!(int(at(&v, "version")), covers::CACHE_VERSION as i64);
    assert_eq!(int(at(&v, "paths")), covers::CACHE_PATHS as i64);
    for case in list(at(&v, "names")) {
        let paths = strings(at(case, "paths"));
        let size = int(at(case, "size")) as u32;
        assert_eq!(covers::cache_key(&paths, size), at(case, "key").as_str().unwrap(), "{paths:?}");
        assert_eq!(covers::cache_name(&paths, size), at(case, "name").as_str().unwrap(), "{paths:?}");
    }
}

#[test]
fn keys_of_real_files_match_python() {
    let v = load();
    let root = std::env::temp_dir().join(format!("rhr-cover-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let root_s = root.to_string_lossy().into_owned();
    for f in list(at(at(&v, "layout"), "files")) {
        let p = format!("{root_s}/{}", at(f, "path").as_str().unwrap());
        std::fs::create_dir_all(Path::new(&p).parent().unwrap()).unwrap();
        std::fs::write(&p, vec![0u8; int(at(f, "bytes")) as usize]).unwrap();
        set_mtime(Path::new(&p), int(at(f, "mtime_ns")));
    }
    for f in list(at(at(&v, "layout"), "folders")) {
        set_mtime(Path::new(&format!("{root_s}/{}", at(f, "path").as_str().unwrap())), int(at(f, "mtime_ns")));
    }
    for case in list(at(&v, "files")) {
        let paths: Vec<String> = strings(at(case, "paths")).iter().map(|r| format!("{root_s}/{r}")).collect();
        let size = int(at(case, "size")) as u32;
        let want = at(case, "key").as_str().unwrap().replace("{root}", &root_s);
        assert_eq!(covers::cache_key(&paths, size), want);
    }
    let _ = std::fs::remove_dir_all(&root);
}

fn ffmpeg_ok(args: &[&str]) -> bool {
    Command::new(probe::tool("ffmpeg")).args(["-v", "error", "-y"]).args(args).status().is_ok_and(|s| s.success())
}

#[test]
fn covers_are_made_once_and_read_from_disk() {
    if !ffmpeg_ok(&["-f", "lavfi", "-i", "anullsrc", "-t", "0.1", "-f", "null", "-"]) {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("rhr-cover-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cache = dir.join("covers");
    std::env::set_var(covers::COVERS_ENV, &cache); // the only test here that reads the variable
    assert_eq!(covers::cache_dir(), cache);
    let album = dir.join("Album");
    let plain = dir.join("Plain");
    std::fs::create_dir_all(&album).unwrap();
    std::fs::create_dir_all(&plain).unwrap();
    let song = |d: &Path, n: &str| {
        let p = d.join(n).to_string_lossy().into_owned();
        assert!(ffmpeg_ok(&["-f", "lavfi", "-i", "sine=duration=0.2", &p]));
        p
    };
    let paths = vec![song(&album, "01.wav"), song(&album, "02.wav")];
    let bare = vec![song(&plain, "01.wav")];
    assert!(ffmpeg_ok(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=red:s=96x80",
        "-frames:v",
        "1",
        &album.join("folder.png").to_string_lossy()
    ]));
    // keep the folder times apart from what the songs were made at
    set_mtime(&album, 1_700_000_000_000_000_000);
    set_mtime(&plain, 1_700_000_000_000_000_000);

    let png = covers::cached_cover_png(&paths, 64).expect("a cover from folder.png");
    assert!(png.starts_with(b"\x89PNG"));
    let file = covers::cache_file(&paths, 64);
    assert_eq!(std::fs::read(&file).unwrap(), png);
    // read from disk: replace the folder image, the cover stays the saved one
    std::fs::remove_file(album.join("folder.png")).unwrap();
    set_mtime(&album, 1_700_000_000_000_000_000);
    set_mtime(&file, 1_000_000_000_000_000);
    assert_eq!(covers::cached_cover_png(&paths, 64).as_deref(), Some(&png[..]));
    let touched = std::fs::metadata(&file).unwrap().modified().unwrap();
    assert!(touched > SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000), "reading touches the file");
    assert!(covers::cached_cover_png(&paths[..1], 64).is_none(), "another key: made again, no image left");

    // no cover: an empty mark, then no FFmpeg (the mark is read)
    assert!(covers::cached_cover_png(&bare, 64).is_none());
    let mark = covers::cache_file(&bare, 64);
    assert_eq!(std::fs::metadata(&mark).unwrap().len(), 0);
    assert!(covers::is_cached(&bare, 64));
    // FFmpeg that cannot run leaves no mark
    let other = vec![song(&plain, "02.wav")];
    assert!(covers::cached_cover_png_in(&cache, Path::new("/no/such/ffmpeg"), &other, 64).is_none());
    assert!(!covers::is_cached(&other, 64));

    // a changed song gives a new entry
    let before = covers::cache_name(&paths, 64);
    set_mtime(Path::new(&paths[1]), 1_600_000_000_000_000_000);
    assert_ne!(covers::cache_name(&paths, 64), before);

    // a damaged file is made again
    let bare_file = covers::cache_file(&bare, 32);
    assert!(covers::cached_cover_png(&bare, 32).is_none());
    std::fs::write(&bare_file, b"\x89PNG\r\n\x1a\nhalf").unwrap();
    assert!(covers::read_cached(&bare_file).is_none());
    assert!(covers::cached_cover_png(&bare, 32).is_none());
    assert_eq!(std::fs::metadata(&bare_file).unwrap().len(), 0);
    assert!(!std::fs::read_dir(&cache).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    let _ = std::fs::remove_dir_all(&dir);
}
