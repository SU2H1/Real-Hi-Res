//! Album cover thumbnails for the album/artist views (FFmpeg only): the
//! picture embedded in a track, or a cover image in the album folder, scaled
//! and cropped to a square PNG.
//!
//! Covers are kept on disk (`cached_cover_png()`), one PNG per album and size
//! in `covers/` in the data folder (`cache_dir()`), shared with the Python
//! app, so a cover is made by FFmpeg only once and not again after a restart.
//! An album without a cover is remembered as an empty file.  The file name
//! (`cache_name()`, the same as Python's) changes when one of the album's
//! first four songs or one of their folders changes; `trim_cache()` keeps the
//! folder under `CACHE_LIMIT_BYTES` by deleting the least recently used files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use sha1::{Digest, Sha1};

use crate::paths::{extension, parent, stem};
use crate::probe::{command, tool};

pub const FOLDER_IMAGES: [&str; 7] = ["cover", "folder", "front", "album", "albumart", "albumartsmall", "artwork"];
pub const IMAGE_EXTS: [&str; 6] = [".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"];

/// The covers folder (tests point it at a temporary one).
pub const COVERS_ENV: &str = "REALHIRES_COVERS";
/// Part of every key: a new way of making covers gives new names.
pub const CACHE_VERSION: u32 = 1;
/// The songs of an album that a cover is made from (and keyed by).
pub const CACHE_PATHS: usize = 4;
/// `trim_cache()` keeps the folder under this.
pub const CACHE_LIMIT_BYTES: u64 = 500 * 1024 * 1024;
/// A temp file older than this was left by a crash.
pub const TEMP_MAX_AGE: Duration = Duration::from_secs(3600);
const BLOCK: u64 = 4096;
const PNG_START: &[u8] = b"\x89PNG\r\n\x1a\n";
const PNG_END: &[u8] = b"\x00\x00\x00\x00IEND\xaeB`\x82";

/// (PNG or None, whether FFmpeg ran to the end).  False means it could not be
/// started or timed out, so "no picture" is not certain.
fn thumbnail(ffmpeg: &Path, source: &str, size: u32, embedded: bool) -> (Option<Vec<u8>>, bool) {
    let vf = format!("scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}");
    let mut cmd = command(ffmpeg);
    cmd.args(["-v", "error", "-nostdin", "-i", source]);
    if embedded {
        cmd.args(["-map", "0:v:0"]);
    }
    cmd.args(["-an", "-frames:v", "1", "-vf", &vf, "-f", "image2pipe", "-c:v", "png", "-"]);
    let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        return (None, false);
    };
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return (None, false);
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        buf
    });
    let t0 = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if t0.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                let _ = child.wait();
                return (None, false);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return (None, false),
        }
    };
    let Ok(data) = reader.join() else { return (None, false) };
    ((status.success() && data.starts_with(b"\x89PNG")).then_some(data), true)
}

/// Best cover-like image file in `folder`, if any.
pub fn folder_image(folder: &str) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| IMAGE_EXTS.contains(&extension(n).to_lowercase().as_str()))
        .collect();
    names.sort();
    for s in FOLDER_IMAGES {
        for n in &names {
            let base = stem(n).to_lowercase();
            if base == s || base.starts_with(&format!("{s}_")) || base.starts_with("albumart_") {
                return Some(std::path::Path::new(folder).join(n).to_string_lossy().into_owned());
            }
        }
    }
    (names.len() == 1).then(|| std::path::Path::new(folder).join(&names[0]).to_string_lossy().into_owned())
}

