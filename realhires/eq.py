"""Equalizer maths shared by the player in both versions (2.0).

A graphic equalizer (ten fixed bands, the frequencies Apple Music uses) and a
parametric equalizer (up to ten bands of any type) are both built from the
biquad filters of Robert Bristow-Johnson's "Audio EQ Cookbook".  This module
computes their coefficients, the combined frequency response (for the curves
the EQ windows draw and for the automatic headroom that prevents clipping), and
the FFmpeg filter chain the Python player decodes through.  The Rust version
computes exactly the same numbers (`rust/crates/core/src/eq.rs`), checked
against `rust/crates/core/tests/python_eq.json`.

The state is plain JSON, stored in the player state (`playerstate.py`):

    {"enabled": true, "prevent_clipping": true,
     "graphic": {"enabled": true, "preset": "Flat", "preamp": 0.0, "gains": [0.0] * 10},
     "parametric": {"enabled": false, "preamp": 0.0,
                    "bands": [{"type": "peak", "freq": 1000.0, "gain": 0.0, "q": 1.0, "on": true}]}}
"""

from __future__ import annotations

import cmath
import copy
import json
import math
import re
from typing import Dict, List, Optional, Tuple

GRAPHIC_FREQS = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0]
GRAPHIC_LABELS = ["32", "64", "125", "250", "500", "1K", "2K", "4K", "8K", "16K"]
GRAPHIC_Q = math.sqrt(2.0)     # one octave wide: Q = sqrt(2^N) / (2^N - 1) for N = 1
GAIN_LIMIT_DB = 12.0           # every gain and preamp slider runs from -12 to +12 dB
MAX_PARAMETRIC_BANDS = 10
MIN_FREQ, MAX_FREQ = 20.0, 20000.0
MIN_Q, MAX_Q = 0.1, 10.0

BAND_TYPES = ["peak", "low_shelf", "high_shelf", "low_pass", "high_pass", "notch"]
BAND_TYPE_LABELS = {"peak": "Peak", "low_shelf": "Low shelf", "high_shelf": "High shelf",
                    "low_pass": "Low pass", "high_pass": "High pass", "notch": "Notch"}
GAIN_TYPES = ("peak", "low_shelf", "high_shelf")  # the other types ignore the gain

# Real-Hi-Res's own presets for the graphic equalizer (dB per band, 32 Hz ... 16 kHz).
PRESETS: Dict[str, List[float]] = {
    "Flat": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    "Bass Boost": [6, 5, 4, 2.5, 1, 0, 0, 0, 0, 0],
    "Bass Cut": [-6, -5, -4, -2.5, -1, 0, 0, 0, 0, 0],
    "Treble Boost": [0, 0, 0, 0, 0, 1, 2.5, 4, 5, 6],
    "Treble Cut": [0, 0, 0, 0, 0, -1, -2.5, -4, -5, -6],
    "Vocal": [-2, -2, -1, 1, 3, 3.5, 3, 1.5, 0, -1],
    "Loudness": [5, 4, 2, 0, -1, -1, 0, 1, 3, 4],
    "Late Night": [3, 2.5, 1.5, 0.5, 0, 0, 0.5, 1.5, 2, 2],
    "Small Speakers": [5, 4.5, 3.5, 2, 1, 0, -0.5, -1, -1.5, -2],
    "Acoustic": [3, 3, 2, 0.5, 1, 1, 2, 2.5, 2, 1],
    "Classical": [3, 2.5, 2, 1, -1, -1, 0, 1.5, 2.5, 3],
    "Electronic": [4, 3.5, 1, 0, -1.5, 1, 0.5, 1, 3.5, 4.5],
    "Hip-Hop": [4.5, 4, 1.5, 2.5, -1, -1, 1, -0.5, 1.5, 2.5],
    "Jazz": [3, 2, 1, 2, -1.5, -1.5, 0, 1, 2, 3],
    "Pop": [-1, -0.5, 0, 1.5, 3.5, 3.5, 1.5, 0, -0.5, -1],
    "Rock": [4.5, 3.5, 2.5, 1, -0.5, -1, 0.5, 2, 3, 3.5],
    "Spoken Word": [-3, -2, 0, 1, 3, 4, 4, 3, 1.5, 0],
}
PRESET_NAMES = list(PRESETS)
CUSTOM = "Custom"

Coeffs = Tuple[float, float, float, float, float]  # b0, b1, b2, a1, a2 (normalised by a0)
IDENTITY: Coeffs = (1.0, 0.0, 0.0, 0.0, 0.0)


