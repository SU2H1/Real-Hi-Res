"""Album cover thumbnails for the album/artist views (FFmpeg only, no Pillow).

The cover is taken from the first track that has an embedded picture, or else
from a cover image in the album folder (cover.jpg, folder.jpg, front.jpg ...).
FFmpeg scales and crops it to a square PNG, which Tk can show directly (Tk
cannot decode JPEG by itself).
"""

from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass
from typing import Iterable, Optional, Tuple

from .probe import _no_window, tool

FOLDER_IMAGES = ("cover", "folder", "front", "album", "albumart", "albumartsmall", "artwork")
IMAGE_EXTS = (".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif")


def _thumbnail(source: str, size: int, embedded: bool) -> Optional[bytes]:
    vf = f"scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}"
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-i", source]
    cmd += (["-map", "0:v:0"] if embedded else []) + ["-an", "-frames:v", "1", "-vf", vf,
                                                       "-f", "image2pipe", "-c:v", "png", "-"]
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=30, **_no_window())
    except (OSError, subprocess.TimeoutExpired):
        return None
    data = proc.stdout
    return data if proc.returncode == 0 and data.startswith(b"\x89PNG") else None


def folder_image(folder: str) -> Optional[str]:
    """Best cover-like image file in `folder`, if any."""
    try:
        names = os.listdir(folder)
    except OSError:
        return None
    images = [n for n in names if os.path.splitext(n)[1].lower() in IMAGE_EXTS]
    for stem in FOLDER_IMAGES:
        for n in images:
            base = os.path.splitext(n)[0].lower()
            if base == stem or base.startswith(stem + "_") or base.startswith("albumart_"):
                return os.path.join(folder, n)
    return os.path.join(folder, images[0]) if len(images) == 1 else None


def cover_png(tracks: Iterable[str], size: int = 150) -> Optional[bytes]:
    """PNG bytes of a size×size cover for an album/artist, or None."""
    tracks = list(tracks)
    for t in tracks[:3]:
        data = _thumbnail(t, size, embedded=True)
        if data:
            return data
    seen = set()
    for t in tracks:
        folder = os.path.dirname(t)
        if folder in seen:
            continue
        seen.add(folder)
        img = folder_image(folder)
        if img:
            data = _thumbnail(img, size, embedded=False)
            if data:
                return data
    return None


# ------------------------------------------------- inspecting a cover (2.1) ---
# The Cover window (docs/player-spec.md, "Inspecting and changing a cover") shows
# where the cover comes from and the picture as it is stored.  The Rust version
# (rust/crates/core/src/covers.rs) reads the same headers and gives the same texts.

@dataclass
class PictureInfo:
    """What a FLAC PICTURE block records about an image (metaflac's rules)."""
    mime: str      # "image/jpeg" or "image/png"
    width: int
    height: int
    depth: int     # bits per pixel
    colors: int    # the palette's size for an indexed PNG, else 0


def _be32(b: bytes, at: int) -> int:
    return int.from_bytes(b[at:at + 4], "big")


def _png_info(data: bytes) -> Optional[PictureInfo]:
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        return None
    pos, palette = 8, None
    while len(data) - pos > 12:
        clen, kind = _be32(data, pos), data[pos + 4:pos + 8]
        if kind == b"IHDR" and clen == 13 and len(data) >= pos + 21:
            w, h, bits, color = _be32(data, pos + 8), _be32(data, pos + 12), data[pos + 16], data[pos + 17]
            if color == 3:  # indexed: 8-bit samples, the palette's size from PLTE
                palette = (w, h)
            else:
                channels = {0: 1, 2: 3, 4: 2, 6: 4}.get(color, 0)
                return PictureInfo("image/png", w, h, bits * channels, 0)
        elif palette is not None and kind == b"PLTE":
            return PictureInfo("image/png", palette[0], palette[1], 24, clen // 3)
        if clen + 12 > len(data) - pos:
            return None
        pos += 12 + clen
    return None


_SOF = (0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF)


def _jpeg_info(data: bytes) -> Optional[PictureInfo]:
    if not data.startswith(b"\xff\xd8"):
        return None
    pos, n = 2, len(data)
    while True:
        while pos < n and data[pos] != 0xFF:   # the next marker
            pos += 1
        while pos < n and data[pos] == 0xFF:   # and its fill bytes
            pos += 1
        if pos >= n or data[pos] in (0xDA, 0xD9):  # start of scan or end of image: no size
            return None
        marker = data[pos]
        pos += 1
        if n - pos < 2:
            return None
        seg = (data[pos] << 8) | data[pos + 1]
        if marker in _SOF:
            if seg < 8 or n - pos < seg:
                return None
            h, w = (data[pos + 3] << 8) | data[pos + 4], (data[pos + 5] << 8) | data[pos + 6]
            return PictureInfo("image/jpeg", w, h, data[pos + 2] * data[pos + 7], 0)
        if seg < 2 or n - pos < seg:
            return None
        pos += seg


def image_info(data: bytes) -> Optional[PictureInfo]:
    """The type, size and depth of a JPEG or PNG image, read from its header (None
    for anything else, or a header that cannot be read)."""
    return _png_info(data) or _jpeg_info(data)


def image_format(data: bytes) -> str:
    """"JPEG", "PNG", "GIF", "BMP", "WebP", or "" for something else."""
    if data.startswith(b"\xff\xd8"):
        return "JPEG"
    if data.startswith(b"\x89PNG"):
        return "PNG"
    if data[:6] in (b"GIF87a", b"GIF89a"):
        return "GIF"
    if data[:2] == b"BM":
        return "BMP"
    if data[:4] == b"RIFF" and data[8:12] == b"WEBP":
        return "WebP"
    return ""


MIME = {"JPEG": "image/jpeg", "PNG": "image/png", "GIF": "image/gif", "BMP": "image/bmp", "WebP": "image/webp"}


@dataclass
class CoverSource:
    """Where the cover shown for some songs comes from, and the picture as stored."""
    kind: str          # "embedded" (inside a song) or "folder" (an image file next to it)
    path: str          # the song, or the image file
    data: bytes = b""
    format: str = ""   # image_format()
    width: int = 0
    height: int = 0

    @property
    def mime(self) -> str:
        return MIME.get(self.format, "application/octet-stream")

    @property
    def ext(self) -> str:
        """The file extension to save the picture with (".jpg", ".png" ...)."""
        return {"JPEG": ".jpg", "PNG": ".png", "GIF": ".gif", "BMP": ".bmp", "WebP": ".webp"}.get(self.format, ".img")

    @property
    def where(self) -> str:
        """"Inside 01 Song.flac" or "cover.jpg in the album's folder"."""
        return where_text(self.kind, self.path)

    @property
    def details(self) -> str:
        """"1,200 × 1,200 pixels · JPEG · 245.3 KB"."""
        return details_text(self.width, self.height, self.format, len(self.data))


def where_text(kind: str, path: str) -> str:
    name = os.path.basename(path)
    return f"Inside {name}" if kind == "embedded" else f"{name} in the album's folder"


def details_text(width: int, height: int, fmt: str, size: int) -> str:
    from .fix import fmt_bytes

    parts = [f"{width:,} × {height:,} pixels"] if width and height else []
    return " · ".join(parts + [fmt or "Unknown format", fmt_bytes(size)])


def _embedded_picture(path: str) -> bytes:
    """The first picture inside a song as it is stored (FFmpeg's first video stream,
    which is what cover_png() shows), or b""."""
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-i", path, "-map", "0:v:0", "-c", "copy",
           "-frames:v", "1", "-f", "image2pipe", "-"]
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=30, stdin=subprocess.DEVNULL, **_no_window())
    except (OSError, subprocess.TimeoutExpired):
        return b""
    return proc.stdout if proc.returncode == 0 else b""


