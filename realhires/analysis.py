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


GRID_WINDOW = 1 << 16        # histogram of 24-bit values within +-this (about -42 dBFS)
GRID_MIN_SAMPLES = 5000      # samples needed in that window per channel to decide
GRID_MIN_STEP = 6.0          # smallest grid step searched for (24-bit LSBs)
GRID_MAX_STEP = 2048.0       # largest (a 16-bit master raised by 18 dB)
GRID_STRENGTH = 0.35         # comb strength (0..1) that proves a grid (real 24-bit audio: < 0.05)
GRID_FULL = 0.8              # above this practically every sample is on the grid


@dataclass
class Grid:
    step: float        # spacing of the occupied values, in 24-bit LSBs
    strength: float    # normalised comb strength, 1.0 = every sample exactly on the grid
    bits: int          # resolution of the master the grid implies
    gain_db: float     # gain that turned a `bits`-deep master into this grid


def _grid_step(hist: np.ndarray):
    """(step, strength) of the periodic comb in one channel's value histogram,
    (0.0, strength) when there is none, or None when there are too few samples.

    A gain change maps the integers k of a 16-bit master to round(256*g*k) on
    the 24-bit scale, so the histogram is a comb with period 256*g whatever g
    is; dither only blurs each tooth by a few LSBs.  The comb shows up as a
    peak of the histogram's DFT at 1/period with harmonics at multiples of it.
    Real 24-bit audio has a smooth histogram and no such peak.
    """
    h = hist.astype(np.float64)
    c = h.size // 2
    h[c - 7: c + 8] = 0.0      # digital and dithered silence form a spike at zero
    if h.sum() < GRID_MIN_SAMPLES:
        return None
    n = h.size
    spec = np.abs(np.fft.rfft(h * np.hanning(n)))
    if spec[0] <= 0:
        return None
    r = spec / spec[0]
    kmin, kmax = int(np.ceil(n / GRID_MAX_STEP)), int(n / GRID_MIN_STEP)
    seg = r[kmin: kmax + 1]
    peak = float(seg.max())
    if peak < GRID_STRENGTH:
        return 0.0, peak
    # The fundamental is the lowest-frequency tooth of the comb's spectrum.
    k = kmin + int(np.nonzero(seg >= 0.6 * peak)[0][0])
    lo, hi = max(k - 2, 1), min(k + 3, r.size - 1)
    k = lo + int(np.argmax(r[lo:hi]))
    a, b, g = np.log(r[k - 1] + 1e-12), np.log(r[k] + 1e-12), np.log(r[k + 1] + 1e-12)
    den = a - 2 * b + g
    kf = k + (0.5 * (a - g) / den if den < 0 else 0.0)
    k2 = int(round(2 * kf))
    if k2 + 3 < r.size and r[k2 - 3: k2 + 4].max() < 0.4 * r[k]:
        return 0.0, float(r[k])     # no second harmonic: not a comb
    return float(n / kf), float(r[k])


class GridAccumulator:
    """Per-channel histogram of the 24-bit sample values near zero.

    Feeds the value-grid test, which catches a 16-bit master that was given a
    gain change or dither on its way into a 24-bit or float file: the low bits
    are then no longer zero (so BitDepthAccumulator sees 24 bits), but every
    sample still sits on a regular grid inherited from the 16-bit steps.
    """

    def __init__(self, channels: int, window: int = GRID_WINDOW):
        self.window = window
        self.channels = max(1, min(channels, 8))
        self.hist = np.zeros(self.channels * 2 * window, dtype=np.int64)

    def update(self, block: np.ndarray) -> None:
        """block: int32 array (frames, channels), left-justified as from iter_pcm."""
        w = self.window
        v = (block[:, : self.channels] >> 8).astype(np.int64)
        keep = (v > -w) & (v < w)
        idx = (v + w + np.arange(self.channels) * 2 * w)[keep]
        self.hist += np.bincount(idx, minlength=self.hist.size)

    def result(self, peak: int) -> Optional[Grid]:
        """The grid shared by every channel that has enough samples, or None.

        `peak` is the largest absolute int32 sample, used to turn the step into
        a bit depth independently of the gain: a b-bit master spans at most
        2**b grid steps, so b >= log2(2 * peak / step).
        """
        found = []
        for c in range(self.channels):
            res = _grid_step(self.hist[c * 2 * self.window: (c + 1) * 2 * self.window])
            if res is None:
                continue
            if res[0] <= 0.0:
                return None
            found.append(res)
        if not found or peak <= 0:
            return None
        step = float(np.median([s for s, _ in found]))
        strength = float(min(r for _, r in found))
        spans = np.log2(2.0 * (peak / 256.0) / step)
        bits = 16 if spans <= 16.1 else int(np.ceil(spans - 0.1))
        if bits >= 23:
            return None
        gain_db = float(20.0 * np.log10(step / 2.0 ** (24 - bits)))
        return Grid(step=step, strength=strength, bits=bits, gain_db=gain_db)


