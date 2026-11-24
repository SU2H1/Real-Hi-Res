//! Equalizer maths shared with the Python version (`realhires/eq.py`, 2.0):
//! a ten-band graphic EQ and a parametric EQ, both made of Audio EQ Cookbook
//! biquads, their response curve, the automatic headroom, and the processor
//! the Rust player filters its samples with.  The numbers equal Python's;
//! `tests/python_eq.json` holds Python's answers.

use crate::json::{Obj as ObjectBuilder, Value};

pub const GRAPHIC_FREQS: [f64; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const GRAPHIC_LABELS: [&str; 10] = ["32", "64", "125", "250", "500", "1K", "2K", "4K", "8K", "16K"];
pub const GRAPHIC_Q: f64 = std::f64::consts::SQRT_2;
pub const GAIN_LIMIT_DB: f64 = 12.0;
pub const MAX_PARAMETRIC_BANDS: usize = 10;
pub const MIN_FREQ: f64 = 20.0;
pub const MAX_FREQ: f64 = 20000.0;
pub const MIN_Q: f64 = 0.1;
pub const MAX_Q: f64 = 10.0;
pub const BAND_TYPES: [&str; 6] = ["peak", "low_shelf", "high_shelf", "low_pass", "high_pass", "notch"];
pub const CUSTOM: &str = "Custom";
pub const RESPONSE_POINTS: usize = 241;
pub const RESPONSE_RATE: f64 = 48000.0;

pub fn band_type_label(kind: &str) -> &'static str {
    match kind {
        "low_shelf" => "Low shelf",
        "high_shelf" => "High shelf",
        "low_pass" => "Low pass",
        "high_pass" => "High pass",
        "notch" => "Notch",
        _ => "Peak",
    }
}

fn has_gain(kind: &str) -> bool {
    matches!(kind, "peak" | "low_shelf" | "high_shelf")
}

