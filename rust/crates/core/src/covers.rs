//! Album cover thumbnails for the album/artist views (FFmpeg only): the
//! picture embedded in a track, or a cover image in the album folder, scaled
//! and cropped to a square PNG.

use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::paths::{extension, parent, stem};
use crate::probe::{command, tool};

pub const FOLDER_IMAGES: [&str; 7] = ["cover", "folder", "front", "album", "albumart", "albumartsmall", "artwork"];
pub const IMAGE_EXTS: [&str; 6] = [".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"];

fn thumbnail(source: &str, size: u32, embedded: bool) -> Option<Vec<u8>> {
    let vf = format!("scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}");
    let mut cmd = command(tool("ffmpeg"));
    cmd.args(["-v", "error", "-nostdin", "-i", source]);
    if embedded {
        cmd.args(["-map", "0:v:0"]);
    }
    cmd.args(["-an", "-frames:v", "1", "-vf", &vf, "-f", "image2pipe", "-c:v", "png", "-"]);
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
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
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    };
    let data = reader.join().ok()?;
    (status.success() && data.starts_with(b"\x89PNG")).then_some(data)
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

/// PNG bytes of a size×size cover for an album/artist, or None.
pub fn cover_png(tracks: &[String], size: u32) -> Option<Vec<u8>> {
    for t in tracks.iter().take(3) {
        if let Some(d) = thumbnail(t, size, true) {
            return Some(d);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for t in tracks {
        let folder = parent(t);
        if !seen.insert(folder.clone()) {
            continue;
        }
        if let Some(img) = folder_image(&folder) {
            if let Some(d) = thumbnail(&img, size, false) {
                return Some(d);
            }
        }
    }
    None
}

// ---------------------------------------------------- inspecting a cover (2.1) ---
// The Cover window (docs/player-spec.md, "Inspecting and changing a cover"):
// where the cover comes from and the picture as it is stored, read as Python
// `covers.py` reads it.

/// What a FLAC PICTURE block records about an image (metaflac's rules).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureInfo {
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    /// bits per pixel
    pub depth: u32,
    /// the palette's size for an indexed PNG, else 0
    pub colors: u32,
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn png_info(data: &[u8]) -> Option<PictureInfo> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut pos = 8usize;
    let mut palette: Option<(u32, u32)> = None;
    while data.len() - pos > 12 {
        let clen = be32(data, pos) as usize;
        let kind = &data[pos + 4..pos + 8];
        if kind == b"IHDR" && clen == 13 && data.len() >= pos + 21 {
            let (w, h, bits, color) = (be32(data, pos + 8), be32(data, pos + 12), data[pos + 16] as u32, data[pos + 17]);
            if color == 3 {
                palette = Some((w, h));
            } else {
                let channels = match color {
                    0 => 1,
                    2 => 3,
                    4 => 2,
                    6 => 4,
                    _ => 0,
                };
                return Some(PictureInfo { mime: "image/png", width: w, height: h, depth: bits * channels, colors: 0 });
            }
        } else if let (Some((w, h)), b"PLTE") = (palette, kind) {
            return Some(PictureInfo { mime: "image/png", width: w, height: h, depth: 24, colors: (clen / 3) as u32 });
        }
        if clen + 12 > data.len() - pos {
            return None;
        }
        pos += 12 + clen;
    }
    None
}

const SOF: [u8; 13] = [0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF];

fn jpeg_info(data: &[u8]) -> Option<PictureInfo> {
    if !data.starts_with(b"\xff\xd8") {
        return None;
    }
    let n = data.len();
    let mut pos = 2usize;
    loop {
        while pos < n && data[pos] != 0xFF {
            pos += 1;
        }
        while pos < n && data[pos] == 0xFF {
            pos += 1;
        }
        if pos >= n || data[pos] == 0xDA || data[pos] == 0xD9 {
            return None;
        }
        let marker = data[pos];
        pos += 1;
        if n - pos < 2 {
            return None;
        }
        let seg = ((data[pos] as usize) << 8) | data[pos + 1] as usize;
        if SOF.contains(&marker) {
            if seg < 8 || n - pos < seg {
                return None;
            }
            let h = ((data[pos + 3] as u32) << 8) | data[pos + 4] as u32;
            let w = ((data[pos + 5] as u32) << 8) | data[pos + 6] as u32;
            let depth = data[pos + 2] as u32 * data[pos + 7] as u32;
            return Some(PictureInfo { mime: "image/jpeg", width: w, height: h, depth, colors: 0 });
        }
        if seg < 2 || n - pos < seg {
            return None;
        }
        pos += seg;
    }
}

/// The type, size and depth of a JPEG or PNG image, read from its header
/// (None for anything else, or a header that cannot be read).
pub fn image_info(data: &[u8]) -> Option<PictureInfo> {
    png_info(data).or_else(|| jpeg_info(data))
}

/// "JPEG", "PNG", "GIF", "BMP", "WebP", or "" for something else.
pub fn image_format(data: &[u8]) -> &'static str {
    if data.starts_with(b"\xff\xd8") {
        "JPEG"
    } else if data.starts_with(b"\x89PNG") {
        "PNG"
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "GIF"
    } else if data.starts_with(b"BM") {
        "BMP"
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        "WebP"
    } else {
        ""
    }
}

