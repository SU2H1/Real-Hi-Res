""""Fix": correct a fake file so its properties show the resolution it really has.

The sample rate and bit depth a player or the file manager shows (the file's
"properties") come from the audio stream header.  Simply editing those numbers
would make the file play at the wrong speed or as noise, so the file is
rewritten at its real resolution instead.  A fake 24/96 file upsampled from a
44.1 kHz CD master carries no information above 22.05 kHz and (often) nothing
below its 16th bit, so it becomes a genuine 16/44.1 file without losing
anything, at about a third of the size.

By default the file is replaced in place: same name, same folder, tags and
cover art kept; the original is moved to a backup folder.  Writing a corrected
copy instead is also possible.

The plan for each file is derived from the analysis result:

* sample rate: the detected source rate (``upsampled_from``); for files made
  from a lossy source the nearest CD/DVD rate (44.1 or 48 kHz family);
* bit depth: the bits really used (``effective_bits``), rounded to 16/24,
  optionally forced to 16 for CD-sourced files (see settings ``fix_bits``).

Resampling uses the SoX resampler when the FFmpeg build has it and adds
triangular dither when a resampled signal is reduced to 16 bits, or when a
16-bit master was given a gain change (its samples sit on a grid that is not
the 16-bit one).  A 24->16 conversion without resampling goes through 32-bit
float, because FFmpeg's direct 32->16-bit integer conversion truncates while
float->16-bit rounds: padded and only-dithered audio then come back as the
exact 16-bit values of the master, with no dither.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from fractions import Fraction
from typing import Callable, Dict, List, Optional, Tuple

import numpy as np

from .analysis import detection_params
from .probe import _no_window, iter_pcm, tool
from .verdict import ERROR, FAKE, FAKE_LOSSLESS, LOSSY, FileResult, analyze_file, fmt_rate

FIXABLE_VERDICTS = (FAKE, FAKE_LOSSLESS)
BACKUP_DIR_NAME = "Real-Hi-Res backups"

# Gapless joins: audio of the neighbouring track fed to the resampler on each side
# (soxr and swr need about 1,000 input samples; 8,192 leaves a wide margin), and
# the peak below which a join counts as silent, so that no neighbour is needed.
CONTEXT_FRAMES = 8192
SILENT_JOIN = int(2 ** 31 * 10 ** (-70 / 20))   # -70 dBFS on the int32 scale
EXACT_MIN_SAMPLES = 20000   # non-zero samples per phase needed to prove an untouched master

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
    backup: str = "folder"        # replace mode: "folder" | "bak" | "none"
    backup_dir: Optional[str] = None
    notes: List[str] = field(default_factory=list)
    dither: bool = False          # dither a 24->16 reduction that is not resampled
    channels: int = 2
    source_rate: int = 0
    exact_ratio: int = 0          # >1: try restoring the master by keeping every Nth sample
    frames: Optional[int] = None  # exact length of the source (gapless joins)
    offset: int = 0               # start of the source on its gapless run, in source samples
    context_before: Optional[np.ndarray] = None  # end of the previous track (int32 frames)
    context_after: Optional[np.ndarray] = None   # start of the next track


@dataclass
class FixOutcome:
    plan: FixPlan
    ok: bool
    message: str
    output: Optional[str] = None
    new_result: Optional[FileResult] = None
    bytes_before: int = 0
    bytes_after: int = 0
    how: str = ""                 # "exact", "gapless" or "" (plain conversion)


def _family_rate(rate: int) -> int:
    return 44100 if rate % 11025 == 0 else 48000


def _cut_is_safe(r: FileResult, rate: int, c) -> bool:
    """True when the file holds nothing real above rate/2: its spectrum has
    collapsed there (narrow or deep cliff test passed at that very rate) and the
    music had already dropped 6 dB before that frequency."""
    narrow = (r.cliffs_db or {}).get(rate, float("-inf"))
    deep = (r.deep_cliffs_db or {}).get(rate, float("-inf"))
    if not (narrow >= c.cliff or deep >= c.deep_cliff):
        return False
    mid = r.upsample_mid_hz
    return not (mid is not None and mid == mid and mid > 1.02 * rate / 2.0)


def safe_source_rate(r: FileResult, cfg=None) -> int:
    """The rate Fix may convert an upsampled file to without cutting real content.

    Normally that is the detected source rate.  When the evidence at that rate
    is weak (it was chosen as the weaker neighbour of a stronger candidate, or
    by the brick-wall fallback), the next higher candidate that is safe is used,
    or the file keeps its own rate.  Erring upwards never loses anything."""
    c = detection_params(cfg)
    rate = r.upsampled_from
    if not rate or _cut_is_safe(r, rate, c):
        return rate or r.sample_rate
    for cand in sorted(set(r.cliffs_db or {}) | set(r.deep_cliffs_db or {})):
        if rate < cand < r.sample_rate and _cut_is_safe(r, cand, c):
            return cand
    return r.sample_rate


def target_resolution(r: FileResult, cfg) -> Optional[tuple]:
    """(rate, bits, notes) the file really has, or None when unknown."""
    if not r.sample_rate or r.effective_bits is None:
        return None
    notes: List[str] = []
    rate = r.upsampled_from or r.sample_rate
    cd_sourced = bool(r.upsampled_from and r.upsampled_from <= 48000)
    if r.upsampled_from and r.verdict == FAKE:
        safe = safe_source_rate(r, cfg)
        if safe != rate:
            notes.append(f"Kept {fmt_rate(safe)} kHz instead of the estimated {fmt_rate(rate)} kHz: the "
                         f"spectrum does not collapse clearly enough at {rate / 2000:g} kHz to cut there "
                         "without losing content.")
            rate = safe
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
        if r.verdict == FAKE_LOSSLESS:
            return None, (f"Already {_fmt(bits, rate)}, the resolution it really has: there is nothing to "
                          "convert, and the damage of the lossy source cannot be repaired.")
        kept = [n for n in notes if n.startswith("Kept")]
        if kept:
            return None, f"{kept[0]} All its {bits} bits are used, so nothing would change."
        return None, f"Already at its real resolution ({_fmt(bits, rate)})."
    ext = os.path.splitext(r.path)[1].lower()
    fmt = _FLAC if getattr(cfg, "fix_format", "") == "FLAC" else _SAME_FORMAT.get(ext, _FLAC)
    out_ext, c16, c24, f16, f24 = fmt
    codec, sfmt = (c16, f16) if bits == 16 else (c24, f24)
    if fmt is _FLAC and ext not in (".flac",) and getattr(cfg, "fix_format", "") != "FLAC":
        notes.append(f"{ext or 'This format'} cannot be written by FFmpeg; saving as FLAC.")

    mode = getattr(cfg, "fix_output", "Replace the file")
    folder, name = os.path.split(r.path)
    stem = os.path.splitext(name)[0]
    replace = mode.startswith("Replace")
    backup, backup_dir = "folder", None
    if replace:
        output = os.path.join(folder, stem + out_ext)  # same name (extension only changes for e.g. APE)
        b = getattr(cfg, "fix_backup", "Move the original to a backup folder")
        backup = "bak" if b.startswith("Keep") else "none" if b.startswith("Delete") else "folder"
        if backup == "folder":
            backup_dir = getattr(cfg, "fix_backup_folder", "") or os.path.join(folder, BACKUP_DIR_NAME)
    else:
        suffix = getattr(cfg, "fix_suffix", " [{bits}-{rate}]") or ""
        suffix = suffix.replace("{bits}", str(bits)).replace("{rate}", fmt_rate(rate))
        out_dir = folder
        if mode.endswith("into a folder"):
            out_dir = getattr(cfg, "fix_folder", "") or os.path.join(folder, "Fixed")
        output = _unique(os.path.join(out_dir, stem + suffix + out_ext))
    # A 16-bit master scaled by a gain sits on a grid that is not the 16-bit one, so
    # rounding it back to 16 bits needs dither; an unscaled one rounds back exactly.
    dither = bool(bits == 16 and rate == r.sample_rate and getattr(r, "grid_step", None)
                  and abs(getattr(r, "grid_gain_db", 0.0) or 0.0) >= 0.05)
    if dither:
        notes.append(f"Reduced to 16-bit with dither (the 16-bit master had been scaled by "
                     f"{r.grid_gain_db:+.1f} dB).")
    # An upsample by a whole-number ratio (FFmpeg's default resampler, for one) can
    # keep every Nth sample of a 16-bit master untouched; run_fix then restores the
    # master bit-exactly instead of resampling, if the samples prove it.
    exact = (r.sample_rate // rate if rate != r.sample_rate and bits == 16 and rate
             and r.sample_rate % rate == 0 else 0)
    plan = FixPlan(source=r.path, target_rate=rate, target_bits=bits, resample=rate != r.sample_rate,
                   output=output, codec=codec, sample_fmt=sfmt, replace_original=replace,
                   description=f"{r.claimed} → {_fmt(bits, rate)}", notes=notes,
                   backup=backup, backup_dir=backup_dir, dither=dither,
                   channels=r.channels or 2, source_rate=r.sample_rate,
                   exact_ratio=exact if exact >= 2 else 0)
    return plan, None


SKIP_GROUPS = {
    "lossy": "made from MP3/AAC but already at CD resolution: nothing to convert, and the lossy "
             "damage cannot be repaired",
    "uncertain": "upsampled, but the source rate is too uncertain to cut without risking real content, "
                 "and all bits are used",
    "already": "already at the resolution they really have",
    "unknown": "real resolution could not be determined",
    "not-fake": "not fake",
}


def skip_group(r: FileResult, why: str) -> str:
    """Key into SKIP_GROUPS for a file plan_fix() did not plan."""
    if r.verdict not in FIXABLE_VERDICTS:
        return "not-fake"
    if "could not be determined" in why:
        return "unknown"
    if r.verdict == FAKE_LOSSLESS:
        return "lossy"
    return "uncertain" if why.startswith("Kept") else "already"


def skip_summary(skipped) -> str:
    """'268 are not changed: 251 made from MP3/AAC but ..., 17 ...' for
    [(result, reason)] from plan_fix(); '' when nothing was skipped."""
    if not skipped:
        return ""
    counts: dict = {}
    for r, why in skipped:
        k = skip_group(r, why)
        counts[k] = counts.get(k, 0) + 1
    parts = [f"{n} {SKIP_GROUPS[k]}" for k, n in sorted(counts.items(), key=lambda kv: -kv[1])]
    return f"{len(skipped)} file(s) are not changed: " + "; ".join(parts) + "."


def _prospect(r: FileResult, cfg=None) -> Optional[Tuple[str, str, str]]:
    """What Fix does with a fake file, without touching the disk: ("unknown" |
    "same" | "convert", "bits/rate" target, the "Kept ..." note or ""), the
    same decision plan_fix() makes; None for other verdicts."""
    if r.verdict not in FIXABLE_VERDICTS:
        return None
    tgt = target_resolution(r, cfg)
    if tgt is None:
        return "unknown", "", ""
    rate, bits, notes = tgt
    cur_bits = 32 if r.is_float else (r.claimed_bits or 16)
    kept = next((n for n in notes if n.startswith("Kept")), "")
    same = rate == r.sample_rate and bits >= cur_bits
    return ("same" if same else "convert"), _fmt(bits, rate), kept


def fix_outlook(r: FileResult, cfg=None) -> str:
    """A few words for the "After fix" column of the GUI list: what Fix turns
    the file into and whether it is still marked fake then; "" if not fake."""
    p = _prospect(r, cfg)
    if p is None:
        return ""
    kind, target, kept = p
    if kind == "unknown":
        return "Not fixable: resolution unknown"
    if r.verdict == FAKE_LOSSLESS:
        return ("Stays fake: MP3/AAC source" if kind == "same"
                else f"{target}, stays fake: MP3/AAC source")
    if kept:
        return "Stays fake: rate kept" if kind == "same" else f"{target}, stays fake: rate kept"
    return f"Already {target}" if kind == "same" else f"Fixable to {target}"


def _keeps(note: str) -> str:
    """'Kept 96 kHz instead of ...: the spectrum ...' -> 'keeps 96 kHz instead of ..., because the spectrum ...'."""
    return "keeps " + note[len("Kept "):].replace(": the spectrum", ", because the spectrum", 1).rstrip(".")


def fix_advice(r: FileResult, cfg=None) -> Optional[str]:
    """What "Fix to real resolution" does with a fake file, and whether it is
    still marked fake afterwards (shown in the details, so a file that stays
    fake after fixing and re-scanning says why); None for other verdicts."""
    p = _prospect(r, cfg)
    if p is None:
        return None
    kind, target, kept = p
    lossy = ("because it was made from an MP3/AAC, and no conversion can bring back what the lossy "
             "encoder removed. Only a genuine lossless copy is better.")
    if kind == "unknown":
        return "Fix to real resolution leaves it unchanged, because its real resolution could not be determined."
    if kind == "same":
        if r.verdict == FAKE_LOSSLESS:
            return (f"Fix to real resolution leaves it unchanged: it is already {target}, the resolution "
                    f"it really has. It stays marked fake lossless, also after fixing and re-scanning, {lossy}")
        if kept:
            bits = target.split("/")[0]
            return (f"Fix to real resolution leaves it unchanged: it {_keeps(kept)}, and all its {bits} bits "
                    "are used, so nothing would change. It stays marked fake hi-res, also after fixing and "
                    "re-scanning.")
        return f"Fix to real resolution leaves it unchanged: it is already {target}."
    if r.verdict == FAKE_LOSSLESS:
        return (f"Fix to real resolution converts it to {target}, the resolution it really has, but it stays "
                f"marked fake lossless even then, {lossy}")
    if kept:
        return (f"Fix to real resolution converts it to {target}, but it {_keeps(kept)}. It therefore "
                "stays marked fake hi-res after fixing and re-scanning.")
    return (f"Fix to real resolution converts it to {target}, the resolution it really has; a re-scan then "
            "no longer marks it fake.")


def fmt_bytes(n: float) -> str:
    """'1.2 GB', '350.4 MB', '812 bytes'."""
    for unit in ("bytes", "KB", "MB", "GB"):
        if abs(n) < 1024:
            return f"{n:.0f} {unit}" if unit == "bytes" else f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def _files(n: int) -> str:
    return f"{n} file{'s' if n != 1 else ''}"


def done_summary(outcomes: List[FixOutcome], skipped: int = 0,
                 label: Callable[[str], str] = str) -> Tuple[str, List[str]]:
    """(headline, sentences) for the pop-up shown when fixing has finished.
    `skipped` counts the fake files that were left alone on purpose, and
    `label` turns a verdict into the name the caller shows (the GUI's
    "CD quality" for NOT HI-RES)."""
    ok = [o for o in outcomes if o.ok]
    bad = [o for o in outcomes if not o.ok]
    copies = bool(ok) and not any(o.plan.replace_original for o in ok)
    if not ok:
        head = "No file could be fixed" if bad else "Nothing was fixed"
    elif copies:
        head = (f"Saved corrected copies of {_files(len(ok))}" if not bad
                else f"Saved corrected copies of {len(ok)} of {_files(len(outcomes))}")
    else:
        head = (f"Fixed {_files(len(ok))} to their real resolution" if not bad
                else f"Fixed {len(ok)} of {_files(len(outcomes))} to their real resolution")
    lines: List[str] = []
    if ok:
        before = sum(o.bytes_before for o in ok)
        after = sum(o.bytes_after for o in ok)
        they = "The copies take" if copies else "They now take"
        if after < before:
            lines.append(f"{they} {fmt_bytes(after)} instead of {fmt_bytes(before)}, "
                         f"{fmt_bytes(before - after)} less.")
        else:
            lines.append(f"{they} {fmt_bytes(after)} (the originals took {fmt_bytes(before)}).")
        exact = sum(1 for o in ok if o.how == "exact")
        gapless = sum(1 for o in ok if o.how == "gapless")
        if exact:
            lines.append(f"{_files(exact)} {'were' if exact != 1 else 'was'} restored bit-exactly from "
                         "the untouched 16-bit master, without resampling.")
        if gapless:
            lines.append(f"{_files(gapless)} {'were' if gapless != 1 else 'was'} converted together with "
                         "the neighbouring tracks, so the album still plays without gaps.")
        if copies:
            dirs = {os.path.dirname(os.path.abspath(o.output or o.plan.output)) for o in ok}
            srcs = {os.path.dirname(os.path.abspath(o.plan.source)) for o in ok}
            where = f"in {dirs.pop()}" if len(dirs) == 1 and dirs != srcs else (
                "next to the originals" if dirs == srcs else "in the folder you chose")
            lines.append(f"The originals were left unchanged; the corrected copies are {where}.")
        else:
            kinds = {o.plan.backup for o in ok if o.plan.replace_original}
            parts = []
            if "folder" in kinds:
                bdirs = {os.path.abspath(o.plan.backup_dir or "") for o in ok if o.plan.backup == "folder"}
                named = {os.path.basename(d) for d in bdirs}
                if len(bdirs) == 1:
                    parts.append(f"moved to {bdirs.pop()}")
                elif named == {BACKUP_DIR_NAME}:
                    parts.append(f'moved to the "{BACKUP_DIR_NAME}" folder next to each file')
                else:
                    parts.append("moved to the backup folders")
            if "bak" in kinds:
                parts.append("kept next to each file as .bak files")
            if "none" in kinds:
                parts.append("deleted, as chosen")
            if parts:
                lines.append("The originals were " + ", or ".join(parts) + ".")
        checked = [o.new_result for o in ok if o.new_result is not None]
        if checked:
            counts: Dict[str, int] = {}
            for r in checked:
                counts[r.verdict] = counts.get(r.verdict, 0) + 1
            parts = [f"{n} {label(v)}" for v, n in sorted(counts.items(), key=lambda kv: -kv[1])]
            line = "Re-check of the fixed files: " + ", ".join(parts) + "."
            if FAKE_LOSSLESS in counts:
                line += (" A file made from MP3/AAC stays fake lossless at any resolution, because "
                         "fixing cannot bring back what the lossy encoder removed.")
            if FAKE in counts:
                line += (" A file whose sample rate had to be kept, to avoid cutting real content, stays "
                         "marked fake hi-res.")
            if FAKE_LOSSLESS in counts or FAKE in counts:
                line += " They keep that mark in later scans, and their details say why."
            lines.append(line)
    if bad:
        first = bad[0]
        lines.append(f"{_files(len(bad))} could not be fixed and {'were' if len(bad) != 1 else 'was'} "
                     f"left as {'they were' if len(bad) != 1 else 'it was'}. "
                     f"{os.path.basename(first.plan.source)}: {first.message}")
    if skipped:
        lines.append(f"{_files(skipped)} {'were' if skipped != 1 else 'was'} left alone on purpose; "
                     "the Fix window lists the reasons.")
    return head, lines


def plan_fixes(results, cfg):
    """(plans, [(result, reason)]) for the fake files among `results`; used
    after a scan, when Fix runs on its own (Settings → Fixing → After a scan)."""
    plans, skipped = [], []
    for r in results:
        if r.verdict not in FIXABLE_VERDICTS:
            continue
        plan, why = plan_fix(r, cfg)
        if plan:
            plans.append(plan)
        else:
            skipped.append((r, why))
    return plans, skipped


def _has_soxr() -> bool:
    try:
        out = subprocess.run([tool("ffmpeg"), "-hide_banner", "-buildconf"], capture_output=True,
                             text=True, **_no_window()).stdout
    except OSError:
        return False
    return "--enable-libsoxr" in out


_SOXR: Optional[bool] = None


def _resample_filter(plan: FixPlan) -> str:
    global _SOXR
    if _SOXR is None:
        _SOXR = _has_soxr()
    af = f"aresample={plan.target_rate}" + (":resampler=soxr:precision=28" if _SOXR else
                                           ":filter_size=256:cutoff=0.97:phase_shift=10")
    if plan.target_bits == 16:
        af += ":dither_method=triangular"
    return af


def _output_args(plan: FixPlan, output: str, cover_input: int) -> List[str]:
    """Cover art, tags, codec and container options for the converted file."""
    out_ext = os.path.splitext(output)[1].lower()
    args: List[str] = []
    if out_ext in _KEEPS_COVER:
        args += ["-map", f"{cover_input}:v?", "-c:v", "copy", "-disposition:v", "attached_pic"]
    args += ["-map_metadata", str(cover_input)]
    return args


def _codec_args(plan: FixPlan, output: str) -> List[str]:
    args = ["-c:a", plan.codec]
    if plan.sample_fmt:
        args += ["-sample_fmt", plan.sample_fmt]
    if plan.target_bits == 24 and plan.codec in ("flac", "alac", "wavpack"):
        args += ["-bits_per_raw_sample", "24"]
    if os.path.splitext(output)[1].lower() == ".m4a":
        args += ["-movflags", "+faststart"]
    return args


def ffmpeg_command(plan: FixPlan, output: str) -> List[str]:
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-y", "-i", plan.source, "-map", "0:a:0"]
    cmd += _output_args(plan, output, 0)
    af = []
    if plan.resample:
        af.append(_resample_filter(plan))
    elif plan.target_bits == 16:
        af.append("aformat=sample_fmts=flt")  # float -> s16 rounds; s32 -> s16 would truncate
        if plan.dither:
            af.append("aresample=dither_method=triangular")
    if af:
        cmd += ["-af", ",".join(af)]
    cmd += _codec_args(plan, output)
    cmd.append(output)
    return cmd


def _pipe_command(plan: FixPlan, output: str, in_fmt: str, in_rate: int, af: Optional[str]) -> List[str]:
    """Encode raw PCM from stdin, taking tags and cover art from the original file."""
    cmd = [tool("ffmpeg"), "-v", "error", "-y", "-f", in_fmt, "-ar", str(in_rate), "-ac", str(plan.channels),
           "-i", "pipe:0", "-i", plan.source, "-map", "0:a"]
    cmd += _output_args(plan, output, 1)
    if af:
        cmd += ["-af", af]
    cmd += _codec_args(plan, output)
    cmd.append(output)
    return cmd


class _Encoder:
    """An FFmpeg process fed with raw PCM blocks."""

    def __init__(self, cmd: List[str]):
        self.err = tempfile.TemporaryFile()
        self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=self.err,
                                     **_no_window())

    def write(self, data: bytes) -> bool:
        try:
            self.proc.stdin.write(data)
            return True
        except (BrokenPipeError, OSError):
            return False

    def abort(self) -> None:
        try:
            self.proc.kill()
        finally:
            self.proc.wait()
            self.err.close()

    def finish(self) -> Tuple[bool, str]:
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        rc = self.proc.wait()
        self.err.seek(0)
        msg = self.err.read().decode("utf-8", "replace").strip().splitlines()
        self.err.close()
        return rc == 0, (msg[-1] if msg else f"exit {rc}")


def _exact_restore(plan: FixPlan, output: str) -> bool:
    """Restore a 16-bit master from a whole-number-ratio upsample by keeping every
    Nth sample, when one sample phase holds nothing but exact 16-bit values (as
    FFmpeg's default resampler leaves it) and the others do not.  Returns False,
    having written nothing usable, when the file does not prove that."""
    n = plan.exact_ratio
    blocks = iter_pcm(plan.source, plan.channels)
    head, counts = [], np.zeros(n, dtype=np.int64)
    pos = 0
    for b in blocks:  # read until every phase has enough signal to judge
        head.append(b)
        for ph in range(n):
            counts[ph] += np.count_nonzero(b[(ph - pos) % n::n])
        pos += b.shape[0]
        if counts.min() >= EXACT_MIN_SAMPLES:
            break
    if not head or counts.min() < EXACT_MIN_SAMPLES:
        blocks.close()
        return False
    x = np.concatenate(head)
    on_grid = []
    for ph in range(n):
        s = x[ph::n]
        nz = s[s != 0]
        on_grid.append(float(np.mean((nz & 0xFFFF) == 0)))
    keep = [ph for ph in range(n) if on_grid[ph] == 1.0]
    if len(keep) != 1 or max(g for ph, g in enumerate(on_grid) if ph != keep[0]) > 0.05:
        blocks.close()
        return False
    phase = keep[0]
    enc = _Encoder(_pipe_command(plan, output, "s16le", plan.target_rate, None))

    def emit(block: np.ndarray, start_pos: int) -> bool:
        sel = block[(phase - start_pos) % n::n]
        if np.any(sel & 0xFFFF):
            return False  # a sample that is not the untouched master: give up
        return enc.write(np.ascontiguousarray(sel >> 16).astype("<i2").tobytes())

    pos = 0
    ok = True
    for b in head:
        ok = ok and emit(b, pos)
        pos += b.shape[0]
    if ok:
        for b in blocks:
            if not emit(b, pos):
                ok = False
                break
            pos += b.shape[0]
    if not ok:
        blocks.close()
        enc.abort()
        return False
    done, _ = enc.finish()
    return done and os.path.getsize(output) > 0


def _ceil_div(a: int, b: int) -> int:
    return -((-a) // b)


def _resample_with_context(plan: FixPlan, output: str) -> bool:
    """Resample with the end of the previous and the start of the next track around
    the file, on the sample grid of the whole gapless run, and keep only this file's
    part: the result joins its neighbours exactly as if the album had been
    converted in one piece, so gapless playback gets no click at the joins."""
    if plan.frames is None:
        return False
    ch = plan.channels
    r = Fraction(plan.target_rate, plan.source_rate)
    p, q = r.numerator, r.denominator
    before = plan.context_before if plan.context_before is not None else np.zeros((0, ch), np.int32)
    after = plan.context_after if plan.context_after is not None else np.zeros((0, ch), np.int32)
    t = before.shape[0]
    cb = t - ((t - plan.offset) % q) if t else 0
    if (plan.offset - cb) % q:
        return False  # only a run's first track may start without context, at offset 0
    a = plan.offset - cb
    first = _ceil_div(plan.offset * p, q) - a * p // q
    last = _ceil_div((plan.offset + plan.frames) * p, q) - a * p // q
    af = _resample_filter(plan) + f",atrim=start_sample={first}:end_sample={last}"
    enc = _Encoder(_pipe_command(plan, output, "s32le", plan.source_rate, af))
    ok = enc.write(np.ascontiguousarray(before[t - cb:], "<i4").tobytes()) if cb else True
    n = 0
    for b in iter_pcm(plan.source, ch):
        if ok:
            ok = enc.write(np.ascontiguousarray(b, "<i4").tobytes())
        n += b.shape[0]
    if not ok or n != plan.frames:
        enc.abort()
        return False
    if after.shape[0] and not enc.write(np.ascontiguousarray(after, "<i4").tobytes()):
        enc.abort()
        return False
    done, _ = enc.finish()
    return done and os.path.getsize(output) > 0


def _convert(plan: FixPlan, output: str) -> Tuple[bool, str, str]:
    """(ok, error, how): how is "exact", "gapless" or "" for a plain conversion."""
    if plan.exact_ratio:
        try:
            if _exact_restore(plan, output):
                return True, "", "exact"
        except (OSError, RuntimeError, ValueError):
            pass
    if plan.context_before is not None or plan.context_after is not None:
        try:
            if _resample_with_context(plan, output):
                return True, "", "gapless"
        except (OSError, RuntimeError, ValueError):
            pass
    proc = subprocess.run(ffmpeg_command(plan, output), capture_output=True, text=True,
                          encoding="utf-8", errors="replace", **_no_window())
    if proc.returncode != 0 or os.path.getsize(output) == 0:
        msg = (proc.stderr or "").strip().splitlines()
        return False, (msg[-1] if msg else f"exit {proc.returncode}"), ""
    return True, "", ""


# --------------------------------------------------------------- gapless ---
def _number(value) -> int:
    m = re.match(r"\s*(\d+)", str(value or ""))
    return int(m.group(1)) if m else 10 ** 6


def _track_key(r: FileResult):
    tags = r.tags or {}
    return (_number(tags.get("disc")), _number(tags.get("track")), os.path.basename(r.path).lower())


def _read_head(path: str, channels: int, rate: int, frames: int) -> np.ndarray:
    out, n = [], 0
    gen = iter_pcm(path, channels, chunk_frames=frames, max_seconds=frames / rate * 1.2 + 0.05)
    try:
        for b in gen:
            out.append(b)
            n += b.shape[0]
            if n >= frames:
                break
    finally:
        gen.close()
    return np.concatenate(out)[:frames] if out else np.zeros((0, channels), np.int32)


def _read_tail(path: str, channels: int, rate: int, frames: int) -> np.ndarray:
    secs = frames / rate * 1.5 + 0.05
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-sseof", f"-{secs:.3f}", "-i", path, "-map", "0:a:0",
           "-vn", "-f", "s32le", "-acodec", "pcm_s32le", "-"]
    proc = subprocess.run(cmd, capture_output=True, **_no_window())
    if proc.returncode != 0:
        raise RuntimeError("tail decode failed")
    x = np.frombuffer(proc.stdout[: len(proc.stdout) // (4 * channels) * 4 * channels], "<i4")
    return x.reshape(-1, channels)[-frames:].copy()


def _frame_count(path: str, rate: int) -> Optional[int]:
    """Exact number of sample frames (FLAC/WAV/AIFF/ALAC/WavPack headers), or None."""
    cmd = [tool("ffprobe"), "-v", "error", "-select_streams", "a:0",
           "-show_entries", "stream=duration_ts,time_base", "-of", "json", path]
    proc = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace",
                          **_no_window())
    try:
        st = json.loads(proc.stdout or "{}")["streams"][0]
        if Fraction(st["time_base"]) != Fraction(1, rate):
            return None
        n = int(st["duration_ts"])
        return n if n > 0 else None
    except (KeyError, IndexError, ValueError, TypeError, ZeroDivisionError):
        return None


def prepare_gapless(plans: List[FixPlan], results, progress: Optional[Callable[[str], None]] = None) -> int:
    """Give each file that will be resampled the audio of its album neighbours, so the
    converted tracks join without a click (live albums, DJ mixes, classical works).

    Neighbours are the files next to it in its folder, ordered by disc and track
    number (then file name), with the same sample rate and channel count.  A join
    whose two sides are both quieter than -70 dBFS needs nothing and is skipped.
    Everything is read now, before any file is replaced.  Returns the number of
    gapless joins found; on any problem a file simply gets the plain conversion."""
    todo = {os.path.abspath(p.source): p for p in plans if p.resample}
    if not todo:
        return 0
    by_folder: Dict[str, List[FileResult]] = {}
    seen = set()
    for r in results:
        key = os.path.abspath(r.path)
        if key in seen or not r.sample_rate or r.verdict in (LOSSY, ERROR):
            continue
        seen.add(key)
        by_folder.setdefault(os.path.dirname(key), []).append(r)
    joins: List[Tuple[FileResult, FileResult]] = []
    for items in by_folder.values():
        items.sort(key=_track_key)
        for a, b in zip(items, items[1:]):
            if (a.sample_rate, a.channels) != (b.sample_rate, b.channels):
                continue
            if os.path.abspath(a.path) in todo or os.path.abspath(b.path) in todo:
                joins.append((a, b))
    if not joins:
        return 0
    if progress:
        progress(f"Checking {len(joins)} track join(s) for gapless playback…")

    def edges(pair):
        a, b = pair
        try:
            tail = _read_tail(a.path, a.channels or 2, a.sample_rate, CONTEXT_FRAMES)
            head = _read_head(b.path, b.channels or 2, b.sample_rate, CONTEXT_FRAMES)
        except (OSError, RuntimeError, ValueError):
            return None
        loud = max(int(np.abs(tail.astype(np.int64)).max(initial=0)),
                   int(np.abs(head.astype(np.int64)).max(initial=0)))
        return (tail, head) if loud >= SILENT_JOIN else None

    with ThreadPoolExecutor(max_workers=8) as ex:
        found = list(ex.map(edges, joins))
    gapless = [(a, b, e) for (a, b), e in zip(joins, found) if e is not None]
    if not gapless:
        return 0
    # Runs of consecutive gapless joins share one sample grid, starting at the run's
    # first track, so every track needs the exact lengths of the tracks before it.
    nxt = {os.path.abspath(a.path): (b, e) for a, b, e in gapless}
    firsts = {os.path.abspath(a.path): a for a, _, _ in gapless}
    starts = set(firsts) - {os.path.abspath(b.path) for _, b, _ in gapless}
    count = 0
    for start in starts:
        run = [firsts[start]]
        links = []
        while os.path.abspath(run[-1].path) in nxt:
            b, e = nxt[os.path.abspath(run[-1].path)]
            links.append(e)
            run.append(b)
        frames = [_frame_count(r.path, r.sample_rate) for r in run]
        if any(f is None for f in frames):
            continue
        offset = 0
        for i, r in enumerate(run):
            plan = todo.get(os.path.abspath(r.path))
            if plan is not None:
                plan.frames, plan.offset = frames[i], offset
                if i > 0:
                    plan.context_before = links[i - 1][0]
                if i < len(links):
                    plan.context_after = links[i][1]
            offset += frames[i]
        count += len(links)
    return count


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
        ok, err, how = _convert(plan, tmp)
        if not ok:
            return FixOutcome(plan, False, "FFmpeg failed: " + err)
        output = plan.output
        if plan.replace_original:
            if os.path.exists(output) and os.path.normcase(os.path.abspath(output)) != \
                    os.path.normcase(os.path.abspath(plan.source)):
                output = _unique(output)  # extension changed (e.g. APE -> FLAC): never overwrite others
            if plan.backup == "none":
                os.replace(tmp, output)
                if os.path.abspath(output) != os.path.abspath(plan.source):
                    os.remove(plan.source)
                note = "original deleted"
            else:
                if plan.backup == "bak":
                    backup = _unique(plan.source + ".bak")
                else:
                    os.makedirs(plan.backup_dir, exist_ok=True)
                    backup = _unique(os.path.join(plan.backup_dir, os.path.basename(plan.source)))
                shutil.move(plan.source, backup)
                try:
                    os.replace(tmp, output)
                except OSError:
                    shutil.move(backup, plan.source)  # put the original back
                    raise
                where = (os.path.basename(backup) if plan.backup == "bak"
                         else os.path.join(os.path.basename(plan.backup_dir), os.path.basename(backup)))
                note = f"original moved to {where}"
        else:
            output = _unique(output) if os.path.exists(output) else output
            os.replace(tmp, output)
            note = "original left unchanged"
        tmp = None
        after = os.path.getsize(output)
        new = None
        if getattr(cfg, "fix_verify", True):
            new = analyze_file(output, settings=cfg)
        verb = "Fixed" if plan.replace_original else "Saved"
        msg = f"{verb} {os.path.basename(output)} ({plan.description}, {after / before:.0%} of the size; {note})."
        if how == "exact":
            msg += (f" Every {_ordinal(plan.exact_ratio)} sample was the untouched 16-bit master, so it was "
                    "restored bit-exactly, without resampling.")
        elif how == "gapless":
            msg += " Converted together with the neighbouring track(s), so the joins stay gapless."
        if new is not None:
            msg += f" Re-check: {new.verdict} – {new.summary}"
        return FixOutcome(plan, True, msg, output, new, before, after, how)
    except OSError as exc:
        return FixOutcome(plan, False, f"Could not save the fixed file: {exc}")
    finally:
        if tmp and os.path.exists(tmp):
            try:
                os.remove(tmp)
            except OSError:
                pass


def _ordinal(n: int) -> str:
    return {2: "second", 3: "third", 4: "fourth"}.get(n, f"{n}th")


def ffmpeg_available() -> bool:
    return shutil.which(tool("ffmpeg")) is not None or os.path.isfile(tool("ffmpeg"))