@dataclass
class SpectrumStats:
    sample_rate: int
    nyquist: float
    noise_floor_db: float            # level of the top ~6 % of the spectrum
    bandwidth_hz: float              # highest frequency with real content
    cutoff_drop_db: float            # level drop across the bandwidth edge
    cutoff_steep: bool               # brick-wall style edge?
    cliffs_db: Dict[int, float] = field(default_factory=dict)  # drop at each candidate source Nyquist
    deep_cliffs_db: Dict[int, float] = field(default_factory=dict)  # drop to the quietest part above it
    upsampled_from: Optional[int] = None
    upsample_confidence: Optional[str] = None
    upsample_edge_hz: float = float("nan")   # where the level has fallen 20 dB
    upsample_mid_hz: float = float("nan")    # where the level has fallen 6 dB (middle of the transition)
    ultra_floor_db: float = float("nan")     # median level of the 26-40 kHz band (high-rate files only)
    ultra_tilt_db: float = float("nan")      # lower half minus upper half of that band
    ultra_band_hz: float = float("nan")      # upper end of that band
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
DEEP_CLIFF_DB = 45.0       # drop from below a candidate Nyquist to the quietest quarter of 1.04-1.30 x it
DEEP_CLIFF_STRONG_DB = 60.0
MID_RATIO = 0.975          # typical (-6 dB point / source Nyquist) of a resampler (0.95 soxr ... 1.0 swr)
MAX_TREND_DB_PER_OCT = 45.0  # steepest music roll-off extrapolated into the transition band
ANALOG_FLOOR_DB = 15.0     # converter noise must sit this far above the rounding noise
FLOOR_TILT_DB = 6.0        # ... be this flat across 26-40 kHz
FLOOR_STEP_DB = 8.0        # ... and show no narrow step larger than this at 22.05/24 kHz
FLOOR_DEEP_STEP_DB = 15.0  # (nor a deep one larger than this)
LOSSY_MAX_HZ = 20300.0     # MP3/AAC encoders low-pass below this
LOSSY_CLIFF_DB = 25.0      # drop within ~1 kHz that marks an encoder low-pass
RELEVANT_RANGE_DB = 110.0  # content more than this below the spectral peak is ignored
BANDWIDTH_GAP_HZ = 3000.0  # content above an empty stretch wider than this is not bandwidth
ULTRASONIC_HZ = 24500.0    # anything above this cannot come from a 44.1/48 kHz master
SMEAR_HZ = 28800.0         # ... unless a soft resampler smeared the 24 kHz edge up to here


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


def _percentile_band(freqs, db, lo, hi, q) -> float:
    m = (freqs >= lo) & (freqs <= hi)
    return float(np.percentile(db[m], q)) if m.any() else float("nan")


