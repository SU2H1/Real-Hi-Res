"""Spectrogram rendering with NumPy only (no Pillow/matplotlib).

Produces RGB arrays that the GUI shows through a Tk PhotoImage (PPM data) and
that the CLI writes as PNG files (--spectrogram DIR).
"""

from __future__ import annotations

import struct
import zlib
from typing import Optional

import numpy as np

from .analysis import decode_spectrogram

# Colour-map anchor points (position 0..1, RGB 0..255), linearly interpolated.
_ANCHORS = {
    "inferno": [(0.0, (0, 0, 4)), (0.15, (31, 12, 72)), (0.3, (85, 15, 109)), (0.45, (136, 34, 106)),
                (0.6, (186, 54, 85)), (0.75, (227, 89, 51)), (0.87, (249, 140, 10)), (1.0, (252, 255, 164))],
    "magma": [(0.0, (0, 0, 4)), (0.2, (44, 17, 95)), (0.4, (114, 31, 129)), (0.6, (183, 55, 121)),
              (0.8, (251, 135, 97)), (1.0, (252, 253, 191))],
    "viridis": [(0.0, (68, 1, 84)), (0.25, (59, 82, 139)), (0.5, (33, 145, 140)),
                (0.75, (94, 201, 98)), (1.0, (253, 231, 37))],
    "gray": [(0.0, (0, 0, 0)), (1.0, (255, 255, 255))],
}


def colormap(name: str) -> np.ndarray:
    """256x3 uint8 lookup table."""
    anchors = _ANCHORS.get(name, _ANCHORS["inferno"])
    pos = np.array([a[0] for a in anchors])
    cols = np.array([a[1] for a in anchors], dtype=np.float64)
    x = np.linspace(0.0, 1.0, 256)
    lut = np.stack([np.interp(x, pos, cols[:, i]) for i in range(3)], axis=1)
    return lut.round().astype(np.uint8)


def spectrogram_rgb(sg: dict, width: int, height: int, range_db: float = 120.0,
                    cmap: str = "inferno", top_db: Optional[float] = None) -> np.ndarray:
    """Render a compact spectrogram to (height, width, 3) uint8.

    Time runs left to right, frequency bottom (0 Hz) to top (Nyquist).  The
    colour scale spans `range_db` below `top_db` (default: the loudest cell).
    """
    db = decode_spectrogram(sg)  # (time, freq)
    t, f = db.shape
    if top_db is None:
        top_db = float(np.percentile(db, 99.9))
    lo = top_db - range_db
    norm = np.clip((db - lo) / max(range_db, 1e-6), 0.0, 1.0)
    # nearest-neighbour resampling onto the output grid
    ti = np.minimum((np.arange(width) * t / max(width, 1)).astype(int), t - 1)
    fi = np.minimum(((height - 1 - np.arange(height)) * f / max(height, 1)).astype(int), f - 1)
    img = norm[ti][:, fi].T  # (height, width)
    lut = colormap(cmap)
    return lut[(img * 255).astype(np.uint8)]


def to_ppm(rgb: np.ndarray) -> bytes:
    h, w, _ = rgb.shape
    return b"P6 %d %d 255\n" % (w, h) + np.ascontiguousarray(rgb, dtype=np.uint8).tobytes()


def to_png(rgb: np.ndarray) -> bytes:
    """Minimal PNG encoder for RGB/RGBA uint8 images."""
    h, w, c = rgb.shape
    raw = b"".join(b"\x00" + rgb[y].tobytes() for y in range(h))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    color_type = 6 if c == 4 else 2
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, color_type, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6))
            + chunk(b"IEND", b""))


def _font_digit_mask():
    """Tiny 3x5 bitmap font for axis labels in PNG output (digits, k, s, '.')."""
    glyphs = {
        "0": "111101101101111", "1": "010110010010111", "2": "111001111100111",
        "3": "111001111001111", "4": "101101111001001", "5": "111100111001111",
        "6": "111100111101111", "7": "111001001001001", "8": "111101111101111",
        "9": "111101111001111", "k": "100101110101101", "s": "000011010001110",
        ".": "000000000000010", " ": "000000000000000", "H": "101101111101101",
        "z": "000111001010111",
    }
    return {ch: np.array([int(b) for b in bits], dtype=bool).reshape(5, 3) for ch, bits in glyphs.items()}


_GLYPHS = _font_digit_mask()


def _draw_text(img: np.ndarray, x: int, y: int, text: str, color, scale: int = 2) -> None:
    for i, ch in enumerate(text):
        g = _GLYPHS.get(ch)
        if g is None:
            continue
        big = np.kron(g, np.ones((scale, scale), dtype=bool))
        h, w = big.shape
        x0 = x + i * (3 * scale + scale)
        y0, y1, x1 = y, min(y + h, img.shape[0]), min(x0 + w, img.shape[1])
        if x0 >= img.shape[1] or y0 >= img.shape[0] or x0 < 0 or y0 < 0:
            continue
        img[y0:y1, x0:x1][big[: y1 - y0, : x1 - x0]] = color


def spectrogram_png(sg: dict, width: int = 1000, height: int = 400, range_db: float = 120.0,
                    cmap: str = "inferno", markers=(), title_color=(255, 255, 255)) -> bytes:
    """PNG with frequency gridlines/labels; `markers` = [(hz, (r, g, b))] lines."""
    left, bottom = 46, 22
    pw, ph = width - left - 6, height - bottom - 6
    img = np.full((height, width, 3), 24, dtype=np.uint8)
    img[6:6 + ph, left:left + pw] = spectrogram_rgb(sg, pw, ph, range_db, cmap)
    nyq = sg["nyquist"]
    step = 2000 if nyq <= 24000 else 5000 if nyq <= 50000 else 10000 if nyq <= 100000 else 20000
    f = 0
    while f <= nyq:
        y = 6 + ph - 1 - int(round(f / nyq * (ph - 1)))
        img[y, left - 4:left] = (180, 180, 180)
        _draw_text(img, 2, max(0, y - 5), f"{f / 1000:g}k".rjust(4), (200, 200, 200))
        f += step
    for hz, color in markers:
        if 0 < hz < nyq:
            y = 6 + ph - 1 - int(round(hz / nyq * (ph - 1)))
            img[y, left:left + pw:3] = color
    total = sg["shape"][0] * sg["seconds_per_column"]
    for frac in (0.0, 0.25, 0.5, 0.75):
        x = left + int(frac * (pw - 1))
        img[6 + ph:6 + ph + 4, x] = (180, 180, 180)
        label = f"{frac * total:.0f}s" if total >= 20 else f"{frac * total:.1f}s"
        _draw_text(img, x + 2, 6 + ph + 6, label, (200, 200, 200))
    return to_png(img)
