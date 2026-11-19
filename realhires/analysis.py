"""Signal analysis: averaged power spectrum and effective bit depth.

Both accumulators are streaming so arbitrarily long files can be analysed with
bounded memory.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Optional

import numpy as np

# Sample rates a master could realistically have been produced at before being
# upsampled.  Used as candidates for "this file was upsampled from X".
SOURCE_RATES = (44100, 48000, 88200, 96000, 176400, 192000)

# Floor for log conversion (-300 dB): keeps digital silence finite.
_EPS = 1e-30


def _blackman_harris(n: int) -> np.ndarray:
    """4-term Blackman-Harris window (~-92 dB side lobes, enough dynamic range
    to see a 24-bit noise floor next to full-scale music)."""
    k = np.arange(n)
    x = 2.0 * np.pi * k / n
    return (0.35875 - 0.48829 * np.cos(x) + 0.14128 * np.cos(2 * x)
            - 0.01168 * np.cos(3 * x))


def choose_nfft(sample_rate: int) -> int:
    """FFT size giving roughly 6 Hz resolution at any sample rate."""
    n = 1 << int(np.ceil(np.log2(max(sample_rate, 8000) / 6.0)))
    return int(min(max(n, 4096), 1 << 17))


class SpectrumAccumulator:
    """Welch-style averaged power spectrum (50 % overlap, all channels summed)."""

    def __init__(self, sample_rate: int, channels: int):
        self.sample_rate = sample_rate
        self.channels = channels
        self.nfft = choose_nfft(sample_rate)
        self.hop = self.nfft // 2
        self.window = _blackman_harris(self.nfft)
        self.power = np.zeros(self.nfft // 2 + 1)
        self.frames = 0
        self._tail = np.zeros((0, channels))
        # A full-scale sine then reads 0 dB per channel.
        self._norm = (self.window.sum() / 2.0) ** 2 * channels

    def update(self, block: np.ndarray) -> None:
        """block: float64 array (frames, channels) scaled to [-1, 1)."""
        x = np.concatenate([self._tail, block]) if self._tail.size else block
        n = x.shape[0]
        if n < self.nfft:
            self._tail = x
            return
        count = 1 + (n - self.nfft) // self.hop
        view = np.lib.stride_tricks.sliding_window_view(x, self.nfft, axis=0)
        frames = view[: (count - 1) * self.hop + 1 : self.hop]  # (count, ch, nfft)
        spec = np.fft.rfft(frames * self.window, axis=-1)
        self.power += (spec.real ** 2 + spec.imag ** 2).sum(axis=(0, 1))
        self.frames += count
        self._tail = x[count * self.hop:].copy()

    def result(self):
        if self.frames == 0 and self._tail.shape[0] > 0:
            # Very short file: analyse the single zero-padded frame we have.
            pad = np.zeros((self.nfft, self.channels))
            pad[: self._tail.shape[0]] = self._tail
            spec = np.fft.rfft(pad.T * self.window, axis=-1)
            self.power += (np.abs(spec) ** 2).sum(axis=0)
            self.frames = 1
        freqs = np.fft.rfftfreq(self.nfft, 1.0 / self.sample_rate)
        mean = self.power / max(self.frames, 1) / self._norm
        return freqs, mean

    @property
    def noise_gain(self) -> float:
        """Spectrum level of unit-variance white noise (per channel)."""
        return float(4.0 * np.sum(self.window ** 2) / np.sum(self.window) ** 2)


class BitDepthAccumulator:
    """Histogram of the lowest set bit of every non-zero int32 sample.

    Real 24-bit audio sets bit 8 (the 24-bit LSB after ffmpeg's left
    justification) in about half of all samples.  16-bit audio padded to 24 bits
    never sets anything below bit 16.
    """

    def __init__(self):
        self.lowest_bit_hist = np.zeros(33, dtype=np.int64)
        self.nonzero = 0
        self.total = 0
        self.peak = 0

    def update(self, block: np.ndarray) -> None:
        u = np.ascontiguousarray(block).reshape(-1).view(np.uint32)
        self.total += u.size
        nz = u[u != 0]
        if nz.size == 0:
            return
        self.nonzero += nz.size
        low = nz & (~nz + np.uint32(1))  # isolate lowest set bit
        tz = np.log2(low.astype(np.float64)).astype(np.int64)  # exact for powers of 2
        self.lowest_bit_hist += np.bincount(tz, minlength=33)[:33]
        peak = int(np.abs(block.astype(np.int64)).max())
        self.peak = max(self.peak, peak)

    def effective_bits(self, threshold: float = 0.005) -> Optional[int]:
        """Number of significant bits, ignoring stray outliers (e.g. a clipped
        float sample landing on 0x7FFFFFFF) below `threshold` of samples."""
        if self.nonzero == 0:
            return None
        frac = np.cumsum(self.lowest_bit_hist) / self.nonzero
        lsb = int(np.argmax(frac >= threshold))
        return 32 - lsb

    def bit_usage(self) -> Dict[int, float]:
        """Fraction of non-zero samples whose lowest set bit is at each
        bit-depth position (24 => the 24-bit LSB, 16 => the 16-bit LSB...)."""
        if self.nonzero == 0:
            return {}
        return {32 - b: float(self.lowest_bit_hist[b] / self.nonzero)
                for b in range(33) if self.lowest_bit_hist[b]}


@dataclass
class SpectrumStats:
    sample_rate: int
    nyquist: float
    noise_floor_db: float            # level of the top ~6 % of the spectrum
    bandwidth_hz: float              # highest frequency with real content
    cutoff_drop_db: float            # level drop across the bandwidth edge
    cutoff_steep: bool               # brick-wall style edge?
    cliffs_db: Dict[int, float] = field(default_factory=dict)  # drop at each candidate source Nyquist
    upsampled_from: Optional[int] = None
    upsample_confidence: Optional[str] = None
    upsample_edge_hz: float = float("nan")   # where the level has fallen 20 dB
    lossy_cutoff: bool = False
    lossy_cutoff_hz: float = float("nan")
    lossy_drop_db: float = float("nan")
    quantization_floor_db: float = -300.0
    threshold_db: float = -300.0          # level that counts as "content"
    peak_db: float = -300.0
    freqs: Optional[np.ndarray] = None   # smoothed spectrum, kept for plots
    level_db: Optional[np.ndarray] = None


# Tunables (dB).  See README "How detection works" for the reasoning.
CONTENT_MARGIN_DB = 10.0   # content must sit this far above the top-band floor
STEEP_DROP_DB = 20.0       # drop across the bandwidth edge that counts as a brick wall
CLIFF_DB = 25.0            # drop across a candidate source Nyquist that proves upsampling
CLIFF_STRONG_DB = 40.0
EDGE_RATIO = 0.98          # typical (-20 dB point / source Nyquist) of a resampler
LOSSY_MAX_HZ = 20300.0     # MP3/AAC encoders low-pass below this
LOSSY_CLIFF_DB = 25.0      # drop within ~1 kHz that marks an encoder low-pass
RELEVANT_RANGE_DB = 110.0  # content more than this below the spectral peak is ignored
ULTRASONIC_HZ = 24500.0    # anything above this cannot come from a 44.1/48 kHz master


def _smooth_db(freqs: np.ndarray, power: np.ndarray, width_hz: float = 150.0) -> np.ndarray:
    df = freqs[1] - freqs[0]
    k = max(1, int(round(width_hz / df))) | 1
    sm = np.convolve(power, np.ones(k) / k, mode="same")
    return 10.0 * np.log10(sm + _EPS)


def _band(freqs, db, lo, hi) -> float:
    m = (freqs >= lo) & (freqs <= hi)
    return float(np.median(db[m])) if m.any() else float("nan")


def quantization_floor_db(bits: int, noise_gain: float) -> float:
    """Per-bin level (same scale as the spectrum) of white rounding noise for a
    `bits`-deep integer signal: sigma^2 = step^2 / 12 with step = 2 / 2**bits."""
    step = 2.0 ** (1 - bits)
    return float(10.0 * np.log10(step * step / 12.0 * noise_gain))


def _edge_frequency(freqs, db, lo_hz, hi_hz, ref_lo, ref_hi, fall_db=20.0) -> float:
    """First frequency in [lo_hz, hi_hz] where the level has fallen `fall_db`
    below the median level of [ref_lo, ref_hi]."""
    ref = _band(freqs, db, ref_lo, ref_hi)
    m = np.nonzero((freqs >= lo_hz) & (freqs <= hi_hz) & (db < ref - fall_db))[0]
    return float(freqs[m[0]]) if m.size else float("nan")


def analyze_spectrum(freqs: np.ndarray, power: np.ndarray, sample_rate: int,
                     noise_gain: float, effective_bits: Optional[int] = 24) -> SpectrumStats:
    nyq = sample_rate / 2.0
    db = _smooth_db(freqs, power)
    df = freqs[1] - freqs[0]
    valid = (freqs >= 1000.0) & (freqs <= 0.995 * nyq)
    peak = float(db[valid].max()) if valid.any() else float(db.max())

    top = (freqs >= 0.93 * nyq) & (freqs <= 0.99 * nyq)
    floor = float(np.percentile(db[top], 10)) if top.any() else float(db[-1])

    # The top of the spectrum is only a usable noise reference when it is close
    # to the quantisation noise of the file.  If it is far above (music or
    # analogue hiss reaching Nyquist) cap the reference so that such content
    # still counts as bandwidth.
    bits = min(max(effective_bits or 24, 8), 24)
    qfloor = quantization_floor_db(bits, noise_gain)
    reference = min(floor, qfloor + 30.0)
    threshold = max(reference + CONTENT_MARGIN_DB, peak - RELEVANT_RANGE_DB)

    # Effective bandwidth: highest frequency where a contiguous ~300 Hz band sits
    # above the threshold (isolated tones / resampler images are ignored).
    above = ((db > threshold) & valid).astype(np.int32)
    run = max(1, int(300.0 / df))
    counts = np.convolve(above, np.ones(run, dtype=np.int32), mode="valid")
    idx = np.nonzero(counts == run)[0]
    bandwidth = float(freqs[idx[-1] + run - 1]) if idx.size else 0.0

    drop = float("nan")
    if 0 < bandwidth < 0.98 * nyq:
        below_l = _band(freqs, db, bandwidth - 2500.0, bandwidth - 500.0)
        above_l = _band(freqs, db, bandwidth + 500.0, min(bandwidth + 2500.0, 0.995 * nyq))
        drop = below_l - above_l
    steep = bool(np.isfinite(drop) and drop >= STEEP_DROP_DB)

    # --- Upsampling: cliff test at the Nyquist of every plausible lower rate.
    cliffs: Dict[int, float] = {}
    for rate in SOURCE_RATES:
        if rate * 1.5 > sample_rate:
            continue
        n = rate / 2.0
        lo = _band(freqs, db, 0.88 * n, 0.96 * n)
        hi = _band(freqs, db, 1.02 * n, 1.12 * n)
        if np.isfinite(lo) and np.isfinite(hi):
            cliffs[rate] = lo - hi

    upsampled_from = None
    confidence = None
    edge = float("nan")
    strong = sorted(r for r, d in cliffs.items() if d >= CLIFF_DB)
    if strong:
        best = max(strong, key=cliffs.get)
        upsampled_from = best
        confidence = "high" if cliffs[best] >= CLIFF_STRONG_DB else "medium"
        # Neighbouring rates (44.1 vs 48 kHz) can both show a big cliff when the
        # resampler's transition band straddles them.  Decide by where the level
        # actually falls 20 dB: resamplers put that point at ~0.97-1.02 x the
        # source Nyquist, so pick the candidate whose Nyquist fits it best.
        lo_n, hi_n = strong[0] / 2.0, strong[-1] / 2.0
        edge = _edge_frequency(freqs, db, 0.80 * lo_n, 1.2 * hi_n, 0.70 * lo_n, 0.80 * lo_n)
        if len(strong) > 1 and np.isfinite(edge):
            upsampled_from = min(strong, key=lambda r: abs(edge / (r / 2.0) - EDGE_RATIO))
            if cliffs[upsampled_from] < CLIFF_STRONG_DB:
                confidence = "medium"
    if upsampled_from is None and steep and bandwidth >= LOSSY_MAX_HZ:
        # Fallback: a brick wall right at (or just under) a lower Nyquist.
        for rate in sorted(cliffs):
            if bandwidth <= rate / 2.0 * 1.04:
                upsampled_from, confidence = rate, "medium"
                break
    if upsampled_from and not np.isfinite(edge):
        n = upsampled_from / 2.0
        edge = _edge_frequency(freqs, db, 0.80 * n, 1.2 * n, 0.70 * n, 0.80 * n)

    # --- Lossy source: a brick wall between 11 and 20.3 kHz (MP3/AAC/Vorbis
    # encoders low-pass there) that never recovers above.
    lossy = False
    lossy_hz = float("nan")
    lossy_drop = float("nan")
    scan = np.nonzero((freqs >= 11000.0) & (freqs <= LOSSY_MAX_HZ))[0]
    if scan.size:
        step = max(1, int(100.0 / df))
        best_d, best_f = -np.inf, 0.0
        for i in scan[::step]:
            f = freqs[i]
            d = _band(freqs, db, f - 1000.0, f - 200.0) - _band(freqs, db, f + 200.0, f + 1000.0)
            if d > best_d:
                best_d, best_f = d, f
        if best_d >= LOSSY_CLIFF_DB:
            before = _band(freqs, db, best_f - 1000.0, best_f - 200.0)
            after = _band(freqs, db, best_f + 200.0, min(best_f + 4000.0, 0.995 * nyq))
            if before - after >= LOSSY_CLIFF_DB:
                lossy, lossy_hz, lossy_drop = True, best_f, best_d

    return SpectrumStats(
        sample_rate=sample_rate,
        nyquist=nyq,
        noise_floor_db=floor,
        bandwidth_hz=bandwidth,
        cutoff_drop_db=drop,
        cutoff_steep=steep,
        cliffs_db=cliffs,
        upsampled_from=upsampled_from,
        upsample_confidence=confidence,
        upsample_edge_hz=edge,
        lossy_cutoff=lossy,
        lossy_cutoff_hz=lossy_hz,
        lossy_drop_db=lossy_drop,
        quantization_floor_db=qfloor,
        threshold_db=threshold,
        peak_db=peak,
        freqs=freqs,
        level_db=db,
    )


def compact_spectrum(spec: SpectrumStats, points: int = 900) -> dict:
    """Down-sample a spectrum to `points` max-hold bins for plotting.

    Max-hold (rather than averaging) keeps a brick-wall edge sharp.  The result
    is small, picklable and JSON-friendly, so worker processes can send it back
    to the GUI cheaply.
    """
    f, db = spec.freqs, spec.level_db
    keep = f > 0
    f, db = f[keep], db[keep]
    edges = np.linspace(0.0, spec.nyquist, points + 1)
    idx = np.clip(np.digitize(f, edges) - 1, 0, points - 1)
    ys = np.full(points, -300.0)
    np.maximum.at(ys, idx, db)
    xs = (edges[:-1] + edges[1:]) / 2.0
    ok = ys > -299.0
    return {
        "freqs": xs[ok].round(1).tolist(),
        "db": ys[ok].round(2).tolist(),
        "nyquist": spec.nyquist,
        "floor_db": spec.noise_floor_db,
        "threshold_db": spec.threshold_db,
        "peak_db": spec.peak_db,
        "bandwidth_hz": spec.bandwidth_hz,
    }
