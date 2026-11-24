//! "Fix": correct a fake file so its properties show the resolution it really
//! has (the counterpart of `realhires/fix.py`, with the same plans, messages,
//! exact restore of whole-number-ratio upsamples and gapless joins).
//!
//! The file is rewritten at its real resolution (editing only the header
//! values would change the playback speed).  By default it is replaced in
//! place, keeping name, tags and cover art, and the original is moved to a
//! "Real-Hi-Res backups" folder next to it.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::analysis::{detection_params, imap_get};
use crate::json::{self, Value};
use crate::library::track_key;
use crate::paths::{abspath, extension, file_name, parent, BACKUP_DIR_NAME};
use crate::probe::{command, read_all, tool, PcmReader};
use crate::pyfmt::{f, g, pct};
use crate::settings::Settings;
use crate::verdict::{analyze_file, fmt_rate_u, FileResult, Verdict};

pub const FIXABLE_VERDICTS: [Verdict; 2] = [Verdict::Fake, Verdict::FakeLossless];

/// Audio of the neighbouring track fed to the resampler on each side of a join.
pub const CONTEXT_FRAMES: usize = 8192;
/// -70 dBFS on the int32 scale: quieter joins need no neighbour.
pub const SILENT_JOIN: i64 = 679093;
/// Non-zero samples per phase needed to prove an untouched master.
pub const EXACT_MIN_SAMPLES: i64 = 20000;