def _midpoint_frequency(freqs, db, ref_lo, ref_hi, lo_hz, hi_hz, fall_db=6.0) -> float:
    """First frequency in [lo_hz, hi_hz] where the level stays `fall_db` below
    the music's own trend for at least ~300 Hz.

    The trend is a straight line in dB over log-frequency with the slope of
    [ref_lo, ref_hi], so the natural roll-off of the music is not mistaken for
    the resampler's transition band.  The slope is limited to falling at most
    MAX_TREND_DB_PER_OCT and never rising, and the line is anchored at the top
    of the reference band, right below where the transition can begin.
    """
    m = (freqs >= ref_lo) & (freqs <= ref_hi)
    if m.sum() < 8:
        return float("nan")
    x, y = np.log2(freqs[m]), db[m]
    slope = float(np.polyfit(x, y, 1)[0])
    slope = min(max(slope, -MAX_TREND_DB_PER_OCT), 0.0)
    anchor = _band(freqs, db, 0.97 * ref_hi, ref_hi)
    icpt = float(anchor - slope * np.log2(0.985 * ref_hi))
    s = np.nonzero((freqs >= lo_hz) & (freqs <= hi_hz))[0]
    if s.size == 0:
        return float("nan")
    below = (db[s] < icpt + slope * np.log2(freqs[s]) - fall_db).astype(np.int32)
    run = max(1, int(300.0 / (freqs[1] - freqs[0])))
    counts = np.convolve(below, np.ones(run, dtype=np.int32), mode="valid")
    idx = np.nonzero(counts == run)[0]
    return float(freqs[s[idx[0]]]) if idx.size else float("nan")


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
    # above the threshold (isolated tones are ignored), counting only content
    # that continues from the audio band without a gap wider than
    # BANDWIDTH_GAP_HZ.  Real spectra are continuous; resampler residue (images
    # and spurs of FFmpeg's default swr) sits far above an empty stretch.
    above = ((db > threshold) & valid).astype(np.int32)
    run = max(1, int(300.0 / df))
    counts = np.convolve(above, np.ones(run, dtype=np.int32), mode="valid")
    starts = (counts == run).astype(np.int32)
    covered = np.convolve(starts, np.ones(run, dtype=np.int32))[: freqs.size] > 0
    hits = np.nonzero(covered)[0]
    bandwidth = 0.0
    if hits.size:
        gaps = np.nonzero(np.diff(hits) * df > BANDWIDTH_GAP_HZ)[0]
        bandwidth = float(freqs[hits[gaps[0]] if gaps.size else hits[-1]])

    drop = float("nan")
    if 0 < bandwidth < 0.98 * nyq:
        below_l = _band(freqs, db, bandwidth - 2500.0, bandwidth - 500.0)
        above_l = _band(freqs, db, bandwidth + 500.0, min(bandwidth + 2500.0, 0.995 * nyq))
        drop = below_l - above_l
    steep = bool(np.isfinite(drop) and drop >= STEEP_DROP_DB)

    # --- Upsampling: cliff test at the Nyquist of every plausible lower rate.
    # The narrow cliff compares 88-96 % with 102-112 % of the candidate Nyquist.
    # Resamplers with a soft transition (FFmpeg's default swr) are still
    # falling inside that upper window, so the deep cliff also compares with
    # the quietest quarter of 104-130 %, which they have reached by then.
    cliffs: Dict[int, float] = {}
    deep: Dict[int, float] = {}
    for rate in SOURCE_RATES:
        if rate * 1.5 > sample_rate:
            continue
        n = rate / 2.0
        lo = _band(freqs, db, 0.88 * n, 0.96 * n)
        hi = _band(freqs, db, 1.02 * n, 1.12 * n)
        floor_above = _percentile_band(freqs, db, 1.04 * n, min(1.30 * n, 0.995 * nyq), 25)
        if np.isfinite(lo) and np.isfinite(hi):
            cliffs[rate] = lo - hi
        if np.isfinite(lo) and np.isfinite(floor_above):
            deep[rate] = lo - floor_above

    def _strength(rate: int) -> Optional[str]:
        c, d = cliffs.get(rate, -np.inf), deep.get(rate, -np.inf)
        if c >= CLIFF_STRONG_DB or d >= DEEP_CLIFF_STRONG_DB:
            return "high"
        if c >= CLIFF_DB or d >= DEEP_CLIFF_DB:
            return "medium"
        return None

    upsampled_from = None
    confidence = None
    edge = float("nan")
    mid = float("nan")
    strong = sorted(r for r in cliffs if _strength(r))
    if strong:
        upsampled_from = max(strong, key=lambda r: (cliffs[r], deep.get(r, -np.inf)))
        # Neighbouring rates (44.1 vs 48 kHz) can both show a big cliff when the
        # resampler's transition band straddles them.  Decide by where the level
        # has fallen 6 dB below the music's trend: common resamplers put that
        # point at 0.95-1.0 x the source Nyquist (steep ones such as soxr just
        # below it, soft ones such as FFmpeg's swr right on it), so pick the
        # candidate whose Nyquist fits it best.
        # A very soft filter can leave the lower neighbour of a strong candidate
        # with only half the cliff, so such neighbours take part as well.
        near = [r for r in cliffs if r not in strong
                and any(0.9 < r / s < 1.1 for s in strong)
                and (cliffs[r] >= CLIFF_DB / 2 or deep.get(r, -np.inf) >= DEEP_CLIFF_DB / 2)]
        choice = sorted(strong + near)
        lo_n, hi_n = choice[0] / 2.0, choice[-1] / 2.0
        mid = _midpoint_frequency(freqs, db, 0.68 * lo_n, 0.80 * lo_n, 0.80 * lo_n, 1.2 * hi_n)
        if len(choice) > 1 and np.isfinite(mid):
            upsampled_from = min(choice, key=lambda r: abs(mid / (r / 2.0) - MID_RATIO))
        confidence = _strength(upsampled_from) or "medium"
    if upsampled_from is None and steep and bandwidth >= LOSSY_MAX_HZ:
        # Fallback: a brick wall right at (or just under) a lower Nyquist.
        for rate in sorted(cliffs):
            if bandwidth <= rate / 2.0 * 1.04:
                upsampled_from, confidence = rate, "medium"
                break
    if upsampled_from:
        n = upsampled_from / 2.0
        edge = _edge_frequency(freqs, db, 0.80 * n, 1.3 * n, 0.70 * n, 0.80 * n)
        if not np.isfinite(mid):
            mid = _midpoint_frequency(freqs, db, 0.68 * n, 0.80 * n, 0.80 * n, 1.3 * n)
        # A master cannot hold content above its own Nyquist limit: whatever lies
        # above it here is resampler residue (transition band, filter leakage,
        # images of the bass around the source rate).
        bandwidth = min(bandwidth, n)

    # --- Converter noise floor in the ultrasonic band (high-rate files only).
    # Every analogue-to-digital converter adds thermal noise well above the
    # rounding noise of a 24-bit file, and it is flat across 26-40 kHz.  An
    # upsampled file holds only rounding noise there.
    ultra = ultra_tilt = float("nan")
    u_hi = min(40000.0, 0.85 * nyq)
    if u_hi >= 36000.0:
        u_mid = (26000.0 + u_hi) / 2.0
        ultra = _band(freqs, db, 26000.0, u_hi)
        ultra_tilt = _band(freqs, db, 26000.0, u_mid) - _band(freqs, db, u_mid, u_hi)

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
        deep_cliffs_db=deep,
        upsampled_from=upsampled_from,
        upsample_confidence=confidence,
        upsample_edge_hz=edge,
        upsample_mid_hz=mid,
        ultra_floor_db=ultra,
        ultra_tilt_db=ultra_tilt,
        ultra_band_hz=u_hi if np.isfinite(ultra) else float("nan"),
        lossy_cutoff=lossy,
        lossy_cutoff_hz=lossy_hz,
        lossy_drop_db=lossy_drop,
        quantization_floor_db=qfloor,
        threshold_db=threshold,
        peak_db=peak,
        freqs=freqs,
        level_db=db,
    )


def converter_noise_floor(spec: SpectrumStats) -> bool:
    """True when the 26-40 kHz band holds an even analogue noise floor that
    continues from the audio band, which only a converter running at the
    file's own rate leaves behind.

    It must sit ANALOG_FLOOR_DB above the rounding noise (24-bit TPDF dither
    adds only ~5 dB), be flat within FLOOR_TILT_DB (noise-shaped dither and
    resampler leakage are not), and show no step at the 44.1/48 kHz Nyquist
    limits (a re-recorded CD shows its own, higher 16-bit floor below 22 kHz).
    """
    above = spec.ultra_floor_db - spec.quantization_floor_db
    if not (np.isfinite(above) and above >= ANALOG_FLOOR_DB):
        return False
    if not abs(spec.ultra_tilt_db) <= FLOOR_TILT_DB:
        return False
    for rate in (44100, 48000):
        if spec.cliffs_db.get(rate, 0.0) > FLOOR_STEP_DB:
            return False
        if spec.deep_cliffs_db.get(rate, 0.0) > FLOOR_DEEP_STEP_DB:
            return False
    return True


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
