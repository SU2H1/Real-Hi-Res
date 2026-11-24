"""Album cover thumbnails for the album/artist views (FFmpeg only, no Pillow).

The cover is taken from the first track that has an embedded picture, or else
from a cover image in the album folder (cover.jpg, folder.jpg, front.jpg ...).
FFmpeg scales and crops it to a square PNG, which Tk can show directly (Tk
cannot decode JPEG by itself).

Covers are kept on disk (`cached_cover_png()`), one PNG per album and size in
`covers/` in the data folder (`cache_dir()`), shared by the Python and the Rust
app, so a cover is made by FFmpeg only once and not again after a restart.  An
album without a cover is remembered as an empty file.  The file name
(`cache_name()`) changes when one of the album's first four songs or one of
their folders changes, so a new cover image or a re-tagged song gives a new
entry; `trim_cache()` keeps the folder under `CACHE_LIMIT_BYTES` by deleting
the least recently used files.
"""

from __future__ import annotations

import hashlib
import os
import stat
import subprocess
import tempfile
import time
from typing import Iterable, List, Optional, Sequence, Tuple

from .probe import _no_window, tool

FOLDER_IMAGES = ("cover", "folder", "front", "album", "albumart", "albumartsmall", "artwork")
IMAGE_EXTS = (".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif")

COVERS_ENV = "REALHIRES_COVERS"        # the covers folder (tests point it at a temporary one)
CACHE_VERSION = 1                      # part of every key: a new way of making covers gives new names
CACHE_PATHS = 4                        # the songs of an album that a cover is made from (and keyed by)
CACHE_LIMIT_BYTES = 500 * 1024 * 1024  # trim_cache() keeps the folder under this
TEMP_MAX_AGE_S = 3600                  # a temp file older than this was left by a crash
_BLOCK = 4096                          # trim_cache() counts each file as whole 4 KB blocks, as on disk
PNG_START = b"\x89PNG\r\n\x1a\n"
PNG_END = b"\x00\x00\x00\x00IEND\xaeB`\x82"


def _thumbnail(source: str, size: int, embedded: bool) -> Tuple[Optional[bytes], bool]:
    """(PNG bytes or None, whether FFmpeg ran to the end).  False means it could
    not be started or timed out, so "no picture" is not certain."""
    vf = f"scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}"
    cmd = [tool("ffmpeg"), "-v", "error", "-nostdin", "-i", source]
    cmd += (["-map", "0:v:0"] if embedded else []) + ["-an", "-frames:v", "1", "-vf", vf,
                                                       "-f", "image2pipe", "-c:v", "png", "-"]
    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=30, **_no_window())
    except (OSError, subprocess.TimeoutExpired):
        return None, False
    data = proc.stdout
    return (data if proc.returncode == 0 and data.startswith(b"\x89PNG") else None), True


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


def _make(tracks: Sequence[str], size: int) -> Tuple[Optional[bytes], bool]:
    """(cover, certain): certain is False when an FFmpeg run failed to start or
    timed out on the way to "no cover"."""
    certain = True
    for t in tracks[:3]:
        data, ok = _thumbnail(t, size, embedded=True)
        certain = certain and ok
        if data:
            return data, True
    seen = set()
    for t in tracks:
        folder = os.path.dirname(t)
        if folder in seen:
            continue
        seen.add(folder)
        img = folder_image(folder)
        if img:
            data, ok = _thumbnail(img, size, embedded=False)
            certain = certain and ok
            if data:
                return data, True
    return None, certain


def cover_png(tracks: Iterable[str], size: int = 150) -> Optional[bytes]:
    """PNG bytes of a size×size cover for an album/artist, or None."""
    return _make(list(tracks), size)[0]


# ------------------------------------------------------------- on disk ---
def cache_dir() -> str:
    """`$REALHIRES_COVERS`, else `covers` in the data folder (`store.data_dir()`)."""
    env = os.environ.get(COVERS_ENV)
    if env:
        return env
    from .store import data_dir

    return os.path.join(data_dir(), "covers")


def _stat(path: str) -> Tuple[int, int]:
    """(size, st_mtime_ns) of a file or folder, (-1, -1) when it cannot be read."""
    try:
        st = os.stat(path)
    except (OSError, ValueError):
        return -1, -1
    return st.st_size, st.st_mtime_ns


