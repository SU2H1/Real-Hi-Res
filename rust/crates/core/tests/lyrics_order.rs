//! Which lyrics a song shows (Python `test_synced_lyrics_win_wherever_they_are`): synced
//! lyrics win wherever they are (the .lrc, then the tag when synced, then the .txt, then the
//! plain tag), and LRC kept in a .txt or in the plain-lyrics tag counts as synced.  The
//! user's LRCGET lyrics showed without timing because a plain .txt beat the synced tag.

use std::process::Command;

use realhires::lyrics::{read_lyrics_file, shown_label, Status};
use realhires::{embed, probe};

#[test]
fn synced_lyrics_win_wherever_they_are() {
    let have = Command::new(probe::tool("ffmpeg")).arg("-version").output().is_ok_and(|o| o.status.success());
    if !have {
        eprintln!("ffmpeg not installed; skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("realhires-lyrics-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let song = dir.join("Song.flac").to_string_lossy().into_owned();
    let txt = dir.join("Song.txt");
    let lrc_file = dir.join("Song.lrc");
    let st = Command::new(probe::tool("ffmpeg"))
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=d=1", &song])
        .status()
        .unwrap();
    assert!(st.success());
    // what Now Playing writes above them
    let label = || {
        let (kind, _, file) = read_lyrics_file(&song).unwrap();
        let (l, s) = shown_label(kind, &song, &file);
        (l.to_string(), s)
    };
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    let lrc = "[ar: Someone]\n[00:00.50] First line\n[00:01.20] Second line\n[00:02.00] Third\n";
    assert!(read_lyrics_file(&song).is_none());
    std::fs::write(&txt, "First line\nSecond line\nThird\n").unwrap();
    assert_eq!(read_lyrics_file(&song).unwrap().0, Status::Plain);
    assert_eq!(label(), pair("Lyrics (not synced)", ".txt file"));
    embed::embed_lyrics(&song, lrc, "First line\nSecond line\nThird").unwrap();
    let (kind, text, file) = read_lyrics_file(&song).unwrap();
    assert_eq!((kind, file.as_str()), (Status::Synced, song.as_str()));
    assert_eq!(embed::lrc_entries(&text).len(), 3);
    assert_eq!(label(), pair("Synced lyrics", "inside the file"));
    std::fs::write(&lrc_file, "[00:00.10] From the file\n").unwrap();
    assert_eq!(read_lyrics_file(&song).unwrap().1, "[00:00.10] From the file\n"); // the .lrc still comes first
    assert_eq!(label(), pair("Synced lyrics", ".lrc file"));
    std::fs::remove_file(&lrc_file).unwrap();
    embed::embed_lyrics(&song, "", "First line\nSecond line\nThird").unwrap(); // plain inside, plain .txt
    let (kind, text, _) = read_lyrics_file(&song).unwrap();
    assert_eq!((kind, text.as_str()), (Status::Plain, "First line\nSecond line\nThird\n"));
    std::fs::write(&txt, lrc).unwrap(); // LRC saved as .txt
    assert_eq!(read_lyrics_file(&song).unwrap().0, Status::Synced);
    assert_eq!(label(), pair("Synced lyrics", ".txt file"));
    std::fs::remove_file(&txt).unwrap();
    embed::embed_lyrics(&song, "", lrc).unwrap(); // LRC in the plain tag (UNSYNCEDLYRICS)
    assert_eq!(read_lyrics_file(&song).unwrap().0, Status::Synced);
    assert!(embed::looks_synced(lrc) && !embed::looks_synced("Words\n[00:01] one\nmore\nwords"));
    assert!(!embed::looks_synced("") && !embed::looks_synced("[ar: x]\n[ti: y]"));
    let upper = dir.join("Song.LRC").to_string_lossy().into_owned();
    assert_eq!(shown_label(Status::Instrumental, &song, &upper), ("Instrumental", ".lrc file".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}
