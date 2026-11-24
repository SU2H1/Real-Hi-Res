//! Output: terminal table, JSON/CSV export and SVG spectrum plots (the Rust
//! counterpart of `realhires/report.py`).

use std::io::Write;
use std::path::Path;

use sha1::{Digest, Sha1};

use crate::json::{self, Style, Value};
use crate::paths::{file_name, relpath, stem};
use crate::pyfmt::{f, fs, g, pct, round_int};
use crate::verdict::{saved_note, verdict_breakdown, FileResult, Verdict};

fn color_code(v: Verdict) -> &'static str {
    match v {
        Verdict::Real => "32",
        Verdict::Fake => "31",
        Verdict::FakeLossless => "1;31",
        Verdict::Suspect => "33",
        Verdict::NotHires => "36",
        Verdict::Lossy => "35",
        Verdict::Error => "90",
    }
}

pub const ARROW: &str = "→";
pub const BAR: &str = "█";

pub fn use_color(is_tty: bool, force_off: bool) -> bool {
    !force_off && is_tty && std::env::var_os("NO_COLOR").is_none()
}

pub fn paint(text: &str, v: Verdict, color: bool) -> String {
    if color {
        format!("\x1b[{}m{text}\x1b[0m", color_code(v))
    } else {
        text.to_string()
    }
}

fn bw(r: &FileResult) -> String {
    match r.bandwidth_hz {
        Some(b) if b != 0.0 => format!("{}k", f(b / 1000.0, 1)),
        _ => "-".into(),
    }
}

pub fn header() -> String {
    format!("{:<14} {:<10} {:<11} {:<9}  FILE", "VERDICT", "CLAIMED", "REAL(EST)", "BANDWIDTH")
}

pub fn row(r: &FileResult, color: bool, base: Option<&str>, verbose: bool) -> String {
    let name = match base {
        Some(b) => relpath(&r.path, b),
        None => r.path.clone(),
    };
    let mut line = paint(&format!("{:<14}", r.verdict.as_str()), r.verdict, color)
        + &format!(" {:<10} {:<11} {:<9}  {name}", r.claimed(), r.estimated(), bw(r));
    let pad = format!("\n{:<15}", "");
    if !r.summary.is_empty() && (verbose || r.is_problem() || r.verdict == Verdict::Error) {
        line += &format!("{pad}{ARROW} {}", r.summary);
    }
    if verbose {
        for reason in &r.reasons {
            line += &format!("{pad}- {reason}");
        }
        for note in &r.notes {
            line += &format!("{pad}· {note}");
        }
        if !r.bit_usage.is_empty() {
            let top: Vec<String> =
                r.bit_usage.iter().take(3).map(|(b, v)| format!("{b}-bit LSB {}", pct(*v, 1))).collect();
            line += &format!("{pad}· Lowest-set-bit usage: {}", top.join(", "));
        }
        if !r.cliffs_db.is_empty() {
            let cl: Vec<String> = r
                .cliffs_db
                .iter()
                .filter(|(_, v)| v.is_finite())
                .map(|(k, v)| {
                    let d = crate::analysis::imap_get(&r.deep_cliffs_db, *k).unwrap_or(f64::NAN);
                    format!("{}k:{}/{}dB", g(*k as f64 / 2000.0), f(*v, 0), f(d, 0))
                })
                .collect();
            line += &format!("{pad}· Edge drop at candidate source Nyquists (narrow/deep): {}", cl.join(", "));
        }
        if let Some(c) = r.converter_floor_db.filter(|c| c.is_finite()) {
            line += &format!("{pad}· 26-40 kHz band: {} dB relative to the rounding noise", fs(c, 0));
        }
        if let Some(step) = r.grid_step.filter(|s| *s != 0.0) {
            line += &format!(
                "{pad}· Value grid: step {} (24-bit LSBs), {}-bit master, gain {} dB",
                f(step, 1),
                r.grid_bits.map(|b| b.to_string()).unwrap_or_else(|| "None".into()),
                fs(r.grid_gain_db.unwrap_or(f64::NAN), 1)
            );
        }
        if r.saved.is_some() {
            line += &format!("{pad}· {}", saved_note(r));
        }
    }
    line
}

/// "Scanned 20 file(s): 5 fake hi-res (25.0%), ...".
pub fn summary(results: &[FileResult], color: bool) -> String {
    let parts: Vec<String> = verdict_breakdown(results)
        .into_iter()
        .map(|(v, n, p)| paint(&format!("{n} {} ({}%)", v.as_str().to_lowercase(), f(p, 1)), v, color))
        .collect();
    format!("Scanned {} file(s): {}", results.len(), parts.join(", "))
}

