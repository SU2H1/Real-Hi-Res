""""Fix resolution": convert a fake file to the resolution it really has.

A fake 24/96 file that was upsampled from a 44.1 kHz CD master carries no
information above 22.05 kHz and (often) nothing below its 16th bit, so it can
be converted back to 16/44.1 without losing anything, at a third of the size,
and honestly labelled.  Tags and cover art are kept.

The plan for each file is derived from the analysis result:

* sample rate: the detected source rate (``upsampled_from``); for files made
  from a lossy source the nearest CD/DVD rate (44.1 or 48 kHz family);
* bit depth: the bits really used (``effective_bits``), rounded to 16/24,
  optionally forced to 16 for CD-sourced files (see settings ``fix_bits``).

Resampling uses the SoX resampler when the FFmpeg build has it and adds
triangular dither only when a resampled signal is reduced to 16 bits; a pure
24->16 conversion of padded audio is exact and gets no dither.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass, field
from typing import List, Optional

from .probe import _no_window, tool
from .verdict import FAKE, FAKE_LOSSLESS, FileResult, analyze_file, fmt_rate

FIXABLE_VERDICTS = (FAKE, FAKE_LOSSLESS)

# input extension -> (output extension, codec for 16 bit, codec for 24 bit, sample_fmt 16, sample_fmt 24)
_SAME_FORMAT = {
    ".flac": (".flac", "flac", "flac", "s16", "s32"),
    ".wav": (".wav", "pcm_s16le", "pcm_s24le", None, None),
    ".wave": (".wav", "pcm_s16le", "pcm_s24le", None, None),
    ".w64": (".w64", "pcm_s16le", "pcm_s24le", None, None),
    ".aif": (".aif", "pcm_s16be", "pcm_s24be", None, None),
    ".aiff": (".aiff", "pcm_s16be", "pcm_s24be", None, None),
    ".aifc": (".aiff", "pcm_s16be", "pcm_s24be", None, None),
    ".m4a": (".m4a", "alac", "alac", "s16p", "s32p"),
    ".alac": (".m4a", "alac", "alac", "s16p", "s32p"),
    ".caf": (".caf", "alac", "alac", "s16p", "s32p"),
    ".wv": (".wv", "wavpack", "wavpack", "s16p", "s32p"),
}
_FLAC = _SAME_FORMAT[".flac"]
_KEEPS_COVER = {".flac", ".m4a"}


@dataclass
class FixPlan:
    source: str
    target_rate: int
    target_bits: int
    resample: bool
    output: str
    codec: str
    sample_fmt: Optional[str]
    replace_original: bool
    description: str              # "24/96 → 16/44.1"
    notes: List[str] = field(default_factory=list)


@dataclass
class FixOutcome:
    plan: FixPlan
    ok: bool
    message: str
    output: Optional[str] = None
    new_result: Optional[FileResult] = None
    bytes_before: int = 0
    bytes_after: int = 0


def _family_rate(rate: int) -> int:
    return 44100 if rate % 11025 == 0 else 48000


def target_resolution(r: FileResult, cfg) -> Optional[tuple]:
    """(rate, bits, notes) the file really has, or None when unknown."""
    if not r.sample_rate or r.effective_bits is None:
        return None
    notes: List[str] = []
    rate = r.upsampled_from or r.sample_rate
    cd_sourced = bool(r.upsampled_from and r.upsampled_from <= 48000)
    if r.verdict == FAKE_LOSSLESS:
        cd_sourced = True
        if rate > 48000:
            rate = _family_rate(rate)
        notes.append("Made from a lossy file: converting removes the fake hi-res part, but the "
                     "result is still lossy-sourced.")
    bits = r.effective_bits
    if r.is_float and bits > 24:
        bits = 24
    bits = 16 if bits <= 16 else 24
    mode = getattr(cfg, "fix_bits", "16-bit for CD-sourced files")
    if mode == "Always 16-bit" or (mode == "16-bit for CD-sourced files" and cd_sourced):
        if bits > 16:
            notes.append("Reduced to 16-bit with dither (the source was a CD-resolution or lossy file).")
        bits = 16
    return rate, bits, notes


def _fmt(bits: int, rate: int) -> str:
    return f"{bits}/{fmt_rate(rate)}"


def _unique(path: str) -> str:
    if not os.path.exists(path):
        return path
    stem, ext = os.path.splitext(path)
    i = 2
    while os.path.exists(f"{stem} ({i}){ext}"):
        i += 1
    return f"{stem} ({i}){ext}"


def plan_fix(r: FileResult, cfg) -> tuple:
    """Return (FixPlan, None) or (None, reason it cannot / need not be fixed)."""
    if r.verdict not in FIXABLE_VERDICTS:
        return None, "Only fake hi-res / fake lossless files need fixing."
    tgt = target_resolution(r, cfg)
    if tgt is None:
        return None, "The real resolution could not be determined."
    rate, bits, notes = tgt
    cur_bits = r.claimed_bits or 16
    if r.is_float:
        cur_bits = 32
    if rate == r.sample_rate and bits >= cur_bits:
        return None, (f"Already at its real resolution ({_fmt(bits, rate)}); "
                      "a lossy source cannot be repaired." if r.verdict == FAKE_LOSSLESS
                      else f"Already at its real resolution ({_fmt(bits, rate)}).")
    ext = os.path.splitext(r.path)[1].lower()
    fmt = _FLAC if getattr(cfg, "fix_format", "") == "FLAC" else _SAME_FORMAT.get(ext, _FLAC)
    out_ext, c16, c24, f16, f24 = fmt
    codec, sfmt = (c16, f16) if bits == 16 else (c24, f24)
    if fmt is _FLAC and ext not in (".flac",) and getattr(cfg, "fix_format", "") != "FLAC":
        notes.append(f"{ext or 'This format'} cannot be written by FFmpeg; saving as FLAC.")

    mode = getattr(cfg, "fix_output", "Next to the original")
    folder, name = os.path.split(r.path)
    stem = os.path.splitext(name)[0]
    replace = mode.startswith("Replace")
    if replace:
        output = os.path.join(folder, stem + out_ext)
    else:
        suffix = getattr(cfg, "fix_suffix", " [{bits}-{rate}]") or ""
        suffix = suffix.replace("{bits}", str(bits)).replace("{rate}", fmt_rate(rate))
        out_dir = folder
        if mode == "Into a folder":
            out_dir = getattr(cfg, "fix_folder", "") or os.path.join(folder, "Fixed")
        output = _unique(os.path.join(out_dir, stem + suffix + out_ext))
    plan = FixPlan(source=r.path, target_rate=rate, target_bits=bits, resample=rate != r.sample_rate,
                   output=output, codec=codec, sample_fmt=sfmt, replace_original=replace,
                   description=f"{r.claimed} → {_fmt(bits, rate)}", notes=notes)
    return plan, None


def _has_soxr() -> bool:
    try:
        out = subprocess.run([tool("ffmpeg"), "-hide_banner", "-buildconf"], capture_output=True,
                             text=True, **_no_window()).stdout
    except OSError:
        return False
    return "--enable-libsoxr" in out


_SOXR: Optional[bool] = None


def ffmpeg_command(plan: FixPlan, output: str) -> List[str]:
    global _SOXR
    if _SOXR is None:
        _SOXR = _has_soxr()
    out_ext = os.path.splitext(output)[1].lower()
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-y", "-i", plan.source, "-map", "0:a:0"]
    if out_ext in _KEEPS_COVER:
        cmd += ["-map", "0:v?", "-c:v", "copy", "-disposition:v", "attached_pic"]
    cmd += ["-map_metadata", "0"]
    af = []
    if plan.resample:
        af.append(f"aresample={plan.target_rate}" + (":resampler=soxr:precision=28" if _SOXR else
                                                    ":filter_size=256:cutoff=0.97:phase_shift=10"))
        if plan.target_bits == 16:
            af[-1] += ":dither_method=triangular"
    if af:
        cmd += ["-af", ",".join(af)]
    cmd += ["-c:a", plan.codec]
    if plan.sample_fmt:
        cmd += ["-sample_fmt", plan.sample_fmt]
    if plan.target_bits == 24 and plan.codec in ("flac", "alac", "wavpack"):
        cmd += ["-bits_per_raw_sample", "24"]
    if out_ext == ".m4a":
        cmd += ["-movflags", "+faststart"]
    cmd.append(output)
    return cmd


def run_fix(plan: FixPlan, cfg=None) -> FixOutcome:
    """Convert one file.  Writes to a temporary file first, so a failed or
    interrupted conversion never leaves a broken file behind."""
    out_dir = os.path.dirname(plan.output) or "."
    try:
        os.makedirs(out_dir, exist_ok=True)
        before = os.path.getsize(plan.source)
        fd, tmp = tempfile.mkstemp(prefix=".realhires-", suffix=os.path.splitext(plan.output)[1],
                                   dir=out_dir)
        os.close(fd)
    except OSError as exc:
        return FixOutcome(plan, False, f"Cannot write to {out_dir}: {exc}")
    try:
        proc = subprocess.run(ffmpeg_command(plan, tmp), capture_output=True, text=True,
                              encoding="utf-8", errors="replace", **_no_window())
        if proc.returncode != 0 or os.path.getsize(tmp) == 0:
            msg = (proc.stderr or "").strip().splitlines()
            return FixOutcome(plan, False, "FFmpeg failed: " + (msg[-1] if msg else f"exit {proc.returncode}"))
        output = plan.output
        if plan.replace_original:
            backup = _unique(plan.source + ".bak")
            os.replace(plan.source, backup)
            if os.path.exists(output):  # different extension case: never overwrite other files
                output = _unique(output)
            os.replace(tmp, output)
            note = f"original kept as {os.path.basename(backup)}"
        else:
            output = _unique(output) if os.path.exists(output) else output
            os.replace(tmp, output)
            note = "original left unchanged"
        tmp = None
        after = os.path.getsize(output)
        new = None
        if getattr(cfg, "fix_verify", True):
            new = analyze_file(output, settings=cfg)
        msg = f"Saved {os.path.basename(output)} ({plan.description}, {after / before:.0%} of the size; {note})."
        if new is not None:
            msg += f" Re-check: {new.verdict} – {new.summary}"
        return FixOutcome(plan, True, msg, output, new, before, after)
    except OSError as exc:
        return FixOutcome(plan, False, f"Could not save the fixed file: {exc}")
    finally:
        if tmp and os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass


def ffmpeg_available() -> bool:
    return shutil.which(tool("ffmpeg")) is not None or os.path.isfile(tool("ffmpeg"))
