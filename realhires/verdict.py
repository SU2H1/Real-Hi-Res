"""Turn raw measurements into a verdict for one file."""

from __future__ import annotations

import os
import time
from dataclasses import dataclass, field
from typing import List, Optional

import numpy as np

from .analysis import (
    ULTRASONIC_HZ,
    BitDepthAccumulator,
    SpectrumAccumulator,
    SpectrumStats,
    analyze_spectrum,
)
from .probe import AudioInfo, iter_pcm, probe

REAL = "REAL HI-RES"
FAKE = "FAKE HI-RES"
SUSPECT = "SUSPICIOUS"
NOT_HIRES = "NOT HI-RES"
LOSSY = "LOSSY FORMAT"
ERROR = "ERROR"

VERDICT_ORDER = [FAKE, SUSPECT, REAL, NOT_HIRES, LOSSY, ERROR]


def fmt_rate(rate: Optional[float]) -> str:
    if not rate:
        return "?"
    k = rate / 1000.0
    return f"{k:g}" if abs(k - round(k, 1)) < 1e-9 else f"{k:.1f}"


def fmt_res(bits, rate, is_float: bool = False) -> str:
    b = "?" if bits is None else (f"{bits}f" if is_float else str(bits))
    return f"{b}/{fmt_rate(rate)}"


@dataclass
class FileResult:
    path: str
    verdict: str
    reasons: List[str] = field(default_factory=list)
    notes: List[str] = field(default_factory=list)
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
    lossy_cutoff_hz: Optional[float] = None
    cliffs_db: dict = field(default_factory=dict)
    bit_usage: dict = field(default_factory=dict)
    analysed_seconds: float = 0.0
    elapsed: float = 0.0
    spectrum: Optional[SpectrumStats] = None  # not serialised

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

    def to_dict(self) -> dict:
        d = {k: v for k, v in self.__dict__.items() if k != "spectrum"}
        d["claimed"] = self.claimed
        d["estimated_real"] = self.estimated
        for k, v in list(d.items()):
            if isinstance(v, float) and not np.isfinite(v):
                d[k] = None
        d["cliffs_db"] = {str(k): (round(v, 1) if np.isfinite(v) else None)
                          for k, v in self.cliffs_db.items()}
        return d