pub fn summary_bars(results: &[FileResult], color: bool, width: usize) -> String {
    verdict_breakdown(results)
        .into_iter()
        .map(|(v, n, p)| {
            let len = (if n > 0 { 1 } else { 0 }).max(round_int(p / 100.0 * width as f64) as usize);
            let bar = BAR.repeat(len);
            format!(
                "  {} {:>5}%  {n:>5}  {}",
                paint(&format!("{:<14}", v.as_str()), v, color),
                f(p, 1),
                paint(&bar, v, color)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn write_json(results: &[FileResult], path: &str) -> std::io::Result<()> {
    let v = Value::Array(results.iter().map(FileResult::to_dict).collect());
    std::fs::write(path, json::dumps(&v, Style::PRETTY))
}

pub const CSV_FIELDS: [&str; 17] = [
    "path",
    "verdict",
    "summary",
    "flags",
    "claimed",
    "estimated_real",
    "codec",
    "sample_rate",
    "claimed_bits",
    "effective_bits",
    "bandwidth_hz",
    "upsampled_from",
    "lossy_cutoff_hz",
    "cutoff_drop_db",
    "grid_step",
    "duration",
    "reasons",
];

fn csv_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Int(i) => i.to_string(),
        Value::Float(x) => crate::pyfmt::repr_float(*x),
        Value::Str(s) => s.clone(),
        other => json::dumps(other, Style::DEFAULT),
    }
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn write_csv(results: &[FileResult], path: &str) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str(&CSV_FIELDS.join(","));
    out.push_str("\r\n");
    for r in results {
        let d = r.to_dict();
        let cells: Vec<String> = CSV_FIELDS
            .iter()
            .map(|k| match *k {
                "reasons" => r.reasons.join(" | "),
                "flags" => r.flags.join(" "),
                k => d.get(k).map(csv_cell).unwrap_or_default(),
            })
            .map(|c| csv_quote(&c))
            .collect();
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    std::fs::write(path, out)
}

/// `re.sub(r"[^\w.-]+", "_", stem)[:120] or "file"`.
pub fn safe_stem(path: &str) -> String {
    let mut out = String::new();
    let mut in_bad = false;
    for c in stem(path).chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' {
            out.push(c);
            in_bad = false;
        } else if !in_bad {
            out.push('_');
            in_bad = true;
        }
    }
    let s: String = out.chars().take(120).collect();
    if s.is_empty() {
        "file".into()
    } else {
        s
    }
}

pub fn short_hash(path: &str) -> String {
    let d = Sha1::digest(path.as_bytes());
    d.iter().take(3).map(|b| format!("{b:02x}")).collect()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

/// Spectrum plot (no dependencies).  Returns the written path.
pub fn write_svg(r: &FileResult, out_dir: &str) -> std::io::Result<Option<String>> {
    let Some(s) = r.spectrum.as_deref() else {
        return Ok(None);
    };
    std::fs::create_dir_all(out_dir)?;
    let (w, h, l, rr, t, b) = (960.0, 420.0, 64.0, 20.0, 46.0, 46.0);
    let (pw, ph) = (w - l - rr, h - t - b);
    let (_, ys) = crate::analysis::max_hold(s, pw as usize);
    let top = ((s.peak_db + 5.0) / 10.0).ceil() * 10.0;
    let bottom = (((s.noise_floor_db.min(s.threshold_db)) - 15.0) / 10.0).floor() * 10.0;
    let bottom = bottom.max(top - 200.0);
    let x_of = |freq: f64| l + pw * freq / s.nyquist;
    let y_of = |level: f64| t + ph * (top - level.clamp(bottom, top)) / (top - bottom);
    let pts: Vec<String> = ys
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > -299.0)
        .map(|(i, v)| format!("{},{}", f(l + i as f64 + 0.5, 1), f(y_of(*v), 1)))
        .collect();
    let color = match r.verdict {
        Verdict::Real => "#1a7f37",
        Verdict::Fake => "#cf222e",
        Verdict::FakeLossless => "#a40e26",
        Verdict::Suspect => "#9a6700",
        _ => "#0969da",
    };
    let mut out = vec![
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" \
             font-family=\"Helvetica,Arial,sans-serif\" font-size=\"12\">"
        ),
        format!("<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>"),
        format!("<text x=\"{l}\" y=\"20\" font-size=\"14\" font-weight=\"bold\">{}</text>", esc(&file_name(&r.path))),
        format!(
            "<text x=\"{l}\" y=\"37\" fill=\"{color}\" font-weight=\"bold\">{} <tspan fill=\"#555\" \
             font-weight=\"normal\">claimed {}, estimated real {}, bandwidth {}</tspan></text>",
            esc(r.verdict.as_str()),
            esc(&r.claimed()),
            esc(&r.estimated()),
            bw(r)
        ),
    ];
    let step = if top - bottom <= 120.0 { 10.0 } else { 20.0 };
    let mut lvl = top;
    while lvl >= bottom {
        let y = y_of(lvl);
        out.push(format!("<line x1=\"{l}\" x2=\"{}\" y1=\"{y:.1}\" y2=\"{y:.1}\" stroke=\"#eee\"/>", l + pw));
        out.push(format!(
            "<text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#666\">{}</text>",
            l - 6.0,
            y + 4.0,
            f(lvl, 0)
        ));
        lvl -= step;
    }
    let khz_step = if s.nyquist <= 24000.0 {
        2
    } else if s.nyquist <= 50000.0 {
        5
    } else if s.nyquist <= 100000.0 {
        10
    } else {
        20
    };
    let mut k = 0;
    while (k * 1000) as f64 <= s.nyquist {
        let x = x_of((k * 1000) as f64);
        out.push(format!("<line x1=\"{x:.1}\" x2=\"{x:.1}\" y1=\"{t}\" y2=\"{}\" stroke=\"#eee\"/>", t + ph));
        out.push(format!(
            "<text x=\"{x:.1}\" y=\"{}\" text-anchor=\"middle\" fill=\"#666\">{k}k</text>",
            t + ph + 16.0
        ));
        k += khz_step;
    }
    out.push(format!(
        "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" fill=\"#333\">Frequency (Hz)</text>",
        l + pw / 2.0,
        h - 6.0
    ));
    out.push(format!(
        "<text x=\"14\" y=\"{0}\" transform=\"rotate(-90 14 {0})\" text-anchor=\"middle\" fill=\"#333\">Level (dB, \
         averaged)</text>",
        t + ph / 2.0
    ));
    for (i, rf) in [22050.0, 24000.0, 44100.0, 48000.0].iter().enumerate() {
        if *rf < s.nyquist * 0.9 {
            let x = x_of(*rf);
            out.push(format!(
                "<line x1=\"{x:.1}\" x2=\"{x:.1}\" y1=\"{t}\" y2=\"{}\" stroke=\"#999\" stroke-dasharray=\"2,3\"/>",
                t + ph
            ));
            out.push(format!(
                "<text x=\"{:.1}\" y=\"{}\" fill=\"#999\">{}k</text>",
                x + 3.0,
                t + ph - 6.0 - 14.0 * (i % 2) as f64,
                g(rf / 1000.0)
            ));
        }
    }
    if let Some(bwz) = r.bandwidth_hz.filter(|b| *b != 0.0) {
        let x = x_of(bwz);
        out.push(format!(
            "<line x1=\"{x:.1}\" x2=\"{x:.1}\" y1=\"{t}\" y2=\"{}\" stroke=\"{color}\" stroke-dasharray=\"6,4\" \
             stroke-width=\"1.5\"/>",
            t + ph
        ));
        let (anchor, dx) = if x > l + pw - 140.0 { ("end", -4.0) } else { ("start", 4.0) };
        out.push(format!(
            "<text x=\"{:.1}\" y=\"{}\" text-anchor=\"{anchor}\" fill=\"{color}\">bandwidth {}</text>",
            x + dx,
            t + 16.0,
            bw(r)
        ));
    }
    let yf = y_of(s.noise_floor_db);
    out.push(format!(
        "<line x1=\"{l}\" x2=\"{}\" y1=\"{yf:.1}\" y2=\"{yf:.1}\" stroke=\"#8250df\" stroke-dasharray=\"4,4\"/>",
        l + pw
    ));
    out.push(format!(
        "<text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#8250df\">top-band floor {} dB</text>",
        l + pw - 4.0,
        yf - 4.0,
        f(s.noise_floor_db, 0)
    ));
    out.push(format!("<polyline fill=\"none\" stroke=\"#1f2328\" stroke-width=\"1.2\" points=\"{}\"/>", pts.join(" ")));
    out.push(format!("<rect x=\"{l}\" y=\"{t}\" width=\"{pw}\" height=\"{ph}\" fill=\"none\" stroke=\"#999\"/>"));
    out.push("</svg>".into());
    let path = Path::new(out_dir).join(format!("{}_{}.svg", safe_stem(&r.path), short_hash(&r.path)));
    let mut fh = std::fs::File::create(&path)?;
    fh.write_all(out.join("\n").as_bytes())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
