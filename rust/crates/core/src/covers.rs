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
