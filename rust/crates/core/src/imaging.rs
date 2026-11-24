//! Spectrogram rendering without image libraries (the counterpart of
//! `realhires/imaging.py`): colour maps, RGB rendering, a minimal PNG encoder
//! and the CLI's labelled `--spectrogram` PNG.

use std::io::Write;

use crate::analysis::Spectrogram;
use crate::dsp::percentile_sorted;
use crate::pyfmt::{f, g, rjust, round_int};

const INFERNO: &[(f64, [u8; 3])] = &[
    (0.0, [0, 0, 4]),
    (0.15, [31, 12, 72]),
    (0.3, [85, 15, 109]),
    (0.45, [136, 34, 106]),
    (0.6, [186, 54, 85]),
    (0.75, [227, 89, 51]),
    (0.87, [249, 140, 10]),
    (1.0, [252, 255, 164]),
];
const MAGMA: &[(f64, [u8; 3])] = &[
    (0.0, [0, 0, 4]),
    (0.2, [44, 17, 95]),
    (0.4, [114, 31, 129]),
    (0.6, [183, 55, 121]),
    (0.8, [251, 135, 97]),
    (1.0, [252, 253, 191]),
];
const VIRIDIS: &[(f64, [u8; 3])] =
    &[(0.0, [68, 1, 84]), (0.25, [59, 82, 139]), (0.5, [33, 145, 140]), (0.75, [94, 201, 98]), (1.0, [253, 231, 37])];
const GRAY: &[(f64, [u8; 3])] = &[(0.0, [0, 0, 0]), (1.0, [255, 255, 255])];

/// 256-entry RGB lookup table of a colour map (inferno when unknown).
pub fn colormap(name: &str) -> Vec<[u8; 3]> {
    let anchors = match name {
        "magma" => MAGMA,
        "viridis" => VIRIDIS,
        "gray" => GRAY,
        _ => INFERNO,
    };
    (0..256)
        .map(|i| {
            let x = i as f64 / 255.0;
            let j = anchors.iter().position(|(p, _)| *p >= x).unwrap_or(anchors.len() - 1).max(1);
            let (p0, c0) = anchors[j - 1];
            let (p1, c1) = anchors[j];
            let t = if p1 > p0 { ((x - p0) / (p1 - p0)).clamp(0.0, 1.0) } else { 0.0 };
            let mut out = [0u8; 3];
            for k in 0..3 {
                out[k] = (c0[k] as f64 + (c1[k] as f64 - c0[k] as f64) * t).round_ties_even() as u8;
            }
            out
        })
        .collect()
}

/// An RGB image, row-major.
#[derive(Clone)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Rgb {
    pub fn new(width: usize, height: usize, fill: u8) -> Self {
        Rgb { width, height, data: vec![fill; width * height * 3] }
    }
    pub fn set(&mut self, x: usize, y: usize, c: [u8; 3]) {
        if x < self.width && y < self.height {
            let i = (y * self.width + x) * 3;
            self.data[i..i + 3].copy_from_slice(&c);
        }
    }
}

/// Render a compact spectrogram (time left to right, 0 Hz at the bottom); the
/// colour scale spans `range_db` below the loudest cells (99.9th percentile).
pub fn spectrogram_rgb(sg: &Spectrogram, width: usize, height: usize, range_db: f64, cmap: &str) -> Option<Rgb> {
    let db = sg.decode()?;
    let (t, fq) = sg.shape;
    if t == 0 || fq == 0 || width == 0 || height == 0 {
        return None;
    }
    let mut sorted: Vec<f64> = db.iter().map(|v| *v as f64).collect();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let top_db = percentile_sorted(&sorted, 99.9);
    let lo = top_db - range_db;
    let lut = colormap(cmap);
    let mut img = Rgb::new(width, height, 0);
    let fi: Vec<usize> = (0..height).map(|y| (((height - 1 - y) * fq) / height).min(fq - 1)).collect();
    for x in 0..width {
        let ti = ((x * t) / width).min(t - 1);
        for (y, fidx) in fi.iter().enumerate() {
            let v = db[ti * fq + fidx] as f64;
            let norm = ((v - lo) / range_db.max(1e-6)).clamp(0.0, 1.0);
            img.set(x, y, lut[(norm * 255.0) as usize]);
        }
    }
    Some(img)
}

