//! Editing a song's information (2.0), the counterpart of
//! `realhires/tagedit.py`: title, artist, album, album artist, genre, release
//! date, track, disc and composer.  FLAC, MP3, Ogg Vorbis and Opus are written
//! natively with the same bytes as the Python version (only the Vorbis comments,
//! the ID3v2 tag or the Ogg comment header pages change); other formats are
//! copied by FFmpeg with `-c copy` and the same arguments, and the copy must
//! decode to exactly the original's samples before it replaces the file.  The saved results move to the new size and time with the new
//! tags, and the library reads the file again.

use std::path::Path;

use crate::embed::{self, comment_key, EmbedError, Frame};
use crate::paths::{extension, file_name};
use crate::probe::{self, Tags};
use crate::settings::Settings;

pub const FIELDS: [&str; 9] = ["title", "artist", "album", "album_artist", "genre", "date", "track", "disc", "composer"];
pub const MAX_VALUE: usize = 1000;
pub const NATIVE: [&str; 5] = [".flac", ".mp3", ".ogg", ".oga", ".opus"];
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

/// Vorbis comments with the (checked) changes (Python `edit_comments`).
pub fn edit_comments(mut comments: Vec<Vec<u8>>, changes: &Changes) -> Vec<Vec<u8>> {
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
}

/// The FLAC file with the (checked) changes in its Vorbis comments.
pub fn flac_with_tags(data: &[u8], changes: &Changes) -> Result<Vec<u8>, TagError> {
    Ok(embed::flac_rewrite(data, |c| edit_comments(c, changes), "The tags are too long for a FLAC file.")?)
}

// -------------------------------------------------------------------- Ogg ---

fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            }
            *slot = r;
        }
        t
    })
}

/// The Ogg page checksum (Python `ogg_crc`).
pub fn ogg_crc(data: &[u8]) -> u32 {
    let t = crc_table();
    data.iter().fold(0u32, |crc, &b| (crc << 8) ^ t[(((crc >> 24) as u8) ^ b) as usize])
}

struct OggPage<'a> {
    htype: u8,
    granule: &'a [u8],
    serial: u32,
    seq: u32,
    segs: &'a [u8],
    body: &'a [u8],
    end: usize,
}

fn ogg_page_at(data: &[u8], pos: usize) -> Result<OggPage<'_>, TagError> {
    if data.len() < pos + 27 || &data[pos..pos + 4] != b"OggS" {
        return Err(TagError("The Ogg file is damaged: a page does not start where it should.".into()));
    }
    let nsegs = data[pos + 26] as usize;
    let start = pos + 27 + nsegs;
    if start > data.len() {
        return Err(TagError("The Ogg file ends inside a page.".into()));
    }
    let segs = &data[pos + 27..start];
    let end = start + segs.iter().map(|v| *v as usize).sum::<usize>();
    if end > data.len() {
        return Err(TagError("The Ogg file ends inside a page.".into()));
    }
    let le = |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    Ok(OggPage {
        htype: data[pos + 5],
        granule: &data[pos + 6..pos + 14],
        serial: le(pos + 14),
        seq: le(pos + 18),
        segs,
        body: &data[start..end],
        end,
    })
}

