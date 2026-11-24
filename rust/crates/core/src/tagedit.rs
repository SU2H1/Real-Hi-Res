//! Editing a song's information (2.0), the counterpart of
//! `realhires/tagedit.py`: title, artist, album, album artist, genre, release
//! date, track, disc and composer.  FLAC and MP3 are written natively with the
//! same bytes as the Python version (only the Vorbis comments or the ID3v2 tag
//! change); other formats are copied by FFmpeg with `-c copy` and the same
//! arguments, and the copy must have the original's audio before it replaces
//! the file.  The saved results move to the new size and time with the new
//! tags, and the library reads the file again.

use std::path::Path;

use crate::embed::{self, comment_key, EmbedError, Frame};
use crate::paths::{extension, file_name};
use crate::probe::{self, Tags};
use crate::settings::Settings;

pub const FIELDS: [&str; 9] = ["title", "artist", "album", "album_artist", "genre", "date", "track", "disc", "composer"];
pub const MAX_VALUE: usize = 1000;
pub const NATIVE: [&str; 2] = [".flac", ".mp3"];
pub const STREAM_TAGS: [&str; 4] = [".ogg", ".oga", ".opus", ".spx"];
pub const ID3_CHUNK: [&str; 3] = [".aif", ".aiff", ".aifc"];
pub const NO_MUXER: [&str; 7] = [".ape", ".dsf", ".dff", ".tak", ".mpc", ".ofr", ".tta"];

pub fn label(field: &str) -> &'static str {
    match field {
        "title" => "Title",
        "artist" => "Artist",
        "album" => "Album",
        "album_artist" => "Album artist",
        "genre" => "Genre",
        "date" => "Release date",
        "track" => "Track",
        "disc" => "Disc",
        _ => "Composer",
    }
}

pub fn hint(field: &str) -> &'static str {
    match field {
        "date" => "a year (2004) or a date (2004-05-28)",
        "track" => "3, or 3/12 with the number of tracks",
        "disc" => "1, or 1/2 with the number of discs",
        _ => "",
    }
}

/// The changes to write: (field, value) in any order; only FIELDS count.
pub type Changes = Vec<(String, String)>;

fn get<'a>(changes: &'a Changes, field: &str) -> Option<&'a str> {
    changes.iter().find(|(k, _)| k == field).map(|(_, v)| v.as_str())
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagError(pub String);

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<EmbedError> for TagError {
    fn from(e: EmbedError) -> Self {
        TagError(e.to_string())
    }
}