/// input extension -> (output extension, codec 16 bit, codec 24 bit, sample_fmt 16, sample_fmt 24)
type Format = (&'static str, &'static str, &'static str, Option<&'static str>, Option<&'static str>);
const FLAC: Format = (".flac", "flac", "flac", Some("s16"), Some("s32"));

fn same_format(ext: &str) -> Option<Format> {
    Some(match ext {
        ".flac" => FLAC,
        ".wav" | ".wave" => (".wav", "pcm_s16le", "pcm_s24le", None, None),
        ".w64" => (".w64", "pcm_s16le", "pcm_s24le", None, None),
        ".aif" => (".aif", "pcm_s16be", "pcm_s24be", None, None),
        ".aiff" | ".aifc" => (".aiff", "pcm_s16be", "pcm_s24be", None, None),
        ".m4a" | ".alac" => (".m4a", "alac", "alac", Some("s16p"), Some("s32p")),
        ".caf" => (".caf", "alac", "alac", Some("s16p"), Some("s32p")),
        ".wv" => (".wv", "wavpack", "wavpack", Some("s16p"), Some("s32p")),
        _ => return None,
    })
}

const KEEPS_COVER: [&str; 2] = [".flac", ".m4a"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Backup {
    Folder,
    Bak,
    None,
}

#[derive(Clone, Debug)]
pub struct FixPlan {
    pub source: String,
    pub target_rate: u32,
    pub target_bits: u32,
    pub resample: bool,
    pub output: String,
    pub codec: String,
    pub sample_fmt: Option<String>,
    pub replace_original: bool,
    /// "24/96 → 16/44.1"
    pub description: String,
    pub backup: Backup,
    pub backup_dir: Option<String>,
    pub notes: Vec<String>,
    /// dither a 24->16 reduction that is not resampled
    pub dither: bool,
    pub channels: u32,
    pub source_rate: u32,
    /// >1: try restoring the master by keeping every Nth sample
    pub exact_ratio: u32,
    /// exact length of the source (gapless joins)
    pub frames: Option<i64>,
    /// start of the source on its gapless run, in source samples
    pub offset: i64,
    /// end of the previous track (interleaved int32)
    pub context_before: Option<Vec<i32>>,
    /// start of the next track
    pub context_after: Option<Vec<i32>>,
}

#[derive(Clone, Debug)]
pub struct FixOutcome {
    pub plan: FixPlan,
    pub ok: bool,
    pub message: String,
    pub output: Option<String>,
    pub new_result: Option<FileResult>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// "exact", "gapless" or "" (plain conversion)
    pub how: String,
}

fn family_rate(rate: u32) -> u32 {
    if rate % 11025 == 0 {
        44100
    } else {
        48000
    }
}

fn cut_is_safe(r: &FileResult, rate: u32, cliff: f64, deep_cliff: f64) -> bool {
    let narrow = imap_get(&r.cliffs_db, rate).unwrap_or(f64::NEG_INFINITY);
    let deep = imap_get(&r.deep_cliffs_db, rate).unwrap_or(f64::NEG_INFINITY);
    if !(narrow >= cliff || deep >= deep_cliff) {
        return false;
    }
    !matches!(r.upsample_mid_hz, Some(m) if !m.is_nan() && m > 1.02 * rate as f64 / 2.0)
}

/// The rate Fix may convert an upsampled file to without cutting real content:
/// the detected source rate when the evidence there is clear, else the next
/// higher safe candidate, else the file's own rate.
pub fn safe_source_rate(r: &FileResult, cfg: Option<&Settings>) -> u32 {
    let c = detection_params(cfg);
    let Some(rate) = r.upsampled_from.filter(|x| *x != 0) else {
        return r.sample_rate;
    };
    if cut_is_safe(r, rate, c.cliff, c.deep_cliff) {
        return rate;
    }
    let mut cands: Vec<u32> = r.cliffs_db.iter().chain(&r.deep_cliffs_db).map(|(k, _)| *k).collect();
    cands.sort();
    cands.dedup();
    cands
        .into_iter()
        .find(|cand| rate < *cand && *cand < r.sample_rate && cut_is_safe(r, *cand, c.cliff, c.deep_cliff))
        .unwrap_or(r.sample_rate)
}

/// (rate, bits, notes) the file really has, or None when unknown.
pub fn target_resolution(r: &FileResult, cfg: &Settings) -> Option<(u32, u32, Vec<String>)> {
    let eff = r.effective_bits?;
    if r.sample_rate == 0 {
        return None;
    }
    let mut notes = vec![];
    let up = r.upsampled_from.filter(|x| *x != 0);
    let mut rate = up.unwrap_or(r.sample_rate);
    let mut cd_sourced = up.is_some_and(|u| u <= 48000);
    if up.is_some() && r.verdict == Verdict::Fake {
        let safe = safe_source_rate(r, Some(cfg));
        if safe != rate {
            notes.push(format!(
                "Kept {} kHz instead of the estimated {} kHz: the spectrum does not collapse clearly enough at {} kHz \
                 to cut there without losing content.",
                fmt_rate_u(safe),
                fmt_rate_u(rate),
                g(rate as f64 / 2000.0)
            ));
            rate = safe;
        }
    }
    if r.verdict == Verdict::FakeLossless {
        cd_sourced = true;
        if rate > 48000 {
            rate = family_rate(rate);
        }
        notes.push(
            "Made from a lossy file: converting removes the fake hi-res part, but the result is still lossy-sourced."
                .into(),
        );
    }
    let mut bits = eff;
    if r.is_float && bits > 24 {
        bits = 24;
    }
    bits = if bits <= 16 { 16 } else { 24 };
    let mode = cfg.fix_bits.as_str();
    if mode == "Always 16-bit" || (mode == "16-bit for CD-sourced files" && cd_sourced) {
        if bits > 16 {
            notes.push("Reduced to 16-bit with dither (the source was a CD-resolution or lossy file).".into());
        }
        bits = 16;
    }
    Some((rate, bits, notes))
}

fn fmt(bits: u32, rate: u32) -> String {
    format!("{bits}/{}", fmt_rate_u(rate))
}

fn split_ext(path: &str) -> (String, String) {
    let ext = extension(&file_name(path));
    (path[..path.len() - ext.len()].to_string(), ext)
}

/// `path`, or "name (2).ext", "name (3).ext"... when it exists.
pub fn unique(path: &str) -> String {
    if !Path::new(path).exists() {
        return path.to_string();
    }
    let (stem, ext) = split_ext(path);
    let mut i = 2;
    while Path::new(&format!("{stem} ({i}){ext}")).exists() {
        i += 1;
    }
    format!("{stem} ({i}){ext}")
}

fn join(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

/// A plan, or the reason the file cannot or need not be fixed.
pub fn plan_fix(r: &FileResult, cfg: &Settings) -> Result<FixPlan, String> {
    if !FIXABLE_VERDICTS.contains(&r.verdict) {
        return Err("Only fake hi-res / fake lossless files need fixing.".into());
    }
    let Some((rate, bits, mut notes)) = target_resolution(r, cfg) else {
        return Err("The real resolution could not be determined.".into());
    };
    let cur_bits = if r.is_float { 32 } else { r.claimed_bits.filter(|b| *b != 0).unwrap_or(16) };
    if rate == r.sample_rate && bits >= cur_bits {
        if r.verdict == Verdict::FakeLossless {
            return Err(format!(
                "Already {}, the resolution it really has: there is nothing to convert, and the damage of the lossy \
                 source cannot be repaired.",
                fmt(bits, rate)
            ));
        }
        if let Some(kept) = notes.iter().find(|n| n.starts_with("Kept")) {
            return Err(format!("{kept} All its {bits} bits are used, so nothing would change."));
        }
        return Err(format!("Already at its real resolution ({}).", fmt(bits, rate)));
    }
    let ext = extension(&file_name(&r.path)).to_lowercase();
    let force_flac = cfg.fix_format == "FLAC";
    let format = if force_flac { FLAC } else { same_format(&ext).unwrap_or(FLAC) };
    let (out_ext, c16, c24, f16, f24) = format;
    let (codec, sfmt) = if bits == 16 { (c16, f16) } else { (c24, f24) };
    if !force_flac && same_format(&ext).is_none() {
        notes.push(format!(
            "{} cannot be written by FFmpeg; saving as FLAC.",
            if ext.is_empty() { "This format" } else { &ext }
        ));
    }
    let folder = parent(&r.path);
    let (stem, _) = split_ext(&file_name(&r.path));
    let replace = cfg.fix_output.starts_with("Replace");
    let (mut backup, mut backup_dir) = (Backup::Folder, None);
    let output;
    if replace {
        output = join(&folder, &format!("{stem}{out_ext}"));
        let b = cfg.fix_backup.as_str();
        backup = if b.starts_with("Keep") {
            Backup::Bak
        } else if b.starts_with("Delete") {
            Backup::None
        } else {
            Backup::Folder
        };
        if backup == Backup::Folder {
            backup_dir = Some(if cfg.fix_backup_folder.is_empty() {
                join(&folder, BACKUP_DIR_NAME)
            } else {
                cfg.fix_backup_folder.clone()
            });
        }
    } else {
        let suffix = cfg.fix_suffix.replace("{bits}", &bits.to_string()).replace("{rate}", &fmt_rate_u(rate));
        let out_dir = if cfg.fix_output.ends_with("into a folder") {
            if cfg.fix_folder.is_empty() {
                join(&folder, "Fixed")
            } else {
                cfg.fix_folder.clone()
            }
        } else {
            folder.clone()
        };
        output = unique(&join(&out_dir, &format!("{stem}{suffix}{out_ext}")));
    }
    // A 16-bit master scaled by a gain sits on a grid that is not the 16-bit one,
    // so rounding it back to 16 bits needs dither; an unscaled one rounds back exactly.
    let dither = bits == 16
        && rate == r.sample_rate
        && r.grid_step.is_some_and(|s| s != 0.0)
        && r.grid_gain_db.unwrap_or(0.0).abs() >= 0.05;
    if dither {
        notes.push(format!(
            "Reduced to 16-bit with dither (the 16-bit master had been scaled by {} dB).",
            crate::pyfmt::fs(r.grid_gain_db.unwrap_or(0.0), 1)
        ));
    }
    let exact = if rate != r.sample_rate && bits == 16 && rate != 0 && r.sample_rate % rate == 0 {
        r.sample_rate / rate
    } else {
        0
    };
    Ok(FixPlan {
        source: r.path.clone(),
        target_rate: rate,
        target_bits: bits,
        resample: rate != r.sample_rate,
        output,
        codec: codec.into(),
        sample_fmt: sfmt.map(str::to_string),
        replace_original: replace,
        description: format!("{} → {}", r.claimed(), fmt(bits, rate)),
        backup,
        backup_dir,
        notes,
        dither,
        channels: if r.channels == 0 { 2 } else { r.channels },
        source_rate: r.sample_rate,
        exact_ratio: if exact >= 2 { exact } else { 0 },
        frames: None,
        offset: 0,
        context_before: None,
        context_after: None,
    })
}

pub const SKIP_GROUPS: [(&str, &str); 5] = [
    (
        "lossy",
        "made from MP3/AAC but already at CD resolution: nothing to convert, and the lossy damage cannot be repaired",
    ),
    (
        "uncertain",
        "upsampled, but the source rate is too uncertain to cut without risking real content, and all bits are used",
    ),
    ("already", "already at the resolution they really have"),
    ("unknown", "real resolution could not be determined"),
    ("not-fake", "not fake"),
];

/// Key into SKIP_GROUPS for a file plan_fix() did not plan.
pub fn skip_group(r: &FileResult, why: &str) -> &'static str {
    if !FIXABLE_VERDICTS.contains(&r.verdict) {
        return "not-fake";
    }
    if why.contains("could not be determined") {
        return "unknown";
    }
    if r.verdict == Verdict::FakeLossless {
        return "lossy";
    }
    if why.starts_with("Kept") {
        "uncertain"
    } else {
        "already"
    }
}

/// "268 file(s) are not changed: 251 made from MP3/AAC but ...; 17 ..." ('' when nothing was skipped).
pub fn skip_summary(skipped: &[(FileResult, String)]) -> String {
    if skipped.is_empty() {
        return String::new();
    }
    let mut counts: Vec<(&str, usize)> = vec![];
    for (r, why) in skipped {
        let k = skip_group(r, why);
        match counts.iter_mut().find(|(key, _)| *key == k) {
            Some(c) => c.1 += 1,
            None => counts.push((k, 1)),
        }
    }
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    let parts: Vec<String> = counts
        .iter()
        .map(|(k, n)| format!("{n} {}", SKIP_GROUPS.iter().find(|(key, _)| key == k).unwrap().1))
        .collect();
    format!("{} file(s) are not changed: {}.", skipped.len(), parts.join("; "))
}

/// What Fix does with a fake file, without touching the disk: ("unknown" |
/// "same" | "convert", "bits/rate" target, the "Kept ..." note or ""), the
/// same decision plan_fix() makes; None for other verdicts.
fn prospect(r: &FileResult, cfg: &Settings) -> Option<(&'static str, String, String)> {
    if !FIXABLE_VERDICTS.contains(&r.verdict) {
        return None;
    }
    let Some((rate, bits, notes)) = target_resolution(r, cfg) else {
        return Some(("unknown", String::new(), String::new()));
    };
    let cur_bits = if r.is_float { 32 } else { r.claimed_bits.filter(|b| *b != 0).unwrap_or(16) };
    let kept = notes.iter().find(|n| n.starts_with("Kept")).cloned().unwrap_or_default();
    let same = rate == r.sample_rate && bits >= cur_bits;
    Some((if same { "same" } else { "convert" }, fmt(bits, rate), kept))
}

/// A few words for the "After fix" column of the desktop app's list: what Fix
/// turns the file into and whether it is still marked fake then; "" if not fake.
pub fn fix_outlook(r: &FileResult, cfg: &Settings) -> String {
    let Some((kind, target, kept)) = prospect(r, cfg) else {
        return String::new();
    };
    if kind == "unknown" {
        return "Not fixable: resolution unknown".into();
    }
    if r.verdict == Verdict::FakeLossless {
        return if kind == "same" {
            "Stays fake: MP3/AAC source".into()
        } else {
            format!("{target}, stays fake: MP3/AAC source")
        };
    }
    if !kept.is_empty() {
        return if kind == "same" {
            "Stays fake: rate kept".into()
        } else {
            format!("{target}, stays fake: rate kept")
        };
    }
    if kind == "same" {
        format!("Already {target}")
    } else {
        format!("Fixable to {target}")
    }
}

/// "Kept 96 kHz instead of ...: the spectrum ..." -> "keeps 96 kHz instead of ..., because the spectrum ...".
fn keeps(note: &str) -> String {
    let rest = note.strip_prefix("Kept ").unwrap_or(&note[note.len().min(5)..]);
    let rest = rest.replacen(": the spectrum", ", because the spectrum", 1);
    format!("keeps {}", rest.trim_end_matches('.'))
}

/// What "Fix to real resolution" does with a fake file, and whether it is
/// still marked fake afterwards (shown in the details, so a file that stays
/// fake after fixing and re-scanning says why); None for other verdicts.
pub fn fix_advice(r: &FileResult, cfg: &Settings) -> Option<String> {
    let (kind, target, kept) = prospect(r, cfg)?;
    let lossy = "because it was made from an MP3/AAC, and no conversion can bring back what the lossy \
                 encoder removed. Only a genuine lossless copy is better.";
    let fl = r.verdict == Verdict::FakeLossless;
    Some(match kind {
        "unknown" => {
            "Fix to real resolution leaves it unchanged, because its real resolution could not be determined.".into()
        }
        "same" if fl => format!(
            "Fix to real resolution leaves it unchanged: it is already {target}, the resolution it really has. \
             It stays marked fake lossless, also after fixing and re-scanning, {lossy}"
        ),
        "same" if !kept.is_empty() => {
            let bits = target.split('/').next().unwrap_or("");
            format!(
                "Fix to real resolution leaves it unchanged: it {}, and all its {bits} bits are used, so nothing \
                 would change. It stays marked fake hi-res, also after fixing and re-scanning.",
                keeps(&kept)
            )
        }
        "same" => format!("Fix to real resolution leaves it unchanged: it is already {target}."),
        _ if fl => format!(
            "Fix to real resolution converts it to {target}, the resolution it really has, but it stays marked \
             fake lossless even then, {lossy}"
        ),
        _ if !kept.is_empty() => format!(
            "Fix to real resolution converts it to {target}, but it {}. It therefore stays marked fake hi-res after \
             fixing and re-scanning.",
            keeps(&kept)
        ),
        _ => format!(
            "Fix to real resolution converts it to {target}, the resolution it really has; a re-scan then no longer \
             marks it fake."
        ),
    })
}

/// "1.2 GB", "350.4 MB", "812 bytes".
pub fn fmt_bytes(n: f64) -> String {
    let mut n = n;
    for unit in ["bytes", "KB", "MB", "GB"] {
        if n.abs() < 1024.0 {
            return if unit == "bytes" { format!("{} {unit}", f(n, 0)) } else { format!("{} {unit}", f(n, 1)) };
        }
        n /= 1024.0;
    }
    format!("{} TB", f(n, 1))
}

fn files(n: usize) -> String {
    format!("{n} file{}", if n != 1 { "s" } else { "" })
}

/// (headline, sentences) for the pop-up shown when fixing has finished.
/// `skipped` counts the fake files that were left alone on purpose, and
/// `label` turns a verdict into the name the caller shows (the desktop app's
/// "CD quality" for NOT HI-RES).
pub fn done_summary(
    outcomes: &[FixOutcome],
    skipped: usize,
    label: &dyn Fn(Verdict) -> String,
) -> (String, Vec<String>) {
    let ok: Vec<&FixOutcome> = outcomes.iter().filter(|o| o.ok).collect();
    let bad: Vec<&FixOutcome> = outcomes.iter().filter(|o| !o.ok).collect();
    let copies = !ok.is_empty() && !ok.iter().any(|o| o.plan.replace_original);
    let head = if ok.is_empty() {
        if bad.is_empty() { "Nothing was fixed" } else { "No file could be fixed" }.to_string()
    } else if copies {
        if bad.is_empty() {
            format!("Saved corrected copies of {}", files(ok.len()))
        } else {
            format!("Saved corrected copies of {} of {}", ok.len(), files(outcomes.len()))
        }
    } else if bad.is_empty() {
        format!("Fixed {} to their real resolution", files(ok.len()))
    } else {
        format!("Fixed {} of {} to their real resolution", ok.len(), files(outcomes.len()))
    };
    let mut lines = vec![];
    let were = |n: usize| if n != 1 { "were" } else { "was" };
    if !ok.is_empty() {
        let before: u64 = ok.iter().map(|o| o.bytes_before).sum();
        let after: u64 = ok.iter().map(|o| o.bytes_after).sum();
        let they = if copies { "The copies take" } else { "They now take" };
        if after < before {
            lines.push(format!(
                "{they} {} instead of {}, {} less.",
                fmt_bytes(after as f64),
                fmt_bytes(before as f64),
                fmt_bytes((before - after) as f64)
            ));
        } else {
            lines.push(format!(
                "{they} {} (the originals took {}).",
                fmt_bytes(after as f64),
                fmt_bytes(before as f64)
            ));
        }
        let exact = ok.iter().filter(|o| o.how == "exact").count();
        let gapless = ok.iter().filter(|o| o.how == "gapless").count();
        if exact > 0 {
            lines.push(format!(
                "{} {} restored bit-exactly from the untouched 16-bit master, without resampling.",
                files(exact),
                were(exact)
            ));
        }
        if gapless > 0 {
            lines.push(format!(
                "{} {} converted together with the neighbouring tracks, so the album still plays without gaps.",
                files(gapless),
                were(gapless)
            ));
        }
        if copies {
            let dirs: HashSet<String> =
                ok.iter().map(|o| parent(&abspath(o.output.as_deref().unwrap_or(&o.plan.output)))).collect();
            let srcs: HashSet<String> = ok.iter().map(|o| parent(&abspath(&o.plan.source))).collect();
            let where_ = if dirs.len() == 1 && dirs != srcs {
                format!("in {}", dirs.iter().next().unwrap())
            } else if dirs == srcs {
                "next to the originals".to_string()
            } else {
                "in the folder you chose".to_string()
            };
            lines.push(format!("The originals were left unchanged; the corrected copies are {where_}."));
        } else {
            let kinds: HashSet<Backup> = ok.iter().filter(|o| o.plan.replace_original).map(|o| o.plan.backup).collect();
            let mut parts: Vec<String> = vec![];
            if kinds.contains(&Backup::Folder) {
                let bdirs: HashSet<String> = ok
                    .iter()
                    .filter(|o| o.plan.backup == Backup::Folder)
                    .map(|o| abspath(o.plan.backup_dir.as_deref().unwrap_or("")))
                    .collect();
                let named: HashSet<String> = bdirs.iter().map(|d| file_name(d)).collect();
                if bdirs.len() == 1 {
                    parts.push(format!("moved to {}", bdirs.iter().next().unwrap()));
                } else if named.len() == 1 && named.contains(BACKUP_DIR_NAME) {
                    parts.push(format!("moved to the \"{BACKUP_DIR_NAME}\" folder next to each file"));
                } else {
                    parts.push("moved to the backup folders".into());
                }
            }
            if kinds.contains(&Backup::Bak) {
                parts.push("kept next to each file as .bak files".into());
            }
            if kinds.contains(&Backup::None) {
                parts.push("deleted, as chosen".into());
            }
            if !parts.is_empty() {
                lines.push(format!("The originals were {}.", parts.join(", or ")));
            }
        }
        let checked: Vec<&FileResult> = ok.iter().filter_map(|o| o.new_result.as_ref()).collect();
        if !checked.is_empty() {
            let mut counts: Vec<(Verdict, usize)> = vec![];
            for r in &checked {
                match counts.iter_mut().find(|(v, _)| *v == r.verdict) {
                    Some(c) => c.1 += 1,
                    None => counts.push((r.verdict, 1)),
                }
            }
            counts.sort_by_key(|c| std::cmp::Reverse(c.1));
            let parts: Vec<String> = counts.iter().map(|(v, n)| format!("{n} {}", label(*v))).collect();
            let mut line = format!("Re-check of the fixed files: {}.", parts.join(", "));
            let has = |v: Verdict| counts.iter().any(|(k, _)| *k == v);
            if has(Verdict::FakeLossless) {
                line += " A file made from MP3/AAC stays fake lossless at any resolution, because fixing cannot \
                         bring back what the lossy encoder removed.";
            }
            if has(Verdict::Fake) {
                line += " A file whose sample rate had to be kept, to avoid cutting real content, stays marked fake \
                         hi-res.";
            }
            if has(Verdict::FakeLossless) || has(Verdict::Fake) {
                line += " They keep that mark in later scans, and their details say why.";
            }
            lines.push(line);
        }
    }
    if let Some(first) = bad.first() {
        let n = bad.len();
        lines.push(format!(
            "{} could not be fixed and {} left as {}. {}: {}",
            files(n),
            were(n),
            if n != 1 { "they were" } else { "it was" },
            file_name(&first.plan.source),
            first.message
        ));
    }
    if skipped > 0 {
        lines.push(format!(
            "{} {} left alone on purpose; the Fix window lists the reasons.",
            files(skipped),
            were(skipped)
        ));
    }
    (head, lines)
}

/// (plans, [(result, reason)]) for the fake files among `results`.
pub fn plan_fixes(results: &[FileResult], cfg: &Settings) -> (Vec<FixPlan>, Vec<(FileResult, String)>) {
    let mut plans = vec![];
    let mut skipped = vec![];
    for r in results.iter().filter(|r| FIXABLE_VERDICTS.contains(&r.verdict)) {
        match plan_fix(r, cfg) {
            Ok(p) => plans.push(p),
            Err(why) => skipped.push((r.clone(), why)),
        }
    }
    (plans, skipped)
}

pub fn has_soxr() -> bool {
    static SOXR: OnceLock<bool> = OnceLock::new();
    *SOXR.get_or_init(|| {
        command(tool("ffmpeg"))
            .args(["-hide_banner", "-buildconf"])
            .stdin(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("--enable-libsoxr"))
            .unwrap_or(false)
    })
}

pub fn resample_filter(plan: &FixPlan) -> String {
    let mut af = format!(
        "aresample={}{}",
        plan.target_rate,
        if has_soxr() { ":resampler=soxr:precision=28" } else { ":filter_size=256:cutoff=0.97:phase_shift=10" }
    );
    if plan.target_bits == 16 {
        af += ":dither_method=triangular";
    }
    af
}

fn output_args(output: &str, cover_input: u32) -> Vec<String> {
    let mut args = vec![];
    if KEEPS_COVER.contains(&extension(output).to_lowercase().as_str()) {
        args.extend(["-map".into(), format!("{cover_input}:v?"), "-c:v".into(), "copy".into()]);
        args.extend(["-disposition:v".into(), "attached_pic".into()]);
    }
    args.extend(["-map_metadata".into(), cover_input.to_string()]);
    args
}

fn codec_args(plan: &FixPlan, output: &str) -> Vec<String> {
    let mut args = vec!["-c:a".to_string(), plan.codec.clone()];
    if let Some(f) = &plan.sample_fmt {
        args.extend(["-sample_fmt".into(), f.clone()]);
    }
    if plan.target_bits == 24 && ["flac", "alac", "wavpack"].contains(&plan.codec.as_str()) {
        args.extend(["-bits_per_raw_sample".into(), "24".into()]);
    }
    if extension(output).to_lowercase() == ".m4a" {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args
}

/// The plain conversion: FFmpeg reads the original and writes `output`.
pub fn ffmpeg_command(plan: &FixPlan, output: &str) -> Vec<String> {
    let mut cmd: Vec<String> = vec![tool("ffmpeg").to_string_lossy().into_owned()];
    cmd.extend(["-v", "error", "-nostdin", "-y", "-i"].map(String::from));
    cmd.push(plan.source.clone());
    cmd.extend(["-map".into(), "0:a:0".into()]);
    cmd.extend(output_args(output, 0));
    let mut af = vec![];
    if plan.resample {
        af.push(resample_filter(plan));
    } else if plan.target_bits == 16 {
        af.push("aformat=sample_fmts=flt".to_string()); // float -> s16 rounds; s32 -> s16 would truncate
        if plan.dither {
            af.push("aresample=dither_method=triangular".into());
        }
    }
    if !af.is_empty() {
        cmd.extend(["-af".into(), af.join(",")]);
    }
    cmd.extend(codec_args(plan, output));
    cmd.push(output.to_string());
    cmd
}

/// Encode raw PCM from stdin, taking tags and cover art from the original.
fn pipe_command(plan: &FixPlan, output: &str, in_fmt: &str, in_rate: u32, af: Option<&str>) -> Vec<String> {
    let mut cmd: Vec<String> = vec![tool("ffmpeg").to_string_lossy().into_owned()];
    cmd.extend(["-v", "error", "-y", "-f"].map(String::from));
    cmd.extend([in_fmt.to_string(), "-ar".into(), in_rate.to_string(), "-ac".into(), plan.channels.to_string()]);
    cmd.extend(["-i".into(), "pipe:0".into(), "-i".into(), plan.source.clone(), "-map".into(), "0:a".into()]);
    cmd.extend(output_args(output, 1));
    if let Some(af) = af {
        cmd.extend(["-af".into(), af.to_string()]);
    }
    cmd.extend(codec_args(plan, output));
    cmd.push(output.to_string());
    cmd
}

/// An FFmpeg process fed with raw PCM blocks.
struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<std::thread::JoinHandle<String>>,
}

impl Encoder {
    fn start(cmd: &[String]) -> std::io::Result<Encoder> {
        let mut child = command(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take();
        let mut err = child.stderr.take();
        let stderr = std::thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_end(&mut s);
            }
            String::from_utf8_lossy(&s).into_owned()
        });
        Ok(Encoder { child, stdin, stderr: Some(stderr) })
    }

    fn write(&mut self, data: &[u8]) -> bool {
        self.stdin.as_mut().is_some_and(|s| s.write_all(data).is_ok())
    }

    fn abort(mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.stderr.take() {
            let _ = h.join();
        }
    }

    fn finish(mut self) -> (bool, String) {
        self.stdin = None;
        let ok = self.child.wait().map(|s| s.success()).unwrap_or(false);
        let msg = self.stderr.take().and_then(|h| h.join().ok()).unwrap_or_default();
        (ok, msg.trim().lines().last().unwrap_or("").to_string())
    }
}