/// Where the cover shown for some songs comes from, and the picture as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct CoverSource {
    /// "embedded" (inside a song) or "folder" (an image file next to it)
    pub kind: &'static str,
    /// the song, or the image file
    pub path: String,
    pub data: Vec<u8>,
    /// `image_format()`
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
}

impl CoverSource {
    pub fn mime(&self) -> &'static str {
        match self.format {
            "JPEG" => "image/jpeg",
            "PNG" => "image/png",
            "GIF" => "image/gif",
            "BMP" => "image/bmp",
            "WebP" => "image/webp",
            _ => "application/octet-stream",
        }
    }

    /// The file extension to save the picture with (".jpg", ".png" ...).
    pub fn ext(&self) -> &'static str {
        match self.format {
            "JPEG" => ".jpg",
            "PNG" => ".png",
            "GIF" => ".gif",
            "BMP" => ".bmp",
            "WebP" => ".webp",
            _ => ".img",
        }
    }

    /// "Inside 01 Song.flac" or "cover.jpg in the album's folder".
    pub fn where_text(&self) -> String {
        where_text(self.kind, &self.path)
    }

    /// "1,200 × 1,200 pixels · JPEG · 245.3 KB".
    pub fn details(&self) -> String {
        details_text(self.width, self.height, self.format, self.data.len())
    }
}

/// "Inside 01 Song.flac" or "cover.jpg in the album's folder" (Python `where_text`).
pub fn where_text(kind: &str, path: &str) -> String {
    let name = crate::paths::file_name(path);
    if kind == "embedded" { format!("Inside {name}") } else { format!("{name} in the album's folder") }
}

/// "1,200 × 1,200 pixels · JPEG · 245.3 KB" (Python `details_text`).
pub fn details_text(width: u32, height: u32, format: &str, size: usize) -> String {
    use crate::stats::thousands;
    let mut parts = vec![];
    if width > 0 && height > 0 {
        parts.push(format!("{} × {} pixels", thousands(width as i64), thousands(height as i64)));
    }
    parts.push(if format.is_empty() { "Unknown format".to_string() } else { format.to_string() });
    parts.push(crate::fix::fmt_bytes(size as f64));
    parts.join(" · ")
}

/// Run FFmpeg (or ffprobe) with a 30 s limit; its stdout when it succeeded.
fn run_output(mut cmd: std::process::Command, input: Option<&[u8]>) -> Option<Vec<u8>> {
    let mut child = cmd
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(data), Some(mut stdin)) = (input.map(|d| d.to_vec()), child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut stdin, &data);
        });
    }
    let mut out = child.stdout.take()?;
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
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    };
    let data = reader.join().ok()?;
    status.success().then_some(data)
}

