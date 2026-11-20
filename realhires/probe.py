"""Read container/stream metadata with ffprobe and stream decoded PCM with ffmpeg."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from typing import Iterator, Optional

import numpy as np

# Codecs whose output is lossy by design.  A file in one of these formats can
# never be genuine hi-res, whatever sample rate the container advertises.
LOSSY_CODECS = {
    "mp3", "mp3float", "mp2", "mp2float", "mp1", "aac", "aac_latm", "aac_fixed",
    "vorbis", "opus", "wmav1", "wmav2", "wmavoice", "ac3", "eac3", "atrac1",
    "atrac3", "atrac3p", "atrac9", "musepack7", "musepack8", "speex", "amr_nb",
    "amr_wb", "gsm", "qdm2", "cook", "ra_144", "ra_288", "nellymoser", "dts",
}

# Formats that do not carry a meaningful integer bit depth.
FLOAT_SAMPLE_FMTS = {"flt", "fltp", "dbl", "dblp"}


class ToolMissingError(RuntimeError):
    """Raised when ffmpeg/ffprobe cannot be found on PATH."""


def _bundle_dirs():
    """Folders that may contain an FFmpeg shipped with a packaged release."""
    env = os.environ.get("REALHIRES_FFMPEG_DIR")
    if env:
        yield env
    meipass = getattr(sys, "_MEIPASS", None)  # PyInstaller bundle root
    if meipass:
        yield os.path.join(meipass, "ffmpeg")
    exe_dir = os.path.dirname(os.path.abspath(sys.executable if getattr(sys, "frozen", False)
                                              else __file__))
    yield os.path.join(exe_dir, "ffmpeg")
    yield exe_dir


def find_tool(name: str) -> Optional[str]:
    """Locate ffmpeg/ffprobe: bundled copy first, then the system PATH."""
    exe = name + (".exe" if os.name == "nt" else "")
    for d in _bundle_dirs():
        cand = os.path.join(d, exe)
        if os.path.isfile(cand) and os.access(cand, os.X_OK):
            return cand
    return shutil.which(name)


_TOOLS: dict = {}


def tool(name: str) -> str:
    if name not in _TOOLS:
        _TOOLS[name] = find_tool(name) or name
    return _TOOLS[name]


def external_env() -> dict:
    """Environment for launching *other* programs (ffmpeg, the file manager).

    A frozen (PyInstaller) Linux build points LD_LIBRARY_PATH at its bundled
    libraries; a system ffmpeg or file manager inheriting that would load the
    bundle's older libstdc++/libX11 and crash.  Restore the user's original
    value instead.  (macOS: same for DYLD_LIBRARY_PATH.)
    """
    env = dict(os.environ)
    if getattr(sys, "frozen", False):
        for var in ("LD_LIBRARY_PATH", "DYLD_LIBRARY_PATH"):
            orig = env.pop(var + "_ORIG", None)
            if orig is not None:
                env[var] = orig
            else:
                env.pop(var, None)
    return env


def _no_window() -> dict:
    """Keyword arguments for every external process: a clean environment and,
    on Windows, no console window flashing up from the windowed GUI build."""
    kw = {"env": external_env()}
    if os.name == "nt":
        kw["creationflags"] = getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)
    return kw


def require_tools() -> None:
    missing = [t for t in ("ffmpeg", "ffprobe") if find_tool(t) is None]
    if missing:
        raise ToolMissingError(
            "Required tool(s) not found: " + ", ".join(missing)
            + ". Install FFmpeg (https://ffmpeg.org/download.html) or use a release "
            "package, which bundles it."
        )


@dataclass
class AudioInfo:
    path: str
    codec: str
    container: str
    sample_rate: int
    channels: int
    sample_fmt: str
    claimed_bits: Optional[int]  # None for lossy codecs
    is_float: bool
    is_lossy: bool
    duration: Optional[float]
    bit_rate: Optional[int]
    extra: dict = field(default_factory=dict)
    tags: dict = field(default_factory=dict)   # artist, album, album_artist, title, track, disc, date, genre

    @property
    def claims_hires(self) -> bool:
        """True when the file advertises better-than-CD resolution."""
        if self.is_lossy:
            return False
        bits = self.claimed_bits or 16
        return self.sample_rate > 48000 or bits > 16


def _int_or_none(value) -> Optional[int]:
    try:
        v = int(value)
        return v if v > 0 else None
    except (TypeError, ValueError):
        return None


def _float_or_none(value) -> Optional[float]:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def probe(path: str) -> AudioInfo:
    cmd = [
        tool("ffprobe"), "-v", "error", "-select_streams", "a:0",
        "-show_entries",
        "stream=codec_name,sample_rate,channels,sample_fmt,bits_per_sample,"
        "bits_per_raw_sample,bit_rate,duration:stream_tags:format=format_name,duration,bit_rate:format_tags",
        "-of", "json", path,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                          errors="replace", **_no_window())
    if proc.returncode != 0:
        raise RuntimeError(f"ffprobe failed: {proc.stderr.strip() or 'unknown error'}")
    data = json.loads(proc.stdout or "{}")
    streams = data.get("streams") or []
    if not streams:
        raise RuntimeError("no audio stream found")
    s = streams[0]
    fmt = data.get("format") or {}

    codec = s.get("codec_name", "unknown")
    sample_fmt = s.get("sample_fmt", "")
    is_lossy = codec in LOSSY_CODECS
    is_float = sample_fmt in FLOAT_SAMPLE_FMTS or codec.startswith("pcm_f")

    bits = _int_or_none(s.get("bits_per_raw_sample")) or _int_or_none(s.get("bits_per_sample"))
    if bits is None:
        if sample_fmt.startswith("s16") or sample_fmt.startswith("u8"):
            bits = 16 if sample_fmt.startswith("s16") else 8
        elif sample_fmt.startswith("s32"):
            bits = 32
        elif sample_fmt.startswith("flt"):
            bits = 32
        elif sample_fmt.startswith("dbl"):
            bits = 64
    if codec.startswith("dsd_"):
        # DSD is 1-bit/2.8 MHz; ffmpeg decodes it to float PCM at fs/8.
        is_float = True
    if is_lossy:
        bits = None

    return AudioInfo(
        path=path,
        codec=codec,
        container=fmt.get("format_name", ""),
        sample_rate=_int_or_none(s.get("sample_rate")) or 0,
        channels=_int_or_none(s.get("channels")) or 1,
        sample_fmt=sample_fmt,
        claimed_bits=bits,
        is_float=is_float,
        is_lossy=is_lossy,
        duration=_float_or_none(s.get("duration")) or _float_or_none(fmt.get("duration")),
        bit_rate=_int_or_none(s.get("bit_rate")) or _int_or_none(fmt.get("bit_rate")),
        tags=normalize_tags(fmt.get("tags"), s.get("tags")),
    )


# tag name variants used by FLAC/Vorbis, ID3, MP4/iTunes, APE, RIFF INFO...
_TAG_ALIASES = {
    "artist": ("artist", "iart", "author"),
    "album_artist": ("album_artist", "albumartist", "album artist", "aart", "band", "tpe2"),
    "album": ("album", "iprd"),
    "title": ("title", "inam"),
    "track": ("track", "tracknumber", "trck", "itrk"),
    "disc": ("disc", "discnumber", "tpos"),
    "date": ("date", "year", "icrd"),
    "genre": ("genre", "ignr"),
}


def normalize_tags(*sources) -> dict:
    """Merge ffprobe format/stream tag dicts into a small, case-insensitive set."""
    raw: dict = {}
    for src in sources:
        if isinstance(src, dict):
            for k, v in src.items():
                if isinstance(v, str) and v.strip():
                    raw.setdefault(k.strip().lower(), v.strip())
    out = {}
    for key, names in _TAG_ALIASES.items():
        for n in names:
            if n in raw:
                out[key] = raw[n]
                break
    return out


def iter_pcm(
    path: str,
    channels: int,
    chunk_frames: int = 1 << 16,
    max_seconds: Optional[float] = None,
) -> Iterator[np.ndarray]:
    """Yield decoded audio as int32 arrays of shape (frames, channels).

    ffmpeg left-justifies every integer format into 32 bits (16-bit samples are
    shifted left by 16, 24-bit by 8), so unused low bits stay exactly zero and
    the bit-depth test can count them.  Float sources are scaled to the int32
    range without dithering.
    """
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-i", path, "-map", "0:a:0", "-vn"]
    if max_seconds:
        cmd += ["-t", f"{max_seconds:.3f}"]
    cmd += ["-f", "s32le", "-acodec", "pcm_s32le", "-"]

    frame_bytes = 4 * channels
    chunk_bytes = frame_bytes * chunk_frames
    with tempfile.TemporaryFile() as err:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=err, stdin=subprocess.DEVNULL,
                                **_no_window())
        assert proc.stdout is not None
        pending = b""
        try:
            while True:
                buf = proc.stdout.read(chunk_bytes)
                if not buf:
                    break
                buf = pending + buf
                usable = len(buf) - (len(buf) % frame_bytes)
                pending = buf[usable:]
                if usable:
                    yield np.frombuffer(buf[:usable], dtype="<i4").reshape(-1, channels)
        finally:
            proc.stdout.close()
            rc = proc.wait()
        if rc != 0:
            err.seek(0)
            msg = err.read().decode("utf-8", "replace").strip()
            raise RuntimeError(f"ffmpeg decode failed: {msg or f'exit code {rc}'}")
