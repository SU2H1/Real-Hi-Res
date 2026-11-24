//! Signal analysis: averaged power spectrum, effective bit depth, value grid
//! and the spectral detection rules (the Rust counterpart of
//! `realhires/analysis.py`, kept line for line equivalent).

use std::io::{Read, Write};
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

use crate::dsp::{blackman_harris, hanning, linspace, median, moving_average_same, percentile, slope};
use crate::settings::Settings;

/// Sample rates a master could realistically have been made at before it was upsampled.
pub const SOURCE_RATES: [u32; 6] = [44100, 48000, 88200, 96000, 176400, 192000];

const EPS: f64 = 1e-30;

pub const SPECTRO_BANDS: usize = 256;
pub const SPECTRO_ROW_SECONDS: f64 = 0.25;
pub const SPECTRO_MAX_COLUMNS: usize = 1200;
pub const SPECTRO_DB_MIN: f64 = -230.0;

// Tunables (dB).  See README "How detection works" for the reasoning.
pub const CONTENT_MARGIN_DB: f64 = 10.0;
pub const STEEP_DROP_DB: f64 = 20.0;
pub const CLIFF_DB: f64 = 25.0;
pub const CLIFF_STRONG_DB: f64 = 40.0;
pub const DEEP_CLIFF_DB: f64 = 45.0;
pub const DEEP_CLIFF_STRONG_DB: f64 = 60.0;
pub const MID_RATIO: f64 = 0.975;
pub const MAX_TREND_DB_PER_OCT: f64 = 45.0;
pub const ANALOG_FLOOR_DB: f64 = 15.0;
pub const FLOOR_TILT_DB: f64 = 6.0;
pub const FLOOR_STEP_DB: f64 = 8.0;
pub const FLOOR_DEEP_STEP_DB: f64 = 15.0;
pub const LOSSY_MAX_HZ: f64 = 20300.0;
pub const LOSSY_CLIFF_DB: f64 = 25.0;
pub const LOSSY_MIN_HZ: f64 = 11000.0;
pub const BIT_THRESHOLD: f64 = 0.005;
pub const RELEVANT_RANGE_DB: f64 = 110.0;
pub const BANDWIDTH_GAP_HZ: f64 = 3000.0;
pub const ULTRASONIC_HZ: f64 = 24500.0;
pub const SMEAR_HZ: f64 = 4300.0;

pub const GRID_WINDOW: i64 = 1 << 16;
pub const GRID_MIN_SAMPLES: f64 = 5000.0;
pub const GRID_MIN_STEP: f64 = 6.0;
pub const GRID_MAX_STEP: f64 = 2048.0;
pub const GRID_STRENGTH: f64 = 0.35;
pub const GRID_FULL: f64 = 0.8;

/// An ordered map with integer keys (a Python dict keyed by sample rate or bit position).
pub type IntMap = Vec<(u32, f64)>;

pub fn imap_get(m: &IntMap, k: u32) -> Option<f64> {
    m.iter().find(|(key, _)| *key == k).map(|(_, v)| *v)
}

/// FFT size giving roughly 6 Hz resolution at any sample rate.
pub fn choose_nfft(sample_rate: u32) -> usize {
    let r = (sample_rate.max(8000) as f64 / 6.0).log2().ceil() as u32;
    (1usize << r).clamp(4096, 1 << 17)
}

/// Compact spectrogram: dB levels quantised to bytes (1 dB steps from
/// `db_min`), zlib-compressed, time on axis 0 and frequency on axis 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrogram {
    pub data: Vec<u8>,
    pub shape: (usize, usize),
    pub db_min: f64,
    pub nyquist: f64,
    pub seconds_per_column: f64,
}

impl Spectrogram {
    /// The dB matrix, row-major (time, frequency).
    pub fn decode(&self) -> Option<Vec<f32>> {
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&self.data[..]).read_to_end(&mut raw).ok()?;
        if raw.len() != self.shape.0 * self.shape.1 {
            return None;
        }
        let base = self.db_min as f32;
        Some(raw.into_iter().map(|q| q as f32 + base).collect())
    }
}

/// Welch-style averaged power spectrum (50 % overlap, all channels summed).
pub struct SpectrumAccumulator {
    pub sample_rate: u32,
    pub channels: usize,
    pub nfft: usize,
    pub hop: usize,
    window: Vec<f64>,
    power: Vec<f64>,
    pub frames: u64,
    /// samples not yet used, per channel
    tail: Vec<Vec<f64>>,
    norm: f64,
    fft: Arc<dyn RealToComplex<f64>>,
    scratch_in: Vec<f64>,
    scratch_out: Vec<realfft::num_complex::Complex<f64>>,
    frame_power: Vec<f64>,
    // spectrogram
    spectrogram: bool,
    edges: Vec<usize>,
    rows_per: usize,
    rows: Vec<Vec<f32>>,
    pending: Vec<Vec<f64>>,
}

