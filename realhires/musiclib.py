"""The music library of the player (2.0).

The library is every audio file in the folders set in Settings → Library
(`library_folders`).  Each song's tags and format come from one ffprobe call
and are kept in `library.sqlite` next to the saved results (`store.data_dir()`,
or `$REALHIRES_LIBRARY`), so the library opens at once and only new or changed
files are read again.  It is a separate file from `results.sqlite`, whose
format stays the one Real-Hi-Res 1.12 reads.

The Rust version reads and writes the same database
(`rust/crates/core/src/musiclib.rs`).
"""

from __future__ import annotations

import json
import os
import sqlite3
import threading
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict, dataclass, fields
from typing import Callable, Dict, Iterable, List, Optional

from .playqueue import Song, number

LIBRARY_ENV = "REALHIRES_LIBRARY"
DB_NAME = "library.sqlite"
LIBRARY_VERSION = 1


def default_path() -> str:
    from .store import data_dir

    return os.environ.get(LIBRARY_ENV) or os.path.join(data_dir(), DB_NAME)


@dataclass
class Track:
    path: str
    title: str = ""
    artist: str = ""
    album: str = ""
    album_artist: str = ""
    genre: str = ""
    disc: int = 0
    track: int = 0
    year: str = ""
    duration: float = 0.0
    codec: str = ""
    sample_rate: int = 0
    bits: int = 0           # 0 for lossy codecs
    is_float: bool = False
    is_lossy: bool = False
    channels: int = 0
    bit_rate: int = 0
    size: int = 0
    mtime_ns: int = 0

    @property
    def display_title(self) -> str:
        return self.title or os.path.splitext(os.path.basename(self.path))[0]

    @property
    def display_artist(self) -> str:
        return self.artist or self.album_artist or "Unknown Artist"

    @property
    def display_album(self) -> str:
        return self.album or os.path.basename(os.path.dirname(self.path)) or "Unknown Album"

    def song(self) -> Song:
        return Song(self.path, self.display_title, self.artist, self.album, self.album_artist, self.genre,
                    self.disc, self.track)

    def to_json(self) -> dict:
        return asdict(self)

    @classmethod
    def from_json(cls, data: dict) -> "Track":
        known = {f.name for f in fields(cls)}
        return cls(**{k: v for k, v in data.items() if k in known})


def quality(t: Track) -> str:
    """Apple Music's way of naming the format: "Hi-Res Lossless", "Lossless" or the codec."""
    if t.is_lossy:
        kbps = f" {round(t.bit_rate / 1000)} kbps" if t.bit_rate else ""
        return f"{t.codec.upper()}{kbps}"
    hires = t.sample_rate > 48000 or (t.bits or 16) > 16 or t.is_float
    return "Hi-Res Lossless" if hires else "Lossless"


def resolution(t: Track) -> str:
    """"24-bit/96 kHz" (or "32-bit float/96 kHz")."""
    if not t.sample_rate:
        return ""
    khz = f"{t.sample_rate / 1000:g} kHz"
    if t.is_lossy or not t.bits:
        return khz
    return f"{t.bits}-bit{' float' if t.is_float else ''}/{khz}"


def track_from_info(info, size: int = 0, mtime_ns: int = 0) -> Track:
    tags = info.tags or {}
    bits = 0 if info.is_lossy else (info.claimed_bits or (32 if info.is_float else 0))
    return Track(path=info.path, title=tags.get("title", ""), artist=tags.get("artist", ""),
                 album=tags.get("album", ""), album_artist=tags.get("album_artist", ""),
                 genre=tags.get("genre", ""), disc=number(tags.get("disc")), track=number(tags.get("track")),
                 year=(tags.get("date", "") or "")[:4], duration=float(info.duration or 0.0), codec=info.codec,
                 sample_rate=int(info.sample_rate or 0), bits=int(bits or 0), is_float=bool(info.is_float),
                 is_lossy=bool(info.is_lossy), channels=int(info.channels or 0),
                 bit_rate=int(info.bit_rate or 0), size=size, mtime_ns=mtime_ns)