pub const PRESETS: [(&str, [f64; 10]); 17] = [
    ("Flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Bass Boost", [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Bass Cut", [-6.0, -5.0, -4.0, -2.5, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble Boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0, 6.0]),
    ("Treble Cut", [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.5, -4.0, -5.0, -6.0]),
    ("Vocal", [-2.0, -2.0, -1.0, 1.0, 3.0, 3.5, 3.0, 1.5, 0.0, -1.0]),
    ("Loudness", [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 1.0, 3.0, 4.0]),
    ("Late Night", [3.0, 2.5, 1.5, 0.5, 0.0, 0.0, 0.5, 1.5, 2.0, 2.0]),
    ("Small Speakers", [5.0, 4.5, 3.5, 2.0, 1.0, 0.0, -0.5, -1.0, -1.5, -2.0]),
    ("Acoustic", [3.0, 3.0, 2.0, 0.5, 1.0, 1.0, 2.0, 2.5, 2.0, 1.0]),
    ("Classical", [3.0, 2.5, 2.0, 1.0, -1.0, -1.0, 0.0, 1.5, 2.5, 3.0]),
    ("Electronic", [4.0, 3.5, 1.0, 0.0, -1.5, 1.0, 0.5, 1.0, 3.5, 4.5]),
    ("Hip-Hop", [4.5, 4.0, 1.5, 2.5, -1.0, -1.0, 1.0, -0.5, 1.5, 2.5]),
    ("Jazz", [3.0, 2.0, 1.0, 2.0, -1.5, -1.5, 0.0, 1.0, 2.0, 3.0]),
    ("Pop", [-1.0, -0.5, 0.0, 1.5, 3.5, 3.5, 1.5, 0.0, -0.5, -1.0]),
    ("Rock", [4.5, 3.5, 2.5, 1.0, -0.5, -1.0, 0.5, 2.0, 3.0, 3.5]),
    ("Spoken Word", [-3.0, -2.0, 0.0, 1.0, 3.0, 4.0, 4.0, 3.0, 1.5, 0.0]),
];

/// b0, b1, b2, a1, a2 (normalised by a0)
pub type Coeffs = [f64; 5];
pub const IDENTITY: Coeffs = [1.0, 0.0, 0.0, 0.0, 0.0];

#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub kind: String,
    pub freq: f64,
    pub gain: f64,
    pub q: f64,
    pub on: bool,
}

impl Default for Band {
    fn default() -> Self {
        Band { kind: "peak".into(), freq: 1000.0, gain: 0.0, q: 1.0, on: true }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EqState {
    pub enabled: bool,
    pub prevent_clipping: bool,
    pub graphic_enabled: bool,
    pub preset: String,
    pub graphic_preamp: f64,
    pub gains: [f64; 10],
    pub parametric_enabled: bool,
    pub parametric_preamp: f64,
    pub bands: Vec<Band>,
}

impl Default for EqState {
    fn default() -> Self {
        EqState {
            enabled: false,
            prevent_clipping: true,
            graphic_enabled: true,
            preset: "Flat".into(),
            graphic_preamp: 0.0,
            gains: [0.0; 10],
            parametric_enabled: false,
            parametric_preamp: 0.0,
            bands: vec![Band::default()],
        }
    }
}

/// Python `_num`: float(value), the default when that fails or is NaN, clamped.
fn num(v: Option<&Value>, default: f64, lo: f64, hi: f64) -> f64 {
    let x = match v {
        Some(Value::Int(i)) => *i as f64,
        Some(Value::Float(f)) => *f,
        Some(Value::Bool(b)) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Some(Value::Str(s)) => match crate::pyunicode::strip(s).replace('_', "").parse::<f64>() {
            Ok(f) => f,
            Err(_) => return default,
        },
        _ => return default,
    };
    if x.is_nan() {
        return default;
    }
    x.max(lo).min(hi)
}

fn truthy(v: Option<&Value>, default: bool) -> bool {
    v.map(|v| v.truthy()).unwrap_or(default)
}

pub fn preset_name(gains: &[f64; 10]) -> String {
    for (name, values) in PRESETS {
        if gains.iter().zip(values.iter()).all(|(a, b)| (a - b).abs() < 1e-9) {
            return name.to_string();
        }
    }
    CUSTOM.to_string()
}

impl EqState {
    /// Python `normalize`: a valid state from whatever was stored.
    pub fn from_json(v: &Value) -> EqState {
        let mut out = EqState::default();
        if !matches!(v, Value::Object(_)) {
            return out;
        }
        out.enabled = truthy(v.get("enabled"), out.enabled);
        out.prevent_clipping = truthy(v.get("prevent_clipping"), true);
        if let Some(g @ Value::Object(_)) = v.get("graphic") {
            out.graphic_enabled = truthy(g.get("enabled"), true);
            out.graphic_preamp = num(g.get("preamp"), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB);
            if let Some(Value::Array(gains)) = g.get("gains") {
                if gains.len() == 10 {
                    for (i, x) in gains.iter().enumerate() {
                        out.gains[i] = num(Some(x), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB);
                    }
                }
            }
            out.preset = match g.get("preset").and_then(Value::as_str) {
                Some(p) if p == CUSTOM || PRESETS.iter().any(|(n, _)| *n == p) => p.to_string(),
                _ => preset_name(&out.gains),
            };
        }
        if let Some(p @ Value::Object(_)) = v.get("parametric") {
            out.parametric_enabled = truthy(p.get("enabled"), false);
            out.parametric_preamp = num(p.get("preamp"), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB);
            if let Some(Value::Array(bands)) = p.get("bands") {
                out.bands = bands
                    .iter()
                    .take(MAX_PARAMETRIC_BANDS)
                    .filter(|b| matches!(b, Value::Object(_)))
                    .map(Band::from_json)
                    .collect();
            }
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let bands: Vec<Value> = self.bands.iter().map(Band::to_json).collect();
        let graphic = ObjectBuilder::new()
            .put("enabled", self.graphic_enabled)
            .put("preset", self.preset.as_str())
            .put("preamp", self.graphic_preamp)
            .put("gains", Value::Array(self.gains.iter().map(|g| Value::Float(*g)).collect()))
            .build();
        let parametric = ObjectBuilder::new()
            .put("enabled", self.parametric_enabled)
            .put("preamp", self.parametric_preamp)
            .put("bands", Value::Array(bands))
            .build();
        ObjectBuilder::new()
            .put("enabled", self.enabled)
            .put("prevent_clipping", self.prevent_clipping)
            .put("graphic", graphic)
            .put("parametric", parametric)
            .build()
    }

    pub fn apply_preset(&mut self, name: &str) {
        if let Some((n, values)) = PRESETS.iter().find(|(n, _)| *n == name) {
            self.gains = *values;
            self.preset = n.to_string();
        }
    }
}

impl Band {
    pub fn from_json(b: &Value) -> Band {
        let kind = b.get("type").and_then(Value::as_str).filter(|k| BAND_TYPES.contains(k)).unwrap_or("peak");
        Band {
            kind: kind.to_string(),
            freq: num(b.get("freq"), 1000.0, MIN_FREQ, MAX_FREQ),
            gain: num(b.get("gain"), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB),
            q: num(b.get("q"), 1.0, MIN_Q, MAX_Q),
            on: truthy(b.get("on"), true),
        }
    }

    pub fn to_json(&self) -> Value {
        ObjectBuilder::new()
            .put("type", self.kind.as_str())
            .put("freq", self.freq)
            .put("gain", self.gain)
            .put("q", self.q)
            .put("on", self.on)
            .build()
    }
}

/// Normalised RBJ cookbook coefficients (Python `eq.biquad`).
pub fn biquad(kind: &str, freq: f64, gain_db: f64, q: f64, rate: f64) -> Coeffs {
    if rate <= 0.0 || freq <= 0.0 || freq >= 0.49 * rate || q <= 0.0 {
        return IDENTITY;
    }
    if has_gain(kind) && gain_db == 0.0 {
        return IDENTITY;
    }
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * freq / rate;
    let (cw, sw) = (w0.cos(), w0.sin());
    let alpha = sw / (2.0 * q);
    let (b0, b1, b2, a0, a1, a2);
    match kind {
        "peak" => {
            b0 = 1.0 + alpha * a;
            b1 = -2.0 * cw;
            b2 = 1.0 - alpha * a;
            a0 = 1.0 + alpha / a;
            a1 = -2.0 * cw;
            a2 = 1.0 - alpha / a;
        }
        "low_shelf" | "high_shelf" => {
            let sa = 2.0 * a.sqrt() * alpha;
            if kind == "low_shelf" {
                b0 = a * ((a + 1.0) - (a - 1.0) * cw + sa);
                b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cw);
                b2 = a * ((a + 1.0) - (a - 1.0) * cw - sa);
                a0 = (a + 1.0) + (a - 1.0) * cw + sa;
                a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cw);
                a2 = (a + 1.0) + (a - 1.0) * cw - sa;
            } else {
                b0 = a * ((a + 1.0) + (a - 1.0) * cw + sa);
                b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cw);
                b2 = a * ((a + 1.0) + (a - 1.0) * cw - sa);
                a0 = (a + 1.0) - (a - 1.0) * cw + sa;
                a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cw);
                a2 = (a + 1.0) - (a - 1.0) * cw - sa;
            }
        }
        "low_pass" => {
            b0 = (1.0 - cw) / 2.0;
            b1 = 1.0 - cw;
            b2 = (1.0 - cw) / 2.0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * cw;
            a2 = 1.0 - alpha;
        }
        "high_pass" => {
            b0 = (1.0 + cw) / 2.0;
            b1 = -(1.0 + cw);
            b2 = (1.0 + cw) / 2.0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * cw;
            a2 = 1.0 - alpha;
        }
        "notch" => {
            b0 = 1.0;
            b1 = -2.0 * cw;
            b2 = 1.0;
            a0 = 1.0 + alpha;
            a1 = -2.0 * cw;
            a2 = 1.0 - alpha;
        }
        _ => return IDENTITY,
    }
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// Every active biquad, graphic bands first (Python `eq.stages`).
pub fn stages(s: &EqState, rate: f64) -> Vec<Coeffs> {
    let mut out = vec![];
    if !s.enabled {
        return out;
    }
    if s.graphic_enabled {
        for (f, g) in GRAPHIC_FREQS.iter().zip(s.gains.iter()) {
            let c = biquad("peak", *f, *g, GRAPHIC_Q, rate);
            if c != IDENTITY {
                out.push(c);
            }
        }
    }
    if s.parametric_enabled {
        for b in s.bands.iter().filter(|b| b.on) {
            let c = biquad(&b.kind, b.freq, b.gain, b.q, rate);
            if c != IDENTITY {
                out.push(c);
            }
        }
    }
    out
}

/// The combined gain of the biquads at one frequency, in dB.
pub fn magnitude_db(coeffs: &[Coeffs], freq: f64, rate: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / rate;
    let (zr, zi) = (w.cos(), -w.sin()); // e^{-jw}
    let (z2r, z2i) = (zr * zr - zi * zi, 2.0 * zr * zi);
    let mut total = 0.0;
    for [b0, b1, b2, a1, a2] in coeffs {
        let (nr, ni) = (b0 + b1 * zr + b2 * z2r, b1 * zi + b2 * z2i);
        let (dr, di) = (1.0 + a1 * zr + a2 * z2r, a1 * zi + a2 * z2i);
        let mag = (nr.hypot(ni)) / (dr.hypot(di));
        total += 20.0 * mag.max(1e-12).log10();
    }
    total
}

pub fn response_freqs() -> Vec<f64> {
    (0..RESPONSE_POINTS).map(|i| 20.0 * 10f64.powf(3.0 * i as f64 / (RESPONSE_POINTS - 1) as f64)).collect()
}

/// (frequency, dB) of the whole EQ including the preamp, for drawing the curve.
pub fn response(s: &EqState, rate: f64) -> Vec<(f64, f64)> {
    let coeffs = stages(s, rate);
    let pre = preamp_db(s, RESPONSE_RATE);
    response_freqs().into_iter().map(|f| (f, magnitude_db(&coeffs, f, rate) + pre)).collect()
}

/// The gain before the filters: the preamps, lowered by the curve's largest
/// boost when "prevent clipping" is on (Python `eq.preamp_db`).
pub fn preamp_db(s: &EqState, rate: f64) -> f64 {
    if !s.enabled {
        return 0.0;
    }
    let mut pre = 0.0;
    if s.graphic_enabled {
        pre += s.graphic_preamp;
    }
    if s.parametric_enabled {
        pre += s.parametric_preamp;
    }
    if s.prevent_clipping {
        let coeffs = stages(s, rate);
        let peak = response_freqs()
            .into_iter()
            .map(|f| magnitude_db(&coeffs, f, rate))
            .fold(f64::NEG_INFINITY, f64::max)
            .max(f64::NEG_INFINITY);
        let peak = if peak.is_finite() { peak } else { 0.0 } + pre;
        if peak > 0.0 {
            pre -= peak;
        }
    }
    pre
}

/// Filters interleaved samples through the EQ (transposed direct form II,
/// double precision state), as the Python reference `eq.Biquad` does.
pub struct EqProcessor {
    rate: f64,
    channels: usize,
    gain: f64,
    coeffs: Vec<Coeffs>,
    z1: Vec<f64>,
    z2: Vec<f64>,
}

impl EqProcessor {
    pub fn new(state: &EqState, rate: f64, channels: usize) -> EqProcessor {
        let mut p = EqProcessor { rate, channels, gain: 1.0, coeffs: vec![], z1: vec![], z2: vec![] };
        p.set(state);
        p
    }

    /// New settings; the filter memory is kept when the stages stay the same.
    pub fn set(&mut self, state: &EqState) {
        let coeffs = stages(state, self.rate);
        self.gain = 10f64.powf(preamp_db(state, RESPONSE_RATE) / 20.0);
        if coeffs.len() != self.coeffs.len() {
            self.z1 = vec![0.0; coeffs.len() * self.channels];
            self.z2 = vec![0.0; coeffs.len() * self.channels];
        }
        self.coeffs = coeffs;
    }

    pub fn is_active(&self) -> bool {
        !self.coeffs.is_empty() || (self.gain - 1.0).abs() > 1e-12
    }

    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.is_active() {
            return;
        }
        let ch = self.channels.max(1);
        for (i, s) in samples.iter_mut().enumerate() {
            let c = i % ch;
            let mut x = *s as f64 * self.gain;
            for (k, [b0, b1, b2, a1, a2]) in self.coeffs.iter().enumerate() {
                let idx = k * ch + c;
                let y = b0 * x + self.z1[idx];
                self.z1[idx] = b1 * x - a1 * y + self.z2[idx];
                self.z2[idx] = b2 * x - a2 * y;
                x = y;
            }
            *s = x as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn coefficients_and_states_equal_python() {
        let data = json::parse(include_str!("../tests/python_eq.json")).unwrap();
        for case in data.get("biquads").unwrap().as_array().unwrap() {
            let c = biquad(
                case.get("kind").unwrap().as_str().unwrap(),
                case.get("freq").unwrap().as_f64().unwrap(),
                case.get("gain").unwrap().as_f64().unwrap(),
                case.get("q").unwrap().as_f64().unwrap(),
                case.get("rate").unwrap().as_f64().unwrap(),
            );
            let want: Vec<f64> =
                case.get("coeffs").unwrap().as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            for (a, b) in c.iter().zip(want.iter()) {
                assert!((a - b).abs() <= 1e-12 * b.abs().max(1.0), "{case:?}: {c:?}");
            }
        }
        for st in data.get("states").unwrap().as_array().unwrap() {
            let s = EqState::from_json(st.get("state").unwrap());
            assert_eq!(s, EqState::from_json(st.get("normalized").unwrap()));
            assert_eq!(EqState::from_json(&s.to_json()), s);
            let pre = st.get("preamp").unwrap().as_f64().unwrap();
            assert!((preamp_db(&s, RESPONSE_RATE) - pre).abs() < 1e-9, "{pre}");
            let stg = stages(&s, 44100.0);
            let want = st.get("stages_44100").unwrap().as_array().unwrap();
            assert_eq!(stg.len(), want.len());
            let r = response(&s, RESPONSE_RATE);
            for (i, pair) in st.get("response").unwrap().as_array().unwrap().iter().enumerate() {
                let p = pair.as_array().unwrap();
                let (f, db) = r[i * 20];
                assert!((f - p[0].as_f64().unwrap()).abs() < 1e-9);
                assert!((db - p[1].as_f64().unwrap()).abs() < 1e-9, "{db} vs {:?}", p[1]);
            }
        }
        let presets = data.get("presets").unwrap().as_object().unwrap();
        assert_eq!(presets.len(), PRESETS.len());
        for ((name, values), (pn, pv)) in presets.iter().zip(PRESETS.iter()) {
            assert_eq!(name, pn);
            let v: Vec<f64> = values.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            assert_eq!(v, pv.to_vec());
        }
    }

    #[test]
    fn processor_matches_the_reference_and_skips_when_flat() {
        let mut s = EqState::default();
        s.enabled = true;
        s.apply_preset("Rock");
        let mut p = EqProcessor::new(&s, 44100.0, 2);
        let mut buf: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.37).sin() * 0.2).collect();
        let orig = buf.clone();
        p.process(&mut buf);
        assert!(buf.iter().zip(orig.iter()).any(|(a, b)| (a - b).abs() > 1e-4));
        let mut flat = EqProcessor::new(&EqState::default(), 44100.0, 2);
        let mut same = orig.clone();
        flat.process(&mut same);
        assert_eq!(same, orig);
    }
}