def default_state() -> dict:
    return {"enabled": False, "prevent_clipping": True,
            "graphic": {"enabled": True, "preset": "Flat", "preamp": 0.0, "gains": [0.0] * len(GRAPHIC_FREQS)},
            "parametric": {"enabled": False, "preamp": 0.0,
                           "bands": [{"type": "peak", "freq": 1000.0, "gain": 0.0, "q": 1.0, "on": True}]}}


def _num(value, default: float, lo: float, hi: float) -> float:
    try:
        v = float(value)
    except (TypeError, ValueError):
        return default
    if math.isnan(v):
        return default
    return min(max(v, lo), hi)


def normalize(state) -> dict:
    """A valid EQ state from whatever was stored (unknown or broken values fall back
    to the defaults, numbers are clamped to their ranges)."""
    out = default_state()
    if not isinstance(state, dict):
        return out
    out["enabled"] = bool(state.get("enabled", out["enabled"]))
    out["prevent_clipping"] = bool(state.get("prevent_clipping", True))
    g = state.get("graphic")
    if isinstance(g, dict):
        gr = out["graphic"]
        gr["enabled"] = bool(g.get("enabled", True))
        gr["preamp"] = _num(g.get("preamp"), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB)
        gains = g.get("gains")
        if isinstance(gains, list) and len(gains) == len(GRAPHIC_FREQS):
            gr["gains"] = [_num(x, 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB) for x in gains]
        preset = g.get("preset")
        gr["preset"] = preset if preset in PRESETS or preset == CUSTOM else preset_name(gr["gains"])
    p = state.get("parametric")
    if isinstance(p, dict):
        pa = out["parametric"]
        pa["enabled"] = bool(p.get("enabled", False))
        pa["preamp"] = _num(p.get("preamp"), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB)
        bands = p.get("bands")
        if isinstance(bands, list):
            pa["bands"] = [normalize_band(b) for b in bands[:MAX_PARAMETRIC_BANDS] if isinstance(b, dict)]
    return out


def normalize_band(b: dict) -> dict:
    kind = b.get("type") if b.get("type") in BAND_TYPES else "peak"
    return {"type": kind, "freq": _num(b.get("freq"), 1000.0, MIN_FREQ, MAX_FREQ),
            "gain": _num(b.get("gain"), 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB),
            "q": _num(b.get("q"), 1.0, MIN_Q, MAX_Q), "on": bool(b.get("on", True))}


def preset_name(gains: List[float]) -> str:
    """The preset these gains are, or "Custom"."""
    for name, values in PRESETS.items():
        if all(abs(float(a) - float(b)) < 1e-9 for a, b in zip(gains, values)):
            return name
    return CUSTOM


def apply_preset(state: dict, name: str) -> dict:
    s = normalize(copy.deepcopy(state))
    if name in PRESETS:
        s["graphic"]["gains"] = [float(x) for x in PRESETS[name]]
        s["graphic"]["preset"] = name
    return s


# ------------------------------------------------------------------ biquads ---
def biquad(kind: str, freq: float, gain_db: float, q: float, rate: float) -> Coeffs:
    """Normalised RBJ cookbook coefficients; the identity for a band at or above
    the Nyquist frequency (it would not be stable) or one that does nothing."""
    if rate <= 0 or freq <= 0 or freq >= 0.49 * rate or q <= 0:
        return IDENTITY
    if kind in GAIN_TYPES and gain_db == 0.0:
        return IDENTITY
    a = 10.0 ** (gain_db / 40.0)
    w0 = 2.0 * math.pi * freq / rate
    cw, sw = math.cos(w0), math.sin(w0)
    alpha = sw / (2.0 * q)
    if kind == "peak":
        b0, b1, b2 = 1 + alpha * a, -2 * cw, 1 - alpha * a
        a0, a1, a2 = 1 + alpha / a, -2 * cw, 1 - alpha / a
    elif kind in ("low_shelf", "high_shelf"):
        sa = 2.0 * math.sqrt(a) * alpha
        if kind == "low_shelf":
            b0 = a * ((a + 1) - (a - 1) * cw + sa)
            b1 = 2 * a * ((a - 1) - (a + 1) * cw)
            b2 = a * ((a + 1) - (a - 1) * cw - sa)
            a0 = (a + 1) + (a - 1) * cw + sa
            a1 = -2 * ((a - 1) + (a + 1) * cw)
            a2 = (a + 1) + (a - 1) * cw - sa
        else:
            b0 = a * ((a + 1) + (a - 1) * cw + sa)
            b1 = -2 * a * ((a - 1) + (a + 1) * cw)
            b2 = a * ((a + 1) + (a - 1) * cw - sa)
            a0 = (a + 1) - (a - 1) * cw + sa
            a1 = 2 * ((a - 1) - (a + 1) * cw)
            a2 = (a + 1) - (a - 1) * cw - sa
    elif kind == "low_pass":
        b0, b1, b2 = (1 - cw) / 2, 1 - cw, (1 - cw) / 2
        a0, a1, a2 = 1 + alpha, -2 * cw, 1 - alpha
    elif kind == "high_pass":
        b0, b1, b2 = (1 + cw) / 2, -(1 + cw), (1 + cw) / 2
        a0, a1, a2 = 1 + alpha, -2 * cw, 1 - alpha
    elif kind == "notch":
        b0, b1, b2 = 1.0, -2 * cw, 1.0
        a0, a1, a2 = 1 + alpha, -2 * cw, 1 - alpha
    else:
        return IDENTITY
    return (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)