fn ogg_page(htype: u8, granule: &[u8], serial: u32, seq: u32, segs: &[u8], body: &[u8]) -> Vec<u8> {
    let mut page = b"OggS\x00".to_vec();
    page.push(htype);
    page.extend_from_slice(granule);
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&seq.to_le_bytes());
    page.extend_from_slice(&[0, 0, 0, 0]);
    page.push(segs.len() as u8);
    page.extend_from_slice(segs);
    page.extend_from_slice(body);
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// Header packets laid out in pages as libogg flushes them (Python `_ogg_pages_for`).
fn ogg_pages_for(packets: &[Vec<u8>], serial: u32, mut seq: u32) -> Vec<Vec<u8>> {
    let mut pages = vec![];
    let (mut segs, mut body, mut cont): (Vec<u8>, Vec<u8>, bool) = (vec![], vec![], false);
    let mut flush = |segs: &mut Vec<u8>, body: &mut Vec<u8>, cont: &mut bool, next: bool| {
        let granule: [u8; 8] = if segs.iter().any(|v| *v < 255) { [0; 8] } else { [0xFF; 8] };
        pages.push(ogg_page(u8::from(*cont), &granule, serial, seq, segs, body));
        seq += 1;
        segs.clear();
        body.clear();
        *cont = next;
    };
    for pkt in packets {
        let mut lace = vec![255u8; pkt.len() / 255];
        lace.push((pkt.len() % 255) as u8);
        let mut pos = 0;
        for (i, v) in lace.into_iter().enumerate() {
            if segs.len() == 255 {
                flush(&mut segs, &mut body, &mut cont, i > 0);
            }
            segs.push(v);
            body.extend_from_slice(&pkt[pos..pos + v as usize]);
            pos += v as usize;
        }
    }
    if !segs.is_empty() {
        flush(&mut segs, &mut body, &mut cont, false);
    }
    pages
}

fn damaged_comments() -> TagError {
    TagError("The Ogg file's comment header is damaged.".into())
}

/// (vendor, comments, end offset) of a Vorbis comment structure at `start`.
fn parse_comment_body(b: &[u8], start: usize) -> Result<(Vec<u8>, Vec<Vec<u8>>, usize), TagError> {
    let le = |at: usize| -> Result<usize, TagError> {
        b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as usize).ok_or_else(damaged_comments)
    };
    let vlen = le(start)?;
    let vendor = b.get(start + 4..start + 4 + vlen).ok_or_else(damaged_comments)?.to_vec();
    let mut pos = start + 4 + vlen;
    let count = le(pos)?;
    pos += 4;
    let mut comments = vec![];
    for _ in 0..count {
        let n = le(pos)?;
        comments.push(b.get(pos + 4..pos + 4 + n).ok_or_else(damaged_comments)?.to_vec());
        pos += 4 + n;
    }
    Ok((vendor, comments, pos))
}

