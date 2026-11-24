//! The tag editor on real files made with FFmpeg (skipped without it): FLAC
//! natively and M4A through FFmpeg's stream copy keep their audio sample for
//! sample, drop an emptied field, keep the fields that were not changed, and
//! the saved result follows the new tags.

use std::process::Command;

use realhires::settings::Settings;
use realhires::{probe, store, tagedit, verdict};

fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok_and(|o| o.status.success())
}

fn pcm(path: &str) -> Vec<u8> {
    Command::new("ffmpeg").args(["-v", "error", "-i", path, "-map", "0:a:0", "-f", "s32le", "-"]).output().unwrap().stdout
}

#[test]
fn flac_and_m4a_keep_their_audio_and_saved_results() {
    if !have_ffmpeg() {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("realhires-tagedit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("REALHIRES_RESULTS", dir.join("results.sqlite"));
    std::env::set_var("REALHIRES_LIBRARY", dir.join("library.sqlite"));
    let cfg = Settings::default();
    let skey = store::settings_key(&cfg);
    for (name, codec) in [("a.flac", "flac"), ("b.m4a", "aac")] {
        let song = dir.join(name).to_string_lossy().into_owned();
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-ac", "2"])
            .args(["-metadata", "title=Old", "-metadata", "artist=Art", "-metadata", "album_artist=AA"])
            .args(["-c:a", codec, &song])
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let mut st = store::open_store(&cfg).unwrap();
        st.put(&verdict::analyze_file(&song, None, false, false, Some(&cfg)), &skey, None, None);
        let before = pcm(&song);
        let changes = vec![
            ("title".to_string(), "Neue 日本".to_string()),
            ("album_artist".to_string(), String::new()),
            ("date".to_string(), "2020-05-28".to_string()),
            ("track".to_string(), "04/12".to_string()),
        ];
        let out = tagedit::save_tags(&[song.clone()], &changes, Some(&cfg), |_, _, _| {});
        assert!(out[0].ok, "{}", out[0].message);
        assert_eq!(pcm(&song), before, "{name}: the audio changed");
        let tags = probe::probe(&song).unwrap().tags;
        assert_eq!(probe::tag(&tags, "title"), Some("Neue 日本"));
        assert_eq!(probe::tag(&tags, "artist"), Some("Art"));
        assert_eq!(probe::tag(&tags, "date"), Some("2020-05-28"));
        assert_eq!(probe::tag(&tags, "album_artist"), None);
        let again = st.lookup(&song, &skey, None).expect("the saved result was dropped");
        assert_eq!(probe::tag(&again.tags, "title"), Some("Neue 日本"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