fn i32_bytes(v: &[i32]) -> Vec<u8> {
    v.iter().flat_map(|s| s.to_le_bytes()).collect()
}

fn size_of(p: &str) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Restore a 16-bit master from a whole-number-ratio upsample by keeping every
/// Nth sample, when exactly one sample phase holds nothing but exact 16-bit
/// values and the others do not.  False when the file does not prove that.
fn exact_restore(plan: &FixPlan, output: &str) -> Result<bool, crate::probe::Error> {
    let n = plan.exact_ratio as usize;
    let ch = plan.channels as usize;
    let mut reader = PcmReader::open(&plan.source, plan.channels, 1 << 16, None)?;
    let mut head: Vec<Vec<i32>> = vec![];
    let mut counts = vec![0i64; n];
    let mut pos = 0usize;
    while let Some(b) = reader.next_block()? {
        let frames = b.len() / ch;
        for (fi, frame) in b.chunks_exact(ch).enumerate() {
            counts[(pos + fi) % n] += frame.iter().filter(|s| **s != 0).count() as i64;
        }
        pos += frames;
        head.push(b);
        if counts.iter().min().copied().unwrap_or(0) >= EXACT_MIN_SAMPLES {
            break;
        }
    }
    if head.is_empty() || counts.iter().min().copied().unwrap_or(0) < EXACT_MIN_SAMPLES {
        return Ok(false);
    }
    let mut on = vec![0u64; n];
    let mut nz = vec![0u64; n];
    let mut fi = 0usize;
    for b in &head {
        for frame in b.chunks_exact(ch) {
            for s in frame.iter().filter(|s| **s != 0) {
                nz[fi % n] += 1;
                on[fi % n] += ((s & 0xFFFF) == 0) as u64;
            }
            fi += 1;
        }
    }
    let on_grid: Vec<f64> = (0..n).map(|p| on[p] as f64 / nz[p].max(1) as f64).collect();
    let keep: Vec<usize> = (0..n).filter(|p| on_grid[*p] == 1.0).collect();
    if keep.len() != 1 || (0..n).filter(|p| *p != keep[0]).any(|p| on_grid[p] > 0.05) {
        return Ok(false);
    }
    let phase = keep[0];
    let mut enc = Encoder::start(&pipe_command(plan, output, "s16le", plan.target_rate, None))?;
    let emit = |block: &[i32], start: usize, enc: &mut Encoder| -> bool {
        let mut out = Vec::with_capacity(block.len() / n * 2 + 8);
        for (i, frame) in block.chunks_exact(ch).enumerate() {
            if (start + i) % n != phase {
                continue;
            }
            for s in frame {
                if s & 0xFFFF != 0 {
                    return false; // a sample that is not the untouched master: give up
                }
                out.extend_from_slice(&((s >> 16) as i16).to_le_bytes());
            }
        }
        enc.write(&out)
    };
    let mut pos = 0usize;
    let mut ok = true;
    for b in &head {
        ok = ok && emit(b, pos, &mut enc);
        pos += b.len() / ch;
    }
    if ok {
        while let Some(b) = reader.next_block()? {
            if !emit(&b, pos, &mut enc) {
                ok = false;
                break;
            }
            pos += b.len() / ch;
        }
    }
    if !ok {
        reader.close();
        enc.abort();
        return Ok(false);
    }
    let (done, _) = enc.finish();
    Ok(done && size_of(output) > 0)
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs()
    } else {
        gcd(b, a % b)
    }
}

