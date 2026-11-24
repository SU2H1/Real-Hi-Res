"""Album cover thumbnails for the album/artist views (FFmpeg only, no Pillow).

The cover is taken from the first track that has an embedded picture, or else
from a cover image in the album folder (cover.jpg, folder.jpg, front.jpg ...).
FFmpeg scales and crops it to a square PNG, which Tk can show directly (Tk
cannot decode JPEG by itself).
"""

from __future__ import annotations

import os
import subprocess
from typing import Iterable, Optional

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


__all__ = ["cover_png", "folder_image"]