def cache_key(paths: Iterable[str], size: int) -> str:
    """The text whose SHA-1 names a cover on disk.  Both versions build it
    byte for byte the same way (Rust: `covers::cache_key`), lines joined with
    "\\n" (no newline at the end), encoded as UTF-8:

        v1<TAB><size>                       CACHE_VERSION and the size in pixels
        <path><TAB><bytes><TAB><mtime_ns>   for each of the first four paths, in order
        <folder><TAB><mtime_ns>             for each distinct folder of those paths
                                            (os.path.dirname), in first-seen order

    Numbers are decimal integers; a file or folder that cannot be read gives
    -1 for its size and time.
    """
    paths = list(paths)[:CACHE_PATHS]
    lines = [f"v{CACHE_VERSION}\t{int(size)}"]
    folders: List[str] = []
    for p in paths:
        n, mtime = _stat(p)
        lines.append(f"{p}\t{n}\t{mtime}")
        folder = os.path.dirname(p)
        if folder not in folders:
            folders.append(folder)
    for folder in folders:
        lines.append(f"{folder}\t{_stat(folder)[1]}")
    return "\n".join(lines)


def cache_name(paths: Iterable[str], size: int) -> str:
    """The file name of a cover in the covers folder: the SHA-1 (40 lower-case
    hex digits) of `cache_key()`, then ".png"."""
    key = cache_key(paths, size).encode("utf-8", "surrogateescape")
    return hashlib.sha1(key).hexdigest() + ".png"


def cache_file(paths: Iterable[str], size: int) -> str:
    return os.path.join(cache_dir(), cache_name(paths, size))


def _valid(data: bytes) -> bool:
    """A whole PNG (signature and IEND chunk), or the empty "no cover" mark."""
    return not data or (data.startswith(PNG_START) and data.endswith(PNG_END))


def read_cached(file: str) -> Tuple[bool, Optional[bytes]]:
    """(found, cover) of one cache file; found is False when it is missing,
    unreadable or damaged.  Reading touches its modification time, which is
    what `trim_cache()` goes by."""
    try:
        with open(file, "rb") as fh:
            data = fh.read()
    except OSError:
        return False, None
    if not _valid(data):
        return False, None
    try:
        os.utime(file, None)
    except OSError:
        pass
    return True, data or None


def _write(file: str, data: bytes) -> None:
    """Write through a temp file and os.replace(), so a reader never sees half a
    file; failing to write only costs making the cover again next time."""
    folder = os.path.dirname(file)
    tmp = None
    try:
        os.makedirs(folder, exist_ok=True)
        fd, tmp = tempfile.mkstemp(prefix=".cover-", suffix=".tmp", dir=folder)
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
        os.replace(tmp, file)
        tmp = None
    except OSError:
        pass
    finally:
        if tmp is not None:
            try:
                os.remove(tmp)
            except OSError:
                pass


def is_cached(paths: Iterable[str], size: int) -> bool:
    """Whether the covers folder has this cover (or its "no cover" mark)."""
    try:
        return os.path.isfile(cache_file(paths, size))
    except (OSError, ValueError):
        return False


def cached_cover_png(paths: Iterable[str], size: int) -> Optional[bytes]:
    """`cover_png()` of the first four paths, from the covers folder when it has
    it (None for an album remembered without a cover), else made with FFmpeg and
    saved there.  A damaged or unreadable file is made again.  When FFmpeg could
    not run, nothing is saved, so the album is tried again next time."""
    paths = list(paths)[:CACHE_PATHS]
    try:
        file: Optional[str] = cache_file(paths, size)
    except (OSError, ValueError):
        file = None
    if file is not None:
        found, data = read_cached(file)
        if found:
            return data
    data, certain = _make(paths, size)
    if file is not None and (data is not None or certain):
        _write(file, data or b"")
    return data


def _usage(n: int) -> int:
    return max(1, -(-n // _BLOCK)) * _BLOCK


def trim_cache(limit_bytes: int = CACHE_LIMIT_BYTES, folder: Optional[str] = None) -> int:
    """Delete the least recently used covers (oldest modification time first)
    until the folder holds at most `limit_bytes` (each file counted as whole
    4 KB blocks, an empty one as one block), and temp files older than an hour.
    Returns the number of covers deleted."""
    folder = folder or cache_dir()
    try:
        names = os.listdir(folder)
    except OSError:
        return 0
    now = time.time()
    entries = []
    for name in names:
        path = os.path.join(folder, name)
        try:
            st = os.stat(path)
        except OSError:
            continue
        if not stat.S_ISREG(st.st_mode):
            continue
        if name.endswith(".tmp"):
            if now - st.st_mtime > TEMP_MAX_AGE_S:
                try:
                    os.remove(path)
                except OSError:
                    pass
            continue
        if name.endswith(".png"):
            entries.append((st.st_mtime_ns, name, _usage(st.st_size)))
    total = sum(e[2] for e in entries)
    deleted = 0
    for _, name, used in sorted(entries):
        if total <= limit_bytes:
            break
        try:
            os.remove(os.path.join(folder, name))
        except OSError:
            continue
        total -= used
        deleted += 1
    return deleted


__all__ = ["cover_png", "folder_image", "cache_dir", "cache_key", "cache_name", "cache_file", "cached_cover_png",
           "is_cached", "read_cached", "trim_cache", "COVERS_ENV", "CACHE_LIMIT_BYTES", "CACHE_PATHS"]