fn ceil_div(a: i64, b: i64) -> i64 {
    -((-a).div_euclid(b))
}

/// Resample with the end of the previous and the start of the next track
/// around the file, on the sample grid of the whole gapless run, and keep only
/// this file's part, so the result joins its neighbours exactly as if the album
/// had been converted in one piece.
fn resample_with_context(plan: &FixPlan, output: &str) -> Result<bool, crate::probe::Error> {
    let Some(frames) = plan.frames else {
        return Ok(false);
    };
    let ch = plan.channels as usize;
    let d = gcd(plan.target_rate as i64, plan.source_rate as i64);
    let (p, q) = (plan.target_rate as i64 / d, plan.source_rate as i64 / d);
    let empty = vec![];
    let before = plan.context_before.as_ref().unwrap_or(&empty);
    let after = plan.context_after.as_ref().unwrap_or(&empty);
    let t = (before.len() / ch) as i64;
    let cb = if t > 0 { t - (t - plan.offset).rem_euclid(q) } else { 0 };
    if (plan.offset - cb).rem_euclid(q) != 0 {
        return Ok(false); // only a run's first track may start without context, at offset 0
    }
    let a = plan.offset - cb;
    let first = ceil_div(plan.offset * p, q) - (a * p).div_euclid(q);
    let last = ceil_div((plan.offset + frames) * p, q) - (a * p).div_euclid(q);
    let af = format!("{},atrim=start_sample={first}:end_sample={last}", resample_filter(plan));
    let mut enc = Encoder::start(&pipe_command(plan, output, "s32le", plan.source_rate, Some(&af)))?;
    let mut ok = if cb > 0 { enc.write(&i32_bytes(&before[((t - cb) as usize) * ch..])) } else { true };
    let mut n: i64 = 0;
    let mut reader = PcmReader::open(&plan.source, plan.channels, 1 << 16, None)?;
    while let Some(b) = reader.next_block()? {
        if ok {
            ok = enc.write(&i32_bytes(&b));
        }
        n += (b.len() / ch) as i64;
    }
    if !ok || n != frames {
        enc.abort();
        return Ok(false);
    }
    if !after.is_empty() && !enc.write(&i32_bytes(after)) {
        enc.abort();
        return Ok(false);
    }
    let (done, _) = enc.finish();
    Ok(done && size_of(output) > 0)
}