impl SpectrumAccumulator {
    pub fn new(sample_rate: u32, channels: u32, spectrogram: bool) -> Self {
        let nfft = choose_nfft(sample_rate);
        let hop = nfft / 2;
        let window = blackman_harris(nfft);
        let channels = channels.max(1) as usize;
        let wsum: f64 = window.iter().sum();
        let fft = RealFftPlanner::<f64>::new().plan_fft_forward(nfft);
        let scratch_in = fft.make_input_vec();
        let scratch_out = fft.make_output_vec();
        let nbins = nfft / 2 + 1;
        let edges: Vec<usize> = linspace(0.0, nbins as f64, SPECTRO_BANDS + 1).iter().map(|e| *e as usize).collect();
        let rows_per = ((SPECTRO_ROW_SECONDS * sample_rate as f64 / hop as f64).round_ties_even() as usize).max(1);
        SpectrumAccumulator {
            sample_rate,
            channels,
            nfft,
            hop,
            power: vec![0.0; nbins],
            frames: 0,
            tail: vec![Vec::new(); channels],
            norm: (wsum / 2.0).powi(2) * channels as f64,
            window,
            fft,
            scratch_in,
            scratch_out,
            frame_power: vec![0.0; nbins],
            spectrogram,
            edges,
            rows_per,
            rows: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Spectrum level of unit-variance white noise (per channel).
    pub fn noise_gain(&self) -> f64 {
        let s2: f64 = self.window.iter().map(|w| w * w).sum();
        let s: f64 = self.window.iter().sum();
        4.0 * s2 / (s * s)
    }

    /// `block`: interleaved int32 samples, left-justified as from ffmpeg.
    pub fn update_i32(&mut self, block: &[i32]) {
        let scale = 1.0 / 2147483648.0;
        let ch = self.channels;
        for (i, s) in block.iter().enumerate() {
            self.tail[i % ch].push(*s as f64 * scale);
        }
        let n = self.tail[0].len();
        if n < self.nfft {
            return;
        }
        let count = 1 + (n - self.nfft) / self.hop;
        for f in 0..count {
            let start = f * self.hop;
            self.frame_power.iter_mut().for_each(|p| *p = 0.0);
            for c in 0..ch {
                let x = &self.tail[c][start..start + self.nfft];
                for ((o, s), w) in self.scratch_in.iter_mut().zip(x).zip(&self.window) {
                    *o = s * w;
                }
                self.fft.process(&mut self.scratch_in, &mut self.scratch_out).expect("fft");
                for (p, z) in self.frame_power.iter_mut().zip(&self.scratch_out) {
                    *p += z.re * z.re + z.im * z.im;
                }
            }
            for (acc, p) in self.power.iter_mut().zip(&self.frame_power) {
                *acc += p;
            }
            if self.spectrogram {
                let bands: Vec<f64> = (0..SPECTRO_BANDS)
                    .map(|b| {
                        let (lo, hi) = (self.edges[b], self.edges[b + 1]);
                        self.frame_power[lo..hi].iter().sum::<f64>() / (hi - lo) as f64
                    })
                    .collect();
                self.pending.push(bands);
                if self.pending.len() == self.rows_per {
                    self.rows.push(mean_row(&self.pending));
                    self.pending.clear();
                }
            }
        }
        self.frames += count as u64;
        let used = count * self.hop;
        for t in &mut self.tail {
            t.drain(..used);
        }
    }

    /// (frequencies, mean power per bin, normalised so a full-scale sine reads 0 dB).
    pub fn result(&mut self) -> (Vec<f64>, Vec<f64>) {
        if self.frames == 0 && !self.tail[0].is_empty() {
            // Very short file: analyse the single zero-padded frame we have.
            for c in 0..self.channels {
                let x = &self.tail[c];
                for (i, (o, w)) in self.scratch_in.iter_mut().zip(&self.window).enumerate() {
                    *o = x.get(i).copied().unwrap_or(0.0) * w;
                }
                self.fft.process(&mut self.scratch_in, &mut self.scratch_out).expect("fft");
                for (p, z) in self.power.iter_mut().zip(&self.scratch_out) {
                    *p += z.norm_sqr();
                }
            }
            self.frames = 1;
        }
        let d = 1.0 / self.sample_rate as f64;
        let val = 1.0 / (self.nfft as f64 * d);
        let freqs: Vec<f64> = (0..self.power.len()).map(|k| k as f64 * val).collect();
        let div = self.frames.max(1) as f64;
        let mean = self.power.iter().map(|p| p / div / self.norm).collect();
        (freqs, mean)
    }

    pub fn spectrogram_result(&self, max_columns: usize) -> Option<Spectrogram> {
        if !self.spectrogram {
            return None;
        }
        let mut rows: Vec<Vec<f64>> =
            self.rows.iter().map(|r| r.iter().map(|v| *v as f64 / self.norm).collect()).collect();
        if !self.pending.is_empty() {
            rows.push(mean_row(&self.pending).iter().map(|v| *v as f64 / self.norm).collect());
        }
        if rows.is_empty() {
            return None;
        }
        let t = rows.len();
        let group = t.div_ceil(max_columns);
        if group > 1 {
            let pad = (group - t % group) % group;
            let last = rows[t - 1].clone();
            rows.extend(std::iter::repeat_n(last, pad));
            rows = rows
                .chunks(group)
                .map(|g| (0..SPECTRO_BANDS).map(|b| g.iter().map(|r| r[b]).sum::<f64>() / group as f64).collect())
                .collect();
        }
        let shape = (rows.len(), SPECTRO_BANDS);
        let q: Vec<u8> = rows
            .iter()
            .flatten()
            .map(|m| (10.0 * (m + EPS).log10() - SPECTRO_DB_MIN).round_ties_even().clamp(0.0, 255.0) as u8)
            .collect();
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
        enc.write_all(&q).ok()?;
        let data = enc.finish().ok()?;
        let seconds_per_column = (self.rows_per * group * self.hop) as f64 / self.sample_rate as f64;
        Some(Spectrogram {
            data,
            shape,
            db_min: SPECTRO_DB_MIN,
            nyquist: self.sample_rate as f64 / 2.0,
            seconds_per_column,
        })
    }
}

fn mean_row(rows: &[Vec<f64>]) -> Vec<f32> {
    let n = rows.len() as f64;
    (0..rows[0].len()).map(|b| (rows.iter().map(|r| r[b]).sum::<f64>() / n) as f32).collect()
}

/// Histogram of the lowest set bit of every non-zero int32 sample.
///
/// Real 24-bit audio sets bit 8 (the 24-bit LSB after ffmpeg's left
/// justification) in about half of all samples.  16-bit audio padded to 24 bits
/// never sets anything below bit 16.
#[derive(Clone, Debug)]
pub struct BitDepthAccumulator {
    pub lowest_bit_hist: [u64; 33],
    pub nonzero: u64,
    pub total: u64,
    pub peak: i64,
}

impl Default for BitDepthAccumulator {
    fn default() -> Self {
        BitDepthAccumulator { lowest_bit_hist: [0; 33], nonzero: 0, total: 0, peak: 0 }
    }
}

impl BitDepthAccumulator {
    pub fn update(&mut self, block: &[i32]) {
        self.total += block.len() as u64;
        for &s in block {
            if s != 0 {
                self.nonzero += 1;
                self.lowest_bit_hist[(s as u32).trailing_zeros() as usize] += 1;
                self.peak = self.peak.max((s as i64).abs());
            }
        }
    }

    /// Number of significant bits, ignoring stray outliers below `threshold` of samples.
    pub fn effective_bits(&self, threshold: f64) -> Option<u32> {
        if self.nonzero == 0 {
            return None;
        }
        let mut cum = 0u64;
        let mut lsb = 0usize;
        for (b, n) in self.lowest_bit_hist.iter().enumerate() {
            cum += n;
            if cum as f64 / self.nonzero as f64 >= threshold {
                lsb = b;
                break;
            }
        }
        Some(32 - lsb as u32)
    }

    /// Share of non-zero samples whose lowest set bit is at each bit-depth
    /// position (24 = the 24-bit LSB ...), highest position first.
    pub fn bit_usage(&self) -> IntMap {
        if self.nonzero == 0 {
            return vec![];
        }
        (0..33)
            .filter(|b| self.lowest_bit_hist[*b] > 0)
            .map(|b| (32 - b as u32, self.lowest_bit_hist[b] as f64 / self.nonzero as f64))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// spacing of the occupied values, in 24-bit LSBs
    pub step: f64,
    /// normalised comb strength, 1.0 = every sample exactly on the grid
    pub strength: f64,
    /// resolution of the master the grid implies
    pub bits: u32,
    /// gain that turned a `bits`-deep master into this grid
    pub gain_db: f64,
}

/// (step, strength) of the periodic comb in one channel's value histogram,
/// (0.0, strength) when there is none, or None when there are too few samples.
pub fn grid_step(hist: &[u32], strength: f64) -> Option<(f64, f64)> {
    let n = hist.len();
    let mut h: Vec<f64> = hist.iter().map(|v| *v as f64).collect();
    let c = n / 2;
    for v in &mut h[c - 7..c + 8] {
        *v = 0.0; // digital and dithered silence form a spike at zero
    }
    if h.iter().sum::<f64>() < GRID_MIN_SAMPLES {
        return None;
    }
    let win = hanning(n);
    for (v, w) in h.iter_mut().zip(&win) {
        *v *= w;
    }
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
    let mut out = fft.make_output_vec();
    fft.process(&mut h, &mut out).ok()?;
    let spec: Vec<f64> = out.iter().map(|z| z.norm()).collect();
    if spec[0] <= 0.0 {
        return None;
    }
    let r: Vec<f64> = spec.iter().map(|v| v / spec[0]).collect();
    let kmin = (n as f64 / GRID_MAX_STEP).ceil() as usize;
    let kmax = (n as f64 / GRID_MIN_STEP) as usize;
    let seg = &r[kmin..=kmax.min(r.len() - 1)];
    let peak = seg.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if peak < strength {
        return Some((0.0, peak));
    }
    // The fundamental is the lowest-frequency tooth of the comb's spectrum.
    let mut k = kmin + seg.iter().position(|v| *v >= 0.6 * peak).unwrap_or(0);
    let lo = k.saturating_sub(2).max(1);
    let hi = (k + 3).min(r.len() - 1);
    let mut best = lo;
    for i in lo..hi {
        if r[i] > r[best] {
            best = i;
        }
    }
    k = best;
    let (a, b, g) = ((r[k - 1] + 1e-12).ln(), (r[k] + 1e-12).ln(), (r[k + 1] + 1e-12).ln());
    let den = a - 2.0 * b + g;
    let kf = k as f64 + if den < 0.0 { 0.5 * (a - g) / den } else { 0.0 };
    let k2 = (2.0 * kf).round_ties_even() as usize;
    if k2 + 3 < r.len() {
        let m = r[k2 - 3..k2 + 4].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        if m < 0.4 * r[k] {
            return Some((0.0, r[k])); // no second harmonic: not a comb
        }
    }
    Some((n as f64 / kf, r[k]))
}

/// Per-channel histogram of the 24-bit sample values near zero, for the
/// value-grid test (a 16-bit master given a gain change or dither on its way
/// into a 24-bit or float file still sits on a regular grid).
pub struct GridAccumulator {
    pub window: i64,
    pub channels: usize,
    file_channels: usize,
    hist: Vec<u32>,
}

impl GridAccumulator {
    pub fn new(channels: u32) -> Self {
        let file_channels = channels.max(1) as usize;
        let channels = file_channels.clamp(1, 8);
        GridAccumulator {
            window: GRID_WINDOW,
            channels,
            file_channels,
            hist: vec![0; channels * 2 * GRID_WINDOW as usize],
        }
    }

    pub fn update(&mut self, block: &[i32]) {
        let w = self.window;
        let size = 2 * w as usize;
        for frame in block.chunks_exact(self.file_channels) {
            for c in 0..self.channels {
                let v = (frame[c] >> 8) as i64;
                if v > -w && v < w {
                    self.hist[c * size + (v + w) as usize] += 1;
                }
            }
        }
    }

    /// The grid shared by every channel that has enough samples, or None.
    pub fn result(&self, peak: i64, strength: f64) -> Option<Grid> {
        let size = 2 * self.window as usize;
        let mut found = Vec::new();
        for c in 0..self.channels {
            match grid_step(&self.hist[c * size..(c + 1) * size], strength) {
                None => continue,
                Some((s, _)) if s <= 0.0 => return None,
                Some(res) => found.push(res),
            }
        }
        if found.is_empty() || peak <= 0 {
            return None;
        }
        let steps: Vec<f64> = found.iter().map(|(s, _)| *s).collect();
        let step = median(&steps);
        let strength = found.iter().map(|(_, r)| *r).fold(f64::INFINITY, f64::min);
        let spans = (2.0 * (peak as f64 / 256.0) / step).log2();
        let bits = if spans <= 16.1 { 16 } else { (spans - 0.1).ceil() as u32 };
        if bits >= 23 {
            return None;
        }
        let gain_db = 20.0 * (step / 2f64.powi(24 - bits as i32)).log10();
        Some(Grid { step, strength, bits, gain_db })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SpectrumStats {
    pub sample_rate: u32,
    pub nyquist: f64,
    pub noise_floor_db: f64,
    pub bandwidth_hz: f64,
    pub cutoff_drop_db: f64,
    pub cutoff_steep: bool,
    pub cliffs_db: IntMap,
    pub deep_cliffs_db: IntMap,
    pub upsampled_from: Option<u32>,
    pub upsample_confidence: Option<&'static str>,
    pub upsample_edge_hz: f64,
    pub upsample_mid_hz: f64,
    pub ultra_floor_db: f64,
    pub ultra_tilt_db: f64,
    pub ultra_band_hz: f64,
    pub lossy_cutoff: bool,
    pub lossy_cutoff_hz: f64,
    pub lossy_drop_db: f64,
    pub quantization_floor_db: f64,
    pub threshold_db: f64,
    pub peak_db: f64,
    pub freqs: Vec<f64>,
    pub level_db: Vec<f64>,
}

/// The detection thresholds actually used (from the settings, or the defaults).
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub content_margin: f64,
    pub steep: f64,
    pub cliff: f64,
    pub strong: f64,
    pub deep_cliff: f64,
    pub deep_strong: f64,
    pub mid_ratio: f64,
    pub lossy_min: f64,
    pub lossy_max: f64,
    pub lossy_cliff: f64,
    pub relevant: f64,
    pub ultrasonic: f64,
    pub analog_floor: f64,
    pub grid_strength: f64,
    pub bit_threshold: f64,
}

pub fn detection_params(cfg: Option<&Settings>) -> Params {
    match cfg {
        None => Params {
            content_margin: CONTENT_MARGIN_DB,
            steep: STEEP_DROP_DB,
            cliff: CLIFF_DB,
            strong: CLIFF_STRONG_DB,
            deep_cliff: DEEP_CLIFF_DB,
            deep_strong: DEEP_CLIFF_STRONG_DB,
            mid_ratio: MID_RATIO,
            lossy_min: LOSSY_MIN_HZ,
            lossy_max: LOSSY_MAX_HZ,
            lossy_cliff: LOSSY_CLIFF_DB,
            relevant: RELEVANT_RANGE_DB,
            ultrasonic: ULTRASONIC_HZ,
            analog_floor: ANALOG_FLOOR_DB,
            grid_strength: GRID_STRENGTH,
            bit_threshold: BIT_THRESHOLD,
        },
        Some(c) => Params {
            content_margin: c.content_margin_db,
            steep: c.steep_drop_db,
            cliff: c.cliff_db,
            strong: c.cliff_strong_db,
            deep_cliff: c.deep_cliff_db,
            deep_strong: c.deep_cliff_strong_db,
            mid_ratio: c.mid_ratio,
            lossy_min: c.lossy_min_khz * 1000.0,
            lossy_max: c.lossy_max_khz * 1000.0,
            lossy_cliff: c.lossy_cliff_db,
            relevant: c.relevant_range_db,
            ultrasonic: c.ultrasonic_khz * 1000.0,
            analog_floor: c.analog_floor_db,
            grid_strength: c.grid_strength,
            bit_threshold: c.bit_threshold_pct / 100.0,
        },
    }
}

/// Per-bin level of white rounding noise for a `bits`-deep integer signal.
pub fn quantization_floor_db(bits: u32, noise_gain: f64) -> f64 {
    let step = 2f64.powi(1 - bits as i32);
    10.0 * (step * step / 12.0 * noise_gain).log10()
}

/// The spectrum with its frequencies, and the band queries the rules use.
struct Spec<'a> {
    f: &'a [f64],
    db: &'a [f64],
}

impl<'a> Spec<'a> {
    /// Index range of lo <= f <= hi (frequencies are increasing).
    fn range(&self, lo: f64, hi: f64) -> std::ops::Range<usize> {
        let a = self.f.partition_point(|x| *x < lo);
        let b = self.f.partition_point(|x| *x <= hi);
        a..b.max(a)
    }

    fn band(&self, lo: f64, hi: f64) -> f64 {
        median(&self.db[self.range(lo, hi)])
    }

    fn percentile_band(&self, lo: f64, hi: f64, q: f64) -> f64 {
        percentile(&self.db[self.range(lo, hi)], q)
    }

    fn edge_frequency(&self, lo: f64, hi: f64, ref_lo: f64, ref_hi: f64, fall: f64) -> f64 {
        let r = self.band(ref_lo, ref_hi);
        self.range(lo, hi).find(|i| self.db[*i] < r - fall).map(|i| self.f[i]).unwrap_or(f64::NAN)
    }

    /// First frequency in [lo, hi] where the level stays `fall` dB below the
    /// music's own trend for at least ~300 Hz.
    fn midpoint_frequency(&self, ref_lo: f64, ref_hi: f64, lo: f64, hi: f64, fall: f64) -> f64 {
        let m = self.range(ref_lo, ref_hi);
        if m.len() < 8 {
            return f64::NAN;
        }
        let x: Vec<f64> = self.f[m.clone()].iter().map(|v| v.log2()).collect();
        let s = slope(&x, &self.db[m]).clamp(-MAX_TREND_DB_PER_OCT, 0.0); // NaN stays NaN, as in Python
        let anchor = self.band(0.97 * ref_hi, ref_hi);
        let icpt = anchor - s * (0.985 * ref_hi).log2();
        let idx = self.range(lo, hi);
        if idx.is_empty() {
            return f64::NAN;
        }
        let below: Vec<bool> = idx.clone().map(|i| self.db[i] < icpt + s * self.f[i].log2() - fall).collect();
        let run = ((300.0 / (self.f[1] - self.f[0])) as usize).max(1);
        first_run(&below, run).map(|j| self.f[idx.start + j]).unwrap_or(f64::NAN)
    }
}

/// Index of the first window of `run` consecutive true values.
fn first_run(v: &[bool], run: usize) -> Option<usize> {
    let mut count = 0;
    for (i, b) in v.iter().enumerate() {
        count = if *b { count + 1 } else { 0 };
        if count >= run {
            return Some(i + 1 - run);
        }
    }
    None
}

pub fn analyze_spectrum(
    freqs: &[f64],
    power: &[f64],
    sample_rate: u32,
    noise_gain: f64,
    effective_bits: Option<u32>,
    cfg: Option<&Settings>,
) -> SpectrumStats {
    let c = detection_params(cfg);
    let nyq = sample_rate as f64 / 2.0;
    let df = freqs[1] - freqs[0];
    let k = (((150.0 / df).round_ties_even() as usize).max(1)) | 1;
    let db: Vec<f64> = moving_average_same(power, k).iter().map(|p| 10.0 * (p + EPS).log10()).collect();
    let sp = Spec { f: freqs, db: &db };
    let valid: Vec<bool> = freqs.iter().map(|f| *f >= 1000.0 && *f <= 0.995 * nyq).collect();
    let peak = if valid.iter().any(|v| *v) {
        db.iter().zip(&valid).filter(|(_, v)| **v).map(|(d, _)| *d).fold(f64::NEG_INFINITY, f64::max)
    } else {
        db.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
    };

    let top = sp.range(0.93 * nyq, 0.99 * nyq);
    let floor = if top.is_empty() { db[db.len() - 1] } else { percentile(&db[top], 10.0) };

    // The top of the spectrum is a usable noise reference only when it is close
    // to the quantisation noise of the file.
    let bits = effective_bits.filter(|b| *b != 0).unwrap_or(24).clamp(8, 24);
    let qfloor = quantization_floor_db(bits, noise_gain);
    let reference = floor.min(qfloor + 30.0);
    let threshold = (reference + c.content_margin).max(peak - c.relevant);

    // Effective bandwidth: highest frequency where a contiguous ~300 Hz band sits
    // above the threshold, counting only content that continues from the audio
    // band without a gap wider than BANDWIDTH_GAP_HZ.
    let n = freqs.len();
    let run = ((300.0 / df) as usize).max(1);
    let mut covered = vec![false; n];
    let mut count = 0usize;
    for i in 0..n {
        count = if db[i] > threshold && valid[i] { count + 1 } else { 0 };
        if count >= run {
            for c2 in &mut covered[i + 1 - run..=i] {
                *c2 = true;
            }
        }
    }
    let hits: Vec<usize> = (0..n).filter(|i| covered[*i]).collect();
    let mut bandwidth = 0.0;
    if !hits.is_empty() {
        let gap = hits.windows(2).position(|w| (w[1] - w[0]) as f64 * df > BANDWIDTH_GAP_HZ);
        bandwidth = freqs[match gap {
            Some(g) => hits[g],
            None => hits[hits.len() - 1],
        }];
    }

    let mut drop = f64::NAN;
    if 0.0 < bandwidth && bandwidth < 0.98 * nyq {
        let below_l = sp.band(bandwidth - 2500.0, bandwidth - 500.0);
        let above_l = sp.band(bandwidth + 500.0, (bandwidth + 2500.0).min(0.995 * nyq));
        drop = below_l - above_l;
    }
    let steep = drop.is_finite() && drop >= c.steep;

    // --- Upsampling: cliff test at the Nyquist of every plausible lower rate.
    let mut cliffs: IntMap = Vec::new();
    let mut deep: IntMap = Vec::new();
    for rate in SOURCE_RATES {
        if rate as f64 * 1.5 > sample_rate as f64 {
            continue;
        }
        let n = rate as f64 / 2.0;
        let lo = sp.band(0.88 * n, 0.96 * n);
        let hi = sp.band(1.02 * n, 1.12 * n);
        let floor_above = sp.percentile_band(1.04 * n, (1.30 * n).min(0.995 * nyq), 25.0);
        if lo.is_finite() && hi.is_finite() {
            cliffs.push((rate, lo - hi));
        }
        if lo.is_finite() && floor_above.is_finite() {
            deep.push((rate, lo - floor_above));
        }
    }
    let strength = |rate: u32| -> Option<&'static str> {
        let cl = imap_get(&cliffs, rate).unwrap_or(f64::NEG_INFINITY);
        let d = imap_get(&deep, rate).unwrap_or(f64::NEG_INFINITY);
        let (narrow_ok, deep_ok) = (cl >= c.cliff, d >= c.deep_cliff);
        if (narrow_ok && cl >= c.strong) || (deep_ok && d >= c.deep_strong) {
            Some("high")
        } else if narrow_ok || deep_ok {
            Some("medium")
        } else {
            None
        }
    };

    let mut upsampled_from: Option<u32> = None;
    let mut confidence: Option<&'static str> = None;
    let mut edge = f64::NAN;
    let mut mid = f64::NAN;
    let mut strong: Vec<u32> = cliffs.iter().map(|(r, _)| *r).filter(|r| strength(*r).is_some()).collect();
    strong.sort();
    if !strong.is_empty() {
        let key = |r: u32| (imap_get(&cliffs, r).unwrap(), imap_get(&deep, r).unwrap_or(f64::NEG_INFINITY));
        let mut best = strong[0];
        for &r in &strong[1..] {
            if key(r) > key(best) {
                best = r;
            }
        }
        upsampled_from = Some(best);
        // Neighbouring rates (44.1 vs 48 kHz) can both show a big cliff: decide
        // by where the level has fallen 6 dB below the music's trend.
        let near: Vec<u32> = cliffs
            .iter()
            .map(|(r, _)| *r)
            .filter(|r| {
                !strong.contains(r)
                    && strong.iter().any(|s| {
                        let q = *r as f64 / *s as f64;
                        0.9 < q && q < 1.1
                    })
                    && (imap_get(&cliffs, *r).unwrap() >= c.cliff / 2.0
                        || imap_get(&deep, *r).unwrap_or(f64::NEG_INFINITY) >= c.deep_cliff / 2.0)
            })
            .collect();
        let mut choice: Vec<u32> = strong.iter().chain(near.iter()).copied().collect();
        choice.sort();
        let (lo_n, hi_n) = (choice[0] as f64 / 2.0, choice[choice.len() - 1] as f64 / 2.0);
        mid = sp.midpoint_frequency(0.68 * lo_n, 0.80 * lo_n, 0.80 * lo_n, 1.2 * hi_n, 6.0);
        if choice.len() > 1 && mid.is_finite() {
            let dist = |r: u32| (mid / (r as f64 / 2.0) - c.mid_ratio).abs();
            let mut best = choice[0];
            for &r in &choice[1..] {
                if dist(r) < dist(best) {
                    best = r;
                }
            }
            upsampled_from = Some(best);
        }
        confidence = Some(strength(upsampled_from.unwrap()).unwrap_or("medium"));
    }
    if upsampled_from.is_none() && steep && bandwidth >= c.lossy_max {
        // Fallback: a brick wall right at (or just under) a lower Nyquist.
        let mut rates: Vec<u32> = cliffs.iter().map(|(r, _)| *r).collect();
        rates.sort();
        if let Some(r) = rates.into_iter().find(|r| bandwidth <= *r as f64 / 2.0 * 1.04) {
            upsampled_from = Some(r);
            confidence = Some("medium");
        }
    }
    if let Some(uf) = upsampled_from {
        let n = uf as f64 / 2.0;
        edge = sp.edge_frequency(0.80 * n, 1.3 * n, 0.70 * n, 0.80 * n, 20.0);
        if !mid.is_finite() {
            mid = sp.midpoint_frequency(0.68 * n, 0.80 * n, 0.80 * n, 1.3 * n, 6.0);
        }
        // A master cannot hold content above its own Nyquist limit.
        bandwidth = bandwidth.min(n);
    }

    // --- Converter noise floor in the ultrasonic band (high-rate files only).
    let mut ultra = f64::NAN;
    let mut ultra_tilt = f64::NAN;
    let u_hi = 40000f64.min(0.85 * nyq);
    if u_hi >= 36000.0 {
        let u_mid = (26000.0 + u_hi) / 2.0;
        ultra = sp.band(26000.0, u_hi);
        ultra_tilt = sp.band(26000.0, u_mid) - sp.band(u_mid, u_hi);
    }

    // --- Lossy source: a brick wall between 11 and 20.3 kHz that never recovers.
    let mut lossy = false;
    let mut lossy_hz = f64::NAN;
    let mut lossy_drop = f64::NAN;
    let scan = sp.range(c.lossy_min, c.lossy_max);
    if !scan.is_empty() {
        let step = ((100.0 / df) as usize).max(1);
        let (mut best_d, mut best_f) = (f64::NEG_INFINITY, 0.0);
        for i in scan.step_by(step) {
            let f = freqs[i];
            let d = sp.band(f - 1000.0, f - 200.0) - sp.band(f + 200.0, f + 1000.0);
            if d > best_d {
                best_d = d;
                best_f = f;
            }
        }
        if best_d >= c.lossy_cliff {
            let before = sp.band(best_f - 1000.0, best_f - 200.0);
            let after = sp.band(best_f + 200.0, (best_f + 4000.0).min(0.995 * nyq));
            if before - after >= c.lossy_cliff {
                lossy = true;
                lossy_hz = best_f;
                lossy_drop = best_d;
            }
        }
    }

    SpectrumStats {
        sample_rate,
        nyquist: nyq,
        noise_floor_db: floor,
        bandwidth_hz: bandwidth,
        cutoff_drop_db: drop,
        cutoff_steep: steep,
        cliffs_db: cliffs,
        deep_cliffs_db: deep,
        upsampled_from,
        upsample_confidence: confidence,
        upsample_edge_hz: edge,
        upsample_mid_hz: mid,
        ultra_floor_db: ultra,
        ultra_tilt_db: ultra_tilt,
        ultra_band_hz: if ultra.is_finite() { u_hi } else { f64::NAN },
        lossy_cutoff: lossy,
        lossy_cutoff_hz: lossy_hz,
        lossy_drop_db: lossy_drop,
        quantization_floor_db: qfloor,
        threshold_db: threshold,
        peak_db: peak,
        freqs: freqs.to_vec(),
        level_db: db,
    }
}

/// True when the 26-40 kHz band holds an even analogue noise floor that
/// continues from the audio band, which only a converter running at the file's
/// own rate leaves behind.
pub fn converter_noise_floor(spec: &SpectrumStats, cfg: Option<&Settings>) -> bool {
    let above = spec.ultra_floor_db - spec.quantization_floor_db;
    if !(above.is_finite() && above >= detection_params(cfg).analog_floor) {
        return false;
    }
    if !(spec.ultra_tilt_db.abs() <= FLOOR_TILT_DB) {
        return false;
    }
    for rate in [44100, 48000] {
        if imap_get(&spec.cliffs_db, rate).unwrap_or(0.0) > FLOOR_STEP_DB {
            return false;
        }
        if imap_get(&spec.deep_cliffs_db, rate).unwrap_or(0.0) > FLOOR_DEEP_STEP_DB {
            return false;
        }
    }
    true
}

/// Small spectrum + spectrogram for the GUI (`FileResult.plot` in Python).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    pub freqs: Vec<f64>,
    pub db: Vec<f64>,
    pub nyquist: f64,
    pub floor_db: f64,
    pub threshold_db: f64,
    pub peak_db: f64,
    pub bandwidth_hz: f64,
    pub spectrogram: Option<Spectrogram>,
}

/// Max-hold down-sampling of the spectrum to `points` bins (keeps a cliff sharp).
pub fn max_hold(spec: &SpectrumStats, points: usize) -> (Vec<f64>, Vec<f64>) {
    let edges = linspace(0.0, spec.nyquist, points + 1);
    let mut ys = vec![-300.0f64; points];
    for (f, d) in spec.freqs.iter().zip(&spec.level_db) {
        if *f <= 0.0 {
            continue;
        }
        let idx = edges.partition_point(|e| e <= f).saturating_sub(1).min(points - 1);
        ys[idx] = ys[idx].max(*d);
    }
    let xs: Vec<f64> = (0..points).map(|i| (edges[i] + edges[i + 1]) / 2.0).collect();
    (xs, ys)
}

pub fn compact_spectrum(spec: &SpectrumStats, points: usize) -> Plot {
    let (xs, ys) = max_hold(spec, points);
    let mut freqs = Vec::new();
    let mut db = Vec::new();
    for (x, y) in xs.iter().zip(&ys) {
        if *y > -299.0 {
            freqs.push(crate::pyfmt::np_round(*x, 1));
            db.push(crate::pyfmt::np_round(*y, 2));
        }
    }
    Plot {
        freqs,
        db,
        nyquist: spec.nyquist,
        floor_db: spec.noise_floor_db,
        threshold_db: spec.threshold_db,
        peak_db: spec.peak_db,
        bandwidth_hz: spec.bandwidth_hz,
        spectrogram: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: &mut u64) -> f64 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*seed >> 11) as f64) / ((1u64 << 53) as f64)
    }

    #[test]
    fn bit_depth_counts_lowest_bit() {
        let mut seed = 1;
        let mut acc = BitDepthAccumulator::default();
        let b16: Vec<i32> = (0..10000).map(|_| (((lcg(&mut seed) * 65536.0) as i64 - 32768) << 16) as i32).collect();
        acc.update(&b16);
        assert_eq!(acc.effective_bits(0.005), Some(16));
        let mut acc = BitDepthAccumulator::default();
        let b24: Vec<i32> =
            (0..10000).map(|_| (((lcg(&mut seed) * 16777216.0) as i64 - 8388608) << 8) as i32).collect();
        acc.update(&b24);
        assert_eq!(acc.effective_bits(0.005), Some(24));
    }

    #[test]
    fn grid_found_on_scaled_16_bit_master() {
        let mut seed = 4;
        let gain = 10f64.powf(-2.5 / 20.0);
        let mut block = Vec::new();
        let mut peak = 0i64;
        for _ in 0..400000 {
            // Laplace-ish distribution of 16-bit values
            let u = lcg(&mut seed) - 0.5;
            let k = (-(3000.0) * u.signum() * (1.0 - 2.0 * u.abs()).ln()).round().clamp(-32768.0, 32767.0);
            let v = (k * 256.0 * gain + lcg(&mut seed) - lcg(&mut seed)).round() as i64;
            peak = peak.max(v.abs());
            block.push((v << 8) as i32);
        }
        let mut acc = GridAccumulator::new(2);
        acc.update(&block);
        let g = acc.result(peak << 8, GRID_STRENGTH).expect("grid");
        assert_eq!(g.bits, 16);
        assert!((g.gain_db + 2.5).abs() < 0.05, "{g:?}");
    }

    #[test]
    fn nfft_is_about_6_hz() {
        assert_eq!(choose_nfft(44100), 8192);
        assert_eq!(choose_nfft(96000), 16384);
        assert_eq!(choose_nfft(192000), 32768);
    }
}
