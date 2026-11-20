"""Group scan results by album or by artist (the "By album" / "By artist" views).

Album and artist come from the files' tags.  Untagged files fall back to their
folder: the folder name is taken as the album and its parent folder as the
artist, which matches the usual Artist/Album/track layout of music libraries.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Tuple

from .verdict import FAKE, FAKE_LOSSLESS, SUSPECT, FileResult

UNKNOWN_ARTIST = "Unknown artist"


def _clean(s: str) -> str:
    return re.sub(r"\s+", " ", s or "").strip()


def artist_of(r: FileResult) -> str:
    t = r.tags or {}
    name = _clean(t.get("album_artist") or t.get("artist") or "")
    if name:
        return name
    parent = os.path.basename(os.path.dirname(os.path.dirname(r.path)))
    return parent or UNKNOWN_ARTIST


def album_of(r: FileResult) -> str:
    t = r.tags or {}
    return _clean(t.get("album") or "") or os.path.basename(os.path.dirname(r.path)) or "Unknown album"


def _track_no(r: FileResult) -> Tuple[int, int, str]:
    def num(v):
        m = re.match(r"\s*(\d+)", str(v or ""))
        return int(m.group(1)) if m else 9999
    t = r.tags or {}
    return num(t.get("disc")), num(t.get("track")), os.path.basename(r.path).lower()


@dataclass
class Group:
    key: str            # unique id ("album\x00artist" / "artist")
    title: str          # album name or artist name
    subtitle: str       # artist (album view) or "N albums" (artist view)
    paths: List[str] = field(default_factory=list)
    fake: int = 0
    suspicious: int = 0
    total: int = 0
    albums: int = 0

    @property
    def problems(self) -> int:
        return self.fake + self.suspicious

    @property
    def problem_pct(self) -> float:
        return 100.0 * self.problems / self.total if self.total else 0.0

    @property
    def cover_candidates(self) -> List[str]:
        return self.paths[:4]


def group_results(results: Iterable[FileResult], mode: str = "album") -> List[Group]:
    """mode 'album' or 'artist'.  Tracks inside a group are sorted by disc/track."""
    groups: Dict[str, Group] = {}
    album_sets: Dict[str, set] = {}
    for r in results:
        artist = artist_of(r)
        if mode == "album":
            album = album_of(r)
            key = f"{album.lower()}\x00{artist.lower()}"
            g = groups.get(key) or groups.setdefault(key, Group(key, album, artist))
        else:
            key = artist.lower()
            g = groups.get(key) or groups.setdefault(key, Group(key, artist, ""))
            album_sets.setdefault(key, set()).add(album_of(r).lower())
        g.paths.append(r.path)
        g.total += 1
        if r.verdict in (FAKE, FAKE_LOSSLESS):
            g.fake += 1
        elif r.verdict == SUSPECT:
            g.suspicious += 1
    by_path = {r.path: r for r in results} if not isinstance(results, list) else {r.path: r for r in results}
    for key, g in groups.items():
        g.paths.sort(key=lambda p: _track_no(by_path[p]) if p in by_path else (0, 0, p))
        if mode == "artist":
            g.albums = len(album_sets[key])
            g.subtitle = f"{g.albums} album{'s' if g.albums != 1 else ''}"
    return list(groups.values())


def sort_groups(groups: List[Group], order: str = "Most fake first") -> List[Group]:
    if order == "Name":
        return sorted(groups, key=lambda g: (g.title.lower(), g.subtitle.lower()))
    return sorted(groups, key=lambda g: (-g.problem_pct, -g.problems, g.title.lower()))
