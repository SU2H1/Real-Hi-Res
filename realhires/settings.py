"""User-adjustable settings, shared by the GUI and the CLI.

Settings are stored as JSON in the platform's config folder:

* Windows: %APPDATA%\\RealHiRes\\settings.json
* macOS:   ~/Library/Application Support/RealHiRes/settings.json
* Linux:   $XDG_CONFIG_HOME/realhires/settings.json (default ~/.config/realhires)

Every field carries metadata (label, group, limits, help text) so the GUI can
build its Settings window automatically.  Unknown keys in the file are ignored
and invalid values fall back to the default, so an old or hand-edited file can
never stop the program from starting.
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import MISSING, asdict, dataclass, field, fields
from typing import Any, Dict, List, Optional

GROUPS = ("Detection", "Scanning", "Display")

ALL_VERDICTS = ["FAKE LOSSLESS", "FAKE HI-RES", "SUSPICIOUS", "REAL HI-RES", "NOT HI-RES",
                "LOSSY FORMAT", "ERROR"]
COLORMAPS = ["inferno", "viridis", "magma", "gray"]
DEFAULT_EXTENSIONS = (".flac .wav .wave .w64 .rf64 .aif .aiff .aifc .caf .m4a .mp4 .alac .ape .wv "
                      ".tta .tak .dsf .dff .mka .ogg .oga .opus .mp3 .aac .wma .mpc .ac3 .dts")


def _f(default, group, label, help_text, lo=None, hi=None, choices=None, kind=None):
    md = {"group": group, "label": label, "help": help_text, "min": lo, "max": hi,
          "choices": choices, "kind": kind}
    if isinstance(default, list):
        return field(default_factory=lambda: list(default), metadata=md)
    return field(default=default, metadata=md)


@dataclass
class Settings:
    # ---- Detection ------------------------------------------------------
    cliff_db: float = _f(
        25.0, "Detection", "Upsampling cliff (dB)",
        "Minimum level drop at the Nyquist frequency of a lower sample rate (22.05 kHz for "
        "44.1 kHz, 24 kHz for 48 kHz, ...) that marks a file as upsampled. Lower = stricter, "
        "more files flagged.", 5, 150)
    cliff_strong_db: float = _f(
        40.0, "Detection", "High-confidence cliff (dB)",
        "A cliff at least this deep is reported with high confidence.", 5, 200)
    lossy_cliff_db: float = _f(
        25.0, "Detection", "Lossy cut-off drop (dB)",
        "Minimum drop within ~1 kHz that counts as an MP3/AAC encoder low-pass "
        "(the sign of a fake FLAC/WAV made from a lossy file).", 5, 150)
    lossy_min_khz: float = _f(
        11.0, "Detection", "Lossy cut-off search from (kHz)",
        "Lowest frequency at which a lossy encoder low-pass is searched for.", 4, 22)
    lossy_max_khz: float = _f(
        20.3, "Detection", "Lossy cut-off search up to (kHz)",
        "Highest frequency at which a lossy low-pass is searched for. Raising it towards 21 kHz "
        "also catches 320 kbps MP3s but may flag genuine CD masters with a steep filter.", 10, 23)
    ultrasonic_khz: float = _f(
        24.5, "Detection", "Ultrasonic content expected above (kHz)",
        "A high-sample-rate file with no content above this frequency (and no cliff) is "
        "reported as SUSPICIOUS.", 15, 60)
    steep_drop_db: float = _f(
        20.0, "Detection", "Brick-wall edge (dB)",
        "Drop across the bandwidth edge that counts as a brick-wall filter.", 5, 150)
    content_margin_db: float = _f(
        10.0, "Detection", "Content above noise floor (dB)",
        "How far above the noise floor a frequency band must be to count as real content.", 1, 60)
    relevant_range_db: float = _f(
        110.0, "Detection", "Ignore content below peak by (dB)",
        "Content more than this far below the loudest part of the spectrum is ignored "
        "(removes resampler artefacts).", 40, 200)
    bit_threshold_pct: float = _f(
        0.5, "Detection", "Bit-usage threshold (%)",
        "A bit position counts as used when at least this share of samples uses it. "
        "Real 24-bit audio uses its lowest bit in ~50 % of samples.", 0.001, 25)
    edge_ratio: float = _f(
        0.98, "Detection", "Resampler edge ratio",
        "Expert: where resamplers put their -20 dB point relative to the source Nyquist; "
        "used to choose between neighbouring source rates (44.1 vs 48 kHz).", 0.8, 1.1)

    # ---- Scanning -------------------------------------------------------
    jobs: int = _f(
        max(1, min(4, os.cpu_count() or 1)), "Scanning", "Parallel jobs",
        "How many files are analysed at the same time.", 1, 64, kind="int")
    max_seconds: float = _f(
        0.0, "Scanning", "Analyse at most (seconds, 0 = whole file)",
        "Analysing only the start of each file is faster but can miss content.", 0, 36000)
    quick_scan_seconds: float = _f(
        90.0, "Scanning", "Quick scan length (seconds)",
        "Length analysed when 'Quick scan' is ticked in the toolbar.", 5, 3600)
    recursive: bool = _f(True, "Scanning", "Include sub-folders",
                         "Scan folders recursively.", kind="bool")
    extensions: str = _f(
        DEFAULT_EXTENSIONS, "Scanning", "File extensions",
        "Space- or comma-separated list of file extensions to scan.", kind="str")

    # ---- Display --------------------------------------------------------
    shown_verdicts: List[str] = _f(
        ["FAKE LOSSLESS", "FAKE HI-RES", "SUSPICIOUS"], "Display", "Verdicts listed by default",
        "Which verdicts the result list shows when the app starts.", choices=ALL_VERDICTS,
        kind="multi")
    show_summary_popup: bool = _f(True, "Display", "Show summary after each scan",
                                  "Pop up the percentage breakdown when a scan finishes.",
                                  kind="bool")
    double_click: str = _f("Open folder", "Display", "Double-click / Enter opens",
                           "What double-clicking a row (or pressing Enter) does.",
                           choices=["Open folder", "Open file"], kind="choice")
    colormap: str = _f("inferno", "Display", "Spectrogram colours",
                       "Colour map of the spectrogram.", choices=COLORMAPS, kind="choice")
    spectrogram_range_db: float = _f(
        120.0, "Display", "Spectrogram range (dB)",
        "Dynamic range shown in the spectrogram; quieter detail is drawn black.", 40, 200)

    # ------------------------------------------------------------------
    def extension_set(self) -> set:
        parts = self.extensions.replace(",", " ").split()
        return {("." + p.lstrip(".")).lower() for p in parts if p.strip(".")}

    @classmethod
    def field_info(cls) -> List[Dict[str, Any]]:
        out = []
        for f in fields(cls):
            default = f.default if f.default is not MISSING else f.default_factory()  # type: ignore[misc]
            out.append(dict(name=f.name, default=default, **f.metadata))
        return out

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def from_dict(cls, data: Optional[dict]) -> "Settings":
        s = cls()
        if not isinstance(data, dict):
            return s
        for f in fields(cls):
            if f.name in data:
                v = _coerce(f, data[f.name])
                if v is not None:
                    setattr(s, f.name, v)
        return s

    def set_value(self, name: str, raw: str) -> None:
        """Set one field from text (CLI --set name=value). Raises ValueError."""
        f = next((f for f in fields(self) if f.name == name), None)
        if f is None:
            raise ValueError(f"unknown setting '{name}' (known: {', '.join(x.name for x in fields(self))})")
        kind = f.metadata.get("kind")
        value: Any = raw
        if kind == "multi":
            value = [v.strip().upper() for v in raw.split(",") if v.strip()]
        elif kind == "bool":
            value = raw.strip().lower() in ("1", "true", "yes", "on")
        v = _coerce(f, value)
        if v is None:
            raise ValueError(f"invalid value for {name}: {raw!r}")
        setattr(self, name, v)


def _coerce(f, value):
    """Validate/convert a value for field `f`; None means invalid."""
    md = f.metadata
    kind = md.get("kind")
    try:
        if kind == "bool":
            return bool(value)
        if kind == "str":
            return str(value)
        if kind == "choice":
            return value if value in md["choices"] else None
        if kind == "multi":
            vals = [v for v in value if v in md["choices"]]
            return vals if isinstance(value, (list, tuple)) else None
        num = int(value) if kind == "int" else float(value)
    except (TypeError, ValueError, KeyError):
        return None
    if md.get("min") is not None and num < md["min"]:
        return None
    if md.get("max") is not None and num > md["max"]:
        return None
    return num


def config_dir() -> str:
    if sys.platform.startswith("win"):
        base = os.environ.get("APPDATA") or os.path.expanduser("~\\AppData\\Roaming")
        return os.path.join(base, "RealHiRes")
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support/RealHiRes")
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return os.path.join(base, "realhires")


def default_path() -> str:
    return os.environ.get("REALHIRES_SETTINGS") or os.path.join(config_dir(), "settings.json")


def load(path: Optional[str] = None) -> Settings:
    path = path or default_path()
    try:
        with open(path, encoding="utf-8") as fh:
            return Settings.from_dict(json.load(fh))
    except (OSError, ValueError):
        return Settings()


def save(settings: Settings, path: Optional[str] = None) -> str:
    path = path or default_path()
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as fh:
        json.dump(settings.to_dict(), fh, indent=2)
    os.replace(tmp, path)
    return path