/// The Ogg Vorbis or Opus file with the (checked) changes in its comment
/// header (Python `ogg_with_tags`).
pub fn ogg_with_tags(data: &[u8], changes: &Changes) -> Result<Vec<u8>, TagError> {
    let err = |m: &str| Err(TagError(m.to_string()));
    let p0 = ogg_page_at(data, 0)?;
    let (serial, first, first_end) = (p0.serial, p0.body.to_vec(), p0.end);
    if p0.htype & 2 == 0 || p0.segs.len() != 1 || p0.segs[0] == 255 {
        return err("The Ogg file does not start with a stream header page of its own.");
    }
    let (need, magic): (usize, &[u8]) = if first.starts_with(b"\x01vorbis") {
        (3, b"\x03vorbis")
    } else if first.starts_with(b"OpusHead") {
        (2, b"OpusTags")
    } else {
        return err("Only Ogg files with Vorbis or Opus audio can be changed directly.");
    };
    let mut packets: Vec<Vec<u8>> = vec![first];
    let mut cur: Vec<u8> = vec![];
    let mut old_pages = 0usize;
    let mut pos = first_end;
    while packets.len() < need {
        if pos >= data.len() {
            return err("The Ogg file ends inside its headers.");
        }
        let p = ogg_page_at(data, pos)?;
        if p.serial != serial {
            return err("The Ogg file holds several streams, which cannot be changed directly.");
        }
        let mut off = 0;
        for &v in p.segs {
            if packets.len() == need {
                return err("The Ogg file's headers share a page with the audio.");
            }
            cur.extend_from_slice(&p.body[off..off + v as usize]);
            off += v as usize;
            if v < 255 {
                packets.push(std::mem::take(&mut cur));
            }
        }
        old_pages += 1;
        pos = p.end;
    }
    if !cur.is_empty() || packets.len() != need {
        return err("The Ogg file's headers share a page with the audio.");
    }
    if !packets[1].starts_with(magic) {
        return Err(damaged_comments());
    }
    let (vendor, comments, end) = parse_comment_body(&packets[1], magic.len())?;
    let mut new_tags = magic.to_vec();
    new_tags.extend_from_slice(&embed::vorbis_body(&vendor, &edit_comments(comments, changes)));
    new_tags.extend_from_slice(&packets[1][end..]);
    let mut header = vec![new_tags];
    header.extend(packets[2..].iter().cloned());
    let pages = ogg_pages_for(&header, serial, 1);
    let mut out = data[..first_end].to_vec();
    for p in &pages {
        out.extend_from_slice(p);
    }
    let delta = pages.len() as i64 - old_pages as i64;
    if delta == 0 {
        out.extend_from_slice(&data[pos..]);
        return Ok(out);
    }
    while pos < data.len() && data[pos..].starts_with(b"OggS") {
        let p = ogg_page_at(data, pos)?;
        if p.serial == serial {
            out.extend(ogg_page(p.htype, p.granule, p.serial, (p.seq as i64 + delta) as u32, p.segs, p.body));
        } else {
            out.extend_from_slice(&data[pos..p.end]);
        }
        pos = p.end;
    }
    out.extend_from_slice(&data[pos..]);
    Ok(out)
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

/// SHA-1 of the file's first audio stream decoded to 32-bit samples (Python `pcm_digest`).
pub fn pcm_digest(path: &str) -> Result<String, TagError> {
    use sha1::{Digest, Sha1};
    use std::io::Read;
    let mut child = probe::command(probe::tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-i", path, "-map", "0:a:0", "-f", "s32le", "-acodec", "pcm_s32le", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| TagError(format!("FFmpeg could not be started: {e}.")))?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    if let Some(mut out) = child.stdout.take() {
        loop {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => h.update(&buf[..n]),
            }
        }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    if !ok {
        return Err(TagError("FFmpeg could not decode the file to compare its audio.".into()));
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
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
        if pcm_digest(&tmp)? != pcm_digest(path)? {
            return Err(TagError(
                "FFmpeg's copy did not have exactly the same audio as the original, so the file was left unchanged."
                    .into(),
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
        let new = match ext.as_str() {
            ".flac" => flac_with_tags(&data, changes),
            ".mp3" => mp3_with_tags(&data, changes),
            _ => ogg_with_tags(&data, changes),
        };
        let new = match new {
            Ok(n) => n,
            Err(_) if ext != ".flac" => {
                // ID3v2.2 or unsynchronised MP3 tags, Ogg FLAC or Speex...: FFmpeg copies the file
                ffmpeg_copy(path, changes)?;
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
    progress: impl FnMut(usize, usize, &str),
) -> Vec<TagOutcome> {
    let items: Vec<(String, Changes)> = paths.iter().map(|p| (p.clone(), changes.clone())).collect();
    save_tags_each(&items, cfg, progress)
}

/// `save_tags()` with changes of each file's own: (path, changes) (Python `save_tags_each`).
pub fn save_tags_each(
    items: &[(String, Changes)],
    cfg: Option<&Settings>,
    mut progress: impl FnMut(usize, usize, &str),
) -> Vec<TagOutcome> {
    let checked: Vec<(String, Result<Changes, TagError>)> =
        items.iter().map(|(p, c)| (p.clone(), check_changes(c))).collect();
    if checked.iter().all(|(_, c)| c.is_err()) {
        return checked
            .into_iter()
            .map(|(path, c)| TagOutcome { path, ok: false, message: c.err().map(|e| e.0).unwrap_or_default(), tags: vec![] })
            .collect();
    }
    let store = cfg.and_then(crate::store::open_store);
    let lib = crate::musiclib::Library::open(None).ok();
    let mut out = vec![];
    let n = checked.len();
    for (i, (path, changes)) in checked.iter().enumerate() {
        progress(i, n, path);
        let changes = match changes {
            Ok(c) => c,
            Err(e) => {
                out.push(TagOutcome { path: path.clone(), ok: false, message: e.0.clone(), tags: vec![] });
                continue;
            }
        };
        let name = file_name(path);
        let before = crate::store::stat(path).map_err(|e| TagError(e.to_string())).and_then(|st| {
            write_tags(path, changes)?;
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
        out.push(TagOutcome { path: path.clone(), ok: true, message: kept_note(&name, changes, &tags), tags });
    }
    progress(n, n, "");
    out
}

// ---------------------------------------------- renaming and merging groups ---
// The Genres, Composers and Years pages rename a genre, composer or year in every
// song of it, and merge genres, through save_tags_each(): only the tag changes.

/// The tag field of a grouping ("genres" → "genre", "years" → "date").
pub fn group_field(kind: &str) -> &'static str {
    match kind {
        "genres" => "genre",
        "composers" => "composer",
        _ => "date",
    }
}

/// "genre", "composer" or "year".
pub fn group_noun(kind: &str) -> &'static str {
    match kind {
        "genres" => "genre",
        "composers" => "composer",
        _ => "year",
    }
}

/// The release date with its year replaced and the rest kept: "2001-05-03" with
/// 2004 gives "2004-05-03"; a date without a year ("", "May 2001") gives "2004".
pub fn renamed_date(old: &str, year: &str) -> String {
    let old = clean_value(old);
    if starts_with_year(&old) {
        format!("{year}{}", &old[4..])
    } else {
        year.to_string()
    }
}

/// The new name of a genre, composer or year, cleaned (Python `check_group_name`).
pub fn check_group_name(kind: &str, name: &str) -> Result<String, TagError> {
    let name = clean_value(name);
    let noun = group_noun(kind);
    if name.is_empty() {
        return Err(TagError(if kind == "years" { "Write the year, such as 2004.".into() } else { format!("Give the {noun} a name.") }));
    }
    if kind == "years" && !(name.len() == 4 && digits(&name)) {
        return Err(TagError("Write the year as four digits, such as 2004.".into()));
    }
    if name.chars().count() > MAX_VALUE {
        return Err(TagError(format!(
            "The {noun} is too long: at most {} characters fit.",
            crate::stats::thousands(MAX_VALUE as i64)
        )));
    }
    Ok(name)
}

/// What a song whose tags are `tags` gets when its genre, composer or year becomes `name`.
pub fn group_changes(kind: &str, name: &str, tags: &Tags) -> Changes {
    if kind == "years" {
        let old = tags.iter().find(|(k, _)| k == "date").map(|(_, v)| v.as_str()).unwrap_or("");
        return vec![("date".into(), renamed_date(old, name))];
    }
    vec![(group_field(kind).into(), name.to_string())]
}

/// Give every song in `paths` the genre, composer or year `name` (a year replaces
/// only the year of each song's release date).  Never fails; one outcome per file.
pub fn rename_group(
    kind: &str,
    paths: &[String],
    name: &str,
    cfg: Option<&Settings>,
    progress: impl FnMut(usize, usize, &str),
) -> Vec<TagOutcome> {
    let name = match check_group_name(kind, name) {
        Ok(n) => n,
        Err(e) => {
            return paths.iter().map(|p| TagOutcome { path: p.clone(), ok: false, message: e.0.clone(), tags: vec![] }).collect()
        }
    };
    let items: Vec<(String, Changes)> = paths
        .iter()
        .map(|p| {
            let tags = if kind == "years" { probe::probe(p).map(|i| i.tags).unwrap_or_default() } else { vec![] };
            (p.clone(), group_changes(kind, &name, &tags))
        })
        .collect();
    save_tags_each(&items, cfg, progress)
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