class Library:
    """The library database.  One connection per thread, as for the saved results."""

    def __init__(self, path: Optional[str] = None):
        self.path = path or default_path()
        os.makedirs(os.path.dirname(os.path.abspath(self.path)), exist_ok=True)
        self.db = sqlite3.connect(self.path, timeout=30)
        self.db.execute("CREATE TABLE IF NOT EXISTS tracks (path TEXT PRIMARY KEY, size INTEGER, "
                        "mtime_ns INTEGER, data TEXT)")
        self.db.execute("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT)")
        self.db.execute("INSERT OR IGNORE INTO meta VALUES ('version', ?)", (str(LIBRARY_VERSION),))
        self.db.commit()

    def close(self) -> None:
        self.db.close()

    def tracks(self) -> List[Track]:
        out = []
        for (data,) in self.db.execute("SELECT data FROM tracks ORDER BY path"):
            try:
                out.append(Track.from_json(json.loads(data)))
            except (ValueError, TypeError):
                continue
        return out

    def put(self, t: Track) -> None:
        self.db.execute("INSERT OR REPLACE INTO tracks VALUES (?, ?, ?, ?)",
                        (t.path, t.size, t.mtime_ns, json.dumps(t.to_json(), ensure_ascii=False, sort_keys=True)))

    def refresh(self, files: Iterable[str], jobs: int = 4,
                progress: Optional[Callable[[int, int], None]] = None,
                stop: Optional[threading.Event] = None) -> List[Track]:
        """Bring the library up to date with `files` (the audio files of the library
        folders): new and changed files are read with ffprobe, vanished ones dropped.
        Returns every track."""
        files = [os.path.abspath(f) for f in files]
        wanted = set(files)
        known: Dict[str, tuple] = {p: (s, m) for p, s, m in self.db.execute("SELECT path, size, mtime_ns FROM tracks")}
        for p in set(known) - wanted:
            self.db.execute("DELETE FROM tracks WHERE path = ?", (p,))
        todo = []
        for f in files:
            try:
                st = os.stat(f)
            except OSError:
                continue
            if known.get(f) != (st.st_size, st.st_mtime_ns):
                todo.append((f, st.st_size, st.st_mtime_ns))
        if todo:
            from .probe import probe

            def read(item):
                path, size, mtime = item
                if stop is not None and stop.is_set():
                    return None
                try:
                    return track_from_info(probe(path), size, mtime)
                except Exception:  # noqa: BLE001 - unreadable files are left out
                    return None

            done = 0
            with ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
                for t in pool.map(read, todo):
                    done += 1
                    if t is not None:
                        self.put(t)
                    if progress is not None:
                        progress(done, len(todo))
                    if done % 200 == 0:
                        self.db.commit()
        self.db.commit()
        return self.tracks()

    def update_file(self, path: str) -> None:
        """A file was changed by Real-Hi-Res (lyrics embedded, fixed): read it again."""
        from .probe import probe

        try:
            st = os.stat(path)
            self.put(track_from_info(probe(path), st.st_size, st.st_mtime_ns))
            self.db.commit()
        except Exception:  # noqa: BLE001
            pass


def library_files(cfg) -> List[str]:
    """Every audio file in the library folders (recursively, the scan's extensions)."""
    from .cli import collect

    folders = [f for f in (cfg.library_folders or []) if f and os.path.isdir(f)]
    return collect(folders, recursive=True, extensions=cfg.extension_set()) if folders else []


# -------------------------------------------------------------- browsing ---
def sort_songs(tracks: Iterable[Track]) -> List[Track]:
    return sorted(tracks, key=lambda t: (t.display_title.casefold(), t.display_artist.casefold(), t.path))


def albums(tracks: Iterable[Track]) -> List[dict]:
    """{"key", "title", "artist", "year", "genre", "tracks"} per album, sorted by title."""
    groups: Dict[str, List[Track]] = {}
    for t in tracks:
        groups.setdefault(t.song().album_key, []).append(t)
    out = []
    for key, ts in groups.items():
        ordered = sorted(ts, key=lambda t: (t.disc or 9999, t.track or 9999, t.display_title.casefold(), t.path))
        first = ordered[0]
        out.append({"key": key, "title": first.display_album, "artist": first.album_artist or first.display_artist,
                     "year": next((t.year for t in ordered if t.year), ""),
                     "genre": next((t.genre for t in ordered if t.genre), ""), "tracks": ordered})
    return sorted(out, key=lambda a: (a["title"].casefold(), a["artist"].casefold()))


def artists(tracks: Iterable[Track]) -> List[dict]:
    """{"name", "tracks"} per artist (album artist first), sorted by name."""
    groups: Dict[str, List[Track]] = {}
    names: Dict[str, str] = {}
    for t in tracks:
        name = t.album_artist or t.display_artist
        key = name.strip().casefold()
        groups.setdefault(key, []).append(t)
        names.setdefault(key, name)
    return sorted(({"name": names[k], "tracks": sort_songs(v)} for k, v in groups.items()),
                  key=lambda a: a["name"].casefold())


def genres(tracks: Iterable[Track]) -> List[dict]:
    groups: Dict[str, List[Track]] = {}
    names: Dict[str, str] = {}
    for t in tracks:
        name = t.genre.strip() or "Unknown Genre"
        key = name.casefold()
        groups.setdefault(key, []).append(t)
        names.setdefault(key, name)
    return sorted(({"name": names[k], "tracks": sort_songs(v)} for k, v in groups.items()),
                  key=lambda a: a["name"].casefold())
