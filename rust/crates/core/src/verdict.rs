//! Turn raw measurements into a verdict for one file (the Rust counterpart of
//! `realhires/verdict.py`; every reason and summary is the same text).

use std::time::Instant;

use crate::analysis::{
    analyze_spectrum, compact_spectrum, converter_noise_floor, detection_params, imap_get, BitDepthAccumulator, Grid,
    GridAccumulator, IntMap, Plot, SpectrumAccumulator, SpectrumStats, GRID_FULL, SMEAR_HZ, SPECTRO_MAX_COLUMNS,
};
use crate::json::{Obj, Value};
use crate::paths::abspath;
use crate::probe::{probe, AudioInfo, PcmReader, Tags};
use crate::pyfmt::{self, f, fs, g, pct};
use crate::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Verdict {
    /// a "fake FLAC": lossless container, lossy (MP3/AAC...) source
    FakeLossless,
    /// upsampled and/or bit-padded hi-res
    Fake,
    Suspect,
    Real,
    /// genuine lossless at CD resolution
    NotHires,
    /// MP3, AAC, ... files themselves
    Lossy,
    Error,
}

pub const VERDICT_ORDER: [Verdict; 7] = [
    Verdict::FakeLossless,
    Verdict::Fake,
    Verdict::Suspect,
    Verdict::Real,
    Verdict::NotHires,
    Verdict::Lossy,
    Verdict::Error,
];
pub const PROBLEM_VERDICTS: [Verdict; 3] = [Verdict::FakeLossless, Verdict::Fake, Verdict::Suspect];

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::FakeLossless => "FAKE LOSSLESS",
            Verdict::Fake => "FAKE HI-RES",
            Verdict::Suspect => "SUSPICIOUS",
            Verdict::Real => "REAL HI-RES",
            Verdict::NotHires => "NOT HI-RES",
            Verdict::Lossy => "LOSSY FORMAT",
            Verdict::Error => "ERROR",
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        VERDICT_ORDER.into_iter().find(|v| v.as_str() == s)
    }

    pub fn is_problem(self) -> bool {
        PROBLEM_VERDICTS.contains(&self)
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// Version of the detection rules (the same number as in the Python version,
/// whose results it shares).  Bump it whenever a change can give a different
/// verdict, flag, estimated resolution or measurement for the same file and
/// settings.
///   1: Real-Hi-Res 1.7.0 to 1.11.0 (and the Rust port of 1.11.0)
pub const DETECTOR_VERSION: i64 = 1;

pub const FLAG_LOSSY_SOURCE: &str = "lossy-source";
pub const FLAG_UPSAMPLED: &str = "upsampled";
pub const FLAG_PADDED: &str = "padded-bits";
pub const FLAG_PARTIAL_BITS: &str = "partial-bits";
pub const FLAG_NO_ULTRASONIC: &str = "no-ultrasonic";
pub const FLAG_GRID: &str = "value-grid";
pub const FLAG_CONVERTER_FLOOR: &str = "converter-floor";

pub fn fmt_rate(rate: Option<f64>) -> String {
    match rate {
        Some(r) if r != 0.0 => {
            let k = r / 1000.0;
            if (k - pyfmt::round_to(k, 1)).abs() < 1e-9 {
                g(k)
            } else {
                f(k, 1)
            }
        }
        _ => "?".into(),
    }
}

pub fn fmt_rate_u(rate: u32) -> String {
    fmt_rate(Some(rate as f64))
}

pub fn fmt_res(bits: Option<u32>, rate: u32, is_float: bool) -> String {
    let b = match bits {
        None => "?".to_string(),
        Some(b) if is_float => format!("{b}f"),
        Some(b) => b.to_string(),
    };
    format!("{b}/{}", fmt_rate_u(rate))
}

pub const PASSED: &str = "Passed spectral and bit-depth checks.";

/// Why text of a genuine file whose sample rate is not a hi-res one (48 kHz or less).
pub fn bit_depth_only_summary(bits: &str, rate: u32) -> String {
    let std = if rate == 44100 || rate == 48000 { "standard " } else { "" };
    format!("Genuine {bits}-bit at {std}{} kHz: hi-res by bit depth only", fmt_rate_u(rate))
}

pub fn bit_depth_only_reason(rate: u32) -> String {
    format!(
        "Hi-res by bit depth only: {} kHz is not a hi-res sample rate, and the file does not claim one, so there is \
         no higher sample rate that could have been faked. Only its bit depth goes beyond CD quality, and that bit \
         depth is genuine.",
        fmt_rate_u(rate)
    )
}

/// Where a reused result comes from (`FileResult.saved` in Python).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedInfo {
    pub time: f64,
    pub version: String,
    pub detector: i64,
    pub outdated: bool,
    pub graphs: bool,
    pub moved_from: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FileResult {
    pub path: String,
    pub verdict: Verdict,
    pub summary: String,
    pub reasons: Vec<String>,
    pub notes: Vec<String>,
    pub flags: Vec<String>,
    pub tags: Tags,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub claimed_bits: Option<u32>,
    pub is_float: bool,
    pub duration: Option<f64>,
    pub effective_bits: Option<u32>,
    pub true_rate: Option<u32>,
    pub bandwidth_hz: Option<f64>,
    pub cutoff_drop_db: Option<f64>,
    pub noise_floor_db: Option<f64>,
    pub upsampled_from: Option<u32>,
    pub upsample_edge_hz: Option<f64>,
    pub upsample_mid_hz: Option<f64>,
    pub lossy_cutoff_hz: Option<f64>,
    pub cliffs_db: IntMap,
    pub deep_cliffs_db: IntMap,
    pub converter_floor_db: Option<f64>,
    pub grid_step: Option<f64>,
    pub grid_bits: Option<u32>,
    pub grid_gain_db: Option<f64>,
    pub bit_usage: IntMap,
    pub analysed_seconds: f64,
    pub elapsed: f64,
    /// full spectrum, for SVG plots; never saved
    pub spectrum: Option<Box<SpectrumStats>>,
    /// compact spectrum + spectrogram for the GUI; saved separately
    pub plot: Option<Plot>,
    /// set when the result was loaded from the saved results
    pub saved: Option<SavedInfo>,
}

impl FileResult {
    pub fn new(path: &str, verdict: Verdict) -> Self {
        FileResult {
            path: path.to_string(),
            verdict,
            summary: String::new(),
            reasons: vec![],
            notes: vec![],
            flags: vec![],
            tags: vec![],
            codec: String::new(),
            sample_rate: 0,
            channels: 0,
            claimed_bits: None,
            is_float: false,
            duration: None,
            effective_bits: None,
            true_rate: None,
            bandwidth_hz: None,
            cutoff_drop_db: None,
            noise_floor_db: None,
            upsampled_from: None,
            upsample_edge_hz: None,
            upsample_mid_hz: None,
            lossy_cutoff_hz: None,
            cliffs_db: vec![],
            deep_cliffs_db: vec![],
            converter_floor_db: None,
            grid_step: None,
            grid_bits: None,
            grid_gain_db: None,
            bit_usage: vec![],
            analysed_seconds: 0.0,
            elapsed: 0.0,
            spectrum: None,
            plot: None,
            saved: None,
        }
    }

    pub fn claimed(&self) -> String {
        if self.verdict == Verdict::Lossy {
            return format!("{}/{}", self.codec, fmt_rate_u(self.sample_rate));
        }
        fmt_res(self.claimed_bits, self.sample_rate, self.is_float)
    }

    pub fn estimated(&self) -> String {
        let Some(mut bits) = self.effective_bits else {
            return "-".into();
        };
        if matches!(self.verdict, Verdict::Error | Verdict::Lossy) {
            return "-".into();
        }
        if let Some(c) = self.claimed_bits.filter(|c| *c != 0) {
            bits = bits.min(c);
        }
        if self.is_float && bits > 24 {
            bits = 24; // float carries ~24 bits of mantissa
        }
        if let Some(l) = self.lossy_cutoff_hz.filter(|l| *l != 0.0) {
            return format!("lossy~{}k", f(l / 1000.0, 0));
        }
        fmt_res(Some(bits), self.true_rate.filter(|r| *r != 0).unwrap_or(self.sample_rate), false)
    }

    pub fn is_problem(&self) -> bool {
        self.verdict.is_problem()
    }

    pub fn tag(&self, key: &str) -> Option<&str> {
        crate::probe::tag(&self.tags, key)
    }

    /// The fields Python's `result_to_json` saves, in the same order and form.
    pub fn to_store_json(&self) -> Value {
        self.fields(false)
    }

    /// `FileResult.to_dict()`: the JSON export (non-finite numbers become null,
    /// cliffs rounded to 0.1 dB, plus `claimed` and `estimated_real`).
    pub fn to_dict(&self) -> Value {
        let mut v = self.fields(true);
        if let Value::Object(items) = &mut v {
            items.push(("claimed".into(), self.claimed().into()));
            items.push(("estimated_real".into(), self.estimated().into()));
        }
        v
    }

    fn fields(&self, export: bool) -> Value {
        let num = |x: Option<f64>| -> Value {
            match x {
                Some(v) if export && !v.is_finite() => Value::Null,
                Some(v) => Value::Float(v),
                None => Value::Null,
            }
        };
        let fl = |x: f64| -> Value {
            if export && !x.is_finite() {
                Value::Null
            } else {
                Value::Float(x)
            }
        };
        let cliffs = |m: &IntMap| -> Value {
            Value::Object(
                m.iter()
                    .map(|(k, v)| {
                        let val = if !export {
                            Value::Float(*v)
                        } else if v.is_finite() {
                            Value::Float(pyfmt::round_to(*v, 1))
                        } else {
                            Value::Null
                        };
                        (k.to_string(), val)
                    })
                    .collect(),
            )
        };
        let strings = |v: &Vec<String>| Value::Array(v.iter().map(|s| Value::Str(s.clone())).collect());
        Obj::new()
            .put("path", self.path.as_str())
            .put("verdict", self.verdict.as_str())
            .put("summary", self.summary.as_str())
            .put("reasons", strings(&self.reasons))
            .put("notes", strings(&self.notes))
            .put("flags", strings(&self.flags))
            .put("tags", Value::Object(self.tags.iter().map(|(k, v)| (k.clone(), Value::Str(v.clone()))).collect()))
            .put("codec", self.codec.as_str())
            .put("sample_rate", self.sample_rate)
            .put("channels", self.channels)
            .put("claimed_bits", self.claimed_bits)
            .put("is_float", self.is_float)
            .put("duration", num(self.duration))
            .put("effective_bits", self.effective_bits)
            .put("true_rate", self.true_rate)
            .put("bandwidth_hz", num(self.bandwidth_hz))
            .put("cutoff_drop_db", num(self.cutoff_drop_db))
            .put("noise_floor_db", num(self.noise_floor_db))
            .put("upsampled_from", self.upsampled_from)
            .put("upsample_edge_hz", num(self.upsample_edge_hz))
            .put("upsample_mid_hz", num(self.upsample_mid_hz))
            .put("lossy_cutoff_hz", num(self.lossy_cutoff_hz))
            .put("cliffs_db", cliffs(&self.cliffs_db))
            .put("deep_cliffs_db", cliffs(&self.deep_cliffs_db))
            .put("converter_floor_db", num(self.converter_floor_db))
            .put("grid_step", num(self.grid_step))
            .put("grid_bits", self.grid_bits)
            .put("grid_gain_db", num(self.grid_gain_db))
            .put(
                "bit_usage",
                Value::Object(self.bit_usage.iter().map(|(k, v)| (k.to_string(), Value::Float(*v))).collect()),
            )
            .put("analysed_seconds", fl(self.analysed_seconds))
            .put("elapsed", fl(self.elapsed))
            .build()
    }

    /// A result saved by either version (`store.result_from_json`).
    pub fn from_store_json(v: &Value, path: &str) -> Option<FileResult> {
        v.as_object()?;
        let verdict = Verdict::parse(v.get("verdict")?.as_str()?)?;
        let mut r = FileResult::new(path, verdict);
        let strs = |k: &str| -> Vec<String> {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        let fnum = |k: &str| v.get(k).and_then(Value::as_f64);
        let unum = |k: &str| v.get(k).and_then(Value::as_i64).filter(|x| *x >= 0).map(|x| x as u32);
        let imap = |k: &str| -> IntMap {
            v.get(k)
                .and_then(Value::as_object)
                .map(|o| {
                    o.iter()
                        .filter_map(|(key, val)| Some((key.parse::<u32>().ok()?, val.as_f64().unwrap_or(f64::NAN))))
                        .collect()
                })
                .unwrap_or_default()
        };
        r.summary = v.get("summary").and_then(Value::as_str).unwrap_or("").to_string();
        r.reasons = strs("reasons");
        r.notes = strs("notes");
        r.flags = strs("flags");
        r.tags = v
            .get("tags")
            .and_then(Value::as_object)
            .map(|o| o.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        r.codec = v.get("codec").and_then(Value::as_str).unwrap_or("").to_string();
        r.sample_rate = unum("sample_rate").unwrap_or(0);
        r.channels = unum("channels").unwrap_or(0);
        r.claimed_bits = unum("claimed_bits");
        r.is_float = v.get("is_float").map(Value::truthy).unwrap_or(false);
        r.duration = fnum("duration");
        r.effective_bits = unum("effective_bits");
        r.true_rate = unum("true_rate");
        r.bandwidth_hz = fnum("bandwidth_hz");
        r.cutoff_drop_db = fnum("cutoff_drop_db");
        r.noise_floor_db = fnum("noise_floor_db");
        r.upsampled_from = unum("upsampled_from");
        r.upsample_edge_hz = fnum("upsample_edge_hz");
        r.upsample_mid_hz = fnum("upsample_mid_hz");
        r.lossy_cutoff_hz = fnum("lossy_cutoff_hz");
        r.cliffs_db = imap("cliffs_db");
        r.deep_cliffs_db = imap("deep_cliffs_db");
        r.converter_floor_db = fnum("converter_floor_db");
        r.grid_step = fnum("grid_step");
        r.grid_bits = unum("grid_bits");
        r.grid_gain_db = fnum("grid_gain_db");
        r.bit_usage = imap("bit_usage");
        r.analysed_seconds = fnum("analysed_seconds").unwrap_or(0.0);
        r.elapsed = fnum("elapsed").unwrap_or(0.0);
        Some(update_wording(r))
    }
}

/// Bring the texts of a result saved by an older version up to date (wording only).
pub fn update_wording(mut r: FileResult) -> FileResult {
    if r.verdict == Verdict::Real
        && r.sample_rate != 0
        && r.sample_rate <= 48000
        && r.summary.starts_with("Genuine ")
        && r.summary.ends_with("-bit audio")
    {
        let bits = r.summary["Genuine ".len()..r.summary.len() - "-bit audio".len()].to_string();
        r.summary = bit_depth_only_summary(&bits, r.sample_rate);
        if let Some(i) = r.reasons.iter().position(|x| x == PASSED) {
            if !r.reasons.iter().any(|x| x.starts_with("Hi-res by bit depth only")) {
                r.reasons.insert(i + 1, bit_depth_only_reason(r.sample_rate));
            }
        }
    }
    r
}

fn opt_bits(b: Option<u32>) -> String {
    b.map(|b| b.to_string()).unwrap_or_else(|| "None".into())
}

fn judge(
    info: &AudioInfo,
    spec: &SpectrumStats,
    bits: &BitDepthAccumulator,
    res: &mut FileResult,
    cfg: Option<&Settings>,
    grid: Option<Grid>,
) {
    let c = detection_params(cfg);
    let mut fake: Vec<String> = vec![];
    let mut suspect: Vec<String> = vec![];
    let mut short: Vec<String> = vec![];
    let eff = bits.effective_bits(c.bit_threshold);
    res.effective_bits = match (eff, grid) {
        (Some(e), Some(gr)) => Some(e.min(gr.bits)),
        _ => eff,
    };
    if let Some(gr) = grid {
        res.grid_step = Some(pyfmt::round_to(gr.step, 2));
        res.grid_bits = Some(gr.bits);
        res.grid_gain_db = Some(pyfmt::round_to(gr.gain_db, 2));
    }
    res.bit_usage = bits.bit_usage().into_iter().map(|(k, v)| (k, pyfmt::round_to(v, 4))).collect();
    res.bit_usage.sort_by(|a, b| b.0.cmp(&a.0));
    res.bandwidth_hz = Some(spec.bandwidth_hz);
    res.cutoff_drop_db = Some(spec.cutoff_drop_db);
    res.noise_floor_db = Some(spec.noise_floor_db);
    res.upsampled_from = spec.upsampled_from;
    res.upsample_edge_hz = Some(spec.upsample_edge_hz);
    res.upsample_mid_hz = Some(spec.upsample_mid_hz);
    res.lossy_cutoff_hz = if spec.lossy_cutoff { Some(spec.lossy_cutoff_hz) } else { None };
    res.cliffs_db = spec.cliffs_db.clone();
    res.deep_cliffs_db = spec.deep_cliffs_db.clone();
    res.converter_floor_db = Some(spec.ultra_floor_db - spec.quantization_floor_db);
    res.true_rate = Some(spec.upsampled_from.unwrap_or(info.sample_rate));

    let bw_k = spec.bandwidth_hz / 1000.0;
    let rate = info.sample_rate;

    // ---- lossy source ("fake FLAC") --------------------------------------
    if spec.lossy_cutoff {
        res.flags.push(FLAG_LOSSY_SOURCE.into());
        fake.push(format!(
            "Brick-wall low-pass at {} kHz ({} dB drop within ~1 kHz, never recovers): typical of an MP3/AAC/Vorbis \
             encoder, so this {} file was made from a lossy file and is not truly lossless.",
            f(spec.lossy_cutoff_hz / 1000.0, 1),
            f(spec.lossy_drop_db, 0),
            info.codec.to_uppercase()
        ));
        short.push(format!(
            "Made from a lossy file (MP3/AAC-style cut-off at {} kHz)",
            f(spec.lossy_cutoff_hz / 1000.0, 1)
        ));
    }

    // ---- bit depth -------------------------------------------------------
    let claimed_bits = info.claimed_bits.filter(|b| *b != 0).unwrap_or(16);
    if claimed_bits > 16 || info.is_float {
        let grid16 = grid.filter(|gr| gr.bits <= 16);
        if let Some(e) = eff.filter(|e| *e <= 16) {
            res.flags.push(FLAG_PADDED.into());
            if info.is_float {
                fake.push(format!(
                    "Every sample is an exact {e}-bit value: 16-bit audio stored in a {claimed_bits}-bit float container."
                ));
                short.push(format!("16-bit audio in a {claimed_bits}-bit float file"));
            } else {
                fake.push(format!(
                    "Only {e} of {claimed_bits} bits are used: the low {} bits are always zero (16-bit audio padded into \
                     a {claimed_bits}-bit container).",
                    claimed_bits as i64 - e as i64
                ));
                short.push(format!("Padded: only {e} of {claimed_bits} bits used"));
            }
        } else if let Some(gr) = grid16.filter(|gr| gr.strength >= GRID_FULL) {
            res.flags.push(FLAG_GRID.into());
            let kind = if info.is_float { "float".to_string() } else { format!("{claimed_bits}-bit") };
            let how = if gr.gain_db.abs() >= 0.05 {
                format!("after a gain change of {} dB (with or without dither)", fs(gr.gain_db, 1))
            } else {
                "when it is only dithered, with no gain change,".to_string()
            };
            fake.push(format!(
                "Every sample lies on an even grid with a step of {} levels of the 24-bit scale instead of 1. That is \
                 what a 16-bit master gives {how} on its way into a {kind} file, so the low bits hold no real detail \
                 and the resolution is still 16 bits.",
                f(gr.step, 1)
            ));
            short.push(
                if gr.gain_db.abs() >= 0.05 {
                    "16-bit master given a volume change or dither"
                } else {
                    "16-bit master given dither"
                }
                .into(),
            );
        } else if let Some(gr) = grid16 {
            res.flags.push(FLAG_GRID.into());
            fake.push(format!(
                "About {} of the samples sit exactly on the grid of a 16-bit master (a step of {} levels of the 24-bit \
                 scale), which real 24-bit audio never does. An upsample by a whole-number ratio leaves this trace, \
                 because it keeps every second (or third) sample of the 16-bit master unchanged.",
                pct(gr.strength, 0),
                f(gr.step, 1)
            ));
            short.push(format!("{} of samples on a 16-bit grid", pct(gr.strength, 0)));
        } else if let Some(e) = eff.filter(|e| *e < claimed_bits.min(24) && !info.is_float) {
            res.flags.push(FLAG_PARTIAL_BITS.into());
            suspect.push(format!(
                "Only {e} of {claimed_bits} bits are used (low {} bits always zero); better than CD but not true 24-bit.",
                claimed_bits as i64 - e as i64
            ));
            short.push(format!("Only {e} of {claimed_bits} bits used"));
        } else if let Some(gr) = grid {
            res.flags.push(FLAG_GRID.into());
            suspect.push(format!(
                "Every sample lies on an even grid with a step of {} levels of the 24-bit scale instead of 1, so only \
                 about {} bits carry detail: a {}-bit master scaled by {} dB. Better than CD but not true 24-bit.",
                f(gr.step, 1),
                gr.bits,
                gr.bits,
                fs(gr.gain_db, 1)
            ));
            short.push(format!("{}-bit master scaled into {claimed_bits} bits", gr.bits));
        } else if let Some(e) = eff {
            res.notes
                .push(format!("Bit depth OK: {} significant bits in use.", if info.is_float { e.min(24) } else { e }));
        }
    }

    // ---- sample rate -----------------------------------------------------
    if let Some(uf) = spec.upsampled_from.filter(|uf| rate > *uf) {
        res.flags.push(FLAG_UPSAMPLED.into());
        let drops: Vec<f64> = [imap_get(&spec.cliffs_db, uf), imap_get(&spec.deep_cliffs_db, uf)]
            .into_iter()
            .flatten()
            .filter(|d| d.is_finite())
            .collect();
        let drop = if drops.is_empty() { spec.cutoff_drop_db } else { drops.iter().cloned().fold(f64::MIN, f64::max) };
        let src = fmt_rate_u(uf);
        let mut marks = vec![];
        if spec.upsample_mid_hz.is_finite() && !(spec.upsample_mid_hz > spec.upsample_edge_hz) {
            marks.push(format!("down 6 dB at {} kHz", f(spec.upsample_mid_hz / 1000.0, 1)));
        }
        if spec.upsample_edge_hz.is_finite() {
            marks.push(format!("down 20 dB by {} kHz", f(spec.upsample_edge_hz / 1000.0, 1)));
        }
        let edge = if marks.is_empty() { String::new() } else { format!(" (level is {})", marks.join(" and ")) };
        let mut msg = format!(
            "Spectrum collapses by {} dB around {} kHz{edge}, the Nyquist limit of a {src} kHz master: upsampled from \
             {src} kHz ({} confidence).",
            f(drop, 0),
            f(uf as f64 / 2000.0, 2),
            spec.upsample_confidence.unwrap_or("None")
        );
        if uf >= 88200 {
            msg += &format!(" Still hi-res, but not the advertised {} kHz.", fmt_rate_u(rate));
        }
        fake.push(msg);
        if !spec.lossy_cutoff {
            short.push(format!("Upsampled from {src} kHz: nothing above {} kHz", g(uf as f64 / 2000.0)));
        }
    } else if rate > 48000 && !spec.lossy_cutoff {
        let above_q = spec.ultra_floor_db - spec.quantization_floor_db;
        if spec.bandwidth_hz < c.ultrasonic {
            if converter_noise_floor(spec, cfg) {
                res.flags.push(FLAG_CONVERTER_FLOOR.into());
                res.notes.push(format!(
                    "No musical content above {} kHz, but the converter's analogue noise floor continues evenly from \
                     there to {} kHz, {} dB above the rounding noise and with no step at 22.05 or 24 kHz. An upsampled \
                     file would hold only rounding noise above 22-24 kHz, so this was digitised at {} kHz (typical of \
                     an old or dull analogue recording).",
                    f(bw_k, 1),
                    f(spec.ultra_band_hz / 1000.0, 0),
                    f(above_q, 0),
                    fmt_rate_u(rate)
                ));
            } else if above_q.is_finite() && above_q < c.analog_floor {
                res.flags.push(FLAG_NO_ULTRASONIC.into());
                suspect.push(format!(
                    "No content above {} kHz although the format allows {} kHz, and there is no brick-wall edge. The \
                     ultrasonic band holds only rounding noise, without the analogue noise floor that a converter \
                     running at this rate would leave, so it cannot be shown that it was recorded at {} kHz. Could be \
                     a gently filtered upsample of a quiet digital master, or a digitally low-passed recording; check \
                     the spectrogram.",
                    f(bw_k, 1),
                    f(spec.nyquist / 1000.0, 0),
                    fmt_rate_u(rate)
                ));
                short.push(format!("No content above {} kHz and only rounding noise above it", f(bw_k, 1)));
            } else {
                res.flags.push(FLAG_NO_ULTRASONIC.into());
                suspect.push(format!(
                    "No content above {} kHz although the format allows {} kHz, yet there is no brick-wall edge. The \
                     ultrasonic band holds noise, but it is not an even converter noise floor that continues from the \
                     audio band. Could be a dull/old recording or a gentle-filter upsample; check the spectrogram.",
                    f(bw_k, 1),
                    f(spec.nyquist / 1000.0, 0)
                ));
                short.push(format!("No content above {} kHz, but no brick-wall edge", f(bw_k, 1)));
            }
        } else if spec.bandwidth_hz < c.ultrasonic + SMEAR_HZ && !(above_q >= c.analog_floor) {
            res.flags.push(FLAG_NO_ULTRASONIC.into());
            suspect.push(format!(
                "Content stops at {} kHz, just above the 24 kHz limit of a 48 kHz master, where a soft resampler such \
                 as FFmpeg's default one smears that edge, and the band above holds only rounding noise, without the \
                 analogue noise floor that a converter running at {} kHz would leave. Could be a gently filtered \
                 upsample; check the spectrogram.",
                f(bw_k, 1),
                fmt_rate_u(rate)
            ));
            short.push(format!("Content stops at {} kHz over only rounding noise", f(bw_k, 1)));
        } else {
            res.notes.push(format!(
                "Real ultrasonic content up to {} kHz (format limit {} kHz).",
                f(bw_k, 1),
                f(spec.nyquist / 1000.0, 0)
            ));
        }
    } else if rate <= 48000 {
        res.notes.push(format!("Bandwidth {} kHz.", f(bw_k, 1)));
    }

    // ---- verdict -----------------------------------------------------------
    if spec.lossy_cutoff {
        res.verdict = Verdict::FakeLossless;
        res.reasons = fake.into_iter().chain(suspect).collect();
        if info.claims_hires() {
            res.reasons.push(format!(
                "The file also claims {} hi-res, which a lossy source cannot provide.",
                fmt_res(info.claimed_bits, rate, false)
            ));
        }
    } else if !info.claims_hires() {
        res.verdict = Verdict::NotHires;
        res.reasons = vec![format!(
            "Standard resolution ({}): does not claim to be hi-res. No sign of a lossy source, so it is genuine \
             lossless audio.",
            fmt_res(info.claimed_bits, rate, false)
        )];
        short = vec![format!("Genuine lossless at CD quality, content up to {} kHz", f(bw_k, 1))];
    } else if !fake.is_empty() {
        res.verdict = Verdict::Fake;
        res.reasons = fake.into_iter().chain(suspect).collect();
    } else if !suspect.is_empty() {
        res.verdict = Verdict::Suspect;
        res.reasons = suspect;
    } else {
        res.verdict = Verdict::Real;
        res.reasons = vec![PASSED.into()];
        let real_bits = match eff {
            Some(e) if e != 0 && info.is_float => Some(e.min(24)),
            other => other,
        };
        let rb = opt_bits(real_bits);
        if res.flags.iter().any(|x| x == FLAG_CONVERTER_FLOOR) {
            short = vec![format!(
                "Genuine: dull recording (content up to {} kHz), converter noise to {} kHz, {rb} bits used",
                f(bw_k, 1),
                f(spec.ultra_band_hz / 1000.0, 0)
            )];
        } else if rate > 48000 {
            short = vec![format!("Genuine: content up to {} kHz, {rb} bits used", f(bw_k, 1))];
        } else {
            short = vec![bit_depth_only_summary(&rb, rate)];
            res.reasons.push(bit_depth_only_reason(rate));
        }
    }
    res.summary = short.join("; ");
}

/// Analyse one file.  Never fails: problems become an ERROR result.
/// An explicit `max_seconds` overrides `settings.max_seconds`.
pub fn analyze_file(
    path: &str,
    max_seconds: Option<f64>,
    keep_spectrum: bool,
    keep_plot: bool,
    settings: Option<&Settings>,
) -> FileResult {
    let t0 = Instant::now();
    let mut res = FileResult::new(path, Verdict::Error);
    let max_seconds = max_seconds.or_else(|| settings.map(|s| s.max_seconds).filter(|s| *s > 0.0));
    if let Err(e) = analyze_into(path, max_seconds, keep_spectrum, keep_plot, settings, &mut res) {
        let msg = e.to_string();
        res.verdict = Verdict::Error;
        res.reasons = vec![msg.clone()];
        let first: String = msg.lines().next().unwrap_or("").chars().take(120).collect();
        res.summary = format!("Could not analyse: {}", if msg.is_empty() { "error".into() } else { first });
    }
    res.elapsed = t0.elapsed().as_secs_f64();
    res.path = abspath(path);
    res
}

fn analyze_into(
    path: &str,
    max_seconds: Option<f64>,
    keep_spectrum: bool,
    keep_plot: bool,
    settings: Option<&Settings>,
    res: &mut FileResult,
) -> Result<(), crate::probe::Error> {
    let info = probe(path)?;
    res.codec = info.codec.clone();
    res.sample_rate = info.sample_rate;
    res.channels = info.channels;
    res.claimed_bits = info.claimed_bits;
    res.is_float = info.is_float;
    res.duration = info.duration;
    res.tags = info.tags.clone();
    if info.sample_rate == 0 {
        return Err(crate::probe::err("could not determine sample rate"));
    }
    let mut spec_acc = SpectrumAccumulator::new(info.sample_rate, info.channels, keep_plot);
    let mut bit_acc = BitDepthAccumulator::default();
    // The value-grid test only matters for files that claim more than 16 bits.
    let mut grid_acc = (info.claims_hires()
        && (info.claimed_bits.filter(|b| *b != 0).unwrap_or(16) > 16 || info.is_float))
        .then(|| GridAccumulator::new(info.channels));
    let mut reader = PcmReader::open(path, info.channels, (1usize << 16).max(4 * spec_acc.nfft), max_seconds)?;
    let mut samples: u64 = 0;
    while let Some(block) = reader.next_block()? {
        bit_acc.update(&block);
        if let Some(gacc) = grid_acc.as_mut() {
            gacc.update(&block);
        }
        spec_acc.update_i32(&block);
        samples += block.len() as u64;
    }
    let frames = samples / reader.channels() as u64;
    res.analysed_seconds = frames as f64 / info.sample_rate as f64;

    if bit_acc.nonzero == 0 {
        res.verdict = if info.claims_hires() { Verdict::Suspect } else { Verdict::NotHires };
        res.reasons = vec!["File is digital silence; nothing to analyse.".into()];
        res.summary = "Digital silence: nothing to analyse".into();
        return Ok(());
    }
    let c = detection_params(settings);
    let (freqs, power) = spec_acc.result();
    let grid = grid_acc.as_ref().and_then(|gacc| gacc.result(bit_acc.peak, c.grid_strength));
    let mut eff_bits = bit_acc.effective_bits(c.bit_threshold);
    if let (Some(gr), Some(e)) = (grid, eff_bits) {
        eff_bits = Some(e.min(gr.bits));
    }
    let spec = analyze_spectrum(&freqs, &power, info.sample_rate, spec_acc.noise_gain(), eff_bits, settings);
    if keep_plot {
        let mut p = compact_spectrum(&spec, 900);
        p.spectrogram = spec_acc.spectrogram_result(SPECTRO_MAX_COLUMNS);
        res.plot = Some(p);
    }
    if info.is_lossy {
        res.verdict = Verdict::Lossy;
        res.reasons = vec![format!(
            "Lossy codec ({}); can never be real hi-res or lossless, whatever the {} kHz sample rate says.",
            info.codec,
            fmt_rate_u(info.sample_rate)
        )];
        res.summary = format!("{} file: lossy by design", info.codec.to_uppercase());
        res.bandwidth_hz = Some(spec.bandwidth_hz);
        res.effective_bits = bit_acc.effective_bits(c.bit_threshold);
    } else {
        judge(&info, &spec, &bit_acc, res, settings, grid);
    }
    if keep_spectrum {
        res.spectrum = Some(Box::new(spec));
    }
    Ok(())
}

/// One sentence on where a reused result comes from ("" for a fresh one).
pub fn saved_note(r: &FileResult) -> String {
    let Some(s) = &r.saved else {
        return String::new();
    };
    let when = chrono::DateTime::from_timestamp(s.time as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into());
    let by = format!("Real-Hi-Res {}", if s.version.is_empty() { "?" } else { &s.version });
    let moved = if s.moved_from.is_some() { " (when it had another name or folder)" } else { "" };
    if s.outdated {
        format!(
            "Saved result from {when}{moved}, made by {by} with older detection rules; the file has not changed since, \
             but the current rules may judge it differently. Re-scan it to check."
        )
    } else {
        format!("Saved result from {when}{moved}, made by {by}; the file has not changed since, so it was not analysed again.")
    }
}

/// [(verdict, count, percent)] in VERDICT_ORDER, only verdicts that occur.
pub fn verdict_breakdown<'a>(results: impl IntoIterator<Item = &'a FileResult>) -> Vec<(Verdict, usize, f64)> {
    let mut counts = [0usize; 7];
    let mut total = 0;
    for r in results {
        counts[VERDICT_ORDER.iter().position(|v| *v == r.verdict).unwrap()] += 1;
        total += 1;
    }
    let total = total.max(1) as f64;
    VERDICT_ORDER.iter().zip(counts).filter(|(_, n)| *n > 0).map(|(v, n)| (*v, n, 100.0 * n as f64 / total)).collect()
}