/// (ok, error, how): how is "exact", "gapless" or "" for a plain conversion.
fn convert(plan: &FixPlan, output: &str) -> (bool, String, &'static str) {
    if plan.exact_ratio > 0 {
        if let Ok(true) = exact_restore(plan, output) {
            return (true, String::new(), "exact");
        }
    }
    if plan.context_before.is_some() || plan.context_after.is_some() {
        if let Ok(true) = resample_with_context(plan, output) {
            return (true, String::new(), "gapless");
        }
    }
    let cmd = ffmpeg_command(plan, output);
    match command(&cmd[0]).args(&cmd[1..]).stdin(Stdio::null()).output() {
        Ok(o) if o.status.success() && size_of(output) > 0 => (true, String::new(), ""),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let last = err.trim().lines().last().map(str::to_string);
            (false, last.unwrap_or_else(|| format!("exit {}", o.status.code().unwrap_or(-1))), "")
        }
        Err(e) => (false, e.to_string(), ""),
    }
}

// --------------------------------------------------------------- gapless ---

fn read_head(path: &str, channels: u32, rate: u32, frames: usize) -> Result<Vec<i32>, crate::probe::Error> {
    let ch = channels as usize;
    let mut r = PcmReader::open(path, channels, frames, Some(frames as f64 / rate as f64 * 1.2 + 0.05))?;
    let mut out = vec![];
    while out.len() < frames * ch {
        match r.next_block()? {
            Some(b) => out.extend_from_slice(&b),
            None => break,
        }
    }
    r.close();
    out.truncate(frames * ch);
    Ok(out)
}

