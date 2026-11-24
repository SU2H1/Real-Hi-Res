"""Turn raw measurements into a verdict for one file."""

from __future__ import annotations

import os
import time
from dataclasses import dataclass, field
from typing import List, Optional

import numpy as np

from .analysis import (
    GRID_FULL,
    SMEAR_HZ,
    BitDepthAccumulator,
    Grid,
    GridAccumulator,
    SpectrumAccumulator,
    SpectrumStats,
    analyze_spectrum,
    compact_spectrum,
    converter_noise_floor,
    detection_params,
)
from .probe import AudioInfo, iter_pcm, probe

FAKE_LOSSLESS = "FAKE LOSSLESS"   # a "fake FLAC": lossless container, lossy (MP3/AAC...) source
FAKE = "FAKE HI-RES"              # upsampled and/or bit-padded hi-res
SUSPECT = "SUSPICIOUS"
REAL = "REAL HI-RES"
NOT_HIRES = "NOT HI-RES"          # genuine lossless at CD resolution
LOSSY = "LOSSY FORMAT"            # MP3, AAC, ... files themselves
ERROR = "ERROR"

VERDICT_ORDER = [FAKE_LOSSLESS, FAKE, SUSPECT, REAL, NOT_HIRES, LOSSY, ERROR]

# Version of the detection rules.  Bump it whenever a change can give a different
# verdict, flag, estimated resolution or measurement for the same file and
# settings, so results saved by older versions are marked "outdated" (see
# store.py).  Pure wording changes of reasons or summaries need no bump.
#   1: Real-Hi-Res 1.7.0 to 1.9.0
DETECTOR_VERSION = 1
PROBLEM_VERDICTS = (FAKE_LOSSLESS, FAKE, SUSPECT)

# Short machine-readable flags (JSON/CSV "flags" column).
FLAG_LOSSY_SOURCE = "lossy-source"
FLAG_UPSAMPLED = "upsampled"
FLAG_PADDED = "padded-bits"
FLAG_PARTIAL_BITS = "partial-bits"
FLAG_NO_ULTRASONIC = "no-ultrasonic"
FLAG_GRID = "value-grid"           # 16-bit (or 20-bit) master scaled or dithered into 24-bit/float
FLAG_CONVERTER_FLOOR = "converter-floor"  # dull file proven real by its converter noise floor


def fmt_rate(rate: Optional[float]) -> str:
    if not rate:
        return "?"
    k = rate / 1000.0
    return f"{k:g}" if abs(k - round(k, 1)) < 1e-9 else f"{k:.1f}"


def fmt_res(bits, rate, is_float: bool = False) -> str:
    b = "?" if bits is None else (f"{bits}f" if is_float else str(bits))
    return f"{b}/{fmt_rate(rate)}"


PASSED = "Passed spectral and bit-depth checks."


def bit_depth_only_summary(bits, rate) -> str:
    """Why text of a genuine file whose sample rate is not a hi-res one (48 kHz or less)."""
    std = "standard " if rate in (44100, 48000) else ""
    return f"Genuine {bits}-bit at {std}{fmt_rate(rate)} kHz: hi-res by bit depth only"


def bit_depth_only_reason(rate) -> str:
    return (f"Hi-res by bit depth only: {fmt_rate(rate)} kHz is not a hi-res sample rate, and the file "
            "does not claim one, so there is no higher sample rate that could have been faked. Only its "
            "bit depth goes beyond CD quality, and that bit depth is genuine.")


def update_wording(r: "FileResult") -> "FileResult":
    """Bring the texts of a result saved by an older version up to date (wording only)."""
    if (r.verdict == REAL and r.sample_rate and r.sample_rate <= 48000
            and r.summary.startswith("Genuine ") and r.summary.endswith("-bit audio")):
        bits = r.summary[len("Genuine "):-len("-bit audio")]
        r.summary = bit_depth_only_summary(bits, r.sample_rate)
        if PASSED in r.reasons and not any(x.startswith("Hi-res by bit depth only") for x in r.reasons):
            r.reasons.insert(r.reasons.index(PASSED) + 1, bit_depth_only_reason(r.sample_rate))
    return r