def _probe_size(path: str) -> Tuple[int, int]:
    """Width and height of the first picture of a file, by ffprobe (images that are
    not JPEG or PNG)."""
    cmd = [tool("ffprobe"), "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height",
           "-of", "csv=p=0", path]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL,
                              **_no_window())
        w, h = proc.stdout.strip().split(",")[:2]
        return int(w), int(h)
    except (OSError, subprocess.TimeoutExpired, ValueError):
        return 0, 0


def _source(kind: str, path: str, data: bytes) -> CoverSource:
    info = image_info(data)
    if info is not None:
        w, h = info.width, info.height
    else:
        w, h = _probe_size(path)
    return CoverSource(kind, path, data, image_format(data), w, h)


def cover_source(tracks: Iterable[str]) -> Optional[CoverSource]:
    """Where the cover that cover_png() shows for these songs comes from: the
    picture inside the first of the first three songs that has one, else the
    folder image, with the picture's bytes, size and format.  None when there is
    no cover."""
    tracks = list(tracks)
    for t in tracks[:3]:
        data = _embedded_picture(t)
        if data:
            return _source("embedded", t, data)
    seen = set()
    for t in tracks:
        folder = os.path.dirname(t)
        if folder in seen:
            continue
        seen.add(folder)
        img = folder_image(folder)
        if img:
            try:
                with open(img, "rb") as fh:
                    data = fh.read()
            except OSError:
                continue
            if data:
                return _source("folder", img, data)
    return None


def fit_png(data: bytes, size: int) -> Optional[bytes]:
    """The picture as a PNG that fits into size × size (aspect kept, never enlarged),
    made by FFmpeg, for showing it (Tk cannot decode JPEG)."""
    vf = f"scale=w='min({size},iw)':h='min({size},ih)':force_original_aspect_ratio=decrease"
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-i", "pipe:0", "-frames:v", "1", "-vf", vf,
           "-f", "image2pipe", "-c:v", "png", "-"]
    try:
        proc = subprocess.run(cmd, input=data, capture_output=True, timeout=30, **_no_window())
    except (OSError, subprocess.TimeoutExpired):
        return None
    out = proc.stdout
    return out if proc.returncode == 0 and out.startswith(b"\x89PNG") else None


def folder_cover_name(folder: str, data: bytes) -> str:
    """Where "Also save it as cover.jpg in the album's folder" writes: cover.jpg, or
    cover.png for a PNG, keeping the spelling of one that is already there (Cover.JPG)."""
    name = "cover.png" if image_format(data) == "PNG" else "cover.jpg"
    try:
        for n in sorted(os.listdir(folder)):
            if n.lower() == name:
                return os.path.join(folder, n)
    except OSError:
        pass
    return os.path.join(folder, name)


def save_folder_cover(folder: str, data: bytes) -> str:
    """Write the picture as cover.jpg (cover.png) into the folder, through a temporary
    file; returns its path.  Raises OSError."""
    from .embed import replace_file

    path = folder_cover_name(folder, data)
    replace_file(path, data)
    return path


__all__ = ["cover_png", "folder_image", "image_info", "image_format", "cover_source", "CoverSource",
           "PictureInfo", "fit_png", "save_folder_cover", "folder_cover_name"]