fn read_tail(path: &str, channels: u32, rate: u32, frames: usize) -> Result<Vec<i32>, crate::probe::Error> {
    let secs = frames as f64 / rate as f64 * 1.5 + 0.05;
    let x = read_all(path, channels, None, &["-sseof".into(), format!("-{secs:.3}")])?;
    let ch = channels as usize;
    let n = x.len() / ch;
    Ok(x[n.saturating_sub(frames) * ch..n * ch].to_vec())
}

/// Exact number of sample frames (FLAC/WAV/AIFF/ALAC/WavPack headers), or None.
fn frame_count(path: &str, rate: u32) -> Option<i64> {
    let out = command(tool("ffprobe"))
        .args(["-v", "error", "-select_streams", "a:0", "-show_entries", "stream=duration_ts,time_base", "-of", "json"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let v = json::parse(&String::from_utf8_lossy(&out.stdout)).ok()?;
    let st = v.get("streams")?.as_array()?.first()?.clone();
    let tb = st.get("time_base")?.as_str()?.to_string();
    let (num, den) = tb.split_once('/')?;
    let (num, den): (i64, i64) = (num.trim().parse().ok()?, den.trim().parse().ok()?);
    if num * rate as i64 != den {
        return None;
    }
    let n = match st.get("duration_ts")? {
        Value::Int(i) => *i,
        Value::Str(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    (n > 0).then_some(n)
}

/// Give each file that will be resampled the audio of its album neighbours, so
/// the converted tracks join without a click.  Must run before any file is
/// replaced.  Returns the number of gapless joins found.
pub fn prepare_gapless(plans: &mut [FixPlan], results: &[FileResult], progress: &mut dyn FnMut(&str)) -> usize {
    let todo: HashMap<String, usize> =
        plans.iter().enumerate().filter(|(_, p)| p.resample).map(|(i, p)| (abspath(&p.source), i)).collect();
    if todo.is_empty() {
        return 0;
    }
    let mut folders: Vec<(String, Vec<&FileResult>)> = vec![];
    let mut seen = HashSet::new();
    for r in results {
        let key = abspath(&r.path);
        if r.sample_rate == 0 || matches!(r.verdict, Verdict::Lossy | Verdict::Error) || !seen.insert(key.clone()) {
            continue;
        }
        let dir = parent(&key);
        match folders.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, v)) => v.push(r),
            None => folders.push((dir, vec![r])),
        }
    }
    let mut joins: Vec<(&FileResult, &FileResult)> = vec![];
    for (_, items) in &mut folders {
        items.sort_by_key(|r| track_key(r));
        for w in items.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (a.sample_rate, a.channels) != (b.sample_rate, b.channels) {
                continue;
            }
            if todo.contains_key(&abspath(&a.path)) || todo.contains_key(&abspath(&b.path)) {
                joins.push((a, b));
            }
        }
    }
    if joins.is_empty() {
        return 0;
    }
    progress(&format!("Checking {} track join(s) for gapless playback…", joins.len()));
    let edges = |a: &FileResult, b: &FileResult| -> Option<(Vec<i32>, Vec<i32>)> {
        let ch = if a.channels == 0 { 2 } else { a.channels };
        let tail = read_tail(&a.path, ch, a.sample_rate, CONTEXT_FRAMES).ok()?;
        let head = read_head(&b.path, ch, b.sample_rate, CONTEXT_FRAMES).ok()?;
        let loud = tail.iter().chain(&head).map(|s| (*s as i64).abs()).max().unwrap_or(0);
        (loud >= SILENT_JOIN).then_some((tail, head))
    };
    let found: Vec<Option<(Vec<i32>, Vec<i32>)>> = run_parallel(&joins, 8, |(a, b)| edges(a, b));
    let gapless: Vec<(&FileResult, &FileResult, (Vec<i32>, Vec<i32>))> =
        joins.iter().zip(found).filter_map(|((a, b), e)| Some((*a, *b, e?))).collect();
    if gapless.is_empty() {
        return 0;
    }
    let mut nxt: HashMap<String, (&FileResult, &(Vec<i32>, Vec<i32>))> = HashMap::new();
    let mut firsts: HashMap<String, &FileResult> = HashMap::new();
    let mut seconds: HashSet<String> = HashSet::new();
    for (a, b, e) in &gapless {
        nxt.insert(abspath(&a.path), (*b, e));
        firsts.insert(abspath(&a.path), *a);
        seconds.insert(abspath(&b.path));
    }
    let mut count = 0;
    for (start, first) in firsts.iter().filter(|(k, _)| !seconds.contains(*k)) {
        let _ = start;
        let mut run = vec![*first];
        let mut links: Vec<&(Vec<i32>, Vec<i32>)> = vec![];
        while let Some((b, e)) = nxt.get(&abspath(&run[run.len() - 1].path)) {
            links.push(e);
            run.push(b);
            if run.len() > 100_000 {
                break;
            }
        }
        let frames: Vec<Option<i64>> = run.iter().map(|r| frame_count(&r.path, r.sample_rate)).collect();
        if frames.iter().any(Option::is_none) {
            continue;
        }
        let mut offset = 0i64;
        for (i, r) in run.iter().enumerate() {
            if let Some(&pi) = todo.get(&abspath(&r.path)) {
                let plan = &mut plans[pi];
                plan.frames = frames[i];
                plan.offset = offset;
                if i > 0 {
                    plan.context_before = Some(links[i - 1].0.clone());
                }
                if i < links.len() {
                    plan.context_after = Some(links[i].1.clone());
                }
            }
            offset += frames[i].unwrap();
        }
        count += links.len();
    }
    count
}

