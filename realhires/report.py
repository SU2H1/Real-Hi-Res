"""Output: terminal table, JSON/CSV export and SVG spectrum plots."""

from __future__ import annotations

import csv
import hashlib
import html
import json
import os
import re
import sys
from typing import Iterable, List, Optional, TextIO

import numpy as np

from .verdict import (ERROR, FAKE, FAKE_LOSSLESS, LOSSY, NOT_HIRES, REAL, SUSPECT, FileResult,
                      verdict_breakdown)

_COLORS = {REAL: "32", FAKE: "31", FAKE_LOSSLESS: "1;31", SUSPECT: "33", NOT_HIRES: "36",
           LOSSY: "35", ERROR: "90"}


def setup_console() -> None:
    """Never crash on characters the console cannot show (Japanese file names,
    arrows, bar blocks on a Windows cp1252 console): unencodable characters are
    replaced, and redirected output (files, pipes) is written as UTF-8."""
    for stream in (sys.stdout, sys.stderr):
        reconf = getattr(stream, "reconfigure", None)
        if reconf is None:
            continue
        try:
            if stream.isatty():
                reconf(errors="replace")
            else:
                reconf(encoding="utf-8", errors="replace")
        except (ValueError, OSError):
            pass


def _can_encode(text: str, stream: TextIO = None) -> bool:
    enc = getattr(stream or sys.stdout, "encoding", None) or "ascii"
    try:
        text.encode(enc)
        return True
    except (UnicodeEncodeError, LookupError):
        return False


def arrow() -> str:
    return "→" if _can_encode("→") else "->"


def bar_char() -> str:
    return "█" if _can_encode("█") else "#"


def use_color(stream: TextIO, force_off: bool = False) -> bool:
    return (not force_off) and stream.isatty() and os.environ.get("NO_COLOR") is None


def paint(text: str, verdict: str, color: bool) -> str:
    return f"\033[{_COLORS.get(verdict, '0')}m{text}\033[0m" if color else text


def _bw(r: FileResult) -> str:
    return "-" if not r.bandwidth_hz else f"{r.bandwidth_hz / 1000:.1f}k"


def header() -> str:
    return f"{'VERDICT':<14} {'CLAIMED':<10} {'REAL(EST)':<11} {'BANDWIDTH':<9}  FILE"


def row(r: FileResult, color: bool, base: Optional[str] = None, verbose: bool = False) -> str:
    name = os.path.relpath(r.path, base) if base else r.path
    line = (paint(f"{r.verdict:<14}", r.verdict, color)
            + f" {r.claimed:<10} {r.estimated:<11} {_bw(r):<9}  {name}")
    pad = f"\n{'':<15}"
    if r.summary and (verbose or r.is_problem or r.verdict == ERROR):
        line += f"{pad}{arrow()} {r.summary}"
    if verbose:
        for reason in r.reasons:
            line += f"{pad}- {reason}"
        for note in r.notes:
            line += f"{pad}· {note}"
        if r.bit_usage:
            top = ", ".join(f"{b}-bit LSB {v:.1%}" for b, v in list(r.bit_usage.items())[:3])
            line += f"{pad}· Lowest-set-bit usage: {top}"
        if r.cliffs_db:
            cl = ", ".join(f"{k / 2000:g}k:{v:.0f}dB" for k, v in r.cliffs_db.items() if np.isfinite(v))
            line += f"{pad}· Edge drop at candidate source Nyquists: {cl}"
    return line


def summary(results: List[FileResult], color: bool) -> str:
    """Totals with percentages, e.g. 'Scanned 20 file(s): 5 fake hi-res (25.0%), ...'."""
    parts = [paint(f"{n} {v.lower()} ({pct:.1f}%)", v, color)
             for v, n, pct in verdict_breakdown(results)]
    return f"Scanned {len(results)} file(s): " + ", ".join(parts)


def summary_bars(results: List[FileResult], color: bool, width: int = 30) -> str:
    """Multi-line percentage breakdown with bars, printed after a scan."""
    lines = []
    for v, n, pct in verdict_breakdown(results):
        bar = bar_char() * max(1 if n else 0, int(round(pct / 100 * width)))
        lines.append(f"  {paint(f'{v:<14}', v, color)} {pct:5.1f}%  {n:>5}  {paint(bar, v, color)}")
    return "\n".join(lines)


def write_json(results: Iterable[FileResult], path: str) -> None:
    with open(path, "w", encoding="utf-8") as f:
        json.dump([r.to_dict() for r in results], f, indent=2, ensure_ascii=False)


CSV_FIELDS = ["path", "verdict", "summary", "flags", "claimed", "estimated_real", "codec",
              "sample_rate", "claimed_bits", "effective_bits", "bandwidth_hz", "upsampled_from",
              "lossy_cutoff_hz", "cutoff_drop_db", "duration", "reasons"]


def write_csv(results: Iterable[FileResult], path: str) -> None:
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=CSV_FIELDS, extrasaction="ignore")
        w.writeheader()
        for r in results:
            d = r.to_dict()
            d["reasons"] = " | ".join(r.reasons)
            d["flags"] = " ".join(r.flags)
            w.writerow(d)


def _safe_name(path: str) -> str:
    stem = os.path.splitext(os.path.basename(path))[0]
    return re.sub(r"[^\w.-]+", "_", stem)[:120] or "file"