/// Minimal PNG encoder for RGB images.
pub fn to_png(img: &Rgb) -> Vec<u8> {
    let mut raw = Vec::with_capacity((img.width * 3 + 1) * img.height);
    for y in 0..img.height {
        raw.push(0);
        raw.extend_from_slice(&img.data[y * img.width * 3..(y + 1) * img.width * 3]);
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    let _ = enc.write_all(&raw);
    let idat = enc.finish().unwrap_or_default();
    let chunk = |out: &mut Vec<u8>, tag: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = crc32fast::Hasher::new();
        crc.update(tag);
        crc.update(data);
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc.finalize().to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(img.width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(img.height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", b"");
    out
}

fn glyph(ch: char) -> Option<&'static str> {
    Some(match ch {
        '0' => "111101101101111",
        '1' => "010110010010111",
        '2' => "111001111100111",
        '3' => "111001111001111",
        '4' => "101101111001001",
        '5' => "111100111001111",
        '6' => "111100111101111",
        '7' => "111001001001001",
        '8' => "111101111101111",
        '9' => "111101111001111",
        'k' => "100101110101101",
        's' => "000011010001110",
        '.' => "000000000000010",
        ' ' => "000000000000000",
        'H' => "101101111101101",
        'z' => "000111001010111",
        _ => return None,
    })
}

/// Draw text with the tiny 3x5 bitmap font, scaled by `scale`.
pub fn draw_text(img: &mut Rgb, x: i64, y: i64, text: &str, color: [u8; 3], scale: i64) {
    for (i, ch) in text.chars().enumerate() {
        let Some(bits) = glyph(ch) else { continue };
        let x0 = x + i as i64 * (3 * scale + scale);
        if x0 >= img.width as i64 || y >= img.height as i64 || x0 < 0 || y < 0 {
            continue;
        }
        for (k, b) in bits.chars().enumerate() {
            if b != '1' {
                continue;
            }
            let (gy, gx) = ((k / 3) as i64, (k % 3) as i64);
            for dy in 0..scale {
                for dx in 0..scale {
                    let (px, py) = (x0 + gx * scale + dx, y + gy * scale + dy);
                    if px >= 0 && py >= 0 {
                        img.set(px as usize, py as usize, color);
                    }
                }
            }
        }
    }
}

/// PNG with frequency gridlines and labels; `markers` = [(hz, rgb)] lines.
pub fn spectrogram_png(
    sg: &Spectrogram,
    width: usize,
    height: usize,
    range_db: f64,
    cmap: &str,
    markers: &[(f64, [u8; 3])],
) -> Option<Vec<u8>> {
    let (left, bottom) = (46usize, 22usize);
    let (pw, ph) = (width - left - 6, height - bottom - 6);
    let mut img = Rgb::new(width, height, 24);
    let sp = spectrogram_rgb(sg, pw, ph, range_db, cmap)?;
    for y in 0..ph {
        let src = &sp.data[y * pw * 3..(y + 1) * pw * 3];
        let i = ((6 + y) * width + left) * 3;
        img.data[i..i + pw * 3].copy_from_slice(src);
    }
    let nyq = sg.nyquist;
    let step = if nyq <= 24000.0 {
        2000.0
    } else if nyq <= 50000.0 {
        5000.0
    } else if nyq <= 100000.0 {
        10000.0
    } else {
        20000.0
    };
    let mut fr = 0.0;
    while fr <= nyq {
        let y = (6 + ph - 1) as i64 - round_int(fr / nyq * (ph - 1) as f64);
        for x in left - 4..left {
            img.set(x, y as usize, [180, 180, 180]);
        }
        draw_text(&mut img, 2, (y - 5).max(0), &rjust(&format!("{}k", g(fr / 1000.0)), 4), [200, 200, 200], 2);
        fr += step;
    }
    for (hz, color) in markers {
        if 0.0 < *hz && *hz < nyq {
            let y = (6 + ph - 1) as i64 - round_int(hz / nyq * (ph - 1) as f64);
            for x in (left..left + pw).step_by(3) {
                img.set(x, y as usize, *color);
            }
        }
    }
    let total = sg.shape.0 as f64 * sg.seconds_per_column;
    for frac in [0.0, 0.25, 0.5, 0.75] {
        let x = left + (frac * (pw - 1) as f64) as usize;
        for y in 6 + ph..6 + ph + 4 {
            img.set(x, y, [180, 180, 180]);
        }
        let label = if total >= 20.0 { format!("{}s", f(frac * total, 0)) } else { format!("{}s", f(frac * total, 1)) };
        draw_text(&mut img, x as i64 + 2, (6 + ph + 6) as i64, &label, [200, 200, 200], 2);
    }
    Some(to_png(&img))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colormap_endpoints() {
        let lut = colormap("inferno");
        assert_eq!(lut[0], [0, 0, 4]);
        assert_eq!(lut[255], [252, 255, 164]);
        assert_eq!(colormap("gray")[128], [128, 128, 128]);
    }
}