@dataclass
class FileResult:
    path: str
    verdict: str
    summary: str = ""                 # one-line reason, shown in lists
    reasons: List[str] = field(default_factory=list)
    notes: List[str] = field(default_factory=list)
    flags: List[str] = field(default_factory=list)
    tags: dict = field(default_factory=dict)  # artist, album, album_artist, title, track... (from the file)
    codec: str = ""
    sample_rate: int = 0
    channels: int = 0
    claimed_bits: Optional[int] = None
    is_float: bool = False
    duration: Optional[float] = None
    effective_bits: Optional[int] = None
    true_rate: Optional[int] = None
    bandwidth_hz: Optional[float] = None
    cutoff_drop_db: Optional[float] = None
    noise_floor_db: Optional[float] = None
    upsampled_from: Optional[int] = None
    upsample_edge_hz: Optional[float] = None
    upsample_mid_hz: Optional[float] = None
    lossy_cutoff_hz: Optional[float] = None
    cliffs_db: dict = field(default_factory=dict)
    deep_cliffs_db: dict = field(default_factory=dict)
    converter_floor_db: Optional[float] = None  # 26-40 kHz level above the rounding noise
    grid_step: Optional[float] = None           # value-grid step in 24-bit LSBs, if any
    grid_bits: Optional[int] = None             # resolution of the master that grid implies
    grid_gain_db: Optional[float] = None        # gain applied to that master
    bit_usage: dict = field(default_factory=dict)
    analysed_seconds: float = 0.0
    elapsed: float = 0.0
    spectrum: Optional[SpectrumStats] = None  # full arrays, for SVG plots; not serialised
    plot: Optional[dict] = None               # compact spectrum + spectrogram for the GUI; not serialised
    saved: Optional[dict] = None              # set when the result was loaded from the saved results

    @property
    def claimed(self) -> str:
        if self.verdict == LOSSY:
            return f"{self.codec}/{fmt_rate(self.sample_rate)}"
        return fmt_res(self.claimed_bits, self.sample_rate, self.is_float)

    @property
    def estimated(self) -> str:
        if self.verdict in (ERROR, LOSSY) or self.effective_bits is None:
            return "-"
        bits = self.effective_bits
        if self.claimed_bits:
            bits = min(bits, self.claimed_bits)
        if self.is_float and bits > 24:
            bits = 24  # float carries ~24 bits of mantissa
        if self.lossy_cutoff_hz:
            return f"lossy~{self.lossy_cutoff_hz / 1000:.0f}k"
        return fmt_res(bits, self.true_rate or self.sample_rate)

    @property
    def is_problem(self) -> bool:
        return self.verdict in PROBLEM_VERDICTS

    def to_dict(self) -> dict:
        d = {k: v for k, v in self.__dict__.items() if k not in ("spectrum", "plot", "saved")}
        d["claimed"] = self.claimed
        d["estimated_real"] = self.estimated
        for k, v in list(d.items()):
            if isinstance(v, float) and not np.isfinite(v):
                d[k] = None
        for key in ("cliffs_db", "deep_cliffs_db"):
            d[key] = {str(k): (round(v, 1) if np.isfinite(v) else None)
                      for k, v in getattr(self, key).items()}
        return d