def _judge(info: AudioInfo, spec: SpectrumStats, bits: BitDepthAccumulator, res: FileResult) -> None:
    fake: List[str] = []
    suspect: List[str] = []
    notes: List[str] = res.notes
    eff = bits.effective_bits()
    res.effective_bits = eff
    res.bit_usage = {k: round(v, 4) for k, v in sorted(bits.bit_usage().items(), reverse=True)}
    res.bandwidth_hz = spec.bandwidth_hz
    res.cutoff_drop_db = spec.cutoff_drop_db
    res.noise_floor_db = spec.noise_floor_db
    res.upsampled_from = spec.upsampled_from
    res.upsample_edge_hz = spec.upsample_edge_hz
    res.lossy_cutoff_hz = spec.lossy_cutoff_hz if spec.lossy_cutoff else None
    res.cliffs_db = spec.cliffs_db
    res.true_rate = spec.upsampled_from or info.sample_rate

    bw_k = spec.bandwidth_hz / 1000.0

    # ---- bit depth -------------------------------------------------------
    claimed_bits = info.claimed_bits or 16
    if claimed_bits > 16 or info.is_float:
        if eff is not None and eff <= 16:
            if info.is_float:
                fake.append(f"Every sample is an exact {eff}-bit value: 16-bit audio stored in a "
                            f"{claimed_bits}-bit float container.")
            else:
                fake.append(f"Only {eff} of {claimed_bits} bits are used: the low {claimed_bits - eff} "
                            f"bits are always zero (16-bit audio padded into a {claimed_bits}-bit container).")
        elif eff is not None and eff < min(claimed_bits, 24) and not info.is_float:
            suspect.append(
                f"Only {eff} of {claimed_bits} bits are used (low {claimed_bits - eff} bits always zero); "
                "better than CD but not true 24-bit."
            )
        elif eff is not None:
            notes.append(f"Bit depth OK: {min(eff, 24) if info.is_float else eff} significant bits in use.")

    # ---- spectrum ---------------------------------------------------------
    if spec.lossy_cutoff:
        fake.append(
            f"Brick-wall low-pass at {spec.lossy_cutoff_hz / 1000:.1f} kHz "
            f"({spec.lossy_drop_db:.0f} dB drop within ~1 kHz, never recovers): typical of an "
            "MP3/AAC/Vorbis encoder, so this was made from a lossy file."
        )
    if spec.upsampled_from and info.sample_rate > spec.upsampled_from:
        drop = spec.cliffs_db.get(spec.upsampled_from, spec.cutoff_drop_db)
        src = fmt_rate(spec.upsampled_from)
        edge = (f" (level is down 20 dB by {spec.upsample_edge_hz / 1000:.1f} kHz)"
                if np.isfinite(spec.upsample_edge_hz) else "")
        msg = (f"Spectrum collapses by {drop:.0f} dB around {spec.upsampled_from / 2000:.2f} kHz{edge}, "
               f"the Nyquist limit of a {src} kHz master: upsampled from {src} kHz "
               f"({spec.upsample_confidence} confidence).")
        if spec.upsampled_from >= 88200:
            msg += f" Still hi-res, but not the advertised {fmt_rate(info.sample_rate)} kHz."
        fake.append(msg)
    elif info.sample_rate > 48000 and not spec.lossy_cutoff:
        if spec.bandwidth_hz < ULTRASONIC_HZ:
            suspect.append(
                f"No content above {bw_k:.1f} kHz although the format allows "
                f"{spec.nyquist / 1000:.0f} kHz, yet there is no brick-wall edge. Could be a dull/old "
                "recording or a gentle-filter upsample; listen/inspect the plot."
            )
        else:
            notes.append(f"Real ultrasonic content up to {bw_k:.1f} kHz "
                         f"(format limit {spec.nyquist / 1000:.0f} kHz).")
    elif info.sample_rate <= 48000:
        notes.append(f"Bandwidth {bw_k:.1f} kHz.")

    if not info.claims_hires:
        res.verdict = NOT_HIRES
        res.reasons = ["Standard resolution "
                       f"({fmt_res(info.claimed_bits, info.sample_rate)}): does not claim to be hi-res."]
        if spec.lossy_cutoff:
            res.reasons.append(fake[-1] if fake else "Lossy-sourced.")
        return

    if fake:
        res.verdict, res.reasons = FAKE, fake + suspect
    elif suspect:
        res.verdict, res.reasons = SUSPECT, suspect
    else:
        res.verdict = REAL
        res.reasons = ["Passed spectral and bit-depth checks."]


def analyze_file(path: str, max_seconds: Optional[float] = None, keep_spectrum: bool = False) -> FileResult:
    t0 = time.time()
    res = FileResult(path=path, verdict=ERROR)
    try:
        info = probe(path)
        res.codec, res.sample_rate, res.channels = info.codec, info.sample_rate, info.channels
        res.claimed_bits, res.is_float, res.duration = info.claimed_bits, info.is_float, info.duration
        if info.sample_rate <= 0:
            raise RuntimeError("could not determine sample rate")

        spec_acc = SpectrumAccumulator(info.sample_rate, info.channels)
        bit_acc = BitDepthAccumulator()
        scale = 1.0 / 2147483648.0
        frames = 0
        for block in iter_pcm(path, info.channels, chunk_frames=max(1 << 16, 4 * spec_acc.nfft),
                              max_seconds=max_seconds):
            bit_acc.update(block)
            spec_acc.update(block.astype(np.float64) * scale)
            frames += block.shape[0]
        res.analysed_seconds = frames / info.sample_rate

        if bit_acc.nonzero == 0:
            res.verdict = SUSPECT if info.claims_hires else NOT_HIRES
            res.reasons = ["File is digital silence; nothing to analyse."]
            return res

        freqs, power = spec_acc.result()
        spec = analyze_spectrum(freqs, power, info.sample_rate, spec_acc.noise_gain,
                                bit_acc.effective_bits())
        if keep_spectrum:
            res.spectrum = spec

        if info.is_lossy:
            res.verdict = LOSSY
            res.reasons = [f"Lossy codec ({info.codec}); can never be real hi-res whatever the "
                           f"{fmt_rate(info.sample_rate)} kHz sample rate says."]
            res.bandwidth_hz = spec.bandwidth_hz
            res.effective_bits = bit_acc.effective_bits()
            return res

        _judge(info, spec, bit_acc, res)
    except Exception as exc:  # noqa: BLE001 - report every failure per file
        res.verdict = ERROR
        res.reasons = [str(exc)]
    finally:
        res.elapsed = time.time() - t0
        res.path = os.path.abspath(path)
    return res