def stages(state: dict, rate: float) -> List[Coeffs]:
    """Every active biquad of the state, graphic bands first, at this sample rate."""
    s = normalize(state)
    if not s["enabled"]:
        return []
    out: List[Coeffs] = []
    if s["graphic"]["enabled"]:
        for f, gdb in zip(GRAPHIC_FREQS, s["graphic"]["gains"]):
            c = biquad("peak", f, gdb, GRAPHIC_Q, rate)
            if c != IDENTITY:
                out.append(c)
    if s["parametric"]["enabled"]:
        for b in s["parametric"]["bands"]:
            if b["on"]:
                c = biquad(b["type"], b["freq"], b["gain"], b["q"], rate)
                if c != IDENTITY:
                    out.append(c)
    return out


def magnitude_db(coeffs: List[Coeffs], freq: float, rate: float) -> float:
    """The combined gain of the biquads at one frequency, in dB."""
    z = cmath.exp(-1j * 2.0 * math.pi * freq / rate)
    total = 0.0
    for b0, b1, b2, a1, a2 in coeffs:
        h = (b0 + b1 * z + b2 * z * z) / (1 + a1 * z + a2 * z * z)
        total += 20.0 * math.log10(max(abs(h), 1e-12))
    return total


RESPONSE_POINTS = 241  # 20 Hz ... 20 kHz, 80 points per decade
RESPONSE_RATE = 48000.0


def response_freqs() -> List[float]:
    return [20.0 * 10.0 ** (3.0 * i / (RESPONSE_POINTS - 1)) for i in range(RESPONSE_POINTS)]


def response(state: dict, rate: float = RESPONSE_RATE) -> List[Tuple[float, float]]:
    """(frequency, dB) of the whole EQ including the preamp, for drawing the curve."""
    s = normalize(state)
    coeffs = stages(s, rate)
    pre = preamp_db(s)
    return [(f, magnitude_db(coeffs, f, rate) + pre) for f in response_freqs()]


def preamp_db(state: dict, rate: float = RESPONSE_RATE) -> float:
    """The gain applied before the filters: the user's preamps, lowered by the
    largest boost of the curve when "prevent clipping" is on."""
    s = normalize(state)
    if not s["enabled"]:
        return 0.0
    pre = 0.0
    if s["graphic"]["enabled"]:
        pre += s["graphic"]["preamp"]
    if s["parametric"]["enabled"]:
        pre += s["parametric"]["preamp"]
    if s["prevent_clipping"]:
        coeffs = stages(s, rate)
        peak = max((magnitude_db(coeffs, f, rate) for f in response_freqs()), default=0.0) + pre
        if peak > 0:
            pre -= peak
    return pre


def ffmpeg_filter(state: dict, rate: float) -> Optional[str]:
    """The FFmpeg audio filter chain that applies the EQ (the Python player decodes
    through it), or None when the EQ does nothing."""
    s = normalize(state)
    coeffs = stages(s, rate)
    pre = preamp_db(s, RESPONSE_RATE)
    if not coeffs and abs(pre) < 1e-9:
        return None
    parts = []
    if abs(pre) >= 1e-9:
        parts.append(f"volume={pre!r}dB:precision=double")
    for b0, b1, b2, a1, a2 in coeffs:
        parts.append(f"biquad=b0={b0!r}:b1={b1!r}:b2={b2!r}:a0=1:a1={a1!r}:a2={a2!r}")
    return ",".join(parts)