def _judge(info: AudioInfo, spec: SpectrumStats, bits: BitDepthAccumulator, res: FileResult,
           cfg=None, grid: Optional[Grid] = None) -> None:
    c = detection_params(cfg)
    fake: List[str] = []
    suspect: List[str] = []
    short: List[str] = []
    notes: List[str] = res.notes
    eff = bits.effective_bits(c.bit_threshold)
    res.effective_bits = eff if grid is None or eff is None else min(eff, grid.bits)
    if grid is not None:
        res.grid_step, res.grid_bits = round(grid.step, 2), grid.bits
        res.grid_gain_db = round(grid.gain_db, 2)
    res.bit_usage = {k: round(v, 4) for k, v in sorted(bits.bit_usage().items(), reverse=True)}
    res.bandwidth_hz = spec.bandwidth_hz
    res.cutoff_drop_db = spec.cutoff_drop_db
    res.noise_floor_db = spec.noise_floor_db
    res.upsampled_from = spec.upsampled_from
    res.upsample_edge_hz = spec.upsample_edge_hz
    res.upsample_mid_hz = spec.upsample_mid_hz
    res.lossy_cutoff_hz = spec.lossy_cutoff_hz if spec.lossy_cutoff else None
    res.cliffs_db = spec.cliffs_db
    res.deep_cliffs_db = spec.deep_cliffs_db
    res.converter_floor_db = spec.ultra_floor_db - spec.quantization_floor_db
    res.true_rate = spec.upsampled_from or info.sample_rate

    bw_k = spec.bandwidth_hz / 1000.0

    # ---- lossy source ("fake FLAC") --------------------------------------
    if spec.lossy_cutoff:
        res.flags.append(FLAG_LOSSY_SOURCE)
        fake.append(
            f"Brick-wall low-pass at {spec.lossy_cutoff_hz / 1000:.1f} kHz "
            f"({spec.lossy_drop_db:.0f} dB drop within ~1 kHz, never recovers): typical of an "
            f"MP3/AAC/Vorbis encoder, so this {info.codec.upper()} file was made from a lossy file "
            "and is not truly lossless."
        )
        short.append(f"Made from a lossy file (MP3/AAC-style cut-off at {spec.lossy_cutoff_hz / 1000:.1f} kHz)")

    # ---- bit depth -------------------------------------------------------
    claimed_bits = info.claimed_bits or 16
    if claimed_bits > 16 or info.is_float:
        if eff is not None and eff <= 16:
            res.flags.append(FLAG_PADDED)
            if info.is_float:
                fake.append(f"Every sample is an exact {eff}-bit value: 16-bit audio stored in a "
                            f"{claimed_bits}-bit float container.")
                short.append(f"16-bit audio in a {claimed_bits}-bit float file")
            else:
                fake.append(f"Only {eff} of {claimed_bits} bits are used: the low {claimed_bits - eff} "
                            f"bits are always zero (16-bit audio padded into a {claimed_bits}-bit container).")
                short.append(f"Padded: only {eff} of {claimed_bits} bits used")
        elif grid is not None and grid.bits <= 16 and grid.strength >= GRID_FULL:
            res.flags.append(FLAG_GRID)
            kind = "float" if info.is_float else f"{claimed_bits}-bit"
            how = (f"after a gain change of {grid.gain_db:+.1f} dB (with or without dither)"
                   if abs(grid.gain_db) >= 0.05 else "when it is only dithered, with no gain change,")
            fake.append(
                f"Every sample lies on an even grid with a step of {grid.step:.1f} levels of the 24-bit "
                f"scale instead of 1. That is what a 16-bit master gives {how} on its way into a {kind} "
                "file, so the low bits hold no real detail and the resolution is still 16 bits."
            )
            short.append("16-bit master given a volume change or dither" if abs(grid.gain_db) >= 0.05
                         else "16-bit master given dither")
        elif grid is not None and grid.bits <= 16:
            res.flags.append(FLAG_GRID)
            fake.append(
                f"About {grid.strength:.0%} of the samples sit exactly on the grid of a 16-bit master "
                f"(a step of {grid.step:.1f} levels of the 24-bit scale), which real 24-bit audio never "
                "does. An upsample by a whole-number ratio leaves this trace, because it keeps every "
                "second (or third) sample of the 16-bit master unchanged."
            )
            short.append(f"{grid.strength:.0%} of samples on a 16-bit grid")
        elif eff is not None and eff < min(claimed_bits, 24) and not info.is_float:
            res.flags.append(FLAG_PARTIAL_BITS)
            suspect.append(
                f"Only {eff} of {claimed_bits} bits are used (low {claimed_bits - eff} bits always zero); "
                "better than CD but not true 24-bit."
            )
            short.append(f"Only {eff} of {claimed_bits} bits used")
        elif grid is not None:
            res.flags.append(FLAG_GRID)
            suspect.append(
                f"Every sample lies on an even grid with a step of {grid.step:.1f} levels of the 24-bit "
                f"scale instead of 1, so only about {grid.bits} bits carry detail: a {grid.bits}-bit "
                f"master scaled by {grid.gain_db:+.1f} dB. Better than CD but not true 24-bit."
            )
            short.append(f"{grid.bits}-bit master scaled into {claimed_bits} bits")
        elif eff is not None:
            notes.append(f"Bit depth OK: {min(eff, 24) if info.is_float else eff} significant bits in use.")

    # ---- sample rate -----------------------------------------------------
    if spec.upsampled_from and info.sample_rate > spec.upsampled_from:
        res.flags.append(FLAG_UPSAMPLED)
        drops = [d for d in (spec.cliffs_db.get(spec.upsampled_from),
                             spec.deep_cliffs_db.get(spec.upsampled_from)) if d is not None and np.isfinite(d)]
        drop = max(drops) if drops else spec.cutoff_drop_db
        src = fmt_rate(spec.upsampled_from)
        marks = []
        if np.isfinite(spec.upsample_mid_hz) and not spec.upsample_mid_hz > spec.upsample_edge_hz:
            marks.append(f"down 6 dB at {spec.upsample_mid_hz / 1000:.1f} kHz")
        if np.isfinite(spec.upsample_edge_hz):
            marks.append(f"down 20 dB by {spec.upsample_edge_hz / 1000:.1f} kHz")
        edge = f" (level is {' and '.join(marks)})" if marks else ""
        msg = (f"Spectrum collapses by {drop:.0f} dB around {spec.upsampled_from / 2000:.2f} kHz{edge}, "
               f"the Nyquist limit of a {src} kHz master: upsampled from {src} kHz "
               f"({spec.upsample_confidence} confidence).")
        if spec.upsampled_from >= 88200:
            msg += f" Still hi-res, but not the advertised {fmt_rate(info.sample_rate)} kHz."
        fake.append(msg)
        if not spec.lossy_cutoff:
            short.append(f"Upsampled from {src} kHz: nothing above {spec.upsampled_from / 2000:g} kHz")
    elif info.sample_rate > 48000 and not spec.lossy_cutoff:
        above_q = spec.ultra_floor_db - spec.quantization_floor_db
        if spec.bandwidth_hz < c.ultrasonic:
            if converter_noise_floor(spec, cfg):
                res.flags.append(FLAG_CONVERTER_FLOOR)
                notes.append(
                    f"No musical content above {bw_k:.1f} kHz, but the converter's analogue noise floor "
                    f"continues evenly from there to {spec.ultra_band_hz / 1000:.0f} kHz, {above_q:.0f} dB "
                    "above the rounding noise and with no step at 22.05 or 24 kHz. An upsampled file "
                    "would hold only rounding noise above 22-24 kHz, so this was digitised at "
                    f"{fmt_rate(info.sample_rate)} kHz (typical of an old or dull analogue recording)."
                )
            elif np.isfinite(above_q) and above_q < c.analog_floor:
                res.flags.append(FLAG_NO_ULTRASONIC)
                suspect.append(
                    f"No content above {bw_k:.1f} kHz although the format allows "
                    f"{spec.nyquist / 1000:.0f} kHz, and there is no brick-wall edge. The ultrasonic band "
                    "holds only rounding noise, without the analogue noise floor that a converter "
                    "running at this rate would leave, so it cannot be shown that it was recorded at "
                    f"{fmt_rate(info.sample_rate)} kHz. Could be a gently filtered upsample of a quiet "
                    "digital master, or a digitally low-passed recording; check the spectrogram."
                )
                short.append(f"No content above {bw_k:.1f} kHz and only rounding noise above it")
            else:
                res.flags.append(FLAG_NO_ULTRASONIC)
                suspect.append(
                    f"No content above {bw_k:.1f} kHz although the format allows "
                    f"{spec.nyquist / 1000:.0f} kHz, yet there is no brick-wall edge. The ultrasonic band "
                    "holds noise, but it is not an even converter noise floor that continues from the "
                    "audio band. Could be a dull/old recording or a gentle-filter upsample; check the "
                    "spectrogram."
                )
                short.append(f"No content above {bw_k:.1f} kHz, but no brick-wall edge")
        elif spec.bandwidth_hz < c.ultrasonic + SMEAR_HZ and not above_q >= c.analog_floor:
            res.flags.append(FLAG_NO_ULTRASONIC)
            suspect.append(
                f"Content stops at {bw_k:.1f} kHz, just above the 24 kHz limit of a 48 kHz master, "
                "where a soft resampler such as FFmpeg's default one smears that edge, and the band "
                "above holds only rounding noise, without the analogue noise floor that a converter "
                f"running at {fmt_rate(info.sample_rate)} kHz would leave. Could be a gently filtered "
                "upsample; check the spectrogram."
            )
            short.append(f"Content stops at {bw_k:.1f} kHz over only rounding noise")
        else:
            notes.append(f"Real ultrasonic content up to {bw_k:.1f} kHz "
                         f"(format limit {spec.nyquist / 1000:.0f} kHz).")
    elif info.sample_rate <= 48000:
        notes.append(f"Bandwidth {bw_k:.1f} kHz.")

    # ---- verdict -----------------------------------------------------------
    if spec.lossy_cutoff:
        res.verdict, res.reasons = FAKE_LOSSLESS, fake + suspect
        if info.claims_hires:
            res.reasons.append(f"The file also claims {fmt_res(info.claimed_bits, info.sample_rate)} "
                               "hi-res, which a lossy source cannot provide.")
    elif not info.claims_hires:
        res.verdict = NOT_HIRES
        res.reasons = ["Standard resolution "
                       f"({fmt_res(info.claimed_bits, info.sample_rate)}): does not claim to be hi-res. "
                       "No sign of a lossy source, so it is genuine lossless audio."]
        short = [f"Genuine lossless at CD quality, content up to {bw_k:.1f} kHz"]
    elif fake:
        res.verdict, res.reasons = FAKE, fake + suspect
    elif suspect:
        res.verdict, res.reasons = SUSPECT, suspect
    else:
        res.verdict = REAL
        res.reasons = [PASSED]
        real_bits = min(eff, 24) if (eff and info.is_float) else eff
        if FLAG_CONVERTER_FLOOR in res.flags:
            short = [f"Genuine: dull recording (content up to {bw_k:.1f} kHz), converter noise to "
                     f"{spec.ultra_band_hz / 1000:.0f} kHz, {real_bits} bits used"]
        elif info.sample_rate > 48000:
            short = [f"Genuine: content up to {bw_k:.1f} kHz, {real_bits} bits used"]
        else:
            short = [bit_depth_only_summary(real_bits, info.sample_rate)]
            res.reasons.append(bit_depth_only_reason(info.sample_rate))
    res.summary = "; ".join(short)