def write_svg(r: FileResult, out_dir: str) -> Optional[str]:
    """Spectrum plot (no external dependencies). Returns the written path."""
    s = r.spectrum
    if s is None or s.freqs is None:
        return None
    os.makedirs(out_dir, exist_ok=True)
    W, H, L, R, T, B = 960, 420, 64, 20, 46, 46
    pw, ph = W - L - R, H - T - B
    f, db = s.freqs, s.level_db
    keep = f > 0
    f, db = f[keep], db[keep]
    # Down-sample to ~1 point per pixel using the max (preserves the cliff edge).
    bins = np.linspace(0, s.nyquist, pw + 1)
    idx = np.clip(np.digitize(f, bins) - 1, 0, pw - 1)
    ys = np.full(pw, -300.0)
    np.maximum.at(ys, idx, db)
    top = np.ceil((s.peak_db + 5) / 10) * 10
    bottom = max(np.floor((min(s.noise_floor_db, s.threshold_db) - 15) / 10) * 10, top - 200)

    def X(freq):
        return L + pw * freq / s.nyquist

    def Y(level):
        return T + ph * (top - np.clip(level, bottom, top)) / (top - bottom)

    pts = " ".join(f"{L + i + 0.5:.1f},{Y(v):.1f}" for i, v in enumerate(ys) if v > -299)
    color = {REAL: "#1a7f37", FAKE: "#cf222e", FAKE_LOSSLESS: "#a40e26",
             SUSPECT: "#9a6700"}.get(r.verdict, "#0969da")
    out: List[str] = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" '
        'font-family="Helvetica,Arial,sans-serif" font-size="12">',
        f'<rect width="{W}" height="{H}" fill="#ffffff"/>',
        f'<text x="{L}" y="20" font-size="14" font-weight="bold">{html.escape(os.path.basename(r.path))}</text>',
        f'<text x="{L}" y="37" fill="{color}" font-weight="bold">{html.escape(r.verdict)} '
        f'<tspan fill="#555" font-weight="normal">claimed {html.escape(r.claimed)}, estimated real '
        f'{html.escape(r.estimated)}, bandwidth {_bw(r)}</tspan></text>',
    ]
    step = 10 if (top - bottom) <= 120 else 20
    lvl = top
    while lvl >= bottom:
        y = Y(lvl)
        out.append(f'<line x1="{L}" x2="{L + pw}" y1="{y:.1f}" y2="{y:.1f}" stroke="#eee"/>')
        out.append(f'<text x="{L - 6}" y="{y + 4:.1f}" text-anchor="end" fill="#666">{lvl:.0f}</text>')
        lvl -= step
    khz_step = 2 if s.nyquist <= 24000 else 5 if s.nyquist <= 50000 else 10 if s.nyquist <= 100000 else 20
    k = 0
    while k * 1000 <= s.nyquist:
        x = X(k * 1000)
        out.append(f'<line x1="{x:.1f}" x2="{x:.1f}" y1="{T}" y2="{T + ph}" stroke="#eee"/>')
        out.append(f'<text x="{x:.1f}" y="{T + ph + 16}" text-anchor="middle" fill="#666">{k}k</text>')
        k += khz_step
    out.append(f'<text x="{L + pw / 2}" y="{H - 6}" text-anchor="middle" fill="#333">Frequency (Hz)</text>')
    out.append(f'<text x="14" y="{T + ph / 2}" transform="rotate(-90 14 {T + ph / 2})" '
               'text-anchor="middle" fill="#333">Level (dB, averaged)</text>')
    # Nyquist limits of common lower source rates (where upsampling cliffs appear).
    for i, ref in enumerate((22050, 24000, 44100, 48000)):
        if ref < s.nyquist * 0.9:
            x = X(ref)
            out.append(f'<line x1="{x:.1f}" x2="{x:.1f}" y1="{T}" y2="{T + ph}" stroke="#999" '
                       'stroke-dasharray="2,3"/>')
            out.append(f'<text x="{x + 3:.1f}" y="{T + ph - 6 - 14 * (i % 2)}" fill="#999">'
                       f'{ref / 1000:g}k</text>')
    if r.bandwidth_hz:
        x = X(r.bandwidth_hz)
        out.append(f'<line x1="{x:.1f}" x2="{x:.1f}" y1="{T}" y2="{T + ph}" stroke="{color}" '
                   'stroke-dasharray="6,4" stroke-width="1.5"/>')
        anchor, dx = ("end", -4) if x > L + pw - 140 else ("start", 4)
        out.append(f'<text x="{x + dx:.1f}" y="{T + 16}" text-anchor="{anchor}" fill="{color}">'
                   f'bandwidth {_bw(r)}</text>')
    yf = Y(s.noise_floor_db)
    out.append(f'<line x1="{L}" x2="{L + pw}" y1="{yf:.1f}" y2="{yf:.1f}" stroke="#8250df" '
               'stroke-dasharray="4,4"/>')
    out.append(f'<text x="{L + pw - 4}" y="{yf - 4:.1f}" text-anchor="end" fill="#8250df">'
               f'top-band floor {s.noise_floor_db:.0f} dB</text>')
    out.append(f'<polyline fill="none" stroke="#1f2328" stroke-width="1.2" points="{pts}"/>')
    out.append(f'<rect x="{L}" y="{T}" width="{pw}" height="{ph}" fill="none" stroke="#999"/>')
    out.append("</svg>")
    # Short hash of the full path keeps same-named tracks in different albums apart
    # while re-runs overwrite the previous plot.
    digest = hashlib.sha1(r.path.encode("utf-8", "replace")).hexdigest()[:6]
    path = os.path.join(out_dir, f"{_safe_name(r.path)}_{digest}.svg")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("\n".join(out))
    return path


def eprint(*a, **k):
    print(*a, file=sys.stderr, **k)