/// Map `f` over `items` with up to `workers` threads, keeping the order.
pub fn run_parallel<T: Sync, R: Send>(items: &[T], workers: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: Vec<std::sync::Mutex<Option<R>>> = items.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..workers.max(1).min(items.len().max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                *out[i].lock().unwrap() = Some(r);
            });
        }
    });
    out.into_iter().map(|m| m.into_inner().unwrap().expect("result")).collect()
}

/// `shutil.move`: rename, or copy and delete across file systems.
fn move_file(src: &str, dst: &str) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst)?;
    std::fs::remove_file(src)
}

fn temp_file(dir: &str, ext: &str) -> std::io::Result<String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..1000 {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let name = format!(".realhires-{:x}{:x}{:x}{ext}", std::process::id(), n, nanos);
        let path = join(dir, &name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("cannot create a temporary file"))
}

fn same_file(a: &str, b: &str) -> bool {
    let (a, b) = (abspath(a), abspath(b));
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

fn ordinal(n: u32) -> String {
    match n {
        2 => "second".into(),
        3 => "third".into(),
        4 => "fourth".into(),
        n => format!("{n}th"),
    }
}

/// Convert one file.  Writes to a temporary file first, so a failed or
/// interrupted conversion never leaves a broken file behind.
pub fn run_fix(plan: &FixPlan, cfg: &Settings) -> FixOutcome {
    let fail = |msg: String| FixOutcome {
        plan: plan.clone(),
        ok: false,
        message: msg,
        output: None,
        new_result: None,
        bytes_before: 0,
        bytes_after: 0,
        how: String::new(),
    };
    let out_dir = {
        let d = parent(&plan.output);
        if d.is_empty() {
            ".".to_string()
        } else {
            d
        }
    };
    let prep = (|| -> std::io::Result<(u64, String)> {
        std::fs::create_dir_all(&out_dir)?;
        let before = std::fs::metadata(&plan.source)?.len();
        Ok((before, temp_file(&out_dir, &extension(&plan.output))?))
    })();
    let (before, tmp) = match prep {
        Ok(x) => x,
        Err(e) => return fail(format!("Cannot write to {out_dir}: {e}")),
    };
    let result = (|| -> std::io::Result<FixOutcome> {
        let (ok, err, how) = convert(plan, &tmp);
        if !ok {
            return Ok(fail(format!("FFmpeg failed: {err}")));
        }
        let mut output = plan.output.clone();
        let note;
        if plan.replace_original {
            if Path::new(&output).exists() && !same_file(&output, &plan.source) {
                output = unique(&output); // extension changed (e.g. APE -> FLAC): never overwrite others
            }
            if plan.backup == Backup::None {
                std::fs::rename(&tmp, &output)?;
                if abspath(&output) != abspath(&plan.source) {
                    std::fs::remove_file(&plan.source)?;
                }
                note = "original deleted".to_string();
            } else {
                let backup = if plan.backup == Backup::Bak {
                    unique(&format!("{}.bak", plan.source))
                } else {
                    let dir = plan.backup_dir.clone().unwrap_or_else(|| join(&parent(&plan.source), BACKUP_DIR_NAME));
                    std::fs::create_dir_all(&dir)?;
                    unique(&join(&dir, &file_name(&plan.source)))
                };
                move_file(&plan.source, &backup)?;
                if let Err(e) = std::fs::rename(&tmp, &output) {
                    let _ = move_file(&backup, &plan.source); // put the original back
                    return Err(e);
                }
                let where_ = if plan.backup == Backup::Bak {
                    file_name(&backup)
                } else {
                    join(&file_name(plan.backup_dir.as_deref().unwrap_or(BACKUP_DIR_NAME)), &file_name(&backup))
                };
                note = format!("original moved to {where_}");
            }
        } else {
            if Path::new(&output).exists() {
                output = unique(&output);
            }
            std::fs::rename(&tmp, &output)?;
            note = "original left unchanged".to_string();
        }
        let after = size_of(&output);
        let new = cfg.fix_verify.then(|| analyze_file(&output, None, false, false, Some(cfg)));
        let verb = if plan.replace_original { "Fixed" } else { "Saved" };
        let mut msg = format!(
            "{verb} {} ({}, {} of the size; {note}).",
            file_name(&output),
            plan.description,
            pct(after as f64 / before.max(1) as f64, 0)
        );
        if how == "exact" {
            msg += &format!(
                " Every {} sample was the untouched 16-bit master, so it was restored bit-exactly, without resampling.",
                ordinal(plan.exact_ratio)
            );
        } else if how == "gapless" {
            msg += " Converted together with the neighbouring track(s), so the joins stay gapless.";
        }
        if let Some(n) = &new {
            msg += &format!(" Re-check: {} – {}", n.verdict, n.summary);
        }
        Ok(FixOutcome {
            plan: plan.clone(),
            ok: true,
            message: msg,
            output: Some(output),
            new_result: new,
            bytes_before: before,
            bytes_after: after,
            how: how.to_string(),
        })
    })();
    if Path::new(&tmp).exists() {
        let _ = std::fs::remove_file(&tmp);
    }
    match result {
        Ok(o) => o,
        Err(e) => fail(format!("Could not save the fixed file: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(rate: u32, from: u32, bits: u32) -> FileResult {
        let mut r = FileResult::new("/music/a/Song.flac", Verdict::Fake);
        r.sample_rate = rate;
        r.claimed_bits = Some(24);
        r.channels = 2;
        r.effective_bits = Some(bits);
        r.upsampled_from = Some(from);
        r.cliffs_db = vec![(44100, 60.0), (48000, 30.0)];
        r.deep_cliffs_db = vec![(44100, 80.0), (48000, 40.0)];
        r.upsample_mid_hz = Some(21500.0);
        r
    }

    #[test]
    fn plans_like_python() {
        let cfg = Settings::default();
        let p = plan_fix(&fake(96000, 44100, 24), &cfg).unwrap();
        assert_eq!((p.target_rate, p.target_bits, p.resample), (44100, 16, true));
        assert_eq!(p.description, "24/96 → 16/44.1");
        // joined like os.path.join: "\\" on Windows
        assert_eq!(p.output, join("/music/a", "Song.flac"));
        assert_eq!(p.backup_dir, Some(join("/music/a", "Real-Hi-Res backups")));
        assert_eq!(p.exact_ratio, 0);
        let cmd = ffmpeg_command(&p, "x.flac").join(" ");
        assert!(cmd.contains("aresample=44100") && cmd.contains("dither_method=triangular"));
        let mut r = fake(96000, 48000, 16);
        r.cliffs_db = vec![(48000, 50.0)];
        r.upsample_mid_hz = Some(23800.0);
        assert_eq!(plan_fix(&r, &cfg).unwrap().exact_ratio, 2);
        let mut lossy = fake(44100, 0, 16);
        lossy.verdict = Verdict::FakeLossless;
        lossy.upsampled_from = None;
        lossy.claimed_bits = Some(16);
        let why = plan_fix(&lossy, &cfg).unwrap_err();
        assert!(why.starts_with("Already 16/44.1, the resolution it really has"));
        assert_eq!(
            skip_summary(&[(lossy, why)]),
            "1 file(s) are not changed: 1 made from MP3/AAC but already at CD resolution: nothing to convert, and the \
             lossy damage cannot be repaired."
        );
    }

    #[test]
    fn unsafe_cut_keeps_higher_rate() {
        let mut r = fake(192000, 44100, 24);
        r.cliffs_db = vec![(44100, 10.0), (48000, 12.0), (88200, 50.0)];
        r.deep_cliffs_db = vec![(44100, 20.0), (48000, 20.0), (88200, 70.0)];
        r.upsample_mid_hz = Some(40000.0);
        assert_eq!(safe_source_rate(&r, None), 88200);
    }
}