/// The first picture inside a song as it is stored (FFmpeg's first video
/// stream, which `cover_png()` shows), or empty.
fn embedded_picture(path: &str) -> Vec<u8> {
    let mut cmd = command(tool("ffmpeg"));
    cmd.args(["-v", "error", "-nostdin", "-i", path, "-map", "0:v:0", "-c", "copy", "-frames:v", "1", "-f", "image2pipe", "-"]);
    run_output(cmd, None).unwrap_or_default()
}

/// Width and height of the first picture of a file, by ffprobe (images that
/// are not JPEG or PNG).
fn probe_size(path: &str) -> (u32, u32) {
    let mut cmd = command(tool("ffprobe"));
    cmd.args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", path]);
    let Some(out) = run_output(cmd, None) else { return (0, 0) };
    let text = String::from_utf8_lossy(&out);
    let mut it = text.trim().split(',');
    match (it.next().and_then(|w| w.trim().parse().ok()), it.next().and_then(|h| h.trim().parse().ok())) {
        (Some(w), Some(h)) => (w, h),
        _ => (0, 0),
    }
}

fn source(kind: &'static str, path: &str, data: Vec<u8>) -> CoverSource {
    let (width, height) = match image_info(&data) {
        Some(i) => (i.width, i.height),
        None => probe_size(path),
    };
    CoverSource { kind, path: path.to_string(), format: image_format(&data), data, width, height }
}

/// Where the cover that `cover_png()` shows for these songs comes from: the
/// picture inside the first of the first three songs that has one, else the
/// folder image, with the picture's bytes, size and format (Python
/// `cover_source`).  None when there is no cover.
pub fn cover_source(tracks: &[String]) -> Option<CoverSource> {
    for t in tracks.iter().take(3) {
        let data = embedded_picture(t);
        if !data.is_empty() {
            return Some(source("embedded", t, data));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for t in tracks {
        let folder = parent(t);
        if !seen.insert(folder.clone()) {
            continue;
        }
        if let Some(img) = folder_image(&folder) {
            if let Ok(data) = std::fs::read(&img) {
                if !data.is_empty() {
                    return Some(source("folder", &img, data));
                }
            }
        }
    }
    None
}

/// The picture as a PNG that fits into size × size (aspect kept; a smaller one
/// is enlarged only with `enlarge`), made by FFmpeg, for showing it (Python
/// `fit_png`).
pub fn fit_png(data: &[u8], size: u32, enlarge: bool) -> Option<Vec<u8>> {
    let vf = if enlarge {
        format!("scale=w={size}:h={size}:force_original_aspect_ratio=decrease")
    } else {
        format!("scale=w='min({size},iw)':h='min({size},ih)':force_original_aspect_ratio=decrease")
    };
    let mut cmd = command(tool("ffmpeg"));
    cmd.args(["-v", "error", "-nostdin", "-i", "pipe:0", "-frames:v", "1", "-vf", &vf, "-f", "image2pipe", "-c:v", "png", "-"]);
    run_output(cmd, Some(data)).filter(|d| d.starts_with(b"\x89PNG"))
}

/// Where "Also save it as cover.jpg in the album's folder" writes: cover.jpg,
/// or cover.png for a PNG, keeping the spelling of one already there.
pub fn folder_cover_name(folder: &str, data: &[u8]) -> String {
    let name = if image_format(data) == "PNG" { "cover.png" } else { "cover.jpg" };
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    let chosen = names.into_iter().find(|n| n.to_lowercase() == name).unwrap_or_else(|| name.to_string());
    std::path::Path::new(folder).join(chosen).to_string_lossy().into_owned()
}

/// Write the picture as cover.jpg (cover.png) into the folder, through a
/// temporary file; returns its path (Python `save_folder_cover`).
pub fn save_folder_cover(folder: &str, data: &[u8]) -> std::io::Result<String> {
    let path = folder_cover_name(folder, data);
    crate::embed::replace_file(&path, data)?;
    Ok(path)
}