/// One line (Python `" ".join(value.split())`).
pub fn clean_value(value: &str) -> String {
    value.split(crate::pyunicode::is_space).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `str(int(s))` for a string of ASCII digits.
fn int_text(s: &str) -> String {
    let t = s.trim_start_matches('0');
    if t.is_empty() { "0".into() } else { t.into() }
}

fn number_parts(value: &str) -> Option<(String, Option<String>)> {
    let v: String = value.chars().filter(|c| *c != ' ').collect();
    match v.split_once('/') {
        Some((n, t)) if digits(n) && digits(t) => Some((int_text(n), Some(int_text(t)))),
        None if digits(&v) => Some((int_text(&v), None)),
        _ => None,
    }
}

/// ("3", "12") for "3/12", ("3", "") for "3", ("", "") for "" (Python `split_number`).
pub fn split_number(value: &str) -> (String, String) {
    match number_parts(value) {
        Some((n, t)) => (n, t.unwrap_or_default()),
        None => (String::new(), String::new()),
    }
}

fn starts_with_year(v: &str) -> bool {
    v.len() >= 4 && v.as_bytes()[..4].iter().all(u8::is_ascii_digit)
}

/// The changes, cleaned, in FIELDS order (Python `check_changes`).
pub fn check_changes(changes: &Changes) -> Result<Changes, TagError> {
    let mut out = vec![];
    for f in FIELDS {
        let Some(v) = get(changes, f) else { continue };
        let mut v = clean_value(v);
        if v.chars().count() > MAX_VALUE {
            return Err(TagError(format!(
                "The {} is too long: at most {} characters fit.",
                label(f).to_lowercase(),
                crate::stats::thousands(MAX_VALUE as i64)
            )));
        }
        if !v.is_empty() && (f == "track" || f == "disc") {
            let Some((n, total)) = number_parts(&v) else {
                return Err(TagError(format!("The {} must be a number, such as {}.", label(f).to_lowercase(), hint(f))));
            };
            v = match total {
                Some(t) => format!("{n}/{t}"),
                None => n,
            };
        }
        if !v.is_empty() && f == "date" && !starts_with_year(&v) {
            return Err(TagError("Write the release date as a year (2004) or a date (2004-05-28).".into()));
        }
        out.push((f.to_string(), v));
    }
    Ok(out)
}

/// Each field's value when every song has the same one, None where they differ.
pub fn common_values(tags: &[Tags]) -> Vec<(String, Option<String>)> {
    FIELDS
        .iter()
        .map(|f| {
            let mut values: Vec<String> =
                tags.iter().map(|t| clean_value(probe::tag(t, f).unwrap_or(""))).collect();
            values.sort();
            values.dedup();
            let v = match values.len() {
                0 => Some(String::new()),
                1 => Some(values.remove(0)),
                _ => None,
            };
            (f.to_string(), v)
        })
        .collect()
}

// ------------------------------------------------------------------- FLAC ---

fn put<T>(items: Vec<T>, key_of: impl Fn(&T) -> bool, new: Vec<T>) -> Vec<T> {
    let mut out = Vec::with_capacity(items.len() + new.len());
    let mut new = Some(new);
    for it in items {
        if key_of(&it) {
            if let Some(n) = new.take() {
                out.extend(n);
            }
        } else {
            out.push(it);
        }
    }
    if let Some(n) = new {
        out.extend(n);
    }
    out
}

fn flac_keys(field: &str) -> (&'static str, &'static [&'static str]) {
    match field {
        "title" => ("TITLE", &["TITLE"]),
        "artist" => ("ARTIST", &["ARTIST"]),
        "album" => ("ALBUM", &["ALBUM"]),
        "album_artist" => ("ALBUMARTIST", &["ALBUMARTIST", "ALBUM ARTIST", "ALBUM_ARTIST"]),
        "genre" => ("GENRE", &["GENRE"]),
        "date" => ("DATE", &["DATE", "YEAR"]),
        "track" => ("TRACKNUMBER", &["TRACKNUMBER", "TRACK"]),
        "disc" => ("DISCNUMBER", &["DISCNUMBER", "DISC"]),
        _ => ("COMPOSER", &["COMPOSER"]),
    }
}

fn flac_total(field: &str) -> Option<(&'static str, &'static [&'static str])> {
    match field {
        "track" => Some(("TRACKTOTAL", &["TRACKTOTAL", "TOTALTRACKS"])),
        "disc" => Some(("DISCTOTAL", &["DISCTOTAL", "TOTALDISCS"])),
        _ => None,
    }
}

fn comment(key: &str, value: &str) -> Vec<Vec<u8>> {
    if value.is_empty() { vec![] } else { vec![format!("{key}={value}").into_bytes()] }
}

fn put_comments(comments: Vec<Vec<u8>>, removes: &[&str], new: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    put(comments, |c| removes.contains(&comment_key(c).as_str()), new)
}

/// The FLAC file with the (checked) changes in its Vorbis comments.
pub fn flac_with_tags(data: &[u8], changes: &Changes) -> Result<Vec<u8>, TagError> {
    let edit = |mut comments: Vec<Vec<u8>>| {
        for f in FIELDS {
            let Some(value) = get(changes, f) else { continue };
            let (key, removes) = flac_keys(f);
            if let Some((tkey, tremoves)) = flac_total(f) {
                let (number, total) = split_number(value);
                comments = put_comments(comments, removes, comment(key, &number));
                if !total.is_empty() || number.is_empty() {
                    comments = put_comments(comments, tremoves, comment(tkey, &total));
                }
            } else {
                comments = put_comments(comments, removes, comment(key, value));
            }
        }
        comments
    };
    Ok(embed::flac_rewrite(data, edit, "The tags are too long for a FLAC file.")?)
}

// -------------------------------------------------------------------- ID3 ---

fn text_frame(fid: &[u8; 4], text: &str, major: u8) -> Frame {
    let (e, raw, _) = embed::enc(text, major);
    let mut body = vec![e];
    body.extend_from_slice(&raw);
    (*fid, [0, 0], body)
}

fn id3_id(field: &str) -> &'static [u8; 4] {
    match field {
        "title" => b"TIT2",
        "artist" => b"TPE1",
        "album" => b"TALB",
        "album_artist" => b"TPE2",
        "genre" => b"TCON",
        "track" => b"TRCK",
        "disc" => b"TPOS",
        _ => b"TCOM",
    }
}

fn full_date(v: &str) -> Option<(&str, &str)> {
    let b = v.as_bytes();
    let ok = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    ok.then(|| (&v[5..7], &v[8..10]))
}

/// The MP3 file with the (checked) changes in its ID3v2 tag (Python `mp3_with_tags`).
pub fn mp3_with_tags(data: &[u8], changes: &Changes) -> Result<Vec<u8>, TagError> {
    let edit = |major: u8, mut frames: Vec<Frame>| {
        for f in FIELDS {
            let Some(value) = get(changes, f) else { continue };
            if f == "date" {
                let mut new = vec![];
                if major == 4 {
                    if !value.is_empty() {
                        new.push(text_frame(b"TDRC", value, major));
                    }
                } else if !value.is_empty() {
                    new.push(text_frame(b"TYER", &value.chars().take(4).collect::<String>(), major));
                    if let Some((month, day)) = full_date(value) {
                        new.push(text_frame(b"TDAT", &format!("{day}{month}"), major));
                    }
                }
                frames = put(frames, |fr| [b"TDRC", b"TYER", b"TDAT", b"TIME"].contains(&&fr.0), new);
            } else {
                let fid = id3_id(f);
                let new = if value.is_empty() { vec![] } else { vec![text_frame(fid, value, major)] };
                frames = put(frames, |fr| &fr.0 == fid, new);
            }
        }
        frames
    };
    Ok(embed::mp3_rewrite(data, edit, "The tags are too long for an ID3 tag.")?)
}

// ----------------------------------------------------------------- FFmpeg ---

fn ext_of(path: &str) -> String {
    extension(&file_name(path)).to_lowercase()
}

/// The arguments (after the program) that copy `src` to `dst` with new tags.
pub fn ffmpeg_args(src: &str, dst: &str, changes: &Changes) -> Vec<String> {
    let ext = ext_of(src);
    let spec = if STREAM_TAGS.contains(&ext.as_str()) { "-metadata:s:a:0" } else { "-metadata" };
    let mut args: Vec<String> = [
        "-v", "error", "-nostdin", "-y", "-i", src, "-map", "0", "-c", "copy", "-map_metadata", "0", "-map_chapters",
        "0", "-fflags", "+bitexact",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for f in FIELDS {
        if let Some(v) = get(changes, f) {
            args.push(spec.into());
            args.push(format!("{f}={v}"));
        }
    }
    if ID3_CHUNK.contains(&ext.as_str()) {
        args.extend(["-write_id3v2".to_string(), "1".to_string()]);
    }
    args.push(dst.into());
    args
}

/// Why the tags of this file cannot be changed, or "".
pub fn cannot_edit(path: &str) -> String {
    let ext = ext_of(path);
    if NO_MUXER.contains(&ext.as_str()) {
        return format!(
            "The information in {} files cannot be changed here, because FFmpeg can read them but not write them.",
            ext[1..].to_uppercase()
        );
    }
    String::new()
}

fn ffmpeg_copy(path: &str, changes: &Changes) -> Result<(), TagError> {
    let why = cannot_edit(path);
    if !why.is_empty() {
        return Err(TagError(why));
    }
    let ext = extension(&file_name(path));
    let folder = crate::paths::parent(&crate::paths::abspath(path));
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = Path::new(&folder)
        .join(format!(
            ".realhires-tags-{}-{}{ext}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned();
    let res = (|| -> Result<(), TagError> {
        let out = probe::command(probe::tool("ffmpeg"))
            .args(ffmpeg_args(path, &tmp, changes))
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| TagError(format!("FFmpeg could not be started: {e}.")))?;
        if !out.status.success() {
            let text = String::from_utf8_lossy(&out.stderr).to_string();
            let last = text.trim().lines().last().map(str::to_string);
            let code = out.status.code().map(|c| format!("exit code {c}")).unwrap_or_else(|| "it was stopped".into());
            return Err(TagError(format!(
                "FFmpeg could not copy the file with the new information: {}.",
                last.unwrap_or(code)
            )));
        }
        let before = probe::probe(path).map_err(|e| TagError(e.to_string()))?;
        let after = probe::probe(&tmp).map_err(|e| TagError(e.to_string()))?;
        let (b, a) = (before.duration.unwrap_or(0.0), after.duration.unwrap_or(0.0));
        if after.sample_rate != before.sample_rate || after.channels != before.channels || (a - b).abs() > 0.5f64.max(0.01 * b)
        {
            return Err(TagError(
                "FFmpeg's copy did not have the same audio as the original, so the file was left unchanged.".into(),
            ));
        }
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, path).map_err(|e| TagError(e.to_string()))
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Write the (checked) changes into the file; returns "native" or "ffmpeg".
pub fn write_tags(path: &str, changes: &Changes) -> Result<&'static str, TagError> {
    if changes.is_empty() {
        return Ok("native");
    }
    let ext = ext_of(path);
    if NATIVE.contains(&ext.as_str()) {
        let data = std::fs::read(path).map_err(|e| TagError(e.to_string()))?;
        let new = if ext == ".flac" { flac_with_tags(&data, changes) } else { mp3_with_tags(&data, changes) };
        let new = match new {
            Ok(n) => n,
            Err(_) if ext == ".mp3" => {
                ffmpeg_copy(path, changes)?; // ID3v2.2 or unsynchronised tags: FFmpeg writes a new one
                return Ok("ffmpeg");
            }
            Err(e) => return Err(e),
        };
        embed::replace_file(path, &new).map_err(|e| TagError(e.to_string()))?;
        return Ok("native");
    }
    ffmpeg_copy(path, changes)?;
    Ok("ffmpeg")
}

// ----------------------------------------------------------- saving songs ---

#[derive(Debug, Clone, PartialEq)]
pub struct TagOutcome {
    pub path: String,
    pub ok: bool,
    pub message: String,
    pub tags: Tags,
}

pub fn same_value(field: &str, wanted: &str, got: &str) -> bool {
    if field == "track" || field == "disc" {
        return split_number(wanted).0 == split_number(got).0;
    }
    clean_value(wanted) == clean_value(got)
}

fn join(words: &[String]) -> String {
    match words.len() {
        0 => String::new(),
        1 => words[0].clone(),
        n => format!("{} and {}", words[..n - 1].join(", "), words[n - 1]),
    }
}

/// "" when the file holds every change, else what its format could not keep.
pub fn kept_note(name: &str, changes: &Changes, tags: &Tags) -> String {
    let got = |f: &str| probe::tag(tags, f).unwrap_or("").to_string();
    let mut lost = vec![];
    let mut other = vec![];
    for f in FIELDS {
        let Some(wanted) = get(changes, f) else { continue };
        let g = got(f);
        if !wanted.is_empty() && g.is_empty() {
            lost.push(format!("the {}", label(f).to_lowercase()));
        }
        if !g.is_empty() && !same_value(f, wanted, &g) {
            other.push(format!("the {} reads “{g}”", label(f).to_lowercase()));
        }
    }
    let mut parts = vec![];
    if !lost.is_empty() {
        parts.push(format!("its format has no place for {}", join(&lost)));
    }
    if !other.is_empty() {
        parts.push(format!("{}, as far as its format allows", join(&other)));
    }
    if parts.is_empty() { String::new() } else { format!("{name} was saved, but {}.", parts.join("; ")) }
}

/// Write the changes into every file, keep their saved results and update the
/// library.  Never fails; one outcome per file.
pub fn save_tags(
    paths: &[String],
    changes: &Changes,
    cfg: Option<&Settings>,
    mut progress: impl FnMut(usize, usize, &str),
) -> Vec<TagOutcome> {
    let changes = match check_changes(changes) {
        Ok(c) => c,
        Err(e) => {
            return paths
                .iter()
                .map(|p| TagOutcome { path: p.clone(), ok: false, message: e.0.clone(), tags: vec![] })
                .collect()
        }
    };
    let store = cfg.and_then(crate::store::open_store);
    let lib = crate::musiclib::Library::open(None).ok();
    let mut out = vec![];
    for (i, path) in paths.iter().enumerate() {
        progress(i, paths.len(), path);
        let name = file_name(path);
        let before = crate::store::stat(path).map_err(|e| TagError(e.to_string())).and_then(|st| {
            write_tags(path, &changes)?;
            Ok(st)
        });
        let (size, mtime) = match before {
            Ok(st) => st,
            Err(e) => {
                out.push(TagOutcome {
                    path: path.clone(),
                    ok: false,
                    message: format!("Could not save the information of {name}: {e}"),
                    tags: vec![],
                });
                continue;
            }
        };
        let tags = probe::probe(path).map(|i| i.tags).unwrap_or_default();
        if let Some(st) = &store {
            st.retag_with(path, size, mtime, Some(&tags));
        }
        if let Some(lib) = &lib {
            lib.update_file(path);
        }
        out.push(TagOutcome { path: path.clone(), ok: true, message: kept_note(&name, &changes, &tags), tags });
    }
    progress(paths.len(), paths.len(), "");
    out
}

/// (headline, sentences) for the editor after saving.
pub fn save_summary(outcomes: &[TagOutcome]) -> (String, Vec<String>) {
    let ok: Vec<&TagOutcome> = outcomes.iter().filter(|o| o.ok).collect();
    let bad: Vec<&TagOutcome> = outcomes.iter().filter(|o| !o.ok).collect();
    let n = ok.len();
    let head = if !ok.is_empty() && bad.is_empty() {
        format!("Saved the information of {} song{}.", crate::stats::thousands(n as i64), if n != 1 { "s" } else { "" })
    } else if !ok.is_empty() {
        format!(
            "Saved {} of {} songs.",
            crate::stats::thousands(n as i64),
            crate::stats::thousands(outcomes.len() as i64)
        )
    } else {
        "Nothing was saved.".to_string()
    };
    let mut lines: Vec<String> = vec![];
    for o in ok.iter().chain(bad.iter()) {
        if !o.message.is_empty() && !lines.contains(&o.message) {
            lines.push(o.message.clone());
        }
    }
    (head, lines)
}