class Biquad:
    """A reference implementation of one stage (transposed direct form II, double
    precision), for tests; the Rust player runs the same arithmetic."""

    def __init__(self, c: Coeffs, channels: int):
        self.c = c
        self.z1 = [0.0] * channels
        self.z2 = [0.0] * channels

    def process(self, x: float, ch: int) -> float:
        b0, b1, b2, a1, a2 = self.c
        y = b0 * x + self.z1[ch]
        self.z1[ch] = b1 * x - a1 * y + self.z2[ch]
        self.z2[ch] = b2 * x - a2 * y
        return y


# ------------------------------------------------------------- preset files ---
# The user's named presets are kept in the player state (`playerstate.py`).
# They can be exported and imported as Real-Hi-Res's own JSON, or as the text
# that Equalizer APO reads and AutoEQ publishes for headphones
# ("ParametricEQ.txt": "Preamp: -6.2 dB" and "Filter 1: ON PK Fc 105 Hz Gain
# 3.5 dB Q 0.70" lines; "GraphicEQ: 20 -1.2; 32 0.5; ..." is read too).
EQ_FILE_FORMAT = "real-hi-res-eq"
APO_TYPES = {"peak": "PK", "low_shelf": "LSC", "high_shelf": "HSC", "low_pass": "LPQ", "high_pass": "HPQ",
             "notch": "NO"}
_APO_IN = {"PK": "peak", "PEQ": "peak", "MODAL": "peak", "LS": "low_shelf", "LSC": "low_shelf", "LSQ": "low_shelf",
           "HS": "high_shelf", "HSC": "high_shelf", "HSQ": "high_shelf", "LP": "low_pass", "LPQ": "low_pass",
           "HP": "high_pass", "HPQ": "high_pass", "NO": "notch"}


class EqFileError(ValueError):
    """A file that is not an equalizer preset Real-Hi-Res can read."""


def export_json(name: str, state: dict) -> str:
    """A preset as Real-Hi-Res's own .json file."""
    data = {"format": EQ_FILE_FORMAT, "version": 1, "name": name, "eq": normalize(state)}
    return json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def export_apo(name: str, state: dict) -> str:
    """A preset as Equalizer APO / AutoEQ text: the pre-amp, then one filter per
    band of the graphic (only bands that are not 0 dB) and the parametric EQ."""
    s = normalize(state)
    s["enabled"] = True
    lines = [f"# {name}", "# Equalizer preset exported by Universal Music Player", f"Preamp: {preamp_db(s):.1f} dB"]
    n = 0
    if s["graphic"]["enabled"]:
        for f, g in zip(GRAPHIC_FREQS, s["graphic"]["gains"]):
            if abs(g) >= 1e-9:
                n += 1
                lines.append(f"Filter {n}: ON PK Fc {f:.1f} Hz Gain {g:.1f} dB Q {GRAPHIC_Q:.2f}")
    if s["parametric"]["enabled"]:
        for b in s["parametric"]["bands"]:
            n += 1
            gain = f" Gain {b['gain']:.1f} dB" if b["type"] in GAIN_TYPES else ""
            lines.append(f"Filter {n}: {'ON' if b['on'] else 'OFF'} {APO_TYPES[b['type']]} Fc {b['freq']:.1f} Hz"
                         f"{gain} Q {b['q']:.2f}")
    return "\n".join(lines) + "\n"


def _float(text: str) -> Optional[float]:
    try:
        v = float(text.replace(",", "."))
    except ValueError:
        return None
    return v if math.isfinite(v) else None


def _graphic_from_points(points: List[Tuple[float, float]]) -> List[float]:
    """The ten graphic bands sampled from a GraphicEQ curve (straight lines on a log axis)."""
    pts = sorted(p for p in points if p[0] > 0)
    out = []
    for f in GRAPHIC_FREQS:
        if f <= pts[0][0]:
            g = pts[0][1]
        elif f >= pts[-1][0]:
            g = pts[-1][1]
        else:
            g = pts[0][1]
            for (f0, g0), (f1, g1) in zip(pts, pts[1:]):
                if f0 <= f <= f1:
                    g = g0 if f1 == f0 else g0 + (g1 - g0) * math.log(f / f0) / math.log(f1 / f0)
                    break
        out.append(_num(g, 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB))
    return out