/// (cover, certain): certain is false when an FFmpeg run failed to start or
/// timed out on the way to "no cover".
fn make(ffmpeg: &Path, tracks: &[String], size: u32) -> (Option<Vec<u8>>, bool) {
    let mut certain = true;
    for t in tracks.iter().take(3) {
        let (data, ok) = thumbnail(ffmpeg, t, size, true);
        certain &= ok;
        if data.is_some() {
            return (data, true);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for t in tracks {
        let folder = parent(t);
        if !seen.insert(folder.clone()) {
            continue;
        }
        if let Some(img) = folder_image(&folder) {
            let (data, ok) = thumbnail(ffmpeg, &img, size, false);
            certain &= ok;
            if data.is_some() {
                return (data, true);
            }
        }
    }
    (None, certain)
}

/// PNG bytes of a size×size cover for an album/artist, or None.
pub fn cover_png(tracks: &[String], size: u32) -> Option<Vec<u8>> {
    make(&tool("ffmpeg"), tracks, size).0
}

// ---------------------------------------------------------------- on disk ---

/// `$REALHIRES_COVERS`, else `covers` in the data folder (`store::data_dir()`).
pub fn cache_dir() -> PathBuf {
    std::env::var_os(COVERS_ENV)
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::store::data_dir().join("covers"))
}

/// (size, mtime_ns) of a file or folder, (-1, -1) when it cannot be read.
fn stat_or_none(path: &str) -> (i64, i64) {
    crate::store::stat(path).unwrap_or((-1, -1))
}

/// The text whose SHA-1 names a cover on disk, byte for byte as Python's
/// `covers.cache_key()` builds it: lines joined with "\n" (no newline at the
/// end), encoded as UTF-8:
///
/// ```text
/// v1<TAB><size>                       CACHE_VERSION and the size in pixels
/// <path><TAB><bytes><TAB><mtime_ns>   for each of the first four paths, in order
/// <folder><TAB><mtime_ns>             for each distinct folder of those paths
///                                     (os.path.dirname), in first-seen order
/// ```
///
/// Numbers are decimal integers; a file or folder that cannot be read gives
/// -1 for its size and time.
pub fn cache_key(paths: &[String], size: u32) -> String {
    let mut lines = vec![format!("v{CACHE_VERSION}\t{size}")];
    let mut folders: Vec<String> = vec![];
    for p in paths.iter().take(CACHE_PATHS) {
        let (n, mtime) = stat_or_none(p);
        lines.push(format!("{p}\t{n}\t{mtime}"));
        let folder = parent(p);
        if !folders.contains(&folder) {
            folders.push(folder);
        }
    }
    for folder in &folders {
        lines.push(format!("{folder}\t{}", stat_or_none(folder).1));
    }
    lines.join("\n")
}

/// The file name of a cover in the covers folder: the SHA-1 (40 lower-case hex
/// digits) of `cache_key()`, then ".png".
pub fn cache_name(paths: &[String], size: u32) -> String {
    let digest = Sha1::digest(cache_key(paths, size).as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.png")
}

pub fn cache_file(paths: &[String], size: u32) -> PathBuf {
    cache_dir().join(cache_name(paths, size))
}

/// A whole PNG (signature and IEND chunk), or the empty "no cover" mark.
fn valid(data: &[u8]) -> bool {
    data.is_empty() || (data.starts_with(PNG_START) && data.ends_with(PNG_END))
}

fn touch(file: &Path) {
    let now = SystemTime::now();
    if let Ok(f) = std::fs::File::options().write(true).open(file) {
        let _ = f.set_times(std::fs::FileTimes::new().set_accessed(now).set_modified(now));
    }
}

/// One cache file: None when it is missing, unreadable or damaged, else the
/// cover (None inside for an album remembered without one).  Reading touches
/// its modification time, which is what `trim_cache()` goes by.
pub fn read_cached(file: &Path) -> Option<Option<Vec<u8>>> {
    let data = std::fs::read(file).ok()?;
    if !valid(&data) {
        return None;
    }
    touch(file);
    Some((!data.is_empty()).then_some(data))
}

/// Write through a temp file and a rename, so a reader never sees half a file;
/// failing to write only costs making the cover again next time.
fn write(file: &Path, data: &[u8]) {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let Some(folder) = file.parent() else { return };
    if std::fs::create_dir_all(folder).is_err() {
        return;
    }
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let tmp =
        folder.join(format!(".cover-{}-{}-{nanos}.tmp", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    let ok =
        std::fs::File::create(&tmp).and_then(|mut f| f.write_all(data)).is_ok() && std::fs::rename(&tmp, file).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Whether the covers folder has this cover (or its "no cover" mark).
pub fn is_cached(paths: &[String], size: u32) -> bool {
    is_cached_in(&cache_dir(), paths, size)
}

/// `is_cached()` in a given covers folder.
pub fn is_cached_in(dir: &Path, paths: &[String], size: u32) -> bool {
    dir.join(cache_name(paths, size)).is_file()
}

/// `cached_cover_png()` in a given covers folder with a given FFmpeg (the
/// player's worker keeps the folder it started with; tests: an FFmpeg that
/// cannot run).
pub fn cached_cover_png_in(dir: &Path, ffmpeg: &Path, paths: &[String], size: u32) -> Option<Vec<u8>> {
    let paths: Vec<String> = paths.iter().take(CACHE_PATHS).cloned().collect();
    let file = dir.join(cache_name(&paths, size));
    if let Some(found) = read_cached(&file) {
        return found;
    }
    let (data, certain) = make(ffmpeg, &paths, size);
    if data.is_some() || certain {
        write(&file, data.as_deref().unwrap_or_default());
    }
    data
}

/// `cover_png()` of the first four paths, from the covers folder when it has it
/// (None for an album remembered without a cover), else made with FFmpeg and
/// saved there.  A damaged or unreadable file is made again.  When FFmpeg could
/// not run, nothing is saved, so the album is tried again next time.
pub fn cached_cover_png(paths: &[String], size: u32) -> Option<Vec<u8>> {
    cached_cover_png_in(&cache_dir(), &tool("ffmpeg"), paths, size)
}

fn usage(n: u64) -> u64 {
    n.div_ceil(BLOCK).max(1) * BLOCK
}

/// Delete the least recently used covers (oldest modification time first)
/// until the folder holds at most `limit_bytes` (each file counted as whole
/// 4 KB blocks, an empty one as one block), and temp files older than an hour.
/// Returns the number of covers deleted.  `folder` None is `cache_dir()`.
pub fn trim_cache(limit_bytes: u64, folder: Option<&Path>) -> usize {
    let folder = folder.map(Path::to_path_buf).unwrap_or_else(cache_dir);
    let Ok(rd) = std::fs::read_dir(&folder) else { return 0 };
    let now = SystemTime::now();
    let mut entries: Vec<(i64, String, u64)> = vec![];
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = folder.join(&name);
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if !meta.is_file() {
            continue;
        }
        if name.ends_with(".tmp") {
            let old =
                meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > TEMP_MAX_AGE);
            if old {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        if name.ends_with(".png") {
            let mtime = match meta.modified().map(|m| m.duration_since(SystemTime::UNIX_EPOCH)) {
                Ok(Ok(d)) => d.as_nanos() as i64,
                Ok(Err(e)) => -(e.duration().as_nanos() as i64),
                Err(_) => i64::MIN,
            };
            entries.push((mtime, name, usage(meta.len())));
        }
    }
    entries.sort();
    let mut total: u64 = entries.iter().map(|e| e.2).sum();
    let mut deleted = 0;
    for (_, name, used) in entries {
        if total <= limit_bytes {
            break;
        }
        if std::fs::remove_file(folder.join(&name)).is_ok() {
            total -= used;
            deleted += 1;
        }
    }
    deleted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rhr-covers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn set_mtime(path: &Path, secs: u64) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)).unwrap();
    }

    #[test]
    fn key_uses_the_first_four_songs_and_their_folders() {
        let paths: Vec<String> =
            ["/no/such/A/1.flac", "/no/such/A/2.flac", "/no/such/B/3.flac", "/no/such/A/4.flac", "/no/such/C/5.flac"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(
            cache_key(&paths, 176),
            "v1\t176\n/no/such/A/1.flac\t-1\t-1\n/no/such/A/2.flac\t-1\t-1\n/no/such/B/3.flac\t-1\t-1\n/no/such/A/4.flac\t-1\t-1\n/no/such/A\t-1\n/no/such/B\t-1"
        );
        assert_eq!(cache_name(&paths[..4], 176), cache_name(&paths, 176));
        assert_ne!(cache_name(&paths, 220), cache_name(&paths, 176));
        assert_eq!(cache_name(&paths, 176).len(), 44);
    }

    #[test]
    fn no_mark_when_ffmpeg_cannot_run() {
        let d = tmp("noffmpeg");
        let song = d.join("01.wav");
        std::fs::write(&song, b"RIFF").unwrap();
        std::fs::write(d.join("folder.png"), b"x").unwrap();
        let paths = vec![song.to_string_lossy().into_owned()];
        // the folder is given directly: the environment is shared with other tests
        let (data, certain) = make(Path::new("/no/such/ffmpeg"), &paths, 64);
        assert!(data.is_none() && !certain);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn trim_deletes_the_least_recently_used() {
        let d = tmp("trim");
        for (i, (name, size)) in
            [("a.png", 5000), ("b.png", 100), ("c.png", 0), ("d.png", 9000), ("e.png", 4096)].iter().enumerate()
        {
            let p = d.join(name);
            std::fs::write(&p, vec![b'x'; *size]).unwrap();
            set_mtime(&p, 1_000_000 + i as u64);
        }
        for n in [".cover-1.tmp", ".cover-2.tmp", "notes.txt"] {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        set_mtime(&d.join(".cover-1.tmp"), 1_000_000);
        // usage in 4 KB blocks: a 8 KB, b 4 KB, c 4 KB, d 12 KB, e 4 KB = 32 KB
        assert_eq!(trim_cache(32 * 1024, Some(&d)), 0);
        assert_eq!(trim_cache(20 * 1024, Some(&d)), 2);
        let mut left: Vec<String> =
            std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [".cover-2.tmp", "c.png", "d.png", "e.png", "notes.txt"]);
        assert_eq!(trim_cache(0, Some(&d)), 3);
        assert_eq!(trim_cache(0, Some(&d.join("missing"))), 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn valid_files_and_marks() {
        assert!(valid(b""));
        assert!(valid(&[PNG_START, b"....", PNG_END].concat()));
        assert!(!valid(&PNG_START[..6]));
        assert!(!valid(b"not a png"));
    }
}
