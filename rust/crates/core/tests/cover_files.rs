//! Changing and removing covers on real files made with FFmpeg (skipped without
//! it): FLAC, MP3, Ogg Vorbis and Opus natively, M4A and AIFF through FFmpeg's
//! stream copy.  The audio stays sample for sample the same, the new picture is
//! the one FFmpeg shows, byte for byte, a second change replaces it, removing it
//! leaves no picture, the saved result follows, and `cover_source()` says where
//! a cover comes from (inside a song, or the folder image).

use std::process::Command;

use realhires::settings::Settings;
use realhires::{covers, probe, store, tagedit, verdict};

fn ffmpeg() -> Command {
    Command::new(probe::tool("ffmpeg"))
}

fn have_ffmpeg() -> bool {
    ffmpeg().arg("-version").output().is_ok_and(|o| o.status.success())
}

fn pcm(path: &str) -> Vec<u8> {
    ffmpeg().args(["-v", "error", "-i", path, "-map", "0:a:0", "-f", "s32le", "-"]).output().unwrap().stdout
}

fn picture(path: &std::path::Path, source: &str) -> Vec<u8> {
    let p = path.to_string_lossy().into_owned();
    assert!(ffmpeg().args(["-v", "error", "-y", "-f", "lavfi", "-i", source, "-frames:v", "1", &p]).status().unwrap().success());
    std::fs::read(path).unwrap()
}

#[test]
fn covers_change_without_touching_the_audio() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("realhires-covers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("REALHIRES_RESULTS", dir.join("results.sqlite"));
    std::env::set_var("REALHIRES_LIBRARY", dir.join("library.sqlite"));
    let cfg = Settings::default();
    let skey = store::settings_key(&cfg);
    let jpeg = picture(&dir.join("new.jpg"), "testsrc=s=300x200:d=1");
    let png = picture(&dir.join("new.png"), "color=c=green:s=40x30:d=1");
    let encoders = ffmpeg().args(["-hide_banner", "-encoders"]).output().map(|o| o.stdout).unwrap_or_default();
    let encoders = String::from_utf8_lossy(&encoders).to_string();
    let mut formats: Vec<(&str, Vec<&str>)> = vec![
        ("a.flac", vec!["-c:a", "flac"]),
        ("b.m4a", vec!["-c:a", "aac"]),
        ("c.aiff", vec!["-c:a", "pcm_s16be", "-write_id3v2", "1"]),
    ];
    for (name, codec) in [("d.mp3", "libmp3lame"), ("e.ogg", "libvorbis"), ("f.opus", "libopus")] {
        if encoders.contains(codec) {
            formats.push((name, vec!["-c:a", codec]));
        }
    }
    for (name, args) in &formats {
        let album = dir.join(name.replace('.', "_"));
        std::fs::create_dir_all(&album).unwrap();
        let song = album.join(name).to_string_lossy().into_owned();
        let ok = ffmpeg()
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ac", "2"])
            .args(["-metadata", "title=Old", "-metadata", "artist=Art"])
            .args(args)
            .arg(&song)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut st = store::open_store(&cfg).unwrap();
        st.put(&verdict::analyze_file(&song, None, false, false, Some(&cfg)), &skey, None, None);
        let before = pcm(&song);
        let songs = [song.clone()];
        let out = tagedit::set_cover(&songs, &jpeg, "image/jpeg", Some(&cfg), |_, _, _| {});
        assert!(out[0].ok && out[0].message.is_empty(), "{name}: {}", out[0].message);
        assert_eq!(pcm(&song), before, "{name}: the audio changed");
        let src = covers::cover_source(&songs).expect("no cover after setting one");
        assert_eq!((src.kind, src.format, src.width, src.height), ("embedded", "JPEG", 300, 200), "{name}");
        assert!(src.data == jpeg, "{name}: the picture is not stored as it was given");
        assert_eq!(src.where_text(), format!("Inside {name}"));
        assert!(st.lookup(&song, &skey, None).is_some(), "{name}: the saved result was dropped");
        let out = tagedit::set_cover(&songs, &png, "", Some(&cfg), |_, _, _| {});
        assert!(out[0].ok, "{name}: {}", out[0].message);
        assert_eq!(tagedit::picture_streams(&song).unwrap(), vec![(1, true)], "{name}: one cover, after the audio");
        assert!(covers::cover_source(&songs).unwrap().data == png);
        let out = tagedit::remove_cover(&songs, Some(&cfg), |_, _, _| {});
        assert!(out[0].ok && out[0].message.is_empty(), "{name}: {}", out[0].message);
        assert!(covers::cover_source(&songs).is_none(), "{name}: a cover is left");
        assert_eq!(pcm(&song), before, "{name}: the audio changed");
        assert_eq!(probe::tag(&probe::probe(&song).unwrap().tags, "title"), Some("Old"));
        let out = tagedit::remove_cover(&songs, Some(&cfg), |_, _, _| {});
        assert_eq!(out[0].message, format!("{name} had no cover inside."));
        // the folder image is shown then, and "Also save it as cover.jpg" writes one
        let saved = covers::save_folder_cover(&album.to_string_lossy(), &jpeg).unwrap();
        assert!(saved.ends_with("cover.jpg"));
        let src = covers::cover_source(&songs).unwrap();
        assert_eq!((src.kind, src.where_text()), ("folder", "cover.jpg in the album's folder".to_string()));
        assert!(st.lookup(&song, &skey, None).is_some());
    }
    let wav = dir.join("w.wav").to_string_lossy().into_owned();
    assert!(ffmpeg().args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=d=1", &wav]).status().unwrap().success());
    let bytes = std::fs::read(&wav).unwrap();
    let out = tagedit::set_cover(&[wav.clone()], &jpeg, "", None, |_, _, _| {});
    assert!(!out[0].ok);
    assert_eq!(out[0].message, "WAV files have no place for a cover picture, so they were left as they are.");
    assert_eq!(std::fs::read(&wav).unwrap(), bytes);
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".realhires"))
        .collect();
    assert!(leftovers.is_empty(), "a temporary file was left");
    let _ = std::fs::remove_dir_all(&dir);
}