def import_eq(text: str, fallback_name: str = "Imported") -> Tuple[str, dict, List[str]]:
    """(name, state, notes) from the text of a preset file; notes say what had
    to be left out or limited.  Raises EqFileError for anything else."""
    text = text.lstrip("﻿")
    notes: List[str] = []
    try:
        data = json.loads(text)
    except ValueError:
        data = None
    if isinstance(data, dict):
        if "eq" in data and isinstance(data["eq"], dict):
            name = data.get("name") if isinstance(data.get("name"), str) and data.get("name").strip() else fallback_name
            return name.strip(), normalize(data["eq"]), notes
        if "graphic" in data or "parametric" in data:
            return fallback_name, normalize(data), notes
        raise EqFileError("This .json file is not an equalizer preset of Universal Music Player.")
    preamp = 0.0
    bands: List[dict] = []
    graphic: Optional[List[float]] = None
    skipped_types: List[str] = []
    found = False
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = re.match(r"(?i)^preamp\s*:\s*([-+]?[\d.,]+)\s*db", line)
        if m:
            v = _float(m.group(1))
            if v is not None:
                preamp += v
                found = True
            continue
        m = re.match(r"(?i)^graphiceq\s*:\s*(.*)$", line)
        if m:
            points = []
            for part in m.group(1).split(";"):
                xs = part.split()
                if len(xs) == 2 and _float(xs[0]) is not None and _float(xs[1]) is not None and _float(xs[0]) > 0:
                    points.append((_float(xs[0]), _float(xs[1])))
            if points:
                graphic = _graphic_from_points(points)
                found = True
            continue
        m = re.match(r"(?i)^filter\s*\d*\s*:\s*(on|off)\s+([a-z]+)\s*(.*)$", line)
        if not m:
            continue
        found = True
        kind = _APO_IN.get(m.group(2).upper())
        if kind is None:
            if m.group(2).upper() not in skipped_types:
                skipped_types.append(m.group(2).upper())
            continue
        rest = m.group(3)
        fc = re.search(r"(?i)\bfc\s+([-+]?[\d.,]+)\s*hz", rest)
        gain = re.search(r"(?i)\bgain\s+([-+]?[\d.,]+)\s*db", rest)
        q = re.search(r"(?i)\bq\s+([\d.,]+)", rest)
        bw = re.search(r"(?i)\bbw\s+oct\s+([\d.,]+)", rest)
        if fc is None or _float(fc.group(1)) is None:
            continue
        qv = _float(q.group(1)) if q else None
        if qv is None and bw is not None and _float(bw.group(1)):
            n = 2.0 ** _float(bw.group(1))
            qv = math.sqrt(n) / (n - 1.0)
        if qv is None:
            qv = math.sqrt(0.5) if kind in ("low_shelf", "high_shelf", "low_pass", "high_pass") else 1.0
        bands.append(normalize_band({"type": kind, "freq": _float(fc.group(1)),
                                     "gain": (_float(gain.group(1)) or 0.0) if gain else 0.0, "q": qv,
                                     "on": m.group(1).lower() == "on"}))
    if not found:
        raise EqFileError("This file is not an equalizer preset Universal Music Player can read: it reads its own .json "
                          "presets and Equalizer APO or AutoEQ text (ParametricEQ or GraphicEQ).")
    if skipped_types:
        notes.append(f"Filters of the type {', '.join(skipped_types)} are not supported and were left out.")
    if len(bands) > MAX_PARAMETRIC_BANDS:
        notes.append(f"The file has {len(bands)} filters; the first {MAX_PARAMETRIC_BANDS} were imported.")
        bands = bands[:MAX_PARAMETRIC_BANDS]
    if abs(preamp) > GAIN_LIMIT_DB:
        notes.append(f"The pre-amp of {preamp:.1f} dB was limited to {GAIN_LIMIT_DB:.0f} dB.")
    state = default_state()
    state["enabled"] = True
    if graphic is not None:
        state["graphic"].update({"enabled": True, "gains": graphic, "preset": preset_name(graphic),
                                 "preamp": _num(preamp if not bands else 0.0, 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB)})
    else:
        state["graphic"]["enabled"] = False
    if bands:
        state["parametric"] = {"enabled": True, "preamp": _num(preamp, 0.0, -GAIN_LIMIT_DB, GAIN_LIMIT_DB),
                               "bands": bands}
    return fallback_name, normalize(state), notes