def analyze_file(path: str, max_seconds: Optional[float] = None, keep_spectrum: bool = False,
                 keep_plot: bool = False, settings=None) -> FileResult:
    """Analyse one file.  `settings` is a settings.Settings (None = defaults);
    an explicit `max_seconds` overrides settings.max_seconds."""
    t0 = time.time()
    res = FileResult(path=path, verdict=ERROR)
    if max_seconds is None and settings is not None and settings.max_seconds > 0:
        max_seconds = settings.max_seconds
    try:
        info = probe(path)
        res.codec, res.sample_rate, res.channels = info.codec, info.sample_rate, info.channels
        res.claimed_bits, res.is_float, res.duration = info.claimed_bits, info.is_float, info.duration
        res.tags = dict(info.tags)
        if info.sample_rate <= 0:
            raise RuntimeError("could not determine sample rate")

        spec_acc = SpectrumAccumulator(info.sample_rate, info.channels, spectrogram=keep_plot)
        bit_acc = BitDepthAccumulator()
        # The value-grid test only matters for files that claim more than 16 bits.
        grid_acc = (GridAccumulator(info.channels)
                    if info.claims_hires and ((info.claimed_bits or 16) > 16 or info.is_float) else None)
        scale = 1.0 / 2147483648.0
        frames = 0
        for block in iter_pcm(path, info.channels, chunk_frames=max(1 << 16, 4 * spec_acc.nfft),
                              max_seconds=max_seconds):
            bit_acc.update(block)
            if grid_acc is not None:
                grid_acc.update(block)
            spec_acc.update(block.astype(np.float64) * scale)
            frames += block.shape[0]
        res.analysed_seconds = frames / info.sample_rate

        if bit_acc.nonzero == 0:
            res.verdict = SUSPECT if info.claims_hires else NOT_HIRES
            res.reasons = ["File is digital silence; nothing to analyse."]
            res.summary = "Digital silence: nothing to analyse"
            return res

        c = detection_params(settings)
        freqs, power = spec_acc.result()
        grid = grid_acc.result(bit_acc.peak, c.grid_strength) if grid_acc is not None else None
        eff_bits = bit_acc.effective_bits(c.bit_threshold)
        if grid is not None and eff_bits is not None:
            eff_bits = min(eff_bits, grid.bits)
        spec = analyze_spectrum(freqs, power, info.sample_rate, spec_acc.noise_gain, eff_bits, cfg=settings)
        if keep_spectrum:
            res.spectrum = spec
        if keep_plot:
            res.plot = compact_spectrum(spec)
            res.plot["spectrogram"] = spec_acc.spectrogram_result()

        if info.is_lossy:
            res.verdict = LOSSY
            res.reasons = [f"Lossy codec ({info.codec}); can never be real hi-res or lossless, whatever "
                           f"the {fmt_rate(info.sample_rate)} kHz sample rate says."]
            res.summary = f"{info.codec.upper()} file: lossy by design"
            res.bandwidth_hz = spec.bandwidth_hz
            res.effective_bits = bit_acc.effective_bits(c.bit_threshold)
            return res

        _judge(info, spec, bit_acc, res, settings, grid)
    except Exception as exc:  # noqa: BLE001 - report every failure per file
        res.verdict = ERROR
        res.reasons = [str(exc)]
        res.summary = f"Could not analyse: {str(exc).splitlines()[0][:120] if str(exc) else type(exc).__name__}"
    finally:
        res.elapsed = time.time() - t0
        res.path = os.path.abspath(path)
    return res


def saved_note(r: FileResult) -> str:
    """One sentence on where a reused result comes from ("" for a fresh one)."""
    if not r.saved:
        return ""
    when = time.strftime("%Y-%m-%d", time.localtime(r.saved.get("time") or 0))
    by = f"Real-Hi-Res {r.saved.get('version') or '?'}"
    moved = " (when it had another name or folder)" if r.saved.get("moved_from") else ""
    if r.saved.get("outdated"):
        return (f"Saved result from {when}{moved}, made by {by} with older detection rules; the file has not "
                "changed since, but the current rules may judge it differently. Re-scan it to check.")
    return f"Saved result from {when}{moved}, made by {by}; the file has not changed since, so it was not analysed again."


def verdict_breakdown(results) -> List[tuple]:
    """[(verdict, count, percent)] in VERDICT_ORDER, only verdicts that occur."""
    results = list(results)
    total = len(results) or 1
    out = []
    for v in VERDICT_ORDER:
        n = sum(1 for r in results if r.verdict == v)
        if n:
            out.append((v, n, 100.0 * n / total))
    return out
